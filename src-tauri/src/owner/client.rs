//! The command line's side of the owner protocol (CLI PROTOCOL §3.4, §4):
//! reaching the graph's owner (starting `tine serve` when nobody owns it),
//! and the app's handover from a `tine serve` that owns a graph it opens.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::lease::Claimed;
use super::lock::OwnerLock;
use super::namespace::{GraphPaths, Namespace};
use super::wire::{Drafts, ErrorBody};
use super::OwnerError;

/// How long discovery waits for a started `tine serve` to answer.
const START_WAIT: Duration = Duration::from_secs(10);

/// Why a call got no answer from the owner.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The owner answered with an error.
    Owner(ErrorBody),
    /// No owner could be reached or became the owner.
    Reach(OwnerError),
    /// The connection broke after the request was sent: for a read, ask
    /// again; a write would be `uncertain` (§2.3).
    Lost(String),
}

#[cfg(unix)]
pub(crate) use unix::Conn;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;

    use crate::owner::endpoint::IO_TIMEOUT;
    use crate::owner::wire::{self, Hello, Reply, Request, PROTOCOL, REPLY_CAP, REQUEST_CAP};

    /// A connection to a graph's owner, past the handshake.
    pub(crate) struct Conn {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
        next: u64,
    }

    /// The owner's hello names this protocol, this graph and an owner kind
    /// (§2.2); anything else is not the owner this client asked for.
    fn owner_hello_matches(hello: &Hello, root: &Path) -> Result<(), OwnerError> {
        if hello.protocol != PROTOCOL {
            return Err(OwnerError::Protocol(format!(
                "the graph's owner speaks protocol {}, this tine speaks {PROTOCOL}",
                hello.protocol
            )));
        }
        if hello.graph != root.display().to_string() {
            return Err(OwnerError::Mismatch(format!(
                "the endpoint serves {}, not {}",
                hello.graph,
                root.display()
            )));
        }
        if !matches!(hello.owner.as_deref(), Some("app" | "serve")) {
            return Err(OwnerError::Protocol(
                "the endpoint did not say what kind of owner it is".into(),
            ));
        }
        Ok(())
    }

    /// `Ok(None)`: no endpoint listens there (none, or a dead one's path).
    pub(crate) fn dial(socket: &Path, root: &Path) -> Result<Option<Conn>, OwnerError> {
        dial_with(socket, root, IO_TIMEOUT, None)
    }

    /// [`dial`] with the handshake bounded by `io` and each reply by
    /// `replies` (`None`: a reply waits for the work, however long).
    pub(crate) fn dial_with(
        socket: &Path,
        root: &Path,
        io: Duration,
        replies: Option<Duration>,
    ) -> Result<Option<Conn>, OwnerError> {
        let stream = match UnixStream::connect(socket) {
            Ok(stream) => stream,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Ok(None)
            }
            Err(e) => {
                return Err(OwnerError::Start(format!(
                    "cannot connect to {}: {e}",
                    socket.display()
                )))
            }
        };
        let bound = io;
        let io = |e: std::io::Error| OwnerError::Start(format!("the owner's endpoint failed: {e}"));
        stream.set_read_timeout(Some(bound)).map_err(io)?;
        stream.set_write_timeout(Some(bound)).map_err(io)?;
        let mut reader = BufReader::new(stream.try_clone().map_err(io)?);
        let mut writer = stream;
        let hello = Hello {
            tine: "cli".into(),
            protocol: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").into(),
            graph: root.display().to_string(),
            owner: None,
        };
        wire::write_frame(&mut writer, &hello).map_err(io)?;
        let line = match wire::read_frame(&mut reader, REQUEST_CAP) {
            Ok(Some(line)) => line,
            // An owner that closed at once is stopping: like no endpoint.
            Ok(None) | Err(_) => return Ok(None),
        };
        if let Ok(hello) = wire::read_line::<Hello>(&line) {
            if hello.tine == "owner" {
                owner_hello_matches(&hello, root)?;
                // Slow work is not a slow socket: wait for the answer, unless
                // the caller bounds it (a status probe).
                writer.set_read_timeout(replies).map_err(io)?;
                return Ok(Some(Conn {
                    reader,
                    writer,
                    next: 1,
                }));
            }
        }
        let reply = wire::read_line::<Reply>(&line).ok();
        let error = reply.and_then(|reply| reply.error);
        Err(match error {
            Some(body) if body.code == "owner-mismatch" => OwnerError::Mismatch(body.message),
            Some(body) => OwnerError::Protocol(body.message),
            None => OwnerError::Protocol("the endpoint did not answer as a Tine owner".into()),
        })
    }

    impl Conn {
        /// One request and its reply.
        pub(crate) fn call(&mut self, verb: &str, args: Value) -> Result<Value, Failure> {
            let id = self.next;
            self.next += 1;
            let request = Request {
                id,
                verb: verb.into(),
                args,
            };
            wire::write_frame(&mut self.writer, &request)
                .map_err(|e| Failure::Lost(e.to_string()))?;
            let line = match wire::read_frame(&mut self.reader, REPLY_CAP) {
                Ok(Some(line)) => line,
                Ok(None) => return Err(Failure::Lost("the owner closed the connection".into())),
                Err(e) => return Err(Failure::Lost(format!("{e:?}"))),
            };
            let reply: Reply = wire::read_line(&line)
                .map_err(|e| Failure::Lost(format!("unreadable reply: {e}")))?;
            // One request at a time per connection (§2.3): the reply is this
            // request's, or the connection is not the protocol's.
            if reply.id != id {
                return Err(Failure::Lost(format!(
                    "a reply to request {} arrived for request {id}",
                    reply.id
                )));
            }
            match (reply.ok, reply.error) {
                (_, Some(error)) => Err(Failure::Owner(error)),
                (Some(ok), None) => Ok(ok),
                (None, None) => Err(Failure::Lost("an empty reply".into())),
            }
        }

        pub(crate) fn shutdown(&self) {
            let _ = self.writer.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[cfg(windows)]
pub(crate) struct Conn;

#[cfg(windows)]
impl Conn {
    pub(crate) fn call(&mut self, _verb: &str, _args: Value) -> Result<Value, Failure> {
        Err(Failure::Reach(OwnerError::Unsupported))
    }

    pub(crate) fn shutdown(&self) {}
}

#[cfg(windows)]
pub(crate) fn dial(_socket: &Path, _root: &Path) -> Result<Option<Conn>, OwnerError> {
    Err(OwnerError::Unsupported)
}

#[cfg(unix)]
pub(crate) use unix::{dial, dial_with};

#[cfg(windows)]
pub(crate) fn dial_with(
    _socket: &Path,
    _root: &Path,
    _io: Duration,
    _replies: Option<Duration>,
) -> Result<Option<Conn>, OwnerError> {
    Err(OwnerError::Unsupported)
}

/// Reach `root`'s owner, starting `tine serve` when nobody answers (§3.4).
pub(crate) fn connect(root: &Path) -> Result<Conn, OwnerError> {
    let paths = Namespace::resolve()?.graph(root)?;
    if let Some(conn) = dial(&paths.socket, root)? {
        return Ok(conn);
    }
    spawn_serve(root, &paths)?;
    let deadline = Instant::now() + START_WAIT;
    let mut pause = Duration::from_millis(100);
    loop {
        std::thread::sleep(pause);
        if let Some(conn) = dial(&paths.socket, root)? {
            return Ok(conn);
        }
        if Instant::now() >= deadline {
            break;
        }
        pause = (pause * 2).min(Duration::from_secs(2));
    }
    // Who holds the graph decides what went wrong (§3.4 step 4).
    match OwnerLock::try_take(&paths.lock, "probe")? {
        Some(_free) => Err(OwnerError::Start(format!(
            "`tine serve` did not start; its log is {}",
            paths.start_log.display()
        ))),
        None => Err(OwnerError::Start(
            "the graph is open in Tine but its command endpoint is unavailable".into(),
        )),
    }
}

/// Reach `root`'s owner if one answers; never starts one.
pub(crate) fn connect_existing(root: &Path) -> Result<Option<Conn>, OwnerError> {
    let paths = Namespace::resolve()?.graph(root)?;
    dial(&paths.socket, root)
}

/// Start `tine serve <root>` detached from this terminal, its output to the
/// graph's bounded start log (last run only).
fn spawn_serve(root: &Path, paths: &GraphPaths) -> Result<(), OwnerError> {
    let start = |e: std::io::Error| OwnerError::Start(format!("cannot start `tine serve`: {e}"));
    let exe = std::env::current_exe().map_err(start)?;
    // Emptied (last run only), then appended to, so serve can bound it while
    // it runs by truncating it (`serve::bound_start_log`, §3.4).
    std::fs::File::create(&paths.start_log).map_err(start)?;
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&paths.start_log)
        .map_err(start)?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("serve")
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().map_err(start)?)
        .stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own session: closing the terminal does not stop the owner.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    command.spawn().map_err(start)?;
    Ok(())
}

/// The app's handover (§4): ask the `tine serve` owning `root` to yield,
/// then take the lock itself once serve has exited (the reply is never
/// permission). Waits up to `bound`, reporting serve's progress.
pub(crate) fn take_over(
    root: &Path,
    paths: &GraphPaths,
    drafts: Drafts,
    bound: Duration,
    mut waiting: impl FnMut(&str),
) -> Result<Option<Claimed>, String> {
    let deadline = Instant::now() + bound;
    let (answer, answered) = std::sync::mpsc::channel();
    {
        let socket = paths.socket.clone();
        let root = root.to_owned();
        let spawned = std::thread::Builder::new()
            .name("tine-owner-yield".into())
            .spawn(move || {
                let result = match dial(&socket, &root) {
                    Ok(Some(mut conn)) => conn
                        .call("owner.yield", json!({ "drafts": drafts }))
                        .map(|_| ()),
                    Ok(None) => Ok(()),
                    Err(error) => Err(Failure::Reach(error)),
                };
                let _ = answer.send(result);
            });
        if spawned.is_err() {
            return Err("cannot ask the command line's owner to yield".into());
        }
    }
    let mut progress = String::from("stopping");
    let mut reported = Instant::now() - Duration::from_secs(1);
    // At most one status probe in flight, off this loop: a serve wedged
    // before `accept` can hold even `connect`, never the deadline (§4.5,
    // review CODE-2 #5).
    let mut probe: Option<std::sync::mpsc::Receiver<Option<String>>> = None;
    loop {
        match OwnerLock::try_take(&paths.lock, "app") {
            Ok(Some(lock)) => return Ok(Some(Claimed::from_lock(lock, paths.clone()))),
            Ok(None) => {}
            Err(error) => {
                crate::debug::diag_private("owner-lock-unavailable", &error.to_string());
                return Ok(None);
            }
        }
        if let Ok(Err(failure)) = answered.try_recv() {
            match failure {
                Failure::Owner(body) if body.code == "refused" => {
                    return Err(refusal_text(&body));
                }
                // Lost or unreachable: serve may be exiting; keep polling.
                _ => {}
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Tine's command line is still finishing its work on this graph ({progress}). \
                 Try again in a moment, or stop it with `tine serve --stop {}`.",
                root.display()
            ));
        }
        if let Some(pending) = &probe {
            match pending.try_recv() {
                Ok(state) => {
                    progress = state.unwrap_or(progress);
                    probe = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => probe = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if reported.elapsed() >= Duration::from_secs(1) {
            reported = Instant::now();
            if probe.is_none() {
                probe = probe_status(&paths.socket, root);
            }
            waiting(&progress);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn refusal_text(body: &ErrorBody) -> String {
    let reason = body
        .detail
        .as_ref()
        .and_then(|detail| detail.get("reason"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match reason {
        "draft-namespace" => "Tine's command line owns this graph with a different \
            application-data directory, so its unsaved work could not be recovered here. \
            Stop it with `tine serve --stop`, then open the graph again."
            .into(),
        "app-owner" => "The graph is open in another Tine window.".into(),
        _ => body.message.clone(),
    }
}

/// Probe the owner's state on a thread of its own; its answer (or `None`)
/// arrives on the returned channel. A probe stuck in `connect` stays stuck
/// only until the owner closes its listener.
fn probe_status(socket: &Path, root: &Path) -> Option<std::sync::mpsc::Receiver<Option<String>>> {
    let (answer, answered) = std::sync::mpsc::channel();
    let (socket, root) = (socket.to_owned(), root.to_owned());
    std::thread::Builder::new()
        .name("tine-owner-status".into())
        .spawn(move || {
            let _ = answer.send(status(&socket, &root, Duration::from_secs(2)));
        })
        .ok()
        .map(|_| answered)
}

/// The owner's `owner.status` state, if one answers within `bound` (each
/// of the handshake and the reply).
fn status(socket: &Path, root: &Path, bound: Duration) -> Option<String> {
    let mut conn = dial_with(socket, root, bound, Some(bound)).ok()??;
    let status = conn.call("owner.status", Value::Null).ok()?;
    conn.shutdown();
    status
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
