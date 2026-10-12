//! The owner's endpoint (CLI PROTOCOL §2, §3.2): a Unix socket in the
//! account's owner directory, one thread per connection, bounded frames and
//! I/O, and an admission gate. The gate is how an owner stops: it stops
//! admitting (new requests answer `busy`), lets every admitted request
//! settle, and only then closes the listener and every connection.
//! `owner.status` is answered without admission, so a client can watch an
//! owner that is stopping.

use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::wire::{ErrorBody, Request};
#[cfg(unix)]
use super::wire::{Hello, Reply};
use super::OwnerError;

/// Idle time allowed between frames, and to write a reply (§2.1).
pub(crate) const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// What the connection does once the reply is written.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum After {
    Continue,
    /// Tell the owner's main thread to exit (a `tine serve` yield).
    Exit,
}

/// The owner's answers. `answer` runs for an admitted request on its
/// connection's thread; `status` for `owner.status`, unadmitted.
pub(crate) trait Handler: Send + Sync + 'static {
    fn answer_request(&self, request: &Request) -> (Result<Value, ErrorBody>, After);
    fn status(&self, gate: &Gate) -> Value;
}

/// Admission: whether new requests are taken, how many are in flight, and
/// whether a stop owns the owner. A stop is claimed once, by one caller
/// (a yield, or the idle exit), and never resumes admission (review
/// CODE-2 #1, #2); a pause without a stop (app retirement) can.
pub(crate) struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

struct GateState {
    admitting: bool,
    stopping: bool,
    in_flight: usize,
    last: Instant,
    progress: Option<String>,
}

/// One admitted request; dropping it ends it.
pub(crate) struct Admitted(Arc<Gate>);

impl Drop for Admitted {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.in_flight -= 1;
        state.last = Instant::now();
        self.0.changed.notify_all();
    }
}

impl Gate {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GateState {
                admitting: true,
                stopping: false,
                in_flight: 0,
                last: Instant::now(),
                progress: None,
            }),
            changed: Condvar::new(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn admit(self: &Arc<Self>) -> Option<Admitted> {
        let mut state = self.lock();
        if !state.admitting {
            return None;
        }
        state.in_flight += 1;
        state.last = Instant::now();
        Some(Admitted(self.clone()))
    }

    /// Stop admitting: new requests answer `busy`, not admitted.
    pub(crate) fn pause(&self) {
        self.lock().admitting = false;
    }

    /// Admit again after a pause (an adoption). Never after a stop.
    pub(crate) fn resume(&self) {
        let mut state = self.lock();
        if state.stopping {
            return;
        }
        state.admitting = true;
        state.progress = None;
        state.last = Instant::now();
        self.changed.notify_all();
    }

    /// Claim the stop for an admitted request (a yield): admission ends for
    /// good. False when another stop owns it; the caller must not wait on
    /// it, since that stop may be waiting on the caller's own admission.
    pub(crate) fn claim_stop(&self) -> bool {
        let mut state = self.lock();
        if state.stopping {
            return false;
        }
        state.stopping = true;
        state.admitting = false;
        true
    }

    /// Claim the stop for the idle exit: only when admitting with nothing in
    /// flight for at least `idle`, in one step, so no request is admitted
    /// in between and nothing needs draining.
    pub(crate) fn claim_idle_stop(&self, idle: Duration) -> bool {
        let mut state = self.lock();
        if state.stopping || !state.admitting || state.in_flight > 0 || state.last.elapsed() < idle
        {
            return false;
        }
        state.stopping = true;
        state.admitting = false;
        true
    }

    /// While not admitting, wait until at most `except` requests are in
    /// flight, however long their work takes: admitted work settles before a
    /// stop, it is never cut off (§3.2, §4.2). Socket deadlines bound I/O,
    /// not work. Returns early if admission resumes (an adoption).
    pub(crate) fn settle(&self, except: usize) {
        let mut state = self.lock();
        while state.in_flight > except && !state.admitting {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.lock().in_flight
    }

    /// What `owner.status` reports while stopping (`stopping`,
    /// `retrying: <diag>`).
    pub(crate) fn set_progress(&self, progress: impl Into<String>) {
        self.lock().progress = Some(progress.into());
    }

    /// `serving`, `busy`, or the stop's progress.
    pub(crate) fn state(&self) -> String {
        let state = self.lock();
        match (&state.progress, state.admitting) {
            (Some(progress), _) => progress.clone(),
            (None, true) => "serving".into(),
            (None, false) => "busy".into(),
        }
    }
}

/// The reply to a request that came while the owner was not admitting.
pub(crate) fn busy() -> ErrorBody {
    ErrorBody::new(
        "busy",
        "the graph's owner is handing over or stopping; try again",
        false,
    )
}

#[cfg(unix)]
pub(crate) use unix::Endpoint;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::mpsc::Sender;

    use crate::owner::wire::{self, FrameError, PROTOCOL, REPLY_CAP, REQUEST_CAP};

    const CONNECTION_THREAD: &str = "tine-owner-connection";

    /// A listening endpoint. Dropping it closes it: stop admitting, let
    /// admitted requests settle (`Gate::settle`), close the listener and
    /// every connection, remove the socket path. Its owner still holds the
    /// lock while it drops (`Lease` field order, §3.2).
    pub(crate) struct Endpoint {
        socket: PathBuf,
        gate: Arc<Gate>,
        shared: Arc<Shared>,
        accept: Option<std::thread::JoinHandle<()>>,
    }

    /// The open connections. Sealed by a yield's sweep or the endpoint's
    /// close: a connection registered after it is closed at once, so the
    /// sweep cannot miss one accepted meanwhile (review CODE-2 #3).
    #[derive(Default)]
    struct Connections {
        open: HashMap<u64, UnixStream>,
        sealed: bool,
    }

    impl Connections {
        /// Close every connection but `except` and take no more.
        fn seal(&mut self, except: Option<u64>) {
            self.sealed = true;
            for (id, connection) in self.open.drain() {
                if Some(id) != except {
                    let _ = connection.shutdown(std::net::Shutdown::Both);
                }
            }
        }
    }

    struct Shared {
        closed: AtomicBool,
        connections: Mutex<Connections>,
        next: AtomicU64,
        hello: Hello,
        handler: Arc<dyn Handler>,
        gate: Arc<Gate>,
        exit: Mutex<Option<Sender<()>>>,
    }

    impl Endpoint {
        /// Listen on `socket`. Only the lock holder calls this, so a path
        /// already there is a dead owner's and is removed (§3.2).
        pub(crate) fn bind(
            socket: &Path,
            hello: Hello,
            gate: Arc<Gate>,
            handler: Arc<dyn Handler>,
            exit: Option<Sender<()>>,
        ) -> Result<Self, OwnerError> {
            match std::fs::remove_file(socket) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(OwnerError::Start(format!(
                        "cannot remove the stale endpoint {}: {e}",
                        socket.display()
                    )))
                }
            }
            let listener = UnixListener::bind(socket).map_err(|e| {
                OwnerError::Start(format!("cannot listen on {}: {e}", socket.display()))
            })?;
            let shared = Arc::new(Shared {
                closed: AtomicBool::new(false),
                connections: Mutex::new(Connections::default()),
                next: AtomicU64::new(0),
                hello,
                handler,
                gate: gate.clone(),
                exit: Mutex::new(exit),
            });
            let accepting = shared.clone();
            let accept = std::thread::Builder::new()
                .name("tine-owner-endpoint".into())
                .spawn(move || accept_loop(listener, accepting))
                .map_err(|e| OwnerError::Start(format!("cannot start the endpoint: {e}")))?;
            Ok(Self {
                socket: socket.to_owned(),
                gate,
                shared,
                accept: Some(accept),
            })
        }
    }

    impl Drop for Endpoint {
        fn drop(&mut self) {
            self.gate.pause();
            // A connection thread can drop the last reference to a slot
            // (and so its endpoint) after its own request; it must not wait
            // for itself.
            if std::thread::current().name() != Some(CONNECTION_THREAD) {
                self.gate.settle(0);
            }
            self.shared.closed.store(true, Ordering::SeqCst);
            // Wake the accept loop; it sees `closed` and ends.
            let _ = UnixStream::connect(&self.socket);
            if let Some(accept) = self.accept.take() {
                let _ = accept.join();
            }
            (self.shared.connections.lock())
                .unwrap_or_else(|e| e.into_inner())
                .seal(None);
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    fn accept_loop(listener: UnixListener, shared: Arc<Shared>) {
        for stream in listener.incoming() {
            if shared.closed.load(Ordering::SeqCst) {
                return;
            }
            let stream = match stream {
                Ok(stream) => stream,
                Err(_) => {
                    // Out of descriptors and the like: do not spin.
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            let id = shared.next.fetch_add(1, Ordering::Relaxed);
            {
                let mut connections =
                    (shared.connections.lock()).unwrap_or_else(|e| e.into_inner());
                let Ok(clone) = stream.try_clone() else {
                    continue;
                };
                if connections.sealed {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    continue;
                }
                connections.open.insert(id, clone);
            }
            let serving = shared.clone();
            let spawned = std::thread::Builder::new()
                .name(CONNECTION_THREAD.into())
                .spawn(move || {
                    connection(id, stream, &serving);
                    (serving.connections.lock())
                        .unwrap_or_else(|e| e.into_inner())
                        .open
                        .remove(&id);
                });
            if spawned.is_err() {
                crate::debug::diag("owner-connection-spawn-failed");
            }
        }
    }

    fn send(writer: &mut UnixStream, reply: &Reply) -> bool {
        let mut line = match serde_json::to_vec(reply) {
            Ok(line) => line,
            Err(_) => return false,
        };
        if line.len() > REPLY_CAP {
            // Slice 1 replies are all reads: a read admits nothing (§2.1).
            let too_large = Reply {
                id: reply.id,
                ok: None,
                error: Some(ErrorBody::new(
                    "too-large",
                    format!("the answer is over {REPLY_CAP} bytes; narrow the request"),
                    false,
                )),
            };
            line = serde_json::to_vec(&too_large).unwrap_or_default();
        }
        line.push(b'\n');
        writer
            .write_all(&line)
            .and_then(|()| writer.flush())
            .is_ok()
    }

    fn error(id: u64, body: ErrorBody) -> Reply {
        Reply {
            id,
            ok: None,
            error: Some(body),
        }
    }

    fn connection(id: u64, stream: UnixStream, shared: &Shared) {
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let Ok(reader) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(reader);
        let mut writer = stream;
        // Handshake: the client's hello first (§2.2).
        let client = match wire::read_frame(&mut reader, REQUEST_CAP) {
            Ok(Some(line)) => wire::read_line::<Hello>(&line),
            Err(FrameError::TooLarge) => {
                send(&mut writer, &error(0, frame_too_large()));
                return;
            }
            _ => return,
        };
        let client = match client {
            Ok(client) if client.tine == "cli" => client,
            // Not a client's hello (another owner, a stray peer): §2.2.
            _ => {
                send(&mut writer, &error(0, malformed()));
                return;
            }
        };
        if client.protocol != PROTOCOL {
            let message = format!(
                "the graph's owner speaks protocol {PROTOCOL}, this tine speaks {}",
                client.protocol
            );
            send(
                &mut writer,
                &error(0, ErrorBody::new("protocol", message, false)),
            );
            return;
        }
        if client.graph != shared.hello.graph {
            let message = format!(
                "this endpoint serves {}, not {}",
                shared.hello.graph, client.graph
            );
            send(
                &mut writer,
                &error(0, ErrorBody::new("owner-mismatch", message, false)),
            );
            return;
        }
        if wire::write_frame(&mut writer, &shared.hello).is_err() {
            return;
        }
        loop {
            let line = match wire::read_frame(&mut reader, REQUEST_CAP) {
                Ok(Some(line)) => line,
                Err(FrameError::TooLarge) => {
                    send(&mut writer, &error(0, frame_too_large()));
                    return;
                }
                // EOF, EOF mid-line (never executed), timeout, I/O.
                Ok(None) | Err(_) => return,
            };
            let request = match wire::read_line::<Request>(&line) {
                Ok(request) => request,
                Err(_) => {
                    send(&mut writer, &error(0, malformed()));
                    return;
                }
            };
            if request.verb == "owner.status" {
                let ok = shared.handler.status(&shared.gate);
                let reply = Reply {
                    id: request.id,
                    ok: Some(ok),
                    error: None,
                };
                if !send(&mut writer, &reply) {
                    return;
                }
                continue;
            }
            let Some(admitted) = shared.gate.admit() else {
                if !send(&mut writer, &error(request.id, busy())) {
                    return;
                }
                continue;
            };
            let (result, after) = shared.handler.answer_request(&request);
            if after == After::Exit {
                // Yielded: close every other connection, and any accepted
                // after this, before the reply, which goes out while the
                // lock is still held (§4.3).
                (shared.connections.lock())
                    .unwrap_or_else(|e| e.into_inner())
                    .seal(Some(id));
            }
            let reply = match result {
                Ok(ok) => Reply {
                    id: request.id,
                    ok: Some(ok),
                    error: None,
                },
                Err(body) => error(request.id, body),
            };
            let sent = send(&mut writer, &reply);
            drop(admitted);
            if after == After::Exit {
                if let Some(exit) = shared.exit.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    let _ = exit.send(());
                }
                return;
            }
            if !sent {
                return;
            }
        }
    }

    fn frame_too_large() -> ErrorBody {
        ErrorBody::new(
            "frame-too-large",
            format!("a request line is limited to {REQUEST_CAP} bytes"),
            false,
        )
    }

    fn malformed() -> ErrorBody {
        ErrorBody::new("malformed", "the line is not a request object", false)
    }
}

/// Windows has no endpoint in slice 1a (PROTOCOL §1): the namespace is
/// `Unsupported` there, so nothing binds.
#[cfg(windows)]
pub(crate) struct Endpoint;

#[cfg(windows)]
impl Endpoint {
    pub(crate) fn bind(
        _socket: &Path,
        _hello: super::wire::Hello,
        _gate: Arc<Gate>,
        _handler: Arc<dyn Handler>,
        _exit: Option<std::sync::mpsc::Sender<()>>,
    ) -> Result<Self, OwnerError> {
        Err(OwnerError::Unsupported)
    }
}
