//! Synchronous transport adapter over Tokio's overlapped, local-only pipes.
//! The endpoint's existing accept thread owns registration/sealing; cancelling
//! I/O never needs a new handle, connection, or wake pipe.
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::runtime::{Builder, Runtime};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY,
    ERROR_PIPE_NOT_CONNECTED, ERROR_SEM_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::Threading::{GetCurrentThreadId, OpenThread, THREAD_TERMINATE};
use windows_sys::Win32::System::IO::{CancelIoEx, CancelSynchronousIo};

use super::windows_security::{self as security, Descriptor};
use super::OwnerError;

fn runtime() -> io::Result<&'static Runtime> {
    static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(1)
                .enable_io()
                .enable_time()
                .thread_name("tine-owner-pipe-io")
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|message| io::Error::other(message.clone()))
}

fn create_pipe(path: &Path, first: bool) -> Result<NamedPipeServer, OwnerError> {
    let descriptor = Descriptor::private(false)?;
    create_with(path, first, &descriptor).map_err(|e| pipe_create_error(path, e))
}

fn pipe_create_error(path: &Path, error: io::Error) -> OwnerError {
    OwnerError::Unsafe(format!(
        "cannot create private owner pipe {} (name may be occupied): {error}",
        path.display()
    ))
}

const PREDECESSOR_WAIT: Duration = Duration::from_secs(3);

fn create_with(path: &Path, first: bool, descriptor: &Descriptor) -> io::Result<NamedPipeServer> {
    let runtime = runtime()?;
    let _entered = runtime.enter();
    let mut attributes = descriptor.attributes();
    // Tokio copies the descriptor during CreateNamedPipe; it outlives that call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(path, std::ptr::from_mut(&mut attributes).cast())
    }
}

pub(super) struct Listener {
    path: std::path::PathBuf,
    pending: Mutex<NamedPipeServer>,
}

impl Listener {
    pub(super) fn bind(path: &Path) -> Result<Self, OwnerError> {
        let descriptor = Descriptor::private(false)?;
        let deadline = Instant::now() + PREDECESSOR_WAIT;
        // The caller holds the owner lock. An occupied name can belong to a
        // draining predecessor (including IOCP retirement), or a squatter.
        // Always request the first instance, even when retrying.
        let pending = loop {
            match create_with(path, true, &descriptor) {
                Ok(pipe) => break pipe,
                Err(e)
                    if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        Duration::from_millis(50)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(e) => return Err(OwnerError::Unsafe(format!(
                    "cannot claim first owner pipe {} after up to {} s waiting for predecessor handles (name may still be occupied): {e}",
                    path.display(), PREDECESSOR_WAIT.as_secs()
                ))),
            }
        };
        Ok(Self {
            path: path.to_owned(),
            pending: Mutex::new(pending),
        })
    }

    pub(super) fn accept(&self) -> io::Result<(Stream, ())> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let connected = runtime()?.block_on(async {
            tokio::time::timeout(Duration::from_millis(1), pending.connect()).await
        });
        match connected {
            Ok(result) => result?,
            Err(_) => return Err(io::ErrorKind::WouldBlock.into()),
        }
        // Keep a server instance alive continuously: create the next listener
        // before handing this connected instance to the connection registry.
        let next = create_pipe(&self.path, false).map_err(|e| io::Error::other(e.to_string()))?;
        let connected = std::mem::replace(&mut *pending, next);
        Ok((Stream::new(Pipe::Server(connected)), ()))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let pending = self.pending.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = pending.disconnect();
        // Cancel overlapped connect/read operations before Tokio retires their
        // completion records. This needs no new descriptor or wake connection.
        unsafe { CancelIoEx(pending.as_raw_handle(), std::ptr::null()) };
    }
}

enum Pipe {
    Server(NamedPipeServer),
    Client(NamedPipeClient),
}

impl Pipe {
    fn close(&self) {
        let handle = match self {
            Self::Server(pipe) => {
                // Disconnect synchronously closes the peer's transport; Mio
                // may retain its handle until cancelled I/O completions drain.
                let _ = pipe.disconnect();
                pipe.as_raw_handle()
            }
            Self::Client(pipe) => pipe.as_raw_handle(),
        };
        unsafe { CancelIoEx(handle, std::ptr::null()) };
    }
}

struct State {
    pipe: Option<Arc<Pipe>>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
    reader: Option<Waker>,
    writer: Option<Waker>,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(pipe) = self.pipe.take() {
            pipe.close();
        }
    }
}

pub(super) struct Stream(Arc<Mutex<State>>);

impl Stream {
    fn new(pipe: Pipe) -> Self {
        Self(Arc::new(Mutex::new(State {
            pipe: Some(Arc::new(pipe)),
            read_timeout: None,
            write_timeout: None,
            reader: None,
            writer: None,
        })))
    }

    /// Busy instances still have an owner: wait within the caller's I/O bound.
    /// The default SECURITY_IDENTIFICATION prevents a squatter impersonating us.
    /// Inspect the actual connected handle before sending any graph or text.
    pub(super) fn connect_checked(
        path: &Path,
        io_timeout: Duration,
    ) -> Result<Option<Self>, OwnerError> {
        let runtime = runtime().map_err(|e| OwnerError::Start(e.to_string()))?;
        let _entered = runtime.enter();
        let deadline = Instant::now() + io_timeout;
        let name = security::wide(path.as_os_str());
        let pipe = loop {
            let error = match ClientOptions::new().open(path) {
                Ok(pipe) => break pipe,
                Err(e) => e,
            };
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_FILE_NOT_FOUND) => return Ok(None),
                Some(ERROR_PIPE_BUSY) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(OwnerError::Start(
                            "owner pipe is busy (connection timed out)".into(),
                        ));
                    }
                    // Zero asks for the pipe's default wait, and u32::MAX is
                    // infinite. Use neither; subsequent opens share a deadline.
                    let millis = remaining.as_millis().clamp(1, u32::MAX as u128 - 1) as u32;
                    if unsafe { WaitNamedPipeW(name.as_ptr(), millis) } != 0 {
                        continue; // Another client may win the open race.
                    }
                    let error = io::Error::last_os_error();
                    match error.raw_os_error().map(|code| code as u32) {
                        Some(ERROR_FILE_NOT_FOUND) => return Ok(None),
                        Some(ERROR_SEM_TIMEOUT) => {
                            return Err(OwnerError::Start(
                                "owner pipe is busy (connection timed out)".into(),
                            ))
                        }
                        _ => {
                            return Err(OwnerError::Start(format!(
                                "cannot wait for owner pipe: {error}"
                            )))
                        }
                    }
                }
                Some(ERROR_ACCESS_DENIED) => {
                    return Err(OwnerError::Unsafe(format!(
                        "owner pipe access denied: {error}"
                    )))
                }
                _ => {
                    return Err(OwnerError::Start(format!(
                        "cannot connect to owner pipe: {error}"
                    )))
                }
            }
        };
        security::check_handle(pipe.as_raw_handle(), false)?;
        Ok(Some(Self::new(Pipe::Client(pipe))))
    }

    #[cfg(test)]
    pub(super) fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_checked(path.as_ref(), super::endpoint::IO_TIMEOUT)
            .map_err(|e| io::Error::other(e.to_string()))?
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    pub(super) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self(self.0.clone()))
    }

    #[cfg(test)]
    pub(super) fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.set_read_timeout(if nonblocking {
            Some(Duration::ZERO)
        } else {
            None
        })
    }

    pub(super) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_timeout = timeout;
        Ok(())
    }

    pub(super) fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_timeout = timeout;
        Ok(())
    }

    pub(super) fn shutdown(&self, how: std::net::Shutdown) -> io::Result<()> {
        // Pipes have no half-close. Tests using raw framed requests can leave
        // their write side idle while reading the owner's terminal reply.
        if how == std::net::Shutdown::Write {
            return Ok(());
        }
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Disconnect before acknowledging closure, even while clones survive.
        if let Some(pipe) = state.pipe.take() {
            pipe.close();
        }
        let (reader, writer) = (state.reader.take(), state.writer.take());
        drop(state);
        if let Some(waker) = reader {
            waker.wake();
        }
        if let Some(waker) = writer {
            waker.wake();
        }
        Ok(())
    }

    fn pipe_io(
        &self,
        writing: bool,
        mut operation: impl FnMut(&Pipe) -> io::Result<usize>,
    ) -> io::Result<usize> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let io_timeout = if writing {
            state.write_timeout
        } else {
            state.read_timeout
        };
        drop(state);
        runtime()?.block_on(async {
            let operation = std::future::poll_fn(|cx| {
                let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
                if writing {
                    state.writer = Some(cx.waker().clone());
                } else {
                    state.reader = Some(cx.waker().clone());
                }
                let Some(pipe) = &state.pipe else {
                    return Poll::Ready(if writing {
                        Err(io::ErrorKind::BrokenPipe.into())
                    } else {
                        Ok(0)
                    });
                };
                let pipe = pipe.as_ref();
                loop {
                    let ready = match (pipe, writing) {
                        (Pipe::Server(p), false) => p.poll_read_ready(cx),
                        (Pipe::Server(p), true) => p.poll_write_ready(cx),
                        (Pipe::Client(p), false) => p.poll_read_ready(cx),
                        (Pipe::Client(p), true) => p.poll_write_ready(cx),
                    };
                    std::task::ready!(ready)?;
                    match operation(pipe) {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                        result => return Poll::Ready(result),
                    }
                }
            });
            match io_timeout {
                Some(io_timeout) => tokio::time::timeout(io_timeout, operation)
                    .await
                    .unwrap_or_else(|_| {
                        Err(if io_timeout.is_zero() {
                            io::ErrorKind::WouldBlock
                        } else {
                            io::ErrorKind::TimedOut
                        }
                        .into())
                    }),
                None => operation.await,
            }
        })
    }
}

impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let result = self.pipe_io(false, |pipe| match pipe {
            Pipe::Server(p) => p.try_read(bytes),
            Pipe::Client(p) => p.try_read(bytes),
        });
        match result {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::BrokenPipe | io::ErrorKind::NotConnected
                ) || matches!(
                    e.raw_os_error().map(|code| code as u32),
                    Some(ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED)
                ) =>
            {
                // DisconnectNamedPipe forces the client off the pipe. Rust
                // does not classify ERROR_PIPE_NOT_CONNECTED (233) as
                // NotConnected. At this established-stream boundary both
                // 232 (closing) and 233 mean EOF, including after a complete
                // terminal reply. Writes must still report disconnection.
                Ok(0)
            }
            result => result,
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        self.pipe_io(true, |pipe| match pipe {
            Pipe::Server(p) => p.try_write(bytes),
            Pipe::Client(p) => p.try_write(bytes),
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        // Writes have already been submitted to Windows by Mio. These local
        // byte pipes use PIPE_WAIT (not PIPE_NOWAIT), so Windows completes a
        // write in full. The server's FlushFileBuffers below waits for those
        // bytes to be read before disconnect. Client requests stay connected
        // until their reply, as with Tokio's no-op client flush.
        // Do not probe with try_write(&[]): that submits another WriteFile,
        // whose zero-byte read at the peer can be mistaken for protocol EOF.
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let pipe = state.pipe.clone().ok_or(io::ErrorKind::BrokenPipe)?;
        let io_timeout = state.write_timeout.unwrap_or(super::endpoint::IO_TIMEOUT);
        drop(state);
        let Pipe::Server(server) = pipe.as_ref() else {
            return Ok(());
        };
        flush_server(server, io_timeout)
    }
}

/// DisconnectNamedPipe discards unread bytes. Unlike Tokio's no-op flush,
/// FlushFileBuffers witnesses that the client consumed the reply. Bound its
/// synchronous wait with a runtime timer cancelling this worker's synchronous
/// I/O. Repeated cancellation closes the race before entry into the OS call;
/// the active mutex prevents cancellation spilling into the worker's next call.
fn flush_server(server: &NamedPipeServer, io_timeout: Duration) -> io::Result<()> {
    struct Watch {
        thread: OwnedHandle,
        active: Mutex<bool>,
        expired: AtomicBool,
    }
    let raw = unsafe { OpenThread(THREAD_TERMINATE, 0, GetCurrentThreadId()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let watch = Arc::new(Watch {
        thread: unsafe { OwnedHandle::from_raw_handle(raw) },
        active: Mutex::new(true),
        expired: AtomicBool::new(false),
    });
    let watching = watch.clone();
    let timer = runtime()?.spawn(async move {
        tokio::time::sleep(io_timeout).await;
        loop {
            {
                let active = watching.active.lock().unwrap_or_else(|e| e.into_inner());
                if !*active {
                    break;
                }
                watching.expired.store(true, Ordering::Release);
                unsafe { CancelSynchronousIo(watching.thread.as_raw_handle()) };
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });
    let result = if unsafe { FlushFileBuffers(server.as_raw_handle()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    *watch.active.lock().unwrap_or_else(|e| e.into_inner()) = false;
    timer.abort();
    if watch.expired.load(Ordering::Acquire) {
        return Err(io::ErrorKind::TimedOut.into());
    }
    result
}

/// Rust's append-only Windows handle lacks FILE_WRITE_DATA, which set_len
/// needs. Reopen the same file object with truncate rights; never derive a
/// path from stderr or reopen a possibly replaced startup-log pathname.
pub(super) fn truncate_start_log(log: &std::fs::File) -> io::Result<()> {
    use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        ReOpenFile, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let raw = unsafe {
        ReOpenFile(
            log.as_raw_handle(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            0,
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let writable = unsafe { std::fs::File::from_raw_handle(raw) };
    writable.set_len(0)
}

/// A CLI launched with captured output inherits inheritable stdout/stderr
/// handles. Rust's Command on Windows inherits all such handles, even when
/// the child's standard streams are redirected elsewhere. A detached serve
/// must not retain the CLI's capture pipes: their reader would never get EOF.
/// Clearing the flag keeps these handles usable; Command duplicates the
/// explicitly selected log/NUL handles as inheritable when it spawns serve.
pub(super) fn detach_stdio() -> io::Result<()> {
    use windows_sys::Win32::Foundation::{
        SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for standard in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(standard) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Stream, Stream) {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        let client = Stream::connect(&path).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match listener.accept() {
                Ok((server, _)) => return (server, client),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(std::time::Instant::now() < deadline)
                }
                Err(e) => panic!("{e}"),
            }
        }
    }

    fn name() -> std::path::PathBuf {
        security::pipe_name(&format!(
            "{:016x}",
            crate::settings::graph_hash(Path::new(&uuid::Uuid::new_v4().to_string()))
        ))
        .unwrap()
    }

    #[test]
    fn pipe_is_private_and_a_squatted_first_instance_is_refused() {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        security::check_handle(listener.pending.lock().unwrap().as_raw_handle(), false).unwrap();
        let started = Instant::now();
        assert!(matches!(Listener::bind(&path), Err(OwnerError::Unsafe(_))));
        assert!(started.elapsed() >= PREDECESSOR_WAIT);
        assert!(started.elapsed() < PREDECESSOR_WAIT + Duration::from_secs(2));
        let _client = Stream::connect(&path).unwrap();
        drop(listener);
        drop(_client);
        // IOCP may retire cancelled operations after Drop returns. The name
        // must become reusable once those final handle references drain.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if Listener::bind(&path).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "last handle retained the pipe name"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_busy_pipe_waits_for_the_next_instance_instead_of_reporting_no_owner() {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        let _first = Stream::connect(&path).unwrap();
        let runtime = runtime().unwrap();
        let _entered = runtime.enter();
        assert_eq!(
            ClientOptions::new()
                .open(&path)
                .err()
                .unwrap()
                .raw_os_error(),
            Some(ERROR_PIPE_BUSY as i32)
        );
        let (done, finished) = std::sync::mpsc::channel();
        let (ready, running) = std::sync::mpsc::channel();
        let second_path = path.clone();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            assert!(done
                .send(Stream::connect_checked(
                    &second_path,
                    Duration::from_secs(2)
                ))
                .is_ok());
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            finished.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        let deadline = Instant::now() + Duration::from_secs(2);
        let _accepted = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline)
                }
                Err(e) => panic!("{e}"),
            }
        };
        // Like the production accept loop, poll the newly created instance's
        // ConnectNamedPipe operation so WaitNamedPipe can observe it waiting.
        let _second_server = loop {
            match listener.accept() {
                Ok(accepted) => break accepted,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline)
                }
                Err(e) => panic!("{e}"),
            }
        };
        let _second = finished
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap()
            .expect("busy is not no owner");
        worker.join().unwrap();
    }

    #[test]
    fn a_busy_pipe_times_out_as_an_error_but_a_missing_pipe_has_no_owner() {
        let path = name();
        assert!(Stream::connect_checked(&path, Duration::from_millis(100))
            .unwrap()
            .is_none());
        let _listener = Listener::bind(&path).unwrap();
        let _first = Stream::connect(&path).unwrap();
        let started = Instant::now();
        assert!(matches!(
            Stream::connect_checked(&path, Duration::from_millis(100)),
            Err(OwnerError::Start(message)) if message.contains("busy") && message.contains("timed out")
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn first_instance_bind_waits_for_a_predecessor_to_release_the_name() {
        let path = name();
        let descriptor = Descriptor::private(false).unwrap();
        // A held instance models the name retained by a predecessor's client
        // or delayed IOCP handle retirement. No disconnect before dropping it.
        let predecessor = create_with(&path, true, &descriptor).unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let (ready, running) = std::sync::mpsc::channel();
        let next_path = path.clone();
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            assert!(done.send(Listener::bind(&next_path)).is_ok());
        });
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            finished.recv_timeout(Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(predecessor);
        let successor = finished
            .recv_timeout(PREDECESSOR_WAIT + Duration::from_secs(2))
            .unwrap()
            .expect("a draining predecessor must not disable the endpoint");
        let _client = Stream::connect(&path).unwrap();
        drop(successor);
        worker.join().unwrap();
    }

    #[test]
    fn client_refuses_a_pipe_granting_another_account_read_or_write() {
        for access in ["FR", "FW"] {
            let path = name();
            let sid = security::user_sid().unwrap();
            let descriptor =
                Descriptor::from_sddl(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;{access};;;WD)"))
                    .unwrap();
            let _squatter = create_with(&path, true, &descriptor).unwrap();
            assert!(
                matches!(
                    Stream::connect_checked(&path, Duration::from_secs(2)),
                    Err(OwnerError::Unsafe(_))
                ),
                "{access}"
            );
        }
    }

    #[test]
    fn shutdown_closes_clones_and_wakes_a_blocked_read_without_new_handles() {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        let mut client = Stream::connect(&path).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (server, _) = loop {
            if let Ok(accepted) = listener.accept() {
                break accepted;
            }
            assert!(std::time::Instant::now() < deadline);
        };
        let mut blocked = server.try_clone().unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            done.send(blocked.read(&mut [0])).unwrap();
        });
        server.shutdown(std::net::Shutdown::Both).unwrap();
        assert_eq!(
            finished
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap(),
            0
        );
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        assert_eq!(client.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn flushing_frames_never_injects_a_false_eof_into_either_direction() {
        let (mut server, mut client) = pair();
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            for _ in 0..4 {
                let mut request = [0; 2];
                server.read_exact(&mut request).unwrap();
                assert_eq!(&request, b"q\n");
                server.write_all(b"a\n").unwrap();
                server.flush().unwrap();
            }
            done.send(()).unwrap();
        });
        for _ in 0..4 {
            client.write_all(b"q\n").unwrap();
            client.flush().unwrap();
            let mut reply = [0; 2];
            client.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"a\n");
        }
        finished.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn terminal_reply_is_delivered_whole_before_disconnect() {
        let (mut server, mut client) = pair();
        let payload = vec![b'x'; 128 * 1024];
        let expected = payload.clone();
        let worker = std::thread::spawn(move || {
            server
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            server.write_all(&payload).unwrap();
            server.flush().unwrap();
            server.shutdown(std::net::Shutdown::Both).unwrap();
        });
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut actual = Vec::new();
        client.read_to_end(&mut actual).unwrap();
        worker.join().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn a_peer_that_does_not_read_cannot_hold_the_reply_flush() {
        let (mut server, _client) = pair();
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            server
                .set_write_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            server.write_all(&vec![b'x'; 16 * 1024]).unwrap();
            done.send(server.flush().unwrap_err().kind()).unwrap();
        });
        assert_eq!(
            finished.recv_timeout(Duration::from_secs(3)).unwrap(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn an_append_only_start_log_can_be_truncated_and_keeps_appending() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("start.log");
        std::fs::write(&path, b"old startup log").unwrap();
        let mut append = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        truncate_start_log(&append).unwrap();
        append.write_all(b"next diagnostic\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"next diagnostic\n");
    }
}
