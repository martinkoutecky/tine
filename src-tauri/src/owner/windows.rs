//! Synchronous transport adapter over Tokio's overlapped, local-only pipes.
//! The endpoint's existing accept thread owns registration/sealing; cancelling
//! I/O never needs a new handle, connection, or wake pipe.
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Poll, Waker};
use std::time::Duration;

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::runtime::{Builder, Runtime};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY};
use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
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
    create_with(path, first, &descriptor)
}

fn create_with(
    path: &Path,
    first: bool,
    descriptor: &Descriptor,
) -> Result<NamedPipeServer, OwnerError> {
    let runtime = runtime().map_err(|e| OwnerError::Start(e.to_string()))?;
    let _entered = runtime.enter();
    let mut attributes = descriptor.attributes();
    // Tokio copies the descriptor during CreateNamedPipe; it outlives that call.
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(path, std::ptr::from_mut(&mut attributes).cast())
    }
    .map_err(|e| {
        OwnerError::Unsafe(format!(
            "cannot create private owner pipe {} (possibly already occupied): {e}",
            path.display()
        ))
    })
}

pub(super) struct Listener {
    path: std::path::PathBuf,
    pending: Mutex<NamedPipeServer>,
}

impl Listener {
    pub(super) fn bind(path: &Path) -> Result<Self, OwnerError> {
        Ok(Self {
            path: path.to_owned(),
            pending: Mutex::new(create_pipe(path, true)?),
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

    /// Opening is nonblocking (ERROR_PIPE_BUSY means retry discovery). The
    /// default SECURITY_IDENTIFICATION prevents a squatter impersonating us.
    /// Inspect the actual connected handle before sending any graph or text.
    pub(super) fn connect_checked(path: &Path) -> Result<Option<Self>, OwnerError> {
        let runtime = runtime().map_err(|e| OwnerError::Start(e.to_string()))?;
        let _entered = runtime.enter();
        let pipe = match ClientOptions::new().open(path) {
            Ok(pipe) => pipe,
            Err(e) if matches!(e.raw_os_error(), Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32) => {
                return Ok(None)
            }
            Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                return Err(OwnerError::Unsafe(format!("owner pipe access denied: {e}")))
            }
            Err(e) => {
                return Err(OwnerError::Start(format!(
                    "cannot connect to owner pipe: {e}"
                )))
            }
        };
        security::check_handle(pipe.as_raw_handle(), false)?;
        Ok(Some(Self::new(Pipe::Client(pipe))))
    }

    #[cfg(test)]
    pub(super) fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::connect_checked(path.as_ref())
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
                ) =>
            {
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
        // Mio's try_write can enqueue an overlapped write and return before
        // completion. A subsequent empty write waits for that write to finish
        // (and surfaces its error) before a terminal reply closes the handle.
        self.pipe_io(true, |pipe| match pipe {
            Pipe::Server(p) => p.try_write(&[]),
            Pipe::Client(p) => p.try_write(&[]),
        })
        .map(|_| ())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Stream, Stream) {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        let client = Stream::connect_checked(&path).unwrap().unwrap();
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
        assert!(matches!(Listener::bind(&path), Err(OwnerError::Unsafe(_))));
        let _client = Stream::connect_checked(&path).unwrap().unwrap();
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
    fn client_refuses_a_pipe_granting_another_account_read_or_write() {
        for access in ["FR", "FW"] {
            let path = name();
            let sid = security::user_sid().unwrap();
            let descriptor =
                Descriptor::from_sddl(&format!("O:{sid}D:P(A;;FA;;;{sid})(A;;{access};;;WD)"))
                    .unwrap();
            let _squatter = create_with(&path, true, &descriptor).unwrap();
            assert!(
                matches!(Stream::connect_checked(&path), Err(OwnerError::Unsafe(_))),
                "{access}"
            );
        }
    }

    #[test]
    fn shutdown_closes_clones_and_wakes_a_blocked_read_without_new_handles() {
        let path = name();
        let listener = Listener::bind(&path).unwrap();
        let mut client = Stream::connect_checked(&path).unwrap().unwrap();
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
