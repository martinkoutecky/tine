//! One page host per graph binding (STEP3 §1–§3): the driver thread, the
//! command path that serializes a window's page DTO against the right
//! comparison source and admits it, and the page-mail bridge. No production
//! path starts one yet (step 3b P1 is inert; P2b starts it at graph open);
//! until then the app saves through the old engine.
use super::driver::{Driver, Owner, Sink, SystemClock};
use super::production::ProductionIo;
use super::progress::{backoff, Clock, Notice, Progress, Stopping};
use super::*;
use crate::model::{bytes_hold_block_id, content_rev};
use crate::store::PublishedObservations;
use crate::{ChangeKind, EditKind, FileId, FileRev, Origin, PageId, Store, Why};
use std::path::Path;
use tine_core::doc::Document;
use tine_core::model::PageDto;

/// The graph binding's page host. Every command carries the window
/// generation and a strictly increasing request id (§3.1).
pub struct PageHost {
    driver: Driver<ProductionIo, SystemClock>,
    store: Arc<Store>,
    /// What a relaunch after a restore binds again (§7 step 6).
    launch: Launch,
    /// Index publications to fail before the next success (tests).
    #[cfg(test)]
    index_faults: Arc<std::sync::atomic::AtomicU32>,
    /// Each publication's edit kinds as delivered (tests, Rule 8).
    #[cfg(test)]
    published_kinds: PublishedKinds,
}

/// Why a command did not admit its request. Nothing was sent: the window
/// keeps its text and shows the reason (§3.2, B′).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum PageRefusal {
    /// Not a page path, unsafe nesting, or a virtual Guide page.
    InvalidTarget {
        /// What was refused.
        message: String,
    },
    /// The file cannot be rewritten safely (an Org file that does not
    /// round-trip, unresolved VCS markers, a read-only file).
    ReadOnly {
        /// Why.
        message: String,
    },
    /// Another file already claims the page's name (`.md` ↔ `.org`).
    Twin {
        /// The claiming file.
        existing: String,
    },
    /// The page's bytes are not decodable.
    Undecodable,
    /// An unreadable file owns the name.
    UnreadableOwner {
        /// The owning file.
        file: String,
    },
    /// A content firewall or read failed (the GH #163 page-header check,
    /// parse validation, an I/O error).
    Failed {
        /// What failed.
        message: String,
    },
    /// The host did not admit the request: admission closed (a switch), a
    /// stale session, an id out of order, or a stopped host.
    NotAdmitted,
}

/// A window session (plan v3 §5, R8): process-unique, drawn at every host
/// launch and every window reload. Commands carry it and mail names it, so
/// nothing from an earlier host or window load is ever applied: versions
/// from different sessions are never compared (E101).
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_session() -> u64 {
    NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `page_window_reloaded`'s reply: the window's new session, and the first
/// request id the host can admit (ids never restart below its watermark).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reloaded {
    /// The session every later command carries.
    pub session: u64,
    /// One past the host's last admitted request id.
    pub next_id: u64,
}

/// `page_open`'s reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    /// The host's key for the page (its entry identity, A-H2).
    pub key: String,
    /// The opened path names the key's entry under the shared entry-identity
    /// rule (B1): the window's text, read from that path, is the key's. False
    /// when the path only collides with a registered entry unproved (the
    /// window then sends its text as stale input, D-b, S5).
    pub baseline_entry: bool,
}

impl From<Why> for PageRefusal {
    fn from(why: Why) -> Self {
        use crate::Refusal as R;
        match why {
            Why::Refused(R::ReadOnly(message)) => Self::ReadOnly { message },
            Why::Refused(R::Twin { existing }) => Self::Twin {
                existing: existing.as_str().into(),
            },
            Why::Refused(R::Undecodable) => Self::Undecodable,
            Why::Refused(R::UnreadableOwner { file }) => Self::UnreadableOwner {
                file: file.as_str().into(),
            },
            Why::Refused(R::InvalidTarget(message)) => Self::InvalidTarget { message },
            Why::Refused(other) => Self::InvalidTarget {
                message: format!("{other:?}"),
            },
            Why::Failed(error) => Self::Failed {
                message: error.to_string(),
            },
            Why::Conflict { file, .. } => Self::Failed {
                message: format!("{} changed on disk", file.as_str()),
            },
        }
    }
}

/// A retained writer's contract for unsaved input on the pages it touches
/// (§7 step 1). Each writer keeps today's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Flush then write: input found under the reservation is saved first,
    /// with the reservation released meanwhile, and reserving starts over.
    /// A page whose save cannot complete (conflict, persistent save error)
    /// is returned, as today's stuck pages are.
    Flush,
    /// Refuse a touched page with unsaved input (today's message).
    Refuse,
}

/// How the host stops (§6, §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    /// A graph switch, quit or last-window close: a page that cannot be
    /// saved is drafted.
    Switch,
    /// A backup restore: no fallback draft; every drained edit is saved and
    /// published, and every draft and custody debt retired, first.
    Restore,
}

/// Where a stop stands (§6 steps 4–5, §7 step 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopState {
    /// Saves, drafts, cleanup or publications are still running, or no
    /// stop has begun.
    Waiting,
    /// `stop_finish` stops the host now.
    Ready,
    /// The stop cannot complete without losing custody: a draft failure
    /// (both modes), or in a restore a failed save, a conflict, a custody
    /// or index failure. These pages are affected (none: trash custody
    /// cannot be listed). The caller aborts (`stop_abort`), unfreezes and
    /// shows them; nothing closes over missing custody.
    Aborted(BTreeSet<PageKey>),
}

/// The disk state a window was shown, named by revision: a Keep-mine
/// submit's resolve token (§3.2, Q1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "rev", rename_all = "kebab-case")]
pub enum DiskToken {
    /// The page had no file.
    NoFile,
    /// The file held bytes of this revision.
    File(FileRev),
}

impl DiskToken {
    fn of(bytes: &Text) -> Self {
        match bytes {
            None => Self::NoFile,
            Some(bytes) => Self::File(FileRev::from_bytes(bytes)),
        }
    }
}

/// A page's text in a mail.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MailText {
    /// Exactly the state the window submitted (R3), or a notice-only mail.
    /// `rev` names the buffer's bytes (none for no file), so the window's
    /// baseline advances with the text the host took (S5).
    Unchanged {
        /// The buffer's revision.
        rev: Option<FileRev>,
    },
    /// The page has no file.
    NoFile,
    /// The page's bytes, parsed.
    Page {
        /// The parsed page.
        dto: Box<PageDto>,
    },
    /// Bytes the parser cannot take (they stay the host's buffer).
    Unreadable {
        /// The parse error.
        message: String,
    },
}

/// The host's state of one page, as mailed.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailPage {
    /// The host version of the buffer.
    pub version: u64,
    /// The buffer conflicts with the file on disk.
    pub conflict: bool,
    /// The buffer is not known durable on disk.
    pub risk: bool,
    /// The last observed disk state: what Keep mine resolves against.
    pub disk: Option<DiskToken>,
    /// The buffer's text.
    pub text: MailText,
}

/// A request's typed outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AnswerOutcome {
    /// The host applied it (Open and Discard apply without taking input).
    Applied,
    /// The host refused it; the window keeps its text.
    Refused {
        /// Why.
        reason: Refusal,
    },
}

/// The answer to the window's latest request on a page.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAnswer {
    /// The request id answered.
    pub id: u64,
    /// The page version the answer leaves.
    pub version: u64,
    /// The request's input became the buffer.
    pub took: bool,
    /// Applied or refused, with the reason.
    pub outcome: AnswerOutcome,
}

/// A page's save and draft status for the window's indicators.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailNotice {
    /// Consecutive failed saves.
    pub failures: u32,
    /// The latest save failed.
    pub save_error: bool,
    /// The latest draft write failed.
    pub draft_error: bool,
    /// The conflict has been reported to the window.
    pub conflict_reported: bool,
    /// A trash payload's custody could not be settled.
    pub custody_error: bool,
    /// The page's index publication failed a third time; it keeps retrying
    /// (§5). Search and references may lag the file meanwhile.
    pub index_error: bool,
    /// The page's file could not be read three times running after a
    /// watcher change or a retained writer's release; the driver keeps
    /// retrying, and the notice clears once a read succeeds (§5, §7).
    pub observe_error: bool,
    /// Another page file claims this page's name (the alternate-extension
    /// twin, §3.2, Q9): it failed the page's creation, or appeared beside
    /// it. Cleared once a later save publishes.
    pub twin: Option<String>,
}

/// `page-mail` (§3.3). The window checks `session` before applying anything.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMail {
    /// The window session it is for (0: none, never current).
    pub session: u64,
    /// The page key.
    pub key: String,
    /// The page's state, None once the host released it.
    pub page: Option<MailPage>,
    /// The answer to the window's latest request, if any.
    pub answer: Option<MailAnswer>,
    /// Save and draft status.
    pub notice: MailNotice,
}

/// A host page operation's disposition (delete, rename).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageOperation {
    /// Done.
    Applied,
    /// Under way; its result arrives as mail.
    Pending,
    /// The page is busy; retry once it is clean.
    Waiting,
    /// Refused (unsaved input, a stopped host).
    Refused,
}

/// Derived data a request carries (R8): its serialized bytes, the Document
/// they were serialized from (runtime block ids) and its edit kinds.
struct Handoff {
    bytes: Text,
    document: Document,
    kinds: Vec<EditKind>,
}

/// What the bridge needs beside a mail, taken under the state mutex.
pub(super) struct MailFacts {
    pub notice: Notice,
    /// False when the mail's page is exactly the state the window submitted
    /// (R3), or for a notice-only mail: the text is not sent again.
    pub content: bool,
    pub refused: Option<Refusal>,
    /// The key's current spelling, to parse its bytes as that file.
    pub spelling: String,
    /// The binding's own notices (§5, Q9).
    pub binding: BindingNotice,
    /// The window session current when the mail was collected.
    pub session: u64,
}

/// The page notices the binding keeps beside the host's (A2, REVIEW-3a).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BindingNotice {
    /// The page's index publication failed a third time (§5).
    pub index_error: bool,
    pub observe_error: bool,
    pub twin: Option<String>,
}

/// An index publication the consumer owes (§5): a held key's disk state,
/// from the host's events in host order.
#[derive(Clone)]
pub(super) struct Publication {
    pub key: PageKey,
    /// The key's spelling when the event was taken.
    pub spelling: String,
    pub bytes: Text,
    /// An own save's host version (the index watermark it reaches, §4.4)
    /// and the Document its bytes were serialized from (R8), if it matched.
    pub own: Option<(u64, Option<Document>)>,
}

/// What became of one publication.
pub(super) enum Indexing {
    Indexed,
    Failed,
    /// A reservation holds the key: its retained transaction publishes
    /// meanwhile, and this waits for the release (Q6).
    Reserved,
}

/// A publication awaiting retry. A newer publication of its key, or the
/// observation that ends a reservation, supersedes it.
struct Retry {
    publication: Publication,
    failures: u32,
    due: u64,
}

/// One driver step's results, in host order.
#[derive(Default)]
pub(super) struct Delivery {
    pub events: Vec<Event>,
    /// Keys the host took on without an Open (launch-recovered drafts, an
    /// operation): their index moves to the consumer before anything is
    /// published (§5). (key, spelling).
    pub claimed: Vec<(PageKey, String)>,
    /// Index publications, in host order (§5).
    pub publications: Vec<Publication>,
    /// Edit kinds taken since each published page's last publication.
    pub kinds: Vec<(PageKey, Vec<EditKind>)>,
    /// Keys the host evicted with no publication left: their index returns
    /// to the watcher (§5, Q6). (key, spelling).
    pub evicted: Vec<(PageKey, String)>,
    pub mail: Vec<(PageKey, Mail, MailFacts)>,
}

impl Delivery {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
            && self.mail.is_empty()
            && self.publications.is_empty()
            && self.claimed.is_empty()
            && self.evicted.is_empty()
    }
}

/// Binding bookkeeping beside the host, under the host state mutex. None of
/// it is page authority: losing it costs a parse, never a byte.
#[derive(Default)]
pub(super) struct Book {
    /// Admitted requests' handoffs, by (page, request id), until answered.
    pending: BTreeMap<(PageKey, u64), Handoff>,
    /// Applied versions' handoffs, kept only while the page's current buffer
    /// or its save job is that version (Q8): a conflict or failing-save loop
    /// holds the live entries, never the edit history.
    versions: BTreeMap<PageKey, BTreeMap<u64, Handoff>>,
    /// Edit kinds taken since the page's last publication (OG-RULES Rule 8).
    kinds: BTreeMap<PageKey, Vec<EditKind>>,
    /// The notice each subscribed page's window last received.
    notices: BTreeMap<PageKey, (Notice, BindingNotice)>,
    /// Keys whose index the publication consumer owns (§5): from Open (or
    /// the host taking the key on) until actual eviction (Q6).
    pub owned: BTreeSet<PageKey>,
    /// Each owned key's last indexed bytes and index watermark: the newest
    /// own save version the index has applied (§4.4).
    index: BTreeMap<PageKey, (Text, u64)>,
    retry: BTreeMap<PageKey, Retry>,
    /// Keys whose file read failed a third time, until a read succeeds.
    pub observe_errors: BTreeSet<PageKey>,
    /// Each key's last twin: (existing file, save version, after the rename).
    twins: BTreeMap<PageKey, (String, u64, bool)>,
    /// Released keys whose release observation has not yet succeeded
    /// (V2, REVIEW-3a): the retained transaction's index stands until the
    /// host has read its bytes, so older publications keep waiting.
    pub handover: BTreeSet<PageKey>,
    /// A collected delivery's publications are with the sink and their
    /// results not yet recorded (V5, REVIEW-3a).
    delivering: bool,
    /// The current window session: the host's generation under a
    /// process-unique name (E97). 0 until launch draws one.
    pub session: u64,
}

impl Book {
    /// Record `kind` for each written page until its next publication
    /// (OG-RULES Rule 8): a submit's kinds, or a host operation's own.
    fn took(&mut self, pages: impl IntoIterator<Item = PageKey>, kind: EditKind) {
        for page in pages {
            let kinds = self.kinds.entry(page).or_default();
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }

    /// Take this step's results: events (drained once progress has read
    /// them), handoffs routed to answers and publications, and every mail.
    pub fn collect<F: HostIo, C: Clock>(&mut self, progress: &mut Progress<F, C>) -> Delivery {
        progress.host.held = Some(BTreeSet::new());
        let now = progress.clock.now_ms();
        let events = progress.take_events();
        let mut delivery = Delivery::default();
        let host = &progress.host;
        let taken = host
            .pages
            .keys()
            .chain(host.job.as_ref().map(|job| &job.page))
            .filter(|key| !self.owned.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in taken {
            self.owned.insert(key.clone());
            delivery.claimed.push((key.clone(), host.fs.spelling(&key)));
        }
        let mut answered = BTreeMap::new();
        let mut refused = BTreeMap::new();
        for event in &events {
            match event {
                Event::Answer { page, answer } => {
                    let Some(handoff) = self.pending.remove(&(page.clone(), answer.id)) else {
                        continue;
                    };
                    if answer.took {
                        answered.insert((page.clone(), answer.id), handoff.bytes.clone());
                        for kind in &handoff.kinds {
                            self.took([page.clone()], *kind);
                        }
                        self.versions
                            .entry(page.clone())
                            .or_default()
                            .insert(answer.version, handoff);
                    }
                }
                Event::Published {
                    page,
                    bytes,
                    version,
                    ..
                } => {
                    if let Some(kinds) = self.kinds.remove(page) {
                        delivery.kinds.push((page.clone(), kinds));
                    }
                    // A twin that failed a save clears once one publishes; a
                    // twin found after a rename stays past that save's own
                    // publication (Q9: it neither invents nor cancels it).
                    if self.twins.get(page).is_some_and(|(_, at, saved)| {
                        *version > *at || (!*saved && *version == *at)
                    }) {
                        self.twins.remove(page);
                    }
                    let handoff = self.versions.get_mut(page).and_then(|v| v.remove(version));
                    let document = handoff.filter(|h| h.bytes == *bytes).map(|h| h.document);
                    self.retry.remove(page);
                    delivery.publications.push(Publication {
                        key: page.clone(),
                        spelling: progress.host.fs.spelling(page),
                        bytes: bytes.clone(),
                        own: Some((*version, document)),
                    });
                }
                Event::Observed { page, bytes } => {
                    self.retry.remove(page);
                    delivery.publications.push(Publication {
                        key: page.clone(),
                        spelling: progress.host.fs.spelling(page),
                        bytes: bytes.clone(),
                        own: None,
                    });
                }
                Event::Refused { page, id, reason } => {
                    refused.insert((page.clone(), *id), *reason);
                }
                Event::Twin {
                    page,
                    existing,
                    version,
                    saved,
                } => {
                    self.twins
                        .insert(page.clone(), (existing.clone(), *version, *saved));
                }
                Event::ObserveError(page) => {
                    self.observe_errors.insert(page.clone());
                }
                _ => {}
            }
        }
        // Due retries go first: each is older than any event of its key,
        // and an event of its key has superseded it above.
        let host = &progress.host;
        let due = self
            .retry
            .iter()
            .filter(|(key, retry)| retry.due <= now && !self.reserved(key, host))
            .map(|(_, retry)| retry.publication.clone());
        delivery.publications.splice(0..0, due.collect::<Vec<_>>());
        for key in self.evictable(&progress.host, &delivery.publications) {
            self.owned.remove(&key);
            self.index.remove(&key);
            self.twins.remove(&key);
            self.observe_errors.remove(&key);
            delivery
                .evicted
                .push((key.clone(), progress.host.fs.spelling(&key)));
        }
        let host = &mut progress.host;
        // A request answered for an earlier window generation has no answer
        // event; once applied, nothing names its handoff.
        let applied = host.last_applied;
        self.pending.retain(|(_, id), _| *id > applied);
        let pages = &host.pages;
        let job = host.job.as_ref().map(|job| (&job.page, job.version));
        self.versions.retain(|page, versions| {
            versions.retain(|version, _| {
                pages.get(page).is_some_and(|p| p.version == *version)
                    || job == Some((page, *version))
            });
            !versions.is_empty()
        });
        let keys: Vec<_> = host.outbox.keys().cloned().collect();
        for page in keys {
            let Some(mail) = host.receive(&page) else {
                continue;
            };
            let exact = match (&mail.page, &mail.answer) {
                (Some(state), Some(answer)) => {
                    answer.took
                        && state.version == answer.version
                        && answered.get(&(page.clone(), answer.id)) == Some(&state.buf)
                }
                _ => false,
            };
            let refused = mail
                .answer
                .as_ref()
                .and_then(|answer| refused.get(&(page.clone(), answer.id)).copied());
            delivery.mail.push((
                page,
                mail,
                MailFacts {
                    notice: Notice::default(),
                    content: !exact,
                    refused,
                    spelling: String::new(),
                    binding: BindingNotice::default(),
                    session: 0,
                },
            ));
        }
        // A notice that changed without a page change still reaches the
        // window (§3.3); only subscribed pages are walked.
        let mailed: BTreeSet<_> = delivery
            .mail
            .iter()
            .map(|(page, ..)| page.clone())
            .collect();
        let generation = host.generation;
        let quiet: Vec<_> = host
            .subscriptions
            .iter()
            .filter(|page| !mailed.contains(*page) && host.pages.contains_key(*page))
            .cloned()
            .collect();
        for page in quiet {
            let notice = (progress.notice(&page), self.binding_notice(&page));
            if self.notices.get(&page) != Some(&notice) {
                delivery.mail.push((
                    page.clone(),
                    Mail {
                        page: progress.host.pages.get(&page).cloned(),
                        answer: None,
                        generation,
                    },
                    MailFacts {
                        notice: Notice::default(),
                        content: false,
                        refused: None,
                        spelling: String::new(),
                        binding: BindingNotice::default(),
                        session: 0,
                    },
                ));
            }
        }
        // `window_crash` clears the outbox under this lock, so all mail here
        // is the current window's; mail a reload overtakes in flight keeps
        // its old session, and the window drops it.
        for (page, _, facts) in &mut delivery.mail {
            facts.session = self.session;
            facts.notice = progress.notice(page);
            facts.spelling = progress.host.fs.spelling(page);
            facts.binding = self.binding_notice(page);
            self.notices
                .insert(page.clone(), (facts.notice.clone(), facts.binding.clone()));
        }
        let subscribed = &progress.host.subscriptions;
        self.notices.retain(|page, _| subscribed.contains(page));
        delivery.events = events;
        self.delivering |= !delivery.publications.is_empty();
        delivery
    }

    /// Owned keys the host no longer holds, with nothing left to publish:
    /// no page, job, reservation or unobserved release, queued request,
    /// retry or `pending` publication (Q6: a dirty page closed by the window stays held until
    /// its save and cleanup finish).
    fn evictable<F: HostIo>(&self, host: &Host<F>, pending: &[Publication]) -> Vec<PageKey> {
        self.owned
            .iter()
            .filter(|key| {
                !host.pages.contains_key(*key)
                    && !host.busy(key)
                    && !host.queue.iter().any(|request| &request.page == *key)
                    && !self.retry.contains_key(*key)
                    && !self.handover.contains(*key)
                    && !pending.iter().any(|p| &p.key == *key)
            })
            .cloned()
            .collect()
    }

    /// Record a delivery's index results in order (§5): success advances
    /// the key's watermark; a failure retries with the save backoff; a
    /// reserved key waits for its release.
    pub fn record(&mut self, results: Vec<(Publication, Indexing)>, now: u64) {
        self.delivering = false;
        for (publication, indexing) in results {
            let key = publication.key.clone();
            let failures = self.retry.get(&key).map_or(0, |retry| retry.failures);
            match indexing {
                Indexing::Indexed => {
                    self.retry.remove(&key);
                    let watermark = self.index.get(&key).map_or(0, |(_, v)| *v);
                    let own = publication.own.as_ref().map_or(0, |(v, _)| *v);
                    self.index
                        .insert(key, (publication.bytes, watermark.max(own)));
                }
                Indexing::Failed => {
                    let failures = failures.saturating_add(1);
                    let due = now.saturating_add(backoff(failures));
                    self.retry.insert(
                        key,
                        Retry {
                            publication,
                            failures,
                            due,
                        },
                    );
                }
                Indexing::Reserved => {
                    let retry = Retry {
                        publication,
                        failures,
                        due: now,
                    };
                    self.retry.insert(key, retry);
                }
            }
        }
    }

    /// The earliest retry due, for the driver's sleep (§5).
    pub fn next_retry<F: HostIo>(&self, host: &Host<F>) -> Option<u64> {
        self.retry
            .iter()
            .filter(|(key, _)| !self.reserved(key, host))
            .map(|(_, retry)| retry.due)
            .min()
    }

    /// A retained writer publishes `key`'s index: its reservation holds,
    /// or its release has not yet been observed (Q6, V2).
    fn reserved<F: HostIo>(&self, key: &str, host: &Host<F>) -> bool {
        host.retained.contains(key) || self.handover.contains(key)
    }

    /// Who publishes `key`'s index now (§5, Q6).
    pub fn owner<F: HostIo>(&self, key: &str, host: &Host<F>) -> Owner {
        if self.reserved(key, host) {
            Owner::Reservation
        } else if self.owned.contains(key) {
            Owner::Consumer
        } else {
            Owner::Watcher
        }
    }

    fn binding_notice(&self, key: &str) -> BindingNotice {
        BindingNotice {
            index_error: self.index_error(key),
            observe_error: self.observe_errors.contains(key),
            twin: self.twins.get(key).map(|(existing, ..)| existing.clone()),
        }
    }

    fn index_error(&self, key: &str) -> bool {
        self.retry.get(key).is_some_and(|retry| retry.failures >= 3)
    }
}

/// The driver's sink: the publication consumer (§5), then page mail to the
/// window (§3.3).
struct Bridge {
    store: Arc<Store>,
    mail: MailSink,
    #[cfg(test)]
    index_faults: Arc<std::sync::atomic::AtomicU32>,
    #[cfg(test)]
    published_kinds: PublishedKinds,
}

impl Sink for Bridge {
    /// Under the writer, which orders this against every other index writer
    /// and excludes a running reconcile: claimed keys leave the watcher,
    /// publications apply in host order unless a reservation took their key
    /// (its transaction publishes; checked under the writer, so it cannot
    /// commit before a publication applied here), and evicted keys the
    /// consumer still does not own return to the watcher.
    fn deliver(
        &mut self,
        delivery: Delivery,
        owner: &dyn Fn(&str) -> Owner,
    ) -> Vec<(Publication, Indexing)> {
        let store = &*self.store;
        let mut results = Vec::new();
        #[cfg(test)]
        self.published_kinds
            .lock()
            .unwrap()
            .extend(delivery.kinds.iter().cloned());
        if !delivery.claimed.is_empty()
            || !delivery.publications.is_empty()
            || !delivery.evicted.is_empty()
        {
            let _writer = store.writer.lock().unwrap();
            for (key, _) in delivery.claimed {
                store.watch.hold(key);
            }
            for publication in delivery.publications {
                let indexing = match owner(&publication.key) {
                    Owner::Reservation => Indexing::Reserved,
                    _ if self.fault() => Indexing::Failed,
                    _ if index(store, &publication) => Indexing::Indexed,
                    _ => Indexing::Failed,
                };
                results.push((publication, indexing));
            }
            for (key, _) in delivery.evicted {
                if owner(&key) == Owner::Watcher {
                    store.watch.release_hold(&key);
                }
            }
        }
        for (key, mail, facts) in delivery.mail {
            let mail = page_mail(store, key, mail, facts);
            (self.mail.lock().unwrap())(mail);
        }
        results
    }
}

impl Bridge {
    #[cfg(test)]
    fn fault(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.index_faults
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
    }

    #[cfg(not(test))]
    fn fault(&self) -> bool {
        false
    }
}

/// Publish `publication` to the index (§5): the index part of a
/// transaction's publication (cache, names, references, `GraphChange`),
/// with the R8 Document when it matched. Bytes the index already holds only
/// align the watcher's snapshot. The caller holds the writer. False when
/// the store has closed.
fn index(store: &Store, publication: &Publication) -> bool {
    if store.is_closed() {
        return false;
    }
    let graph = &store.graph;
    let path = graph.root.join(&publication.spelling);
    let id = FileId::from(publication.spelling.clone());
    let bytes = publication.bytes.as_deref();
    let rev = bytes.map(FileRev::from_bytes);
    let cached = graph.cached_rev(&path);
    let text = bytes.and_then(|bytes| std::str::from_utf8(bytes).ok());
    // A new hold retired the page's rows (R1): the published snapshot still
    // knows it.
    let known = cached.is_some() || store.snapshot_knows(&path);
    let kind = match (known, bytes) {
        _ if cached.is_some() && text.map(content_rev) == cached => None,
        (false, None) => None,
        (false, Some(_)) => Some(ChangeKind::Created),
        (true, None) => Some(ChangeKind::Removed),
        (true, Some(_)) => Some(ChangeKind::Modified),
    };
    let document = publication.own.as_ref().and_then(|(_, d)| d.as_ref());
    // The owner's bytes become the page's index and its row in one
    // critical section (R1); an unchanged row is not touched.
    let owned = graph.publish_owned(&publication.key, publication.bytes.clone(), document);
    let Some(kind) = kind else {
        graph.transaction_clear_page_marker(&path);
        let raced = store.watch.note_own(&[(id, rev)]);
        store.watch.reconcile_raced(&raced);
        return true;
    };
    let before = graph.cache_generation();
    let entry = match &owned {
        Ok(entry) => entry.clone(),
        Err(()) => graph.transaction_publish_page_inner(
            &path,
            bytes,
            document,
            kind != ChangeKind::Modified,
            false,
        ),
    };
    if graph.cache_generation() == before {
        graph.transaction_bump_generation();
    }
    let files = vec![(id.clone(), kind, rev)];
    if publication.own.is_some() {
        let mut observations = PublishedObservations::default();
        if let Some(entry) = entry {
            observations.entries.insert(id, entry);
        }
        store.publish_own(files, observations);
    } else {
        // Named from the owner's bytes (none for a removal, which the
        // snapshot names by its previous name); with no owner, by path.
        let entry = match &owned {
            Ok(_) => entry,
            Err(()) => entry.or_else(|| graph.entry_for_path(&path)),
        };
        let pages = entry
            .map(|entry| (id, entry.kind, entry.name))
            .into_iter()
            .collect();
        store.publish_transaction_change(Origin::External, files, pages, Default::default());
    }
    true
}

fn page_mail(store: &Store, key: PageKey, mail: Mail, facts: MailFacts) -> PageMail {
    let graph = &store.graph;
    let page = mail.page.map(|page| MailPage {
        version: page.version,
        conflict: page.conflict,
        risk: page.risk,
        disk: page.obs.as_ref().map(DiskToken::of),
        text: match &page.buf {
            _ if !facts.content => MailText::Unchanged {
                rev: page.buf.as_deref().map(FileRev::from_bytes),
            },
            None => MailText::NoFile,
            Some(bytes) => match graph.page_dto_for_bytes(&graph.root.join(&facts.spelling), bytes)
            {
                Ok(Some(dto)) => MailText::Page { dto: Box::new(dto) },
                Ok(None) => MailText::Unreadable {
                    message: "not a page path".into(),
                },
                Err(error) => MailText::Unreadable {
                    message: error.to_string(),
                },
            },
        },
    });
    let answer = mail.answer.map(|answer| MailAnswer {
        id: answer.id,
        version: answer.version,
        took: answer.took,
        outcome: match facts.refused {
            Some(reason) => AnswerOutcome::Refused { reason },
            None => AnswerOutcome::Applied,
        },
    });
    PageMail {
        session: facts.session,
        key,
        page,
        answer,
        notice: MailNotice {
            failures: facts.notice.failures,
            save_error: facts.notice.save_error,
            draft_error: facts.notice.draft_error,
            conflict_reported: facts.notice.conflict_reported,
            custody_error: !facts.notice.custody_error.is_empty(),
            index_error: facts.binding.index_error,
            observe_error: facts.binding.observe_error,
            twin: facts.binding.twin,
        },
    }
}

/// The window's page-mail sink, kept across a relaunch.
type MailSink = Arc<Mutex<Box<dyn FnMut(PageMail) + Send>>>;
/// Each publication's edit kinds as delivered (tests, Rule 8).
#[cfg(test)]
type PublishedKinds = Arc<Mutex<Vec<(PageKey, Vec<EditKind>)>>>;

/// A binding's launch parameters (`PageHost::start`).
#[derive(Clone)]
struct Launch {
    store: Arc<Store>,
    app_data: std::path::PathBuf,
    graph_id: String,
    mail: MailSink,
}

/// A host stopped for a restore (§7 step 4): the binding holds no host,
/// and only `relaunch` binds one again, on the tree as it then is.
#[must_use = "a stopped binding is relaunched after the restore"]
pub struct Stopped {
    launch: Launch,
}

impl Stopped {
    /// Launch one fresh host for the same binding and page-mail sink (§7
    /// step 6), on success and on a partial restore failure alike. The
    /// error says why no host could start, as `PageHost::start`'s does.
    pub fn relaunch(self) -> Result<PageHost, String> {
        PageHost::launch(self.launch)
    }
}

/// The version a request carries when it was typed on no version the host
/// holds: host versions start at 1, so it is always stale.
const STALE: u64 = 0;

impl PageHost {
    /// Plan, lock, revalidate on the command thread (§1), then wake the
    /// driver. None once the driver is stopping.
    fn locked<R>(&self, mut step: impl FnMut(&mut Host<ProductionIo>) -> R) -> Option<R> {
        self.driver
            .shared
            .locked_step(|state| state.progress.with_host(&mut step))
    }

    /// The session commands must carry now (tests; the window learns it
    /// from `window_reloaded`).
    #[cfg(test)]
    pub(crate) fn session(&self) -> u64 {
        self.driver.shared.state.lock().unwrap().book.session
    }

    /// A window (re)load (`window_crash`): unsent input is lost, admitted
    /// requests keep their custody, and the window gets a fresh session and
    /// the first request id the host can admit (R8).
    pub(crate) fn window_reloaded(&self) -> Reloaded {
        self.driver.shared.with_state(|state| {
            let next_id = state.progress.with_host(|host| {
                host.window_crash();
                host.last_admitted.saturating_add(1)
            });
            state.book.session = next_session();
            Reloaded {
                session: state.book.session,
                next_id,
            }
        })
    }

    fn register(&self, key: &str, spelling: &PageId) {
        let graph = &self.store.graph;
        let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
        self.driver.shared.with_state(|state| {
            state
                .progress
                .with_host(|host| host.register(key.into(), spelling.as_str(), lock))
        });
    }

    /// Admit `request` under `session`'s window generation; on admission
    /// its handoffs wait for its answer, and an Open's key is the
    /// consumer's (§5) in the same step, so no collection can evict it in
    /// between. A session that is not current is not admitted.
    fn admit(
        &self,
        session: u64,
        mut request: Request,
        handoffs: Vec<(PageKey, Handoff)>,
    ) -> Result<(), PageRefusal> {
        self.driver.shared.with_state(|state| {
            if session != state.book.session {
                return Err(PageRefusal::NotAdmitted);
            }
            request.generation = state.progress.host.generation;
            let id = request.id;
            let open = (request.kind == RequestKind::Open).then(|| request.page.clone());
            match state.progress.with_host(|host| host.admit(request)) {
                Disposition::Applied => {
                    for (page, handoff) in handoffs {
                        state.book.pending.insert((page, id), handoff);
                    }
                    state.book.owned.extend(open);
                    Ok(())
                }
                _ => Err(PageRefusal::NotAdmitted),
            }
        })
    }

    /// `page_open` (§2, F4): every editable page, including one with no file
    /// yet, is opened first. With no file the create-only checks run here,
    /// never on its keystrokes. Returns the page's key, and whether `page`
    /// names that key's entry (the window's baseline, S5).
    /// The page's index moves to the publication consumer here (§5), under
    /// the writer: a watcher reconcile already running for it finishes
    /// first, and the consumer publishes the Open read.
    pub(crate) fn open(
        &self,
        session: u64,
        id: u64,
        page: &PageId,
        name: &str,
    ) -> Result<Opened, PageRefusal> {
        let store = &*self.store;
        let (key, spelling, held, baseline_entry) = self.identify(page);
        if !held {
            store
                .transaction(None)
                .page_open_checks(&spelling.file(), name)?;
        }
        self.register(&key, &spelling);
        let request = Request {
            id,
            generation: 0,
            page: key.clone(),
            kind: RequestKind::Open,
        };
        let _writer = store.writer.lock().unwrap();
        store.watch.hold(key.clone());
        let admitted = self.admit(session, request, vec![]);
        if admitted.is_err()
            && !self
                .driver
                .shared
                .with_state(|s| s.book.owned.contains(&key))
        {
            store.watch.release_hold(&key);
        }
        admitted.map(|()| Opened {
            key,
            baseline_entry,
        })
    }

    /// The comparison source for a page DTO typed on `version` (§3.2, Q1),
    /// the resolve base, and the version the request carries. An ordinary
    /// submit on the current version compares with the host buffer at that
    /// version. A resolve compares with exactly the bytes its token names,
    /// which become its base: one snapshot, so the bytes checked are the
    /// bytes the resolve installs and replaces, whatever is observed later.
    /// A stale submit has no trustworthy source. So has a resolve whose token
    /// no longer names the observed disk state: the host no longer holds the
    /// bytes the user chose to overwrite, so it is admitted as stale input
    /// (`STALE`; the model's `submitTo` makes it a conflict with an unknown
    /// base) and the window is shown the newer disk to resolve again.
    fn source(
        &self,
        key: &str,
        version: u64,
        resolve: Option<&DiskToken>,
    ) -> (String, Option<Text>, Option<Text>, u64) {
        let state = self.driver.shared.state.lock().unwrap();
        let host = &state.progress.host;
        let spelling = if host.keys.contains(key) {
            host.fs.spelling(key)
        } else {
            key.into()
        };
        let page = host.pages.get(key);
        match (resolve, page) {
            (Some(token), Some(page)) => match &page.obs {
                Some(obs) if DiskToken::of(obs) == *token => {
                    (spelling, Some(obs.clone()), Some(obs.clone()), version)
                }
                _ => (spelling, None, None, STALE),
            },
            (Some(_), None) => (spelling, None, None, STALE),
            (None, Some(page)) if page.version == version => {
                (spelling, Some(page.buf.clone()), None, version)
            }
            (None, _) => (spelling, None, None, version),
        }
    }

    /// The per-submit checks and serialization (§3.2), before admission.
    fn serialize(
        &self,
        spelling: &str,
        dto: &PageDto,
        source: Option<&Text>,
        kinds: &[EditKind],
    ) -> Result<(Text, Handoff), PageRefusal> {
        let file = PageId::from(spelling).file();
        let tx = self.store.transaction(None);
        tx.page_save_target(&file, dto)?;
        let old = source.and_then(|text| text.as_deref());
        let (bytes, document) = tx.serialize_page(&file, dto, old, false)?;
        let bytes: Text = Some(Arc::from(bytes));
        let handoff = Handoff {
            bytes: bytes.clone(),
            document,
            kinds: kinds.to_vec(),
        };
        Ok((bytes, handoff))
    }

    /// `page_submit` (§3.1): serialize against the request's comparison
    /// source, then admit. A refusal here was never sent.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submit(
        &self,
        session: u64,
        id: u64,
        key: &str,
        dto: &PageDto,
        version: u64,
        resolve: Option<&DiskToken>,
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (spelling, source, base, version) = self.source(key, version, resolve);
        let (bytes, handoff) = self.serialize(&spelling, dto, source.as_ref(), kinds)?;
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Submit {
                bytes,
                version,
                resolve: base,
            },
        };
        self.admit(session, request, vec![(key.into(), handoff)])
    }

    /// `page_move` (§3.1, §8): both endpoints serialized as ordinary submits
    /// on their versions, admitted as the model's two-page move.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn move_blocks(
        &self,
        session: u64,
        id: u64,
        source: (&str, &PageDto, u64),
        receiver: (&str, &PageDto, u64),
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (source_key, source_dto, source_version) = source;
        let (receiver_key, receiver_dto, receiver_version) = receiver;
        let (spelling, old, ..) = self.source(source_key, source_version, None);
        let (source_text, source_handoff) =
            self.serialize(&spelling, source_dto, old.as_ref(), kinds)?;
        let (spelling, old, ..) = self.source(receiver_key, receiver_version, None);
        let (receiver_text, receiver_handoff) =
            self.serialize(&spelling, receiver_dto, old.as_ref(), kinds)?;
        let request = Request {
            id,
            generation: 0,
            page: source_key.into(),
            kind: RequestKind::Move {
                receiver: receiver_key.into(),
                source_text,
                receiver_text,
                source_version,
                receiver_version,
            },
        };
        self.admit(
            session,
            request,
            vec![
                (source_key.into(), source_handoff),
                (receiver_key.into(), receiver_handoff),
            ],
        )
    }

    /// `page_discard` (§3.1): the window consumed its unsent input first.
    pub(crate) fn discard(
        &self,
        session: u64,
        id: u64,
        key: &str,
        version: u64,
    ) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Discard { version },
        };
        self.admit(session, request, vec![])
    }

    /// `page_close` (§3.1): the last surface showing the page released it.
    pub(crate) fn close(&self, session: u64, id: u64, key: &str) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Close,
        };
        self.admit(session, request, vec![])
    }

    /// `page_delete` (§7): the host's delete operation. D4 refuses a page
    /// with unsaved input; Waiting means the page is busy (retry when clean).
    pub(crate) fn delete(&self, page: &PageId) -> PageOperation {
        let (key, spelling, ..) = self.identify(page);
        self.register(&key, &spelling);
        let deleted = self.driver.shared.locked_step(|state| {
            let disposition = state.progress.with_host(|host| host.delete(&key));
            // OG-RULES Rule 8 (A-K1): the operation's write is a deletion.
            if disposition == Disposition::Pending {
                state.book.took([key.clone()], EditKind::DeletePage);
            }
            disposition
        });
        match deleted {
            Some(Disposition::Applied) => PageOperation::Applied,
            Some(Disposition::Pending) => PageOperation::Pending,
            Some(Disposition::Waiting) => PageOperation::Waiting,
            _ => PageOperation::Refused,
        }
    }
}

pub(super) fn stop_state<F: HostIo, C: Clock>(progress: &Progress<F, C>, book: &Book) -> StopState {
    let host = &progress.host;
    let Some(stopping) = &progress.stopping else {
        return StopState::Waiting;
    };
    let mut affected = progress.draft_error_pages();
    if stopping.restore {
        // A restore aborts where its flush would fail today (§7 step 3).
        affected.extend(stopping.failed.iter().cloned());
        let keys = host.pages.keys().chain(host.custody.keys());
        affected.extend(
            keys.filter(|key| {
                host.pages.get(*key).is_some_and(|page| page.conflict)
                    || !host.custody_errors(key).is_empty()
                    || book.index_error(key)
            })
            .cloned(),
        );
        if affected.is_empty() && host.custody_unknown.is_some() {
            return StopState::Aborted(affected);
        }
    }
    if !affected.is_empty() {
        return StopState::Aborted(affected);
    }
    // Readiness waits for every save the stop owes first, whatever the
    // caller polls (V1): a page's exact draft from an earlier failure is a
    // switch's fallback only once this stop's save of it failed.
    if progress.owes_save_first() {
        return StopState::Waiting;
    }
    // A restore closes only over completed work (§7 step 3, V1, V5): every
    // page saved, no recoverable draft, no custody debt, and every
    // publication delivered and indexed. A clean page needs no save (Q5).
    let complete = !stopping.restore
        || (host.pages.values().all(|page| page.clean())
            && host.logical_drafts().is_empty()
            && host.custody.is_empty()
            && host.retire.is_empty()
            && book.retry.is_empty()
            && !book.delivering);
    if host.can_switch() && complete {
        StopState::Ready
    } else {
        StopState::Waiting
    }
}

impl Drop for PageHost {
    /// Stop and join the driver (its last step's results have been
    /// delivered), then hand every held page's index back to the watcher.
    fn drop(&mut self) {
        self.driver.join();
        let _writer = self.store.writer.lock().unwrap();
        self.store.watch.release_holds();
    }
}

#[path = "binding_retained.rs"]
mod retained;

#[path = "binding_publication.rs"]
mod publication;

#[path = "binding_launch.rs"]
mod launch;
pub use retained::{RenameRefusal, Reservation};
#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
