//! GH #701: a termination signal is an ordinary Quit on Linux and macOS.
//!
//! Shutting down or logging out sends SIGTERM (systemd, launchd), often
//! followed by SIGHUP; closing a terminal sends SIGHUP and Ctrl+C sends
//! SIGINT. Their default action ended the process at once, so
//! `RunEvent::Exit` never ran and the unclean-exit marker (ADR 0058) stayed:
//! every next launch said Tine "did not close cleanly".
//!
//! The first signal runs the tray's Quit: every graph window gets a close
//! request, so the frontend's flush runs, and the last window exits through
//! the retirement waiter. A shutdown or a terminal signals the whole process
//! group, WebKit's helpers included, and those die at once, so a window may
//! never answer its close request. After `FORCE_AFTER` the graph windows are
//! therefore destroyed instead: their hosts still retire (saves the host
//! already holds reach disk) and the process exits through the same waiter
//! (G6), so `RunEvent::Exit` clears the marker. Further signals are ignored:
//! the exit is already bounded, and a SIGHUP that follows SIGTERM must not cut
//! the flush short. A signal inherited as ignored (`nohup`) stays ignored.
//!
//! Windows sign-out and shutdown reach `RunEvent::Exit` through tao's
//! WM_ENDSESSION handling (GH #455). Android and iOS end the session on
//! visibility (`src/sessionActivity.ts`). Neither installs a handler.

use std::io::Read;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;
use tauri::Manager;

const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGHUP, libc::SIGINT];
/// How long the windows get to flush before they are destroyed.
const FORCE_AFTER: Duration = Duration::from_secs(5);

/// Whether `signal` is currently ignored (inherited from `nohup` or a parent).
fn inherited_ignore(signal: libc::c_int) -> bool {
    // SAFETY: a null `act` only reads the current disposition into `old`.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        libc::sigaction(signal, std::ptr::null(), &mut old) == 0
            && old.sa_sigaction == libc::SIG_IGN
    }
}

/// Install the handlers. A failure leaves the default actions in place, which
/// is the old behaviour; it never blocks startup.
pub(crate) fn install(app: &tauri::AppHandle) {
    let (mut reader, writer) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(_) => {
            crate::debug::diag("quit-signal-unavailable");
            return;
        }
    };
    // Non-blocking, so a flood of signals can never block the handler on a
    // full buffer; the write end lives until exit, as the handler may run then.
    if writer.set_nonblocking(true).is_err() {
        crate::debug::diag("quit-signal-unavailable");
        return;
    }
    let fd = writer.into_raw_fd();
    for signal in SIGNALS {
        if inherited_ignore(signal) {
            continue;
        }
        // SAFETY: the action only calls write(2) on an fd it owns, which is
        // async-signal-safe; it allocates nothing and takes no lock.
        let registered = unsafe {
            signal_hook_registry::register(signal, move || {
                let byte = signal as u8;
                libc::write(fd, (&byte as *const u8).cast(), 1);
            })
        };
        if registered.is_err() {
            crate::debug::diag("quit-signal-unavailable");
        }
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        if reader.read_exact(&mut byte).is_err() {
            return;
        }
        crate::debug::diag("quit-signal");
        let quit = app.clone();
        let _ = app.run_on_main_thread(move || crate::tray::quit(&quit));
        std::thread::sleep(FORCE_AFTER);
        crate::debug::diag("quit-signal-force");
        let force = app.clone();
        let _ = app.run_on_main_thread(move || {
            for (label, window) in force.webview_windows() {
                if crate::graph_window(&label) {
                    let _ = window.destroy();
                }
            }
        });
        // Later signals land in the pipe unread: the exit is already bounded.
    });
}
