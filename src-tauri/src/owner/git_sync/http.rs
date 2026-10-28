//! libgit2's stateless smart transport over the app's reqwest/TLS stack.
//! Adapted from the ADR 0078 spike. Registration is process-wide; clients,
//! credentials, cancellation and failures belong to one synchronous operation.

use std::cell::RefCell;
use std::future::Future;
use std::io::{self, Read, Write};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use git2::transport::{Service, SmartSubtransport, SmartSubtransportStream, Transport};

use super::{Credentials, Outcome, TransportOptions};

thread_local! {
    static SESSION: RefCell<Option<Arc<Session>>> = const { RefCell::new(None) };
}

pub(super) struct Session {
    client: reqwest::Client,
    credentials: Arc<dyn Credentials>,
    pub(super) cancel: Arc<std::sync::atomic::AtomicBool>,
    max_body: usize,
    failure: Mutex<Option<Outcome>>,
}

impl Session {
    pub(super) fn new(
        credentials: Arc<dyn Credentials>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        options: &TransportOptions,
    ) -> Result<Arc<Self>, Outcome> {
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("git/2.0 (Tine ", env!("CARGO_PKG_VERSION"), ")"))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(options.connect_timeout)
            .read_timeout(options.read_timeout)
            .timeout(options.request_timeout);
        #[cfg(target_os = "windows")]
        {
            builder = builder.tls_backend_native();
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let _ = rustls::crypto::ring::default_provider().install_default();
            builder = builder.tls_backend_rustls();
        }
        // Production always uses the system trust store. Test roots are added,
        // never substituted, and certificate verification is never disabled.
        #[cfg(test)]
        if let Some(pem) = &options.test_root_pem {
            let cert = reqwest::Certificate::from_pem(pem)
                .map_err(|_| Outcome::Refused("invalid test CA".into()))?;
            builder = builder.tls_certs_merge([cert]);
        }
        let client = {
            let _context = runtime().enter();
            builder.build().map_err(network)?
        };
        Ok(Arc::new(Self {
            client,
            credentials,
            cancel,
            max_body: options.max_request_body,
            failure: Mutex::new(None),
        }))
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Acquire)
    }

    fn fail(&self, outcome: Outcome) -> io::Error {
        // The typed error survives git2's conversion of every stream IO error
        // into a generic Net error. Never classify authentication by its text.
        let mut failure = self.failure.lock().unwrap_or_else(|p| p.into_inner());
        if failure.is_none() {
            *failure = Some(outcome);
        }
        io::Error::other("Tine git HTTP transport failed")
    }

    pub(super) fn outcome(&self, error: git2::Error) -> Outcome {
        self.failure
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .unwrap_or_else(|| {
                if self.cancelled() {
                    Outcome::Cancelled
                } else if error.code() == git2::ErrorCode::NotFastForward {
                    Outcome::Diverged
                } else {
                    Outcome::Network(format!("git: {}", error.message()))
                }
            })
    }

    pub(super) fn run<T>(self: &Arc<Self>, f: impl FnOnce() -> T) -> Result<T, Outcome> {
        register()?;
        struct Reset(Option<Arc<Session>>);
        impl Drop for Reset {
            fn drop(&mut self) {
                SESSION.with(|s| *s.borrow_mut() = self.0.take());
            }
        }
        let _reset = Reset(SESSION.with(|s| s.replace(Some(self.clone()))));
        Ok(f())
    }
}

fn network(error: reqwest::Error) -> Outcome {
    // URLs and bodies can contain secrets; do not expose them to status/UI.
    Outcome::Network(if error.is_timeout() {
        "HTTP request timed out".into()
    } else if error.is_connect() {
        "HTTP connection or TLS verification failed".into()
    } else {
        "HTTP request or response failed".into()
    })
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("tine-git-http")
            .enable_all()
            .build()
            .expect("git HTTP runtime")
    })
}

/// All application transport creation is inside RUN_LOCK, including file://
/// operations. No other production code creates a git2 remote connection.
fn register() -> Result<(), Outcome> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            // SAFETY: the caller holds RUN_LOCK; OnceLock registers exactly
            // once, synchronized with every application transport creation.
            unsafe {
                git2::transport::register("https", |remote| {
                    let session = SESSION.with(|s| s.borrow().clone()).ok_or_else(|| {
                        git2::Error::from_str("git HTTP operation has no session")
                    })?;
                    Transport::smart(
                        remote,
                        true,
                        HttpTransport {
                            session,
                            connection: Arc::new(Mutex::new(Connection::default())),
                        },
                    )
                })
            }
            .map_err(|e| e.message().to_owned())
        })
        .clone()
        .map_err(Outcome::Network)
}

#[derive(Default)]
struct Connection {
    base: String,
    original_url: String,
    original_origin: String,
    auth: Option<super::Token>,
    tried_credentials: bool,
}

struct HttpTransport {
    session: Arc<Session>,
    connection: Arc<Mutex<Connection>>,
}

impl SmartSubtransport for HttpTransport {
    fn action(
        &self,
        url: &str,
        action: Service,
    ) -> Result<Box<dyn SmartSubtransportStream>, git2::Error> {
        let mut connection = self.connection.lock().unwrap_or_else(|p| p.into_inner());
        if connection.base.is_empty() {
            let parsed =
                https_url(url).map_err(|_| git2::Error::from_str("unsupported git URL"))?;
            connection.base = url.trim_end_matches('/').into();
            connection.original_url = connection.base.clone();
            connection.original_origin = parsed.origin().ascii_serialization();
        }
        let (service, path, post) = match action {
            Service::UploadPackLs => ("upload-pack", "/info/refs?service=git-upload-pack", false),
            Service::UploadPack => ("upload-pack", "/git-upload-pack", true),
            Service::ReceivePackLs => {
                ("receive-pack", "/info/refs?service=git-receive-pack", false)
            }
            Service::ReceivePack => ("receive-pack", "/git-receive-pack", true),
        };
        Ok(Box::new(HttpStream {
            session: self.session.clone(),
            connection: self.connection.clone(),
            service,
            path,
            post,
            body: Vec::new(),
            response: None,
            sent: false,
            current: Bytes::new(),
            eof: false,
        }))
    }

    fn close(&self) -> Result<(), git2::Error> {
        *self.connection.lock().unwrap_or_else(|p| p.into_inner()) = Connection::default();
        Ok(())
    }
}

pub(super) fn https_url(url: &str) -> Result<reqwest::Url, Outcome> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| Outcome::Refused("invalid HTTPS remote URL".into()))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(Outcome::Refused(
            "use an HTTPS remote without embedded credentials, query or fragment".into(),
        ));
    }
    Ok(parsed)
}

/// Spawn + channel also works on a blocking thread carrying a Tokio context.
/// Aborting drops the in-flight request/body, including a stalled TLS read.
fn wait<T: Send + 'static, F: Future<Output = T> + Send + 'static>(
    session: &Session,
    make: impl FnOnce() -> F,
) -> io::Result<T> {
    if session.cancelled() {
        return Err(session.fail(Outcome::Cancelled));
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let _context = runtime().enter();
    let future = make();
    let task = runtime().spawn(async move {
        let _ = tx.send(future.await);
    });
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Timeout) if !session.cancelled() => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                task.abort();
                return Err(session.fail(Outcome::Cancelled));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(session.fail(Outcome::Network("HTTP task stopped".into())));
            }
        }
    }
}

struct HttpStream {
    session: Arc<Session>,
    connection: Arc<Mutex<Connection>>,
    service: &'static str,
    path: &'static str,
    post: bool,
    body: Vec<u8>,
    response: Option<reqwest::Response>,
    sent: bool,
    current: Bytes,
    eof: bool,
}

impl HttpStream {
    fn send(&mut self) -> io::Result<()> {
        // Bytes makes retry bodies cheap; the potentially large pack is not
        // copied for each request or authentication retry.
        let body = Bytes::from(std::mem::take(&mut self.body));
        for redirects in 0..=5 {
            loop {
                let (base, auth) = {
                    let c = self.connection.lock().unwrap_or_else(|p| p.into_inner());
                    (c.base.clone(), c.auth.clone())
                };
                let url = format!("{base}{}", self.path);
                let mut request = if self.post {
                    self.session
                        .client
                        .post(&url)
                        .header(
                            "Content-Type",
                            format!("application/x-git-{}-request", self.service),
                        )
                        .body(body.clone())
                } else {
                    self.session.client.get(&url)
                }
                .header("Accept", self.content_type())
                .header("Pragma", "no-cache");
                if let Some(token) = auth {
                    request = request.basic_auth(token.username, Some(token.secret));
                }
                let response = wait(&self.session, || request.send())?
                    .map_err(|e| self.session.fail(network(e)))?;
                let status = response.status();
                if status == reqwest::StatusCode::UNAUTHORIZED {
                    let mut c = self.connection.lock().unwrap_or_else(|p| p.into_inner());
                    let origin = https_url(&base)
                        .map_err(|e| self.session.fail(e))?
                        .origin()
                        .ascii_serialization();
                    if c.tried_credentials || origin != c.original_origin {
                        return Err(self.session.fail(Outcome::AuthRequired));
                    }
                    c.tried_credentials = true;
                    c.auth = self.session.credentials.token(&c.original_url);
                    if c.auth.is_none() {
                        return Err(self.session.fail(Outcome::AuthRequired));
                    }
                    continue;
                }
                if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) && !self.post {
                    if redirects == 5 {
                        return Err(self
                            .session
                            .fail(Outcome::Network("too many HTTP redirects".into())));
                    }
                    let target = response
                        .headers()
                        .get("location")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| response.url().join(v).ok())
                        .ok_or_else(|| {
                            self.session
                                .fail(Outcome::Network("invalid HTTP redirect".into()))
                        })?;
                    let base = target.as_str().strip_suffix(self.path).ok_or_else(|| {
                        self.session
                            .fail(Outcome::Network("redirect changed git service path".into()))
                    })?;
                    let parsed = https_url(base).map_err(|e| self.session.fail(e))?;
                    let mut c = self.connection.lock().unwrap_or_else(|p| p.into_inner());
                    if parsed.origin().ascii_serialization()
                        != https_url(&c.base)
                            .map_err(|e| self.session.fail(e))?
                            .origin()
                            .ascii_serialization()
                    {
                        c.auth = None; // scheme, host AND port form the boundary
                    }
                    c.base = base.into();
                    break;
                }
                // POST redirects are never replayed: a push may already have
                // reached the server. A new cycle can inspect the remote tip.
                if status != reqwest::StatusCode::OK {
                    return Err(self
                        .session
                        .fail(Outcome::Network(format!("HTTP {}", status.as_u16()))));
                }
                let got = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if got.split(';').next().unwrap_or("").trim() != self.content_type() {
                    return Err(self
                        .session
                        .fail(Outcome::Network("invalid git HTTP Content-Type".into())));
                }
                self.response = Some(response);
                return Ok(());
            }
        }
        unreachable!("redirect bound handled in loop")
    }

    fn content_type(&self) -> String {
        format!(
            "application/x-git-{}-{}",
            self.service,
            if self.post { "result" } else { "advertisement" }
        )
    }
}

impl Read for HttpStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.session.cancelled() {
            return Err(self.session.fail(Outcome::Cancelled));
        }
        if !self.sent {
            self.sent = true;
            self.send()?;
        }
        loop {
            if !self.current.is_empty() {
                let n = buffer.len().min(self.current.len());
                buffer[..n].copy_from_slice(&self.current[..n]);
                self.current = self.current.slice(n..);
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            let mut response = self.response.take().ok_or_else(|| {
                self.session
                    .fail(Outcome::Network("HTTP response missing".into()))
            })?;
            let (response, chunk) = wait(&self.session, move || async move {
                let chunk = response.chunk().await;
                (response, chunk)
            })?;
            self.response = Some(response);
            match chunk.map_err(|e| self.session.fail(network(e)))? {
                Some(chunk) => self.current = chunk,
                None => self.eof = true,
            }
        }
    }
}

impl Write for HttpStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.session.cancelled() {
            return Err(self.session.fail(Outcome::Cancelled));
        }
        if self.sent {
            return Err(io::Error::other("write after git HTTP request"));
        }
        if data.len() > self.session.max_body.saturating_sub(self.body.len()) {
            return Err(self.session.fail(Outcome::Refused(
                "git HTTP request exceeds buffer limit".into(),
            )));
        }
        self.body.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
