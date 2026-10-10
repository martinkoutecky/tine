//! Exclusive advisory locks on a lock file (og-backup-cas D1), shared by the
//! backup namespaces and the flight store.
//!
//! Every shipped target names its arm. Linux, Android, macOS and iOS share
//! `unix_flock` (`flock(2)`), so the code the Linux tests run is the code
//! Android and Apple run; Windows uses std's `LockFileEx`. Rust's std
//! `File::try_lock` answers `Unsupported` on Android, which silently turned
//! these locks off there; any other target is a compile error, never a
//! runtime fallback (AGENTS §2: a `cfg` list names every shipped target).
//!
//! A wait is a contention bound only: it polls with capped backoff on the
//! caller's own thread until its deadline and spawns nothing. It cannot
//! interrupt a lock syscall that hangs.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    target_os = "windows"
)))]
compile_error!(
    "file_lock: name this target's lock arm (AGENTS §2: a cfg list names every shipped target)"
);

/// An exclusive lock, held until drop. It owns the opened lock file for the
/// whole operation; the file is never unlinked or replaced.
pub(crate) struct FileLock(File);

/// Unlock explicitly: a descriptor a spawned child inherited mid-exec must
/// not keep the lock.
impl Drop for FileLock {
    fn drop(&mut self) {
        unlock_file(&self.0);
    }
}

/// Open (creating if absent) a lock file; Windows gets read/write access
/// with the default sharing.
fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Lock once. `Ok(None)`: another handle holds it.
pub(crate) fn try_lock_exclusive(path: &Path) -> io::Result<Option<FileLock>> {
    let file = open_lock_file(path)?;
    Ok(try_lock_file(&file)?.then_some(FileLock(file)))
}

/// Lock, retrying contention until `deadline` (10 ms backoff doubling to
/// 200 ms). `Ok(None)`: still held at the deadline. Any error other than
/// contention is returned at once.
pub(crate) fn lock_exclusive_until(path: &Path, deadline: Instant) -> io::Result<Option<FileLock>> {
    let file = open_lock_file(path)?;
    let mut pause = Duration::from_millis(10);
    loop {
        if try_lock_file(&file)? {
            return Ok(Some(FileLock(file)));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        std::thread::sleep(pause.min(left));
        pause = (pause * 2).min(Duration::from_millis(200));
    }
}

/// Take `file`'s exclusive lock without waiting. `Ok(false)` is contention;
/// any other error is returned, never read as success.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
pub(crate) fn try_lock_file(file: &File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;
    unix_flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB)
}

#[cfg(target_os = "windows")]
pub(crate) fn try_lock_file(file: &File) -> io::Result<bool> {
    // std maps ERROR_LOCK_VIOLATION to `WouldBlock`.
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
pub(crate) fn unlock_file(file: &File) {
    use std::os::fd::AsRawFd;
    let _ = unix_flock(file.as_raw_fd(), libc::LOCK_UN);
}

#[cfg(target_os = "windows")]
pub(crate) fn unlock_file(file: &File) {
    let _ = file.unlock();
}

/// `flock(2)`, retrying `EINTR`. `Ok(false)` only for `EWOULDBLOCK`.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn unix_flock(fd: std::os::fd::RawFd, operation: libc::c_int) -> io::Result<bool> {
    loop {
        // SAFETY: `flock` reads no memory; `fd` belongs to the caller's open `File`.
        if unsafe { libc::flock(fd, operation) } == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(code) if code == libc::EWOULDBLOCK => return Ok(false),
            _ => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Write};
    #[cfg(unix)]
    use std::os::fd::AsRawFd;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-file-lock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// AGENTS §2: a cfg list names every shipped target. Exemplar: std's
    /// `File::try_lock` is `Unsupported` on Android (Rust 1.96), which turned
    /// the backup lock off there while Linux tests passed.
    #[test]
    fn lock_arms_name_every_shipped_target() {
        let source = include_str!("file_lock.rs");
        let production = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        let cfg_before = |item: &str| -> Vec<String> {
            production
                .match_indices(item)
                .map(|(at, _)| {
                    let head = &production[..at];
                    let start = head.rfind("#[cfg(").expect("arm has a cfg");
                    head[start..].to_owned()
                })
                .collect()
        };
        let unix = ["linux", "android", "macos", "ios"];
        let guard = &production[production.find("#[cfg(not(any(").unwrap()..];
        let guard = &guard[..guard.find("compile_error!").unwrap()];
        for os in unix.iter().chain(&["windows"]) {
            assert!(
                guard.contains(&format!("target_os = \"{os}\"")),
                "AGENTS §2: the compile_error guard must name {os}"
            );
        }
        for item in ["fn try_lock_file", "fn unlock_file"] {
            let arms = cfg_before(item);
            assert_eq!(
                arms.len(),
                2,
                "{item}: one unix-family arm and one Windows arm"
            );
            assert!(
                unix.iter()
                    .all(|os| arms[0].contains(&format!("target_os = \"{os}\""))),
                "AGENTS §2 (Android std lock is Unsupported): {item}'s flock arm must name {unix:?}: {}",
                arms[0]
            );
            assert!(
                arms[1].contains("target_os = \"windows\""),
                "{item}: {}",
                arms[1]
            );
        }
        let unix_arm = &production[production.find("fn try_lock_file").unwrap()..];
        assert!(
            unix_arm[..unix_arm.find("\n}\n").unwrap()].contains("unix_flock("),
            "the Android/Apple arm is the shared unix_flock the Linux tests run"
        );
    }

    /// The Linux arm (the same code Android and Apple compile) takes a real
    /// `flock`: a direct `flock` on a separate open file contends with it,
    /// and dropping the guard releases it.
    #[cfg(unix)]
    #[test]
    fn the_unix_arm_takes_a_flock_and_drop_releases_it() {
        let dir = scratch("flock");
        let path = dir.join("lock");
        let guard = try_lock_exclusive(&path).unwrap().expect("free lock");
        let other = File::open(&path).unwrap();
        // SAFETY: `other` is open for the duration of each call.
        let contended = unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        assert_eq!(contended, -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::EWOULDBLOCK)
        );
        assert!(
            try_lock_exclusive(&path).unwrap().is_none(),
            "a second handle contends"
        );
        drop(guard);
        // SAFETY: as above.
        assert_eq!(
            unsafe { libc::flock(other.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_error_is_returned_not_read_as_success_or_contention() {
        let error = unix_flock(-1, libc::LOCK_EX | libc::LOCK_NB).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
        let dir = scratch("error");
        assert!(try_lock_exclusive(&dir.join("missing").join("lock")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Child mode for the two-process tests: hold the lock at
    /// `TINE_FILE_LOCK_HOLD` until stdin closes or a line arrives.
    #[test]
    fn lock_holder_child() {
        let Ok(path) = std::env::var("TINE_FILE_LOCK_HOLD") else {
            return;
        };
        let guard = try_lock_exclusive(Path::new(&path))
            .unwrap()
            .expect("child locks first");
        println!("HOLDER:locked");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        drop(guard);
    }

    /// The holder process, and its stdout kept open until it exits.
    struct Holder(
        std::process::Child,
        std::io::BufReader<std::process::ChildStdout>,
    );

    impl Holder {
        fn spawn(path: &Path) -> Self {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "file_lock::tests::lock_holder_child",
                    "--exact",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("TINE_FILE_LOCK_HOLD", path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
            let mut line = String::new();
            while !line.contains("HOLDER:locked") {
                line.clear();
                assert_ne!(out.read_line(&mut line).unwrap(), 0, "holder exited early");
            }
            Self(child, out)
        }
    }

    impl Drop for Holder {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Two processes with separate handles: contention, a bounded wait that
    /// times out, and the lock passing on when its holder dies.
    #[test]
    fn another_processs_lock_contends_times_out_and_dies_with_it() {
        let dir = scratch("two-process");
        let path = dir.join("lock");
        let mut holder = Holder::spawn(&path);
        assert!(try_lock_exclusive(&path).unwrap().is_none(), "contention");
        let started = Instant::now();
        let waited = lock_exclusive_until(&path, started + Duration::from_millis(300)).unwrap();
        assert!(waited.is_none(), "times out while held");
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "bounded: {:?}",
            started.elapsed()
        );
        // The holder dies without unlocking; the OS releases its lock.
        holder.0.kill().unwrap();
        holder.0.wait().unwrap();
        let after_death =
            lock_exclusive_until(&path, Instant::now() + Duration::from_secs(5)).unwrap();
        assert!(after_death.is_some(), "a dead holder's lock passes on");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A waiter gets the lock when the other process releases it, before
    /// its deadline, without a thread of its own.
    #[test]
    fn a_bounded_wait_gets_a_lock_released_before_its_deadline() {
        let dir = scratch("handoff");
        let path = dir.join("lock");
        let mut holder = Holder::spawn(&path);
        let mut stdin = holder.0.stdin.take().unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            writeln!(stdin).unwrap();
        });
        let started = Instant::now();
        let got = lock_exclusive_until(&path, started + Duration::from_secs(10)).unwrap();
        assert!(got.is_some());
        assert!(started.elapsed() < Duration::from_secs(5));
        releaser.join().unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut holder.1, &mut rest).unwrap();
        assert!(holder.0.wait().unwrap().success(), "{rest}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
