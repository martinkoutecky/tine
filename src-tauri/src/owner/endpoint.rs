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
    /// The main thread supplied the verdict; close and acknowledge it.
    Close,
}

/// Main's side of a yielding connection. Admission stays with the worker
/// until it has written the verdict and explicitly closed its transport.
pub(crate) struct YieldHandle {
    pub(crate) id: u64,
    pub(crate) verdict: std::sync::mpsc::Sender<Result<Value, ErrorBody>>,
    pub(crate) closed: std::sync::mpsc::Receiver<()>,
}

pub(crate) struct Stop {
    pub(crate) yielder: Option<YieldHandle>,
}

pub(crate) struct YieldRequest {
    id: u64,
    closed: Option<std::sync::mpsc::Sender<()>>,
}

impl YieldRequest {
    #[cfg(test)]
    pub(crate) fn acknowledge_closed(&mut self) {
        if let Some(closed) = self.closed.take() {
            closed.send(()).unwrap();
        }
    }
    pub(crate) fn new(id: u64) -> Self {
        Self { id, closed: None }
    }

    pub(crate) fn stop(
        &mut self,
        stops: &std::sync::mpsc::Sender<Stop>,
    ) -> Result<Value, ErrorBody> {
        let (verdict, reply) = std::sync::mpsc::channel();
        let (closed, ack) = std::sync::mpsc::channel();
        self.closed = Some(closed);
        stops
            .send(Stop {
                yielder: Some(YieldHandle {
                    id: self.id,
                    verdict,
                    closed: ack,
                }),
            })
            .map_err(|_| ErrorBody::new("internal", "owner stop coordinator unavailable", false))?;
        reply.recv().unwrap_or_else(|_| {
            Err(ErrorBody::new(
                "internal",
                "owner stop verdict unavailable",
                false,
            ))
        })
    }
}

/// The owner's answers. `answer` runs for an admitted request on its
/// connection's thread; `status` for `owner.status`, unadmitted.
pub(crate) trait Handler: Send + Sync + 'static {
    fn answer_request(
        &self,
        request: &Request,
        yielder: &mut YieldRequest,
    ) -> (Result<Value, ErrorBody>, After);
    fn status(&self, gate: &Gate) -> Value;
}

/// Admission: whether new requests are taken, how many are in flight, and
/// whether a stop owns the owner. A stop is claimed once, by one caller
/// (a yield, or the idle exit), and never resumes admission (review
/// CODE-2 #1, #2); a pause without a stop (app retirement) can.
pub(crate) struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
    #[cfg(test)]
    lifecycle_hook: Mutex<Option<Arc<dyn Fn(LifecycleBoundary) + Send + Sync>>>,
    #[cfg(test)]
    close_wait_hook: Mutex<Option<Arc<dyn Fn(&std::sync::mpsc::Receiver<()>) + Send + Sync>>>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleBoundary {
    SealWaiting,
    Sealed,
    ReplyReady,
    ReplyWrite,
    CloseWaiting,
    BeforeLeaseRelease,
}

struct GateState {
    #[cfg(test)]
    settling: usize,
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
                #[cfg(test)]
                settling: 0,
                admitting: true,
                stopping: false,
                in_flight: 0,
                last: Instant::now(),
                progress: None,
            }),
            changed: Condvar::new(),
            #[cfg(test)]
            lifecycle_hook: Mutex::new(None),
            #[cfg(test)]
            close_wait_hook: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn observe_lifecycle(
        &self,
        hook: impl Fn(LifecycleBoundary) + Send + Sync + 'static,
    ) {
        *self.lifecycle_hook.lock().unwrap() = Some(Arc::new(hook));
    }

    #[cfg(test)]
    pub(crate) fn lifecycle_boundary(&self, boundary: LifecycleBoundary) {
        let hook = self.lifecycle_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(boundary);
        }
    }

    #[cfg(test)]
    pub(crate) fn observe_close_wait(
        &self,
        hook: impl Fn(&std::sync::mpsc::Receiver<()>) + Send + Sync + 'static,
    ) {
        *self.close_wait_hook.lock().unwrap() = Some(Arc::new(hook));
    }

    #[cfg(test)]
    pub(crate) fn yielder_close_wait(&self, closed: &std::sync::mpsc::Receiver<()>) {
        let hook = self.close_wait_hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(closed);
        }
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
            #[cfg(test)]
            {
                state.settling += 1;
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            #[cfg(test)]
            {
                state.settling -= 1;
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_settling(&self) -> bool {
        self.lock().settling > 0
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
pub(crate) use unix::blocking_connection;
#[cfg(unix)]
pub(crate) use unix::Endpoint;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

    use crate::owner::wire::{self, FrameError, PROTOCOL, REPLY_CAP, REQUEST_CAP};

    const CONNECTION_THREAD: &str = "tine-owner-connection";

    /// A listening endpoint. Dropping it closes it: stop admitting, let
    /// admitted requests settle (`Gate::settle`), close the listener and
    /// every connection, remove the socket path. Its owner still holds the
    /// lock while it drops (`Lease` field order, §3.2).
    pub(crate) struct Endpoint {
        socket: PathBuf,
        gate: Arc<Gate>,
        #[cfg(test)]
        shared: Arc<Shared>,
        commands: Sender<Command>,
        accept: Option<std::thread::JoinHandle<()>>,
    }

    /// Only the accept thread inserts streams and seals this registry.
    #[derive(Default)]
    struct Connections {
        open: HashMap<u64, UnixStream>,
    }

    impl Connections {
        /// Close every connection but `except` and take no more.
        fn seal(&mut self, except: Option<u64>) {
            self.open.retain(|id, connection| {
                if Some(*id) != except {
                    let _ = connection.shutdown(std::net::Shutdown::Both);
                    false
                } else {
                    true
                }
            });
        }
    }

    struct Shared {
        #[cfg(test)]
        accept_pause: Mutex<Option<(Sender<()>, std::sync::mpsc::Receiver<()>)>>,
        #[cfg(test)]
        yield_close_pause: Mutex<Option<(Sender<()>, std::sync::mpsc::Receiver<()>)>>,
        connections: Mutex<Connections>,
        hello: Hello,
        handler: Arc<dyn Handler>,
        gate: Arc<Gate>,
    }

    enum Command {
        Seal {
            except: Option<u64>,
            ack: Sender<()>,
        },
        Shutdown,
    }

    impl Endpoint {
        /// Listen on `socket`. Only the lock holder calls this, so a path
        /// already there is a dead owner's and is removed (§3.2).
        pub(crate) fn bind(
            socket: &Path,
            hello: Hello,
            gate: Arc<Gate>,
            handler: Arc<dyn Handler>,
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
            listener
                .set_nonblocking(true)
                .map_err(|e| OwnerError::Start(format!("cannot poll the endpoint: {e}")))?;
            let (commands, commanded) = mpsc::channel();
            let shared = Arc::new(Shared {
                #[cfg(test)]
                accept_pause: Mutex::new(None),
                #[cfg(test)]
                yield_close_pause: Mutex::new(None),
                connections: Mutex::new(Connections::default()),
                hello,
                handler,
                gate: gate.clone(),
            });
            let accepting = shared.clone();
            let accept = std::thread::Builder::new()
                .name("tine-owner-endpoint".into())
                .spawn(move || accept_loop(listener, accepting, commanded))
                .map_err(|e| OwnerError::Start(format!("cannot start the endpoint: {e}")))?;
            Ok(Self {
                socket: socket.to_owned(),
                gate,
                #[cfg(test)]
                shared,
                commands,
                accept: Some(accept),
            })
        }

        /// Completion barrier: the accept thread registers each accepted
        /// stream before checking commands, closes peers and drops its
        /// listener before acknowledging (CODE-3 RN1).
        pub(crate) fn seal(&self, except: Option<u64>) {
            let (ack, sealed) = mpsc::channel();
            if self.commands.send(Command::Seal { except, ack }).is_ok() {
                #[cfg(test)]
                self.gate.lifecycle_boundary(LifecycleBoundary::SealWaiting);
                let _ = sealed.recv();
            }
        }

        #[cfg(test)]
        pub(crate) fn pause_next_accept(&self) -> (std::sync::mpsc::Receiver<()>, Sender<()>) {
            let (accepted, seen) = std::sync::mpsc::channel();
            let (resume, resumed) = std::sync::mpsc::channel();
            *self.shared.accept_pause.lock().unwrap() = Some((accepted, resumed));
            (seen, resume)
        }

        #[cfg(test)]
        pub(crate) fn pause_yielder_close(&self) -> (std::sync::mpsc::Receiver<()>, Sender<()>) {
            let (written, seen) = mpsc::channel();
            let (resume, resumed) = mpsc::channel();
            *self.shared.yield_close_pause.lock().unwrap() = Some((written, resumed));
            (seen, resume)
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
            let _ = self.commands.send(Command::Shutdown);
            if let Some(accept) = self.accept.take() {
                let _ = accept.join();
            }
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    fn accept_loop(listener: UnixListener, shared: Arc<Shared>, commands: Receiver<Command>) {
        let mut listener = Some(listener);
        let mut next = 0;
        let mut pending = None;
        loop {
            if let Some(command) = pending.take().or_else(|| commands.try_recv().ok()) {
                match command {
                    Command::Seal { except, ack } => {
                        shared
                            .connections
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .seal(except);
                        listener.take();
                        #[cfg(test)]
                        shared.gate.lifecycle_boundary(LifecycleBoundary::Sealed);
                        let _ = ack.send(());
                    }
                    Command::Shutdown => {
                        shared
                            .connections
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .seal(None);
                        return;
                    }
                }
            }
            let stream = listener
                .as_ref()
                .and_then(|listener| listener.accept().ok());
            let Some((stream, _)) = stream else {
                // Includes EMFILE: cancellation is a channel operation and
                // needs no new descriptor (CODE-3 RN2).
                match commands.recv_timeout(Duration::from_millis(50)) {
                    Ok(command) => pending = Some(command),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        shared
                            .connections
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .seal(None);
                        return;
                    }
                }
                continue;
            };
            if blocking_connection(&stream).is_err() {
                continue;
            }
            let id = next;
            next += 1;
            #[cfg(test)]
            if let Some((accepted, resumed)) = shared.accept_pause.lock().unwrap().take() {
                accepted.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            {
                let mut connections =
                    (shared.connections.lock()).unwrap_or_else(|e| e.into_inner());
                let Ok(clone) = stream.try_clone() else {
                    continue;
                };
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
                if let Some(stream) = shared
                    .connections
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .open
                    .remove(&id)
                {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
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

    /// macOS and the BSDs give an accepted socket the listener's O_NONBLOCK;
    /// Linux does not. Connection threads use blocking reads with timeouts, so
    /// an inherited flag made the first request read fail at once and the CLI
    /// saw EINVAL on its next setsockopt (GH #771).
    pub(crate) fn blocking_connection(stream: &UnixStream) -> std::io::Result<()> {
        stream.set_nonblocking(false)
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
            let mut yielder = YieldRequest::new(id);
            let (result, after) = shared.handler.answer_request(&request, &mut yielder);
            let reply = match result {
                Ok(ok) => Reply {
                    id: request.id,
                    ok: Some(ok),
                    error: None,
                },
                Err(body) => error(request.id, body),
            };
            #[cfg(test)]
            if after == After::Close {
                shared
                    .gate
                    .lifecycle_boundary(LifecycleBoundary::ReplyWrite);
            }
            let sent = send(&mut writer, &reply);
            if after == After::Close {
                #[cfg(test)]
                if let Some((written, resumed)) = shared.yield_close_pause.lock().unwrap().take() {
                    written.send(()).unwrap();
                    resumed.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let _ = writer.shutdown(std::net::Shutdown::Both);
                shared
                    .connections
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .open
                    .remove(&id);
                drop(admitted);
                if let Some(closed) = yielder.closed.take() {
                    let _ = closed.send(());
                }
                return;
            }
            drop(admitted);
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
    ) -> Result<Self, OwnerError> {
        Err(OwnerError::Unsupported)
    }

    pub(crate) fn seal(&self, _except: Option<u64>) {}
}
