//! GH #701: a termination signal is an ordinary Quit on Linux and macOS.
//!
//! Shutting down or logging out sends SIGTERM (systemd, launchd) and closing
//! a terminal sends SIGHUP. Their default action ends the process at once, so
//! `RunEvent::Exit` never ran: pending saves were not flushed and the
//! unclean-exit marker (ADR 0058) stayed, and every next launch reported that
//! Tine "did not close cleanly". The first SIGTERM, SIGHUP or SIGINT now runs
//! the tray's Quit (every graph window gets a close request, so the
//! frontend's flush runs and the last window exits through the retirement
//! waiter); a second one ends the process at once, still marked unclean.
//!
//! Windows sign-out and shutdown reach `RunEvent::Exit` through tao's
//! WM_ENDSESSION handling (GH #455). Android and iOS end the session on
//! visibility (`src/sessionActivity.ts`). Neither installs a handler.

use std::io::Read;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;

const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGHUP, libc::SIGINT];

/// Install the handlers. A failure leaves the default actions in place, which
/// is today's behaviour; it never blocks startup.
pub(crate) fn install(app: &tauri::AppHandle) {
    let (mut reader, writer) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(_) => {
            crate::debug::diag("quit-signal-unavailable");
            return;
        }
    };
    // The write end lives for the whole process: the handler may run at any
    // time until exit.
    let fd = writer.into_raw_fd();
    for signal in SIGNALS {
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
        let mut quitting = false;
        while reader.read_exact(&mut byte).is_ok() {
            if quitting {
                // Asked twice: the user or the session manager will not wait.
                std::process::exit(128 + i32::from(byte[0]));
            }
            quitting = true;
            crate::debug::diag("quit-signal");
            let quit = app.clone();
            let _ = app.run_on_main_thread(move || crate::tray::quit(&quit));
        }
    });
}
