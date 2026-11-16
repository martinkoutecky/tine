//! Quick share, phase 2 (DESIGN §3.2–3.6, §3.9): connect a GitHub account
//! through the device flow, prepare a share, upload exactly the prepared
//! bytes as a secret gist, and keep this device's records. The share format
//! itself (link, envelope, document) is `tine_graph_features::quick_share`
//! and `docs/contracts/quick-share.md`.
//!
//! Secrets: the token and `device_code` never leave Rust; a share's key leaves
//! Rust only inside the link Tine writes to the clipboard. No command result,
//! error or log line carries either (`tests.rs` pins it). Tine never writes the
//! graph for sharing: the records are app data.
//!
//! Gate: every GitHub command requires a build-time attestation from the
//! deployed viewer's headless compatibility check. Debug builds and launch
//! environment variables cannot bypass it.

pub(crate) mod github;
#[cfg(test)]
mod quick_share_tests;
pub(crate) mod records;
mod reshare;

use github::{Endpoints, GhError, GitHub, ListedGist, TokenPoll};
use records::{Account, Records, ShareRow, ShareState};
use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tine_graph_features::publish_query::QueryExportRequest;
use tine_graph_features::quick_share::{self as share, SharePreview};

pub(crate) const GIST_DESCRIPTION: &str = "Tine share (encrypted)";
pub(crate) const SHARE_FILE: &str = "tine-share.bin";
pub(crate) const README_FILE: &str = "README.md";
pub(crate) const README_TEXT: &str =
    "This is an encrypted Tine share. Open the link you were given to read it. https://tine.page\n";
/// A prepared payload is uploadable this long (DESIGN §3.2).
const PREPARED_TTL: Duration = Duration::from_secs(10 * 60);
/// Prepared payloads are held in memory until uploaded, discarded or expired.
/// The bound is their total size: four writer-limit payloads (6 MiB each), so
/// the worst case is unchanged while small shares can be prepared together
/// (Re-share all). The count bound only limits bookkeeping. Overflow refuses
/// the new preparation; it never evicts a reviewed payload.
const MAX_PREPARED_BYTES: usize = 4 * 6 * 1024 * 1024;
const MAX_PREPARED: usize = 64;
/// Listing pages read by Check and Find (100 gists each, DESIGN §3.6).
const MAX_LIST_PAGES: usize = 30;

/// A build can offer links only after check-release-readiness rendered this
/// exact fixture corpus using the deployed generation's verified files.
pub(crate) fn enabled() -> bool {
    viewer_attested(option_env!("TINE_QUICK_SHARE_VIEWER_G1_READY"))
}

fn viewer_attested(proof: Option<&str>) -> bool {
    use sha2::{Digest, Sha256};
    let Some(proof) = proof else {
        return false;
    };
    let fixtures: &[(&str, &[u8])] = &[
        (
            "one-page/document.json",
            include_bytes!("../../../tests/fixtures/quick-share/g1/one-page/document.json"),
        ),
        (
            "one-page/link.txt",
            include_bytes!("../../../tests/fixtures/quick-share/g1/one-page/link.txt"),
        ),
        (
            "one-page/tine-share.bin",
            include_bytes!("../../../tests/fixtures/quick-share/g1/one-page/tine-share.bin"),
        ),
        (
            "one-page/writer.json",
            include_bytes!("../../../tests/fixtures/quick-share/g1/one-page/writer.json"),
        ),
        (
            "pages/document.json",
            include_bytes!("../../../tests/fixtures/quick-share/g1/pages/document.json"),
        ),
        (
            "pages/link.txt",
            include_bytes!("../../../tests/fixtures/quick-share/g1/pages/link.txt"),
        ),
        (
            "pages/tine-share.bin",
            include_bytes!("../../../tests/fixtures/quick-share/g1/pages/tine-share.bin"),
        ),
        (
            "pages/writer.json",
            include_bytes!("../../../tests/fixtures/quick-share/g1/pages/writer.json"),
        ),
    ];
    let mut hash = Sha256::new();
    for (name, bytes) in fixtures {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update(bytes);
    }
    proof == format!("{:x}", hash.finalize())
}

/// A secret string (the OAuth token, a device code): `Debug` and `Display`
/// print `<redacted>`, it has no `Serialize`, and only [`Token::expose`]
/// reads it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Token(String);

impl Token {
    pub(crate) fn new(secret: String) -> Self {
        Self(secret)
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Fixed, secret-free log lines: an operation and an outcome.
fn log(line: impl AsRef<str>) {
    #[cfg(test)]
    quick_share_tests::LOG
        .lock()
        .unwrap()
        .push(line.as_ref().to_owned());
    crate::debug::diag_private("quick-share", line);
}

/// What a command reports when it fails. Carries no secret and no transport
/// error text; the dialog maps `kind` to a sentence and keeps itself open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum QsError {
    /// Quick share is off in this build (phase 3 not deployed).
    Disabled,
    NotConnected,
    /// 401: "Reconnect GitHub".
    Reconnect,
    RateLimited {
        minutes: u64,
    },
    /// Another 403, or another refusal: GitHub's message, sanitized.
    #[serde(rename = "github")]
    GitHub {
        message: String,
    },
    /// Nothing reached GitHub (network, DNS, TLS).
    Network,
    /// GitHub did not answer as documented, or failed (5xx).
    Unavailable,
    /// The upload may have finished: the row stays pending; Check.
    MaybeUploaded {
        #[serde(rename = "shareId")]
        share_id: String,
    },
    MaybeUpdated,
    ShareChanged,
    /// The prepared payload expired, was discarded, or was already used.
    PreviewExpired,
    /// The bounded in-memory pool is full; existing previews remain valid.
    PreviewCapacity,
    /// The share belongs to another account (DESIGN §3.6).
    WrongAccount {
        login: String,
    },
    NotFound,
    /// Prepare refused, or the request is not one Tine performs.
    Refused {
        message: String,
    },
    /// The records could not be read or written.
    Records {
        message: String,
    },
}

impl From<GhError> for QsError {
    fn from(error: GhError) -> Self {
        match error {
            GhError::NotSent => QsError::Network,
            GhError::Unauthorized => QsError::Reconnect,
            GhError::RateLimited { minutes } => QsError::RateLimited { minutes },
            GhError::Forbidden(message) => QsError::GitHub { message },
            GhError::NotFound => QsError::NotFound,
            GhError::Refused(code) => QsError::GitHub {
                message: format!("GitHub refused the request ({code})"),
            },
            GhError::Unknown | GhError::Server(_) | GhError::BadAnswer => QsError::Unavailable,
        }
    }
}

fn records_error(message: String) -> QsError {
    QsError::Records { message }
}

/// `connect_status` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum ConnectStatus {
    Pending,
    Connected {
        login: String,
    },
    Expired,
    Denied,
    /// `reason`: `superseded`, `scope`, `refused`, `records`.
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectStart {
    pub attempt_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

struct Attempt {
    id: String,
    generation: u64,
    status: ConnectStatus,
    /// The waits between polls, in seconds (tests check slow_down).
    #[cfg(test)]
    waits: Vec<u64>,
}

#[derive(Default)]
struct Flow {
    /// Bumped by every start, cancel and disconnect: a poll whose generation
    /// is not current stops, and a token it got is dropped, never stored.
    generation: u64,
    attempt: Option<Attempt>,
}

struct Prepared {
    id: String,
    /// Chosen at prepare (D-20261018-QS3): the share dialog writes
    /// `tine.share:: pending/<share_id>` before it asks for the upload.
    share_id: [u8; 16],
    payload: Vec<u8>,
    title: String,
    pages: usize,
    expires: Instant,
    update: Option<reshare::UpdateReview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrepareResult {
    pub prepare_id: String,
    /// The record id the create will use (not secret: it is in the envelope
    /// header). The webview marks the query block with it before Create.
    pub share_id: String,
    pub preview: SharePreview,
    pub membership: Option<reshare::Membership>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Created {
    pub share_id: String,
    pub gist_id: String,
    pub login: String,
    /// The link was written to the clipboard.
    pub copied: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListedShare {
    pub share_id: String,
    pub state: ShareState,
    pub login: String,
    pub gist_id: Option<String>,
    pub created_at: String,
    pub title: String,
    pub pages: usize,
    /// The connected account owns it (Check and Delete are possible).
    pub account_matches: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectedAccount {
    pub login: String,
    pub account_id: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub enabled: bool,
    /// Development builds only: an unauthenticated, read-only HTTPS diagnostic.
    pub connection_test_available: bool,
    pub account: Option<ConnectedAccount>,
    pub shares: Vec<ListedShare>,
    /// The shares file could not be read (shares is then empty).
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CheckOutcome {
    Live,
    NotFoundYet,
    Current {
        title: String,
        #[serde(rename = "createdAt")]
        created_at: String,
        pages: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeleteOutcome {
    Deleted,
    /// 404 for the matching account: "No longer on GitHub" (offer Forget).
    Gone,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FoundShare {
    pub gist_id: String,
    pub created_at: String,
}

/// The process-wide quick share service: one per app, shared by every window.
pub(crate) struct Service {
    github: GitHub,
    records: Records,
    /// One device-flow "second" (1 s in production; shorter in tests).
    poll_unit: Duration,
    flow: Mutex<Flow>,
    prepared: Mutex<Vec<Prepared>>,
    update_locks:
        Mutex<std::collections::BTreeMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
}

fn random_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The gist marker (DESIGN §1.2): exact description, `README.md` and
/// `tine-share.bin`, every other file an extension file `a<16 hex>.bin`.
pub(crate) fn is_tine_share(gist: &ListedGist) -> bool {
    let extension = |name: &str| {
        name.len() == 21
            && name.starts_with('a')
            && name.ends_with(".bin")
            && name[1..17]
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    gist.description.as_deref() == Some(GIST_DESCRIPTION)
        && gist.files.iter().any(|f| f == README_FILE)
        && gist.files.iter().any(|f| f == SHARE_FILE)
        && gist
            .files
            .iter()
            .all(|f| f == README_FILE || f == SHARE_FILE || extension(f))
}

/// The share id in an envelope's authenticated header, if it is generation 1.
fn envelope_share_id(envelope: &str) -> Option<Vec<u8>> {
    let bytes = share::b64u_decode(envelope)?;
    (bytes.len() >= 21 && &bytes[..5] == b"TSH1\x01").then(|| bytes[5..21].to_vec())
}

fn decode_key(row: &ShareRow) -> Option<[u8; 32]> {
    share::b64u_decode(&row.key)?.try_into().ok()
}

fn parse_time(text: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(text).ok()
}

impl Service {
    pub(crate) fn new(
        endpoints: Endpoints,
        records: Records,
        poll_unit: Duration,
    ) -> Result<Self, QsError> {
        Ok(Self {
            github: GitHub::new(endpoints).map_err(|_| QsError::Network)?,
            records,
            poll_unit,
            flow: Mutex::new(Flow::default()),
            prepared: Mutex::new(Vec::new()),
            update_locks: Mutex::new(std::collections::BTreeMap::new()),
        })
    }

    fn flow(&self) -> std::sync::MutexGuard<'_, Flow> {
        self.flow.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn connected(&self) -> Result<Account, QsError> {
        self.records.account().ok_or(QsError::NotConnected)
    }

    // ---- device flow (DESIGN §3.4) ----

    pub(crate) async fn connect_start(self: &Arc<Self>) -> Result<ConnectStart, QsError> {
        let generation = {
            let mut flow = self.flow();
            flow.generation += 1;
            flow.attempt = None;
            flow.generation
        };
        let code = self.github.device_code().await.map_err(|error| {
            log("device code: refused");
            QsError::from(error)
        })?;
        // The code's lifetime is elapsed time from its arrival, network waits
        // included: nothing polls or stores a token past it.
        let deadline = tokio::time::Instant::now()
            + self
                .poll_unit
                .saturating_mul(u32::try_from(code.expires_in).unwrap_or(u32::MAX));
        let user_code_ok = (1..=32).contains(&code.user_code.len())
            && code
                .user_code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        // Tine opens this address in the browser: only GitHub's own page.
        if !user_code_ok || !code.verification_uri.starts_with("https://github.com/") {
            return Err(QsError::Unavailable);
        }
        let id = random_id();
        {
            let mut flow = self.flow();
            if flow.generation != generation {
                return Err(QsError::Refused {
                    message: "another sign-in started".into(),
                });
            }
            flow.attempt = Some(Attempt {
                id: id.clone(),
                generation,
                status: ConnectStatus::Pending,
                #[cfg(test)]
                waits: Vec::new(),
            });
        }
        log("device code: issued");
        let service = Arc::clone(self);
        let device_code = Token::new(code.device_code);
        let interval = code.interval.max(1);
        tauri::async_runtime::spawn(async move {
            service
                .poll(generation, device_code, interval, deadline)
                .await;
        });
        Ok(ConnectStart {
            attempt_id: id,
            user_code: code.user_code,
            verification_uri: code.verification_uri,
            expires_in: code.expires_in,
        })
    }

    /// Set the attempt's status if `generation` is still current.
    fn settle(&self, generation: u64, status: ConnectStatus) -> bool {
        let mut flow = self.flow();
        let current = flow.generation == generation;
        match flow.attempt.as_mut() {
            Some(attempt) if current && attempt.generation == generation => {
                attempt.status = status;
                true
            }
            _ => false,
        }
    }

    fn current(&self, generation: u64) -> bool {
        self.flow().generation == generation
    }

    /// The code ran out: the attempt ends Expired.
    fn expire(&self, generation: u64) {
        self.settle(generation, ConnectStatus::Expired);
        log("device flow: expired");
    }

    async fn poll(
        &self,
        generation: u64,
        device_code: Token,
        mut interval: u64,
        deadline: tokio::time::Instant,
    ) {
        loop {
            #[cfg(test)]
            {
                let mut flow = self.flow();
                if let Some(attempt) = flow.attempt.as_mut().filter(|a| a.generation == generation)
                {
                    attempt.waits.push(interval);
                }
            }
            let wake = tokio::time::Instant::now() + self.poll_unit.saturating_mul(interval as u32);
            tokio::time::sleep_until(wake.min(deadline)).await;
            if !self.current(generation) {
                log("device flow: stopped (superseded)");
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                self.expire(generation);
                return;
            }
            let answer =
                tokio::time::timeout_at(deadline, self.github.poll_token(device_code.expose()))
                    .await;
            let Ok(answer) = answer else {
                self.expire(generation);
                return;
            };
            let token = match answer {
                Ok(TokenPoll::Pending) => continue,
                Ok(TokenPoll::SlowDown) | Err(GhError::RateLimited { .. }) => {
                    interval += 5;
                    continue;
                }
                // A network blip or a GitHub failure: keep polling until the
                // code expires.
                Err(GhError::NotSent | GhError::Unknown | GhError::Server(_)) => continue,
                Ok(TokenPoll::Expired) => {
                    self.expire(generation);
                    return;
                }
                Ok(TokenPoll::Denied) => {
                    self.settle(generation, ConnectStatus::Denied);
                    log("device flow: denied");
                    return;
                }
                Ok(TokenPoll::Refused) | Err(_) => {
                    self.settle(
                        generation,
                        ConnectStatus::Failed {
                            reason: "refused".into(),
                        },
                    );
                    log("device flow: refused");
                    return;
                }
                Ok(TokenPoll::Token(token)) => token,
            };
            self.finish_connect(generation, token, deadline).await;
            return;
        }
    }

    /// A token arrived: check it, then store it only for a current attempt
    /// whose code has not expired.
    async fn finish_connect(&self, generation: u64, token: Token, deadline: tokio::time::Instant) {
        if !self.current(generation) {
            log("device flow: token for an invalidated attempt dropped");
            return;
        }
        let Ok(user) = tokio::time::timeout_at(deadline, self.github.user(&token)).await else {
            self.expire(generation);
            return;
        };
        let user = match user {
            Ok(user) => user,
            Err(_) => {
                self.settle(
                    generation,
                    ConnectStatus::Failed {
                        reason: "refused".into(),
                    },
                );
                log("device flow: user lookup failed");
                return;
            }
        };
        if !user.scopes.iter().any(|s| s == "gist") {
            self.settle(
                generation,
                ConnectStatus::Failed {
                    reason: "scope".into(),
                },
            );
            log("device flow: gist scope missing");
            return;
        }
        let account = Account {
            login: user.login.clone(),
            account_id: user.id,
            token,
            scopes: user.scopes,
            connected_at: now_rfc3339(),
        };
        // The flow lock orders this against cancel and disconnect: either the
        // attempt is still current and the account is stored, or it is not
        // and the token is dropped here.
        let mut flow = self.flow();
        if tokio::time::Instant::now() >= deadline {
            drop(flow);
            self.expire(generation);
            return;
        }
        let current = flow.generation == generation
            && flow
                .attempt
                .as_ref()
                .is_some_and(|a| a.generation == generation);
        if !current {
            log("device flow: token for an invalidated attempt dropped");
            return;
        }
        let stored = {
            let _records = records::lock();
            self.records.store_account(&account)
        };
        let status = match stored {
            Ok(()) => ConnectStatus::Connected { login: user.login },
            Err(_) => ConnectStatus::Failed {
                reason: "records".into(),
            },
        };
        log(match status {
            ConnectStatus::Connected { .. } => "device flow: connected",
            _ => "device flow: account file not written",
        });
        if let Some(attempt) = flow.attempt.as_mut() {
            attempt.status = status;
        }
    }

    pub(crate) fn connect_status(&self, attempt_id: &str) -> ConnectStatus {
        let flow = self.flow();
        match flow.attempt.as_ref() {
            Some(attempt) if attempt.id == attempt_id && attempt.generation == flow.generation => {
                attempt.status.clone()
            }
            _ => ConnectStatus::Failed {
                reason: "superseded".into(),
            },
        }
    }

    /// Cancel this attempt (a newer one started elsewhere is left alone).
    pub(crate) fn connect_cancel(&self, attempt_id: &str) {
        let mut flow = self.flow();
        if flow.attempt.as_ref().is_some_and(|a| a.id == attempt_id) {
            flow.generation += 1;
            flow.attempt = None;
            log("device flow: cancelled");
        }
    }

    /// Forget the account here and invalidate any sign-in in progress. The
    /// grant itself is revoked on GitHub's Applications page (DESIGN §3.9).
    pub(crate) fn disconnect(&self) -> Result<(), QsError> {
        let mut flow = self.flow();
        flow.generation += 1;
        flow.attempt = None;
        let _records = records::lock();
        self.records
            .remove_account()
            .map_err(|_| records_error("Tine could not remove the GitHub account file".into()))?;
        log("disconnected");
        Ok(())
    }

    // ---- prepare / create (DESIGN §3.2, §3.5) ----

    pub(crate) fn prepare_share(
        &self,
        store: &tine_store::Store,
        request: &QueryExportRequest,
        scratch: &std::path::Path,
    ) -> Result<PrepareResult, QsError> {
        let prepared =
            share::prepare(store, request, &now_rfc3339(), scratch).map_err(|error| {
                QsError::Refused {
                    message: error.to_string(),
                }
            })?;
        let id = random_id();
        let share_id = share::fresh_share_id().map_err(|_| QsError::Refused {
            message: "the system random source failed".into(),
        })?;
        let mut all = self.prepared.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        all.retain(|p| p.expires > now);
        let held: usize = all.iter().map(|p| p.payload.len()).sum();
        if all.len() >= MAX_PREPARED || held + prepared.payload.len() > MAX_PREPARED_BYTES {
            return Err(QsError::PreviewCapacity);
        }
        all.push(Prepared {
            id: id.clone(),
            share_id,
            payload: prepared.payload,
            title: prepared.preview.title.clone(),
            pages: prepared.preview.pages.len(),
            expires: now + PREPARED_TTL,
            update: None,
        });
        Ok(PrepareResult {
            prepare_id: id,
            share_id: share::b64u(&share_id),
            preview: prepared.preview,
            membership: None,
        })
    }

    /// Take a prepared payload out, so it can be used at most once.
    fn take_prepared(&self, prepare_id: &str) -> Option<Prepared> {
        let mut all = self.prepared.lock().unwrap_or_else(|p| p.into_inner());
        let index = all.iter().position(|p| p.id == prepare_id)?;
        let prepared = all.remove(index);
        (prepared.expires > Instant::now()).then_some(prepared)
    }

    pub(crate) fn discard_prepared(&self, prepare_id: &str) {
        drop(self.take_prepared(prepare_id));
    }

    /// Encrypt and upload exactly the prepared bytes, once. `copy` receives
    /// the link (the clipboard in the app).
    pub(crate) async fn create_share(
        &self,
        prepare_id: &str,
        copy: impl FnOnce(&str) -> bool,
    ) -> Result<Created, QsError> {
        // Consumed before anything else: a second call with this id fails.
        let prepared = self
            .take_prepared(prepare_id)
            .ok_or(QsError::PreviewExpired)?;
        if prepared.update.is_some() {
            return self.update_share(prepared, copy).await;
        }
        let account = self.connected()?;
        let sealed = share::seal_fresh_as(&prepared.payload, prepared.share_id).map_err(|_| {
            QsError::Refused {
                message: "the system random source failed".into(),
            }
        })?;
        let share_id = share::b64u(&sealed.share_id);
        let row = ShareRow {
            share_id: share_id.clone(),
            state: ShareState::Pending,
            account_id: account.account_id,
            login: account.login.clone(),
            gist_id: None,
            created_at: now_rfc3339(),
            title: prepared.title.clone(),
            pages: prepared.pages,
            key: share::b64u(&sealed.key),
        };
        // The pending row (with key and share id) is on disk before the
        // request: a crash or an unknown outcome can still be recovered.
        self.records
            .update_shares(|rows| rows.push(row.clone()))
            .map_err(records_error)?;
        let files = [
            (README_FILE, README_TEXT),
            (SHARE_FILE, sealed.envelope.as_str()),
        ];
        let outcome = self.github.create_gist(&account.token, &files).await;
        let remove_row = |rows: &mut Vec<ShareRow>| rows.retain(|r| r.share_id != share_id);
        match outcome {
            Ok(gist_id) => {
                log("gist create: 201");
                self.records
                    .update_shares(|rows| {
                        if let Some(r) = rows.iter_mut().find(|r| r.share_id == share_id) {
                            r.state = ShareState::Live;
                            r.gist_id = Some(gist_id.clone());
                        }
                    })
                    .map_err(|_| QsError::MaybeUploaded {
                        share_id: share_id.clone(),
                    })?;
                let copied = copy(&share::link(&account.login, &gist_id, &sealed.key));
                Ok(Created {
                    share_id,
                    gist_id,
                    login: account.login,
                    copied,
                })
            }
            // Known not to have created a gist: nothing was sent, or GitHub
            // answered with a refusal.
            Err(
                error @ (GhError::NotSent
                | GhError::Unauthorized
                | GhError::RateLimited { .. }
                | GhError::Forbidden(_)
                | GhError::NotFound
                | GhError::Refused(_)),
            ) => {
                log("gist create: not created");
                self.records
                    .update_shares(remove_row)
                    .map_err(records_error)?;
                Err(error.into())
            }
            // Timeout after sending, dropped connection, 5xx, an unreadable
            // 201: the gist may exist. Never retried; the row stays pending.
            Err(GhError::Unknown | GhError::Server(_) | GhError::BadAnswer) => {
                log("gist create: outcome unknown");
                Err(QsError::MaybeUploaded { share_id })
            }
        }
    }

    // ---- records, check, find, delete, forget (DESIGN §3.6) ----

    pub(crate) fn listing(&self) -> Listing {
        let account = self.records.account();
        let (shares, error) = match self.records.shares() {
            Ok(rows) => (rows, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let shares = shares
            .into_iter()
            .rev()
            .map(|row| ListedShare {
                account_matches: account
                    .as_ref()
                    .is_some_and(|a| a.account_id == row.account_id),
                share_id: row.share_id,
                state: row.state,
                login: row.login,
                gist_id: row.gist_id,
                created_at: row.created_at,
                title: row.title,
                pages: row.pages,
            })
            .collect();
        Listing {
            enabled: enabled(),
            connection_test_available: cfg!(debug_assertions),
            account: account.map(|a| ConnectedAccount {
                login: a.login,
                account_id: a.account_id,
            }),
            shares,
            error,
        }
    }

    fn row(&self, share_id: &str) -> Result<ShareRow, QsError> {
        self.records
            .shares()
            .map_err(records_error)?
            .into_iter()
            .find(|r| r.share_id == share_id)
            .ok_or(QsError::NotFound)
    }

    fn owned_row(&self, share_id: &str) -> Result<(Account, ShareRow), QsError> {
        let account = self.connected()?;
        let row = self.row(share_id)?;
        if row.account_id != account.account_id {
            return Err(QsError::WrongAccount { login: row.login });
        }
        Ok((account, row))
    }

    pub(crate) async fn check_share(&self, share_id: &str) -> Result<CheckOutcome, QsError> {
        let (account, row) = self.owned_row(share_id)?;
        if row.state == ShareState::Live {
            // Check also resolves an unknown PATCH: authenticate the current
            // snapshot rather than treating a local live row as proof.
            let lock = self.update_lock(share_id);
            let _guard = lock.lock().await;
            let (_, document) = self.read_update_snapshot(&account, &row).await?;
            self.refresh_update_record(&row, &document)?;
            return Ok(CheckOutcome::Current {
                title: document["title"].as_str().unwrap_or(&row.title).to_owned(),
                created_at: document["created_at"].as_str().unwrap_or("").to_owned(),
                pages: reshare::snapshot_pages(&document).into_values().collect(),
            });
        }
        let key =
            decode_key(&row).ok_or_else(|| records_error("a share's key is damaged".into()))?;
        let wanted = share::b64u_decode(&row.share_id)
            .ok_or_else(|| records_error("a share's id is damaged".into()))?;
        let cutoff = parse_time(&row.created_at).map(|t| t - chrono::Duration::days(1));
        let mut url: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let (gists, next) = self
                .github
                .list_gists(&account.token, url.as_deref())
                .await?;
            for gist in gists {
                if let (Some(cutoff), Some(created)) = (cutoff, parse_time(&gist.created_at)) {
                    if created < cutoff {
                        log("check: not found yet");
                        return Ok(CheckOutcome::NotFoundYet);
                    }
                }
                let Some(raw_url) = gist
                    .share_raw_url
                    .as_deref()
                    .filter(|_| is_tine_share(&gist))
                else {
                    continue;
                };
                let envelope = match self.github.raw_envelope(raw_url).await {
                    Ok(text) => text,
                    Err(GhError::NotFound) => continue,
                    Err(error) => return Err(error.into()),
                };
                let envelope = envelope.strip_suffix('\n').unwrap_or(&envelope);
                if envelope_share_id(envelope).as_deref() != Some(wanted.as_slice()) {
                    continue;
                }
                // The share id is not secret: only the key confirms the gist.
                if share::open(envelope, &key).is_err() {
                    continue;
                }
                let gist_id = gist.id.clone();
                self.records
                    .update_shares(|rows| {
                        if let Some(r) = rows.iter_mut().find(|r| r.share_id == row.share_id) {
                            r.state = ShareState::Live;
                            r.gist_id = Some(gist_id.clone());
                        }
                    })
                    .map_err(records_error)?;
                log("check: confirmed");
                return Ok(CheckOutcome::Live);
            }
            match next {
                Some(next) => url = Some(next),
                None => break,
            }
        }
        log("check: not found yet");
        Ok(CheckOutcome::NotFoundYet)
    }

    pub(crate) async fn find_shares(&self) -> Result<Vec<FoundShare>, QsError> {
        let account = self.connected()?;
        let known: std::collections::BTreeSet<String> = self
            .records
            .shares()
            .map_err(records_error)?
            .into_iter()
            .filter_map(|r| r.gist_id)
            .collect();
        let mut found = Vec::new();
        let mut url: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let (gists, next) = self
                .github
                .list_gists(&account.token, url.as_deref())
                .await?;
            found.extend(
                gists
                    .into_iter()
                    .filter(|g| is_tine_share(g) && !known.contains(&g.id))
                    .map(|g| FoundShare {
                        gist_id: g.id,
                        created_at: g.created_at,
                    }),
            );
            match next {
                Some(next) => url = Some(next),
                None => break,
            }
        }
        log(format!("find: {} unknown shares", found.len()));
        Ok(found)
    }

    /// Delete this device's live share, or a share Find listed (by gist id).
    pub(crate) async fn delete(
        &self,
        share_id: Option<&str>,
        gist_id: Option<&str>,
    ) -> Result<DeleteOutcome, QsError> {
        let (account, gist_id, row) = match (share_id, gist_id) {
            (Some(share_id), _) => {
                let (account, row) = self.owned_row(share_id)?;
                let gist_id = row.gist_id.clone().ok_or_else(|| QsError::Refused {
                    message: "this share is not confirmed yet; Check it first".into(),
                })?;
                (account, gist_id, Some(row.share_id))
            }
            (None, Some(gist_id)) => {
                let account = self.connected()?;
                // Tine only touches the shares it made: the gist must carry
                // the marker and belong to the connected account.
                let gist = match self.github.gist(&account.token, gist_id).await {
                    Ok(gist) => gist,
                    Err(GhError::NotFound) => return Ok(DeleteOutcome::Gone),
                    Err(error) => return Err(error.into()),
                };
                if !is_tine_share(&gist) || gist.owner_id != Some(account.account_id) {
                    return Err(QsError::Refused {
                        message: "this gist is not a Tine share of the connected account".into(),
                    });
                }
                (account, gist.id, None)
            }
            (None, None) => return Err(QsError::NotFound),
        };
        match self.github.delete_gist(&account.token, &gist_id).await {
            Ok(()) => {
                log("delete: 204");
                if let Some(share_id) = row {
                    self.records
                        .update_shares(|rows| rows.retain(|r| r.share_id != share_id))
                        .map_err(records_error)?;
                }
                Ok(DeleteOutcome::Deleted)
            }
            Err(GhError::NotFound) => {
                log("delete: 404");
                Ok(DeleteOutcome::Gone)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Remove a row and its key. The link, if the gist exists, can no longer
    /// be recovered here.
    pub(crate) fn forget(&self, share_id: &str) -> Result<(), QsError> {
        self.records
            .update_shares(|rows| rows.retain(|r| r.share_id != share_id))
            .map_err(records_error)?;
        log("forget");
        Ok(())
    }

    /// Write a live share's link to the clipboard via `copy`.
    pub(crate) fn copy_link(
        &self,
        share_id: &str,
        copy: impl FnOnce(&str) -> bool,
    ) -> Result<bool, QsError> {
        let row = self.row(share_id)?;
        let (Some(gist_id), Some(key)) = (row.gist_id.as_deref(), decode_key(&row)) else {
            return Err(QsError::NotFound);
        };
        Ok(copy(&share::link(&row.login, gist_id, &key)))
    }
}

// ---- commands (DESIGN §3.9; not plugin capabilities, ADR 0052) ----

static SERVICE: OnceLock<Arc<Service>> = OnceLock::new();

fn service(app: &tauri::AppHandle) -> Result<Arc<Service>, QsError> {
    use tauri::Manager;
    if let Some(service) = SERVICE.get() {
        return Ok(Arc::clone(service));
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| records_error("Tine has no app data folder".into()))?;
    let service = Arc::new(Service::new(
        Endpoints::github(),
        Records::new(dir),
        Duration::from_secs(1),
    )?);
    Ok(Arc::clone(SERVICE.get_or_init(|| service)))
}

fn gate() -> Result<(), QsError> {
    if enabled() {
        Ok(())
    } else {
        Err(QsError::Disabled)
    }
}

fn clipboard(app: &tauri::AppHandle) -> impl FnOnce(&str) -> bool + '_ {
    move |link| {
        use tauri_plugin_clipboard_manager::ClipboardExt;
        app.clipboard().write_text(link.to_owned()).is_ok()
    }
}

#[tauri::command]
pub(crate) async fn quick_share_prepare(
    request: QueryExportRequest,
    share_id: Option<String>,
    state: crate::state::GraphContext<'_>,
) -> Result<PrepareResult, QsError> {
    use tauri::Manager;
    gate()?;
    let app = state.window.app_handle().clone();
    let slot =
        crate::state::slot_for_context(&state).map_err(|message| QsError::Refused { message })?;
    let service = service(&app)?;
    let scratch = app
        .path()
        .app_cache_dir()
        .map_err(|_| records_error("Tine has no cache folder".into()))?;
    let builder = Arc::clone(&service);
    let prepared = tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(&scratch)
            .map_err(|_| records_error("Tine could not create its cache folder".into()))?;
        builder.prepare_share(&slot.store, &request, &scratch)
    })
    .await
    .map_err(|_| QsError::Unavailable)??;
    match share_id {
        Some(id) => service.review_update(prepared, &id).await,
        None => Ok(prepared),
    }
}

#[tauri::command]
pub(crate) async fn quick_share_create(
    prepare_id: String,
    app: tauri::AppHandle,
) -> Result<Created, QsError> {
    let service = service(&app)?;
    if !enabled() {
        service.discard_prepared(&prepare_id);
        return Err(QsError::Disabled);
    }
    service.create_share(&prepare_id, clipboard(&app)).await
}

#[tauri::command]
pub(crate) fn quick_share_discard(
    prepare_id: String,
    app: tauri::AppHandle,
) -> Result<(), QsError> {
    service(&app)?.discard_prepared(&prepare_id);
    Ok(())
}

#[tauri::command]
pub(crate) async fn quick_share_list(app: tauri::AppHandle) -> Result<Listing, QsError> {
    let service = service(&app)?;
    tauri::async_runtime::spawn_blocking(move || service.listing())
        .await
        .map_err(|_| QsError::Unavailable)
}

/// No service, account, token, graph or caller-supplied URL is involved. Release
/// builds cannot make this diagnostic request even through direct IPC.
#[tauri::command]
pub(crate) async fn quick_share_test_connection() -> github::ConnectionTestResult {
    #[cfg(debug_assertions)]
    {
        match GitHub::new(Endpoints::github_https()) {
            Ok(github) => github.test_connection().await,
            Err(_) => github::ConnectionTestResult::Network,
        }
    }
    #[cfg(not(debug_assertions))]
    {
        github::ConnectionTestResult::Disabled
    }
}

#[tauri::command]
pub(crate) async fn quick_share_check(
    share_id: String,
    app: tauri::AppHandle,
) -> Result<CheckOutcome, QsError> {
    gate()?;
    service(&app)?.check_share(&share_id).await
}

#[tauri::command]
pub(crate) async fn quick_share_find(app: tauri::AppHandle) -> Result<Vec<FoundShare>, QsError> {
    gate()?;
    service(&app)?.find_shares().await
}

#[tauri::command]
pub(crate) async fn quick_share_delete(
    share_id: Option<String>,
    gist_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<DeleteOutcome, QsError> {
    gate()?;
    service(&app)?
        .delete(share_id.as_deref(), gist_id.as_deref())
        .await
}

#[tauri::command]
pub(crate) async fn quick_share_forget(
    share_id: String,
    app: tauri::AppHandle,
) -> Result<(), QsError> {
    let service = service(&app)?;
    tauri::async_runtime::spawn_blocking(move || service.forget(&share_id))
        .await
        .map_err(|_| QsError::Unavailable)?
}

#[tauri::command]
pub(crate) fn quick_share_copy_link(
    share_id: String,
    app: tauri::AppHandle,
) -> Result<bool, QsError> {
    service(&app)?.copy_link(&share_id, clipboard(&app))
}

#[tauri::command]
pub(crate) async fn quick_share_connect_start(
    app: tauri::AppHandle,
) -> Result<ConnectStart, QsError> {
    gate()?;
    service(&app)?.connect_start().await
}

#[tauri::command]
pub(crate) fn quick_share_connect_status(
    attempt_id: String,
    app: tauri::AppHandle,
) -> Result<ConnectStatus, QsError> {
    Ok(service(&app)?.connect_status(&attempt_id))
}

#[tauri::command]
pub(crate) fn quick_share_connect_cancel(
    attempt_id: String,
    app: tauri::AppHandle,
) -> Result<(), QsError> {
    service(&app)?.connect_cancel(&attempt_id);
    Ok(())
}

#[tauri::command]
pub(crate) async fn quick_share_disconnect(app: tauri::AppHandle) -> Result<(), QsError> {
    let service = service(&app)?;
    tauri::async_runtime::spawn_blocking(move || service.disconnect())
        .await
        .map_err(|_| QsError::Unavailable)?
}
