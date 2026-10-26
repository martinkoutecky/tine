//! Quick share's GitHub transport (DESIGN §3.3, §3.5): the device-flow OAuth
//! endpoints, `GET /user`, and the gist calls. Every GitHub call Tine makes is
//! here, in Rust; the token never reaches the webview.
//!
//! Policy: `reqwest` with no redirects (a redirect could carry the
//! `Authorization` header elsewhere), HTTPS only in production, connect and
//! read timeouts, and every response body read through a bound. Errors are
//! classified, never formatted from the transport error or an OAuth body, so
//! no URL, header or token can travel in one. Bodies of the OAuth endpoints
//! are parsed for their documented fields and otherwise dropped.

use super::Token;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The GitHub OAuth App "Tine" (registered 2026-10-17; public, not a secret).
pub(crate) const CLIENT_ID: &str = "Ov23liMHqsgTkxqKqFG6";
const API_VERSION: &str = "2022-11-28";
/// Gist API answers (a listing page of 100 gists is a few hundred KiB).
const MAX_API_BYTES: usize = 4 * 1024 * 1024;
/// OAuth endpoint answers are a few hundred bytes.
const MAX_OAUTH_BYTES: usize = 64 * 1024;
/// A raw `tine-share.bin` read: the writer's cap plus one trailing newline.
pub(crate) const MAX_RAW_BYTES: usize = tine_graph_features::quick_share::MAX_ENVELOPE_BYTES + 1;

/// Where the calls go. Production uses [`Endpoints::github`]; tests point all
/// three at a local mock server.
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    /// `https://github.com` (device code and token endpoints).
    pub oauth: String,
    /// `https://api.github.com`.
    pub api: String,
    /// The only origin a raw gist file is read from.
    pub raw: String,
    pub https_only: bool,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}

impl Endpoints {
    pub(crate) fn github() -> Self {
        // Native E2E runs the real service and dialog against a loopback GitHub.
        // This seam is compiled out of release builds; even debug accepts only
        // a bare local origin, never a remote HTTP destination.
        #[cfg(debug_assertions)]
        if let Ok(origin) = std::env::var("TINE_QUICK_SHARE_TEST_ORIGIN") {
            if let Some(endpoints) = Self::loopback_test(&origin) {
                return endpoints;
            }
        }
        Self::github_https()
    }

    /// Fixed production origins, including for the development connectivity
    /// diagnostic. It cannot be redirected by the loopback E2E override.
    pub(super) fn github_https() -> Self {
        Self {
            oauth: "https://github.com".into(),
            api: "https://api.github.com".into(),
            raw: "https://gist.githubusercontent.com".into(),
            https_only: true,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
        }
    }

    #[cfg(any(debug_assertions, test))]
    pub(super) fn loopback_test(origin: &str) -> Option<Self> {
        let url = reqwest::Url::parse(origin).ok()?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return None;
        }
        let origin = origin.trim_end_matches('/').to_owned();
        Some(Self {
            oauth: origin.clone(),
            api: origin.clone(),
            raw: origin,
            https_only: false,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
        })
    }
}

/// Why a GitHub call failed. Never carries a transport error's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GhError {
    /// No byte of the request can have reached GitHub: DNS, connect refused,
    /// proxy or TLS failure, connect timeout.
    NotSent,
    /// Sent; the outcome is unknown (read timeout, dropped connection).
    Unknown,
    /// 401: the token was revoked or expired.
    Unauthorized,
    /// 403/429 with `Retry-After`, or with `x-ratelimit-remaining: 0` and
    /// `x-ratelimit-reset`.
    RateLimited {
        minutes: u64,
    },
    /// Another 403, with GitHub's message, sanitized.
    Forbidden(String),
    NotFound,
    /// Another 4xx: GitHub answered and did not do it.
    Refused(u16),
    /// 5xx: GitHub failed; for a create, the outcome is unknown.
    Server(u16),
    /// The answer exceeded its bound or was not what GitHub documents.
    BadAnswer,
}

impl GhError {
    fn from_transport(error: &reqwest::Error) -> Self {
        if error.is_connect() {
            GhError::NotSent
        } else {
            GhError::Unknown
        }
    }
}

/// An answer: status, the headers quick share reads, and the bounded body.
pub(crate) struct Answer {
    pub status: u16,
    pub scopes: Option<String>,
    pub next_page: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub(crate) struct GitHub {
    pub endpoints: Endpoints,
    client: reqwest::Client,
}

/// A fixed, secret-free diagnostic. No remote body, URL or transport error text
/// reaches the WebView. Disabled is returned by release builds without I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ConnectionTestResult {
    Connected,
    Tls,
    Network,
    Refused,
    BadAnswer,
    Disabled,
}

#[cfg(debug_assertions)]
fn connection_test_error(error: &reqwest::Error) -> ConnectionTestResult {
    // Preserve a useful TLS diagnosis without serializing error chains (which
    // can contain URLs). Windows keeps Schannel and reports the network code;
    // the four rustls targets can identify its typed certificate/TLS errors.
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios"
    ))]
    {
        use std::error::Error;
        let mut cause = error.source();
        while let Some(inner) = cause {
            if inner.is::<rustls::Error>() {
                return ConnectionTestResult::Tls;
            }
            // io::Error::source can skip its contained error. TLS errors from
            // tokio-rustls are commonly nested in one or more io::Errors.
            cause = inner
                .downcast_ref::<std::io::Error>()
                .and_then(|io| io.get_ref())
                .map(|error| error as &(dyn Error + 'static))
                .or_else(|| inner.source());
        }
    }
    ConnectionTestResult::Network
}

fn tls(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    // DESIGN §3.3: Windows takes Schannel (the updater's policy since GH #241);
    // the other four shipped targets take rustls with the platform verifier.
    #[cfg(target_os = "windows")]
    {
        builder.tls_backend_native()
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios"
    ))]
    {
        // reqwest's `rustls-no-provider` needs a process provider; the updater
        // installs the same one, and a second install is a no-op.
        let _ = rustls::crypto::ring::default_provider().install_default();
        builder.tls_backend_rustls()
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "android",
    target_os = "ios",
    target_os = "windows"
)))]
compile_error!("quick share declares a TLS transport for each shipped target only");

/// `application/x-www-form-urlencoded` for the few ASCII fields the OAuth
/// endpoints take.
fn form(fields: &[(&str, &str)]) -> String {
    let encode = |value: &str| {
        value
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `rel="next"` of a `Link` header.
fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|p| p.trim() == "rel=\"next\"")
            .then(|| {
                url.trim()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_owned()
            })
    })
}

fn header<'a>(response: &'a reqwest::Response, name: &str) -> Option<&'a str> {
    response.headers().get(name).and_then(|v| v.to_str().ok())
}

/// Minutes until GitHub lets requests through again, if this answer says it
/// is limiting them. `now` is Unix seconds.
fn rate_limit_minutes(
    status: u16,
    retry_after: Option<&str>,
    remaining: Option<&str>,
    reset: Option<&str>,
    now: u64,
) -> Option<u64> {
    if status != 403 && status != 429 {
        return None;
    }
    let minutes = |seconds: u64| seconds.div_ceil(60).max(1);
    if let Some(seconds) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return Some(minutes(seconds));
    }
    let exhausted = remaining.map(str::trim) == Some("0");
    if let (true, Some(reset)) = (
        exhausted || status == 429,
        reset.and_then(|v| v.trim().parse::<u64>().ok()),
    ) {
        return Some(minutes(reset.saturating_sub(now)));
    }
    (status == 429).then_some(1)
}

/// GitHub's `message`, as one short line of printable text with every secret
/// removed. Used only for REST API answers, never for the OAuth endpoints.
pub(crate) fn sanitize(body: &[u8], secrets: &[&str]) -> Option<String> {
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    let message = serde_json::from_slice::<Message>(body).ok()?.message;
    let mut line: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        line = line.replace(secret, "<redacted>");
    }
    let mut cut = line.len().min(300);
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    (!line.is_empty()).then_some(line)
}

async fn bounded_body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, GhError> {
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(GhError::BadAnswer);
    }
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > max {
                    return Err(GhError::BadAnswer);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body),
            Err(_) => return Err(GhError::Unknown),
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[derive(Deserialize)]
pub(crate) struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// One answer of the token endpoint.
pub(crate) enum TokenPoll {
    Pending,
    SlowDown,
    Expired,
    Denied,
    Token(Token),
    /// Any other answer. Its body is not kept.
    Refused,
}

pub(crate) struct GitHubUser {
    pub login: String,
    pub id: u64,
    pub scopes: Vec<String>,
}

/// One gist of a listing, as quick share reads it.
#[derive(Debug, Clone)]
pub(crate) struct ListedGist {
    pub id: String,
    pub created_at: String,
    pub owner_id: Option<u64>,
    pub description: Option<String>,
    pub files: Vec<String>,
    pub share_raw_url: Option<String>,
}

#[derive(Deserialize)]
struct RawGist {
    id: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    owner: Option<RawOwner>,
    #[serde(default)]
    files: std::collections::BTreeMap<String, RawFile>,
}
#[derive(Deserialize)]
struct RawOwner {
    id: u64,
}
#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    raw_url: Option<String>,
}

impl From<RawGist> for ListedGist {
    fn from(gist: RawGist) -> Self {
        let share_raw_url = gist
            .files
            .get(super::SHARE_FILE)
            .and_then(|f| f.raw_url.clone());
        ListedGist {
            id: gist.id,
            created_at: gist.created_at,
            owner_id: gist.owner.map(|o| o.id),
            description: gist.description,
            files: gist.files.into_keys().collect(),
            share_raw_url,
        }
    }
}

/// A gist id as GitHub returns it (DESIGN §1.1).
pub(crate) fn valid_gist_id(id: &str) -> bool {
    (20..=32).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl GitHub {
    pub(crate) fn new(endpoints: Endpoints) -> Result<Self, GhError> {
        let builder = reqwest::Client::builder()
            .user_agent(format!("Tine/{}", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(endpoints.https_only)
            .connect_timeout(endpoints.connect_timeout)
            .read_timeout(endpoints.read_timeout);
        let client = tls(builder).build().map_err(|_| GhError::NotSent)?;
        Ok(Self { endpoints, client })
    }

    #[cfg(debug_assertions)]
    pub(crate) async fn test_connection(&self) -> ConnectionTestResult {
        let response = match self
            .client
            .get(format!("{}/zen", self.endpoints.api))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => return connection_test_error(&error),
        };
        if response.status() != reqwest::StatusCode::OK {
            return ConnectionTestResult::Refused;
        }
        match bounded_body(response, MAX_OAUTH_BYTES).await {
            Ok(body) if !body.is_empty() => ConnectionTestResult::Connected,
            _ => ConnectionTestResult::BadAnswer,
        }
    }

    async fn exchange(
        &self,
        request: reqwest::RequestBuilder,
        max: usize,
    ) -> Result<Answer, GhError> {
        let response = request
            .send()
            .await
            .map_err(|e| GhError::from_transport(&e))?;
        let status = response.status().as_u16();
        let scopes = header(&response, "x-oauth-scopes").map(str::to_owned);
        let next_page = header(&response, "link").and_then(next_link);
        let limited = rate_limit_minutes(
            status,
            header(&response, "retry-after"),
            header(&response, "x-ratelimit-remaining"),
            header(&response, "x-ratelimit-reset"),
            now_secs(),
        );
        let body = bounded_body(response, max).await?;
        Ok(Answer {
            status,
            scopes,
            next_page,
            body,
        })
        .and_then(|answer| match limited {
            Some(minutes) => Err(GhError::RateLimited { minutes }),
            None => Ok(answer),
        })
    }

    /// The REST status mapping (§3.5). `ok` lists the success statuses.
    fn check(answer: Answer, ok: &[u16], token: &Token) -> Result<Answer, GhError> {
        match answer.status {
            s if ok.contains(&s) => Ok(answer),
            401 => Err(GhError::Unauthorized),
            403 => Err(GhError::Forbidden(
                sanitize(&answer.body, &[token.expose()])
                    .unwrap_or_else(|| "GitHub refused the request".into()),
            )),
            404 => Err(GhError::NotFound),
            s @ 400..=499 => Err(GhError::Refused(s)),
            s @ 500..=599 => Err(GhError::Server(s)),
            _ => Err(GhError::BadAnswer),
        }
    }

    fn api(&self, method: reqwest::Method, path: &str, token: &Token) -> reqwest::RequestBuilder {
        self.api_url(method, &format!("{}{path}", self.endpoints.api), token)
    }

    fn api_url(
        &self,
        method: reqwest::Method,
        url: &str,
        token: &Token,
    ) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .bearer_auth(token.expose())
    }

    fn oauth(&self, path: &str, fields: &[(&str, &str)]) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}{path}", self.endpoints.oauth))
            .header("Accept", "application/json")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(fields))
    }

    /// `POST /login/device/code` with the gist scope.
    pub(crate) async fn device_code(&self) -> Result<DeviceCode, GhError> {
        let answer = self
            .exchange(
                self.oauth(
                    "/login/device/code",
                    &[("client_id", CLIENT_ID), ("scope", "gist")],
                ),
                MAX_OAUTH_BYTES,
            )
            .await?;
        match answer.status {
            200 => serde_json::from_slice(&answer.body).map_err(|_| GhError::BadAnswer),
            s @ 400..=499 => Err(GhError::Refused(s)),
            s @ 500..=599 => Err(GhError::Server(s)),
            _ => Err(GhError::BadAnswer),
        }
    }

    /// One poll of `POST /login/oauth/access_token`.
    pub(crate) async fn poll_token(&self, device_code: &str) -> Result<TokenPoll, GhError> {
        #[derive(Deserialize)]
        struct Reply {
            access_token: Option<String>,
            error: Option<String>,
        }
        let answer = self
            .exchange(
                self.oauth(
                    "/login/oauth/access_token",
                    &[
                        ("client_id", CLIENT_ID),
                        ("device_code", device_code),
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ],
                ),
                MAX_OAUTH_BYTES,
            )
            .await?;
        if answer.status >= 500 {
            return Err(GhError::Server(answer.status));
        }
        let Ok(reply) = serde_json::from_slice::<Reply>(&answer.body) else {
            return Ok(TokenPoll::Refused);
        };
        Ok(match (reply.access_token, reply.error.as_deref()) {
            (Some(token), None) if answer.status == 200 && !token.is_empty() => {
                TokenPoll::Token(Token::new(token))
            }
            (_, Some("authorization_pending")) => TokenPoll::Pending,
            (_, Some("slow_down")) => TokenPoll::SlowDown,
            (_, Some("expired_token")) => TokenPoll::Expired,
            (_, Some("access_denied")) => TokenPoll::Denied,
            _ => TokenPoll::Refused,
        })
    }

    /// `GET /user`: login, immutable id, and the granted scopes.
    pub(crate) async fn user(&self, token: &Token) -> Result<GitHubUser, GhError> {
        #[derive(Deserialize)]
        struct User {
            login: String,
            id: u64,
        }
        let answer = Self::check(
            self.exchange(
                self.api(reqwest::Method::GET, "/user", token),
                MAX_API_BYTES,
            )
            .await?,
            &[200],
            token,
        )?;
        let user: User = serde_json::from_slice(&answer.body).map_err(|_| GhError::BadAnswer)?;
        let valid_login = (1..=39).contains(&user.login.len())
            && user
                .login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !valid_login {
            return Err(GhError::BadAnswer);
        }
        let scopes = answer
            .scopes
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(GitHubUser {
            login: user.login,
            id: user.id,
            scopes,
        })
    }

    /// `POST /gists`, never retried. Returns the new gist id.
    pub(crate) async fn create_gist(
        &self,
        token: &Token,
        files: &[(&str, &str)],
    ) -> Result<String, GhError> {
        let files: serde_json::Map<String, serde_json::Value> = files
            .iter()
            .map(|(name, content)| {
                (
                    (*name).to_owned(),
                    serde_json::json!({ "content": content }),
                )
            })
            .collect();
        let body = serde_json::json!({
            "description": super::GIST_DESCRIPTION,
            "public": false,
            "files": files,
        });
        let request = self
            .api(reqwest::Method::POST, "/gists", token)
            .header("Content-Type", "application/json")
            .body(serde_json::to_vec(&body).map_err(|_| GhError::NotSent)?);
        let answer = Self::check(self.exchange(request, MAX_API_BYTES).await?, &[201], token)?;
        #[derive(Deserialize)]
        struct Created {
            id: String,
        }
        // A 201 whose body cannot be read still created a gist: unknown.
        let created: Created =
            serde_json::from_slice(&answer.body).map_err(|_| GhError::Unknown)?;
        if !valid_gist_id(&created.id) {
            return Err(GhError::Unknown);
        }
        Ok(created.id)
    }

    /// One page of `GET /gists` (newest first), and the next page's URL.
    pub(crate) async fn list_gists(
        &self,
        token: &Token,
        url: Option<&str>,
    ) -> Result<(Vec<ListedGist>, Option<String>), GhError> {
        let first = format!("{}/gists?per_page=100", self.endpoints.api);
        let url = url.unwrap_or(&first);
        // A `Link` answer may only lead back to the API origin.
        if !url.starts_with(&format!("{}/", self.endpoints.api)) {
            return Err(GhError::BadAnswer);
        }
        let answer = Self::check(
            self.exchange(
                self.api_url(reqwest::Method::GET, url, token),
                MAX_API_BYTES,
            )
            .await?,
            &[200],
            token,
        )?;
        let gists: Vec<RawGist> =
            serde_json::from_slice(&answer.body).map_err(|_| GhError::BadAnswer)?;
        Ok((
            gists.into_iter().map(ListedGist::from).collect(),
            answer.next_page,
        ))
    }

    /// `GET /gists/{id}`.
    pub(crate) async fn gist(&self, token: &Token, id: &str) -> Result<ListedGist, GhError> {
        if !valid_gist_id(id) {
            return Err(GhError::NotFound);
        }
        let answer = Self::check(
            self.exchange(
                self.api(reqwest::Method::GET, &format!("/gists/{id}"), token),
                MAX_API_BYTES,
            )
            .await?,
            &[200],
            token,
        )?;
        let gist: RawGist = serde_json::from_slice(&answer.body).map_err(|_| GhError::BadAnswer)?;
        Ok(gist.into())
    }

    /// A bounded read of a raw gist file, only from the raw origin and without
    /// credentials (raw files of a secret gist are readable by URL).
    pub(crate) async fn raw_envelope(&self, url: &str) -> Result<String, GhError> {
        if !url.starts_with(&format!("{}/", self.endpoints.raw)) {
            return Err(GhError::BadAnswer);
        }
        let answer = self.exchange(self.client.get(url), MAX_RAW_BYTES).await?;
        match answer.status {
            200 => String::from_utf8(answer.body).map_err(|_| GhError::BadAnswer),
            404 => Err(GhError::NotFound),
            s @ 400..=499 => Err(GhError::Refused(s)),
            s @ 500..=599 => Err(GhError::Server(s)),
            _ => Err(GhError::BadAnswer),
        }
    }

    /// `DELETE /gists/{id}`: 204 deleted.
    pub(crate) async fn delete_gist(&self, token: &Token, id: &str) -> Result<(), GhError> {
        if !valid_gist_id(id) {
            return Err(GhError::NotFound);
        }
        Self::check(
            self.exchange(
                self.api(reqwest::Method::DELETE, &format!("/gists/{id}"), token),
                MAX_API_BYTES,
            )
            .await?,
            &[204],
            token,
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_follow_retry_after_or_an_exhausted_reset() {
        assert_eq!(rate_limit_minutes(403, Some("61"), None, None, 0), Some(2));
        assert_eq!(rate_limit_minutes(429, Some("5"), None, None, 0), Some(1));
        assert_eq!(
            rate_limit_minutes(403, None, Some("0"), Some("1300"), 1000),
            Some(5)
        );
        // Every API answer carries x-ratelimit-reset: a 403 with requests left
        // is a permission refusal, not a limit.
        assert_eq!(
            rate_limit_minutes(403, None, Some("12"), Some("1300"), 1000),
            None
        );
        assert_eq!(
            rate_limit_minutes(429, None, None, Some("1300"), 1000),
            Some(5)
        );
        assert_eq!(rate_limit_minutes(429, None, None, None, 0), Some(1));
        assert_eq!(rate_limit_minutes(500, Some("60"), None, None, 0), None);
    }

    #[test]
    fn sanitize_keeps_one_short_line_without_secrets() {
        let body =
            br#"{"message":"Bad\ncredentials gho_secret\u0007 here","documentation_url":"x"}"#;
        assert_eq!(
            sanitize(body, &["gho_secret"]).unwrap(),
            "Bad credentials <redacted> here"
        );
        assert_eq!(sanitize(b"not json", &[]), None);
        let long = format!(r#"{{"message":"{}"}}"#, "é".repeat(400));
        assert!(sanitize(long.as_bytes(), &[]).unwrap().len() <= 300);
    }

    #[test]
    fn next_link_reads_rel_next() {
        let link = r#"<https://api.github.com/gists?page=2>; rel="next", <https://api.github.com/gists?page=9>; rel="last""#;
        assert_eq!(
            next_link(link).unwrap(),
            "https://api.github.com/gists?page=2"
        );
        assert_eq!(next_link(r#"<https://x/?page=1>; rel="prev""#), None);
    }

    #[test]
    fn form_encodes_reserved_characters() {
        assert_eq!(
            form(&[("grant_type", "urn:ietf:params"), ("a b", "x&y")]),
            "grant_type=urn%3Aietf%3Aparams&a%20b=x%26y"
        );
    }
}
