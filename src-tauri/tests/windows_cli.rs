//! Literal CLI journey on Windows. Uses the real per-account coordination
//! namespace, a fresh temporary graph, and portable app data beside a copied
//! executable. Hosted Windows CI is the runtime proof for this test.
use std::io::{self, Read};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenThread, ResumeThread, WaitForSingleObject, CREATE_SUSPENDED,
    PROCESS_SYNCHRONIZE, THREAD_SUSPEND_RESUME,
};

const COMMAND_WAIT: Duration = Duration::from_secs(120);

/// A job retains even DETACHED_PROCESS descendants after their launcher exits.
/// Its non-inheritable handle kills the entire test tree on unwind or success.
struct Job(OwnedHandle);

impl Job {
    fn new() -> Self {
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        assert!(!raw.is_null(), "{}", io::Error::last_os_error());
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        assert_ne!(
            unsafe {
                SetInformationJobObject(
                    job.0.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            },
            0,
            "{}",
            io::Error::last_os_error()
        );
        job
    }

    fn spawn(&self, command: &mut Command) -> ChildGuard {
        // Assign before any user code runs: otherwise a fast CLI can spawn
        // serve before assignment, leaving that descendant outside the job.
        let child = ChildGuard(command.creation_flags(CREATE_SUSPENDED).spawn().unwrap());
        assert_ne!(
            unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.0.as_raw_handle()) },
            0,
            "{}",
            io::Error::last_os_error()
        );
        // Stable std exposes the process handle but not its initial thread.
        // A suspended process has exactly that thread; enumerate a snapshot.
        let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        assert_ne!(raw, INVALID_HANDLE_VALUE, "{}", io::Error::last_os_error());
        let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
        while found != 0 {
            if entry.th32OwnerProcessID == child.0.id() {
                let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                assert!(!raw.is_null(), "{}", io::Error::last_os_error());
                let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
                assert_ne!(
                    unsafe { ResumeThread(thread.as_raw_handle()) },
                    u32::MAX,
                    "{}",
                    io::Error::last_os_error()
                );
                return child;
            }
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
        }
        panic!("no initial thread for suspended CLI {}", child.0.id());
    }
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        // Do not replace a timeout with an unbounded wait during unwinding.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), 5_000) };
    }
}

struct Cli {
    exe: PathBuf,
    graph: PathBuf,
    job: Job,
}

impl Cli {
    fn run(&self, command: &mut Command) -> Output {
        eprintln!("Windows CLI: {command:?}");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("TINE_RUN")
            .env_remove("TINE_AUTHOR");
        let mut child = self.job.spawn(command);
        let mut stdout = child.0.stdout.take().unwrap();
        let mut stderr = child.0.stderr.take().unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let deadline = Instant::now() + COMMAND_WAIT;
        let status = loop {
            let status = child.0.try_wait().unwrap();
            let out_closed = drain(&mut stdout, &mut out);
            let err_closed = drain(&mut stderr, &mut err);
            // Process exit alone misses the original hang: a detached serve
            // retaining capture handles keeps these pipes open after CLI exit.
            if let Some(status) = status {
                if out_closed && err_closed {
                    break status;
                }
            }
            if Instant::now() >= deadline {
                panic!(
                    "CLI {} exceeded {COMMAND_WAIT:?}: {command:?}\nstatus: {status:?}, stdout closed: {out_closed}, stderr closed: {err_closed}\nstdout: {}\nstderr: {}",
                    child.0.id(),
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Output {
            status,
            stdout: out,
            stderr: err,
        }
    }

    fn call(&self, args: &[&str]) -> Output {
        self.run(
            Command::new(&self.exe)
                .args(args)
                .arg("--graph")
                .arg(&self.graph)
                .arg("--json"),
        )
    }
    fn ok(&self, args: &[&str]) -> serde_json::Value {
        let output = self.call(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn stop(&self) {
        let pid = self.ok(&["status"])["pid"].as_u64().unwrap() as u32;
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(
            !raw.is_null(),
            "cannot monitor owner {pid}: {}",
            io::Error::last_os_error()
        );
        let owner = unsafe { OwnedHandle::from_raw_handle(raw) };
        // `serve` takes its graph positionally; --json belongs to graph verbs.
        let output = self.run(
            Command::new(&self.exe)
                .arg("serve")
                .arg(&self.graph)
                .arg("--stop"),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            unsafe { WaitForSingleObject(owner.as_raw_handle(), 10_000) },
            WAIT_OBJECT_0,
            "owner {pid} replied to --stop but did not exit"
        );
    }
}

/// Only read bytes PeekNamedPipe says are available; no blocking pipe read or
/// wait_with_output thread can outlive the command deadline. One chunk per
/// poll also bounds a child producing output continuously.
fn drain(pipe: &mut (impl AsRawHandle + Read), output: &mut Vec<u8>) -> bool {
    let mut available = 0;
    if unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        assert!(
            matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_BROKEN_PIPE | ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED)
            ),
            "{error}"
        );
        return true;
    }
    if available > 0 {
        let mut bytes = [0; 8192];
        let count = pipe
            .read(&mut bytes[..(available as usize).min(8192)])
            .unwrap();
        output.extend_from_slice(&bytes[..count]);
        assert!(output.len() <= 1 << 20, "test CLI output exceeded 1 MiB");
    }
    false
}

fn fixture() -> (tempfile::TempDir, Cli) {
    let data = tempfile::tempdir().unwrap();
    let exe = data.path().join("tine.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_tine"), &exe).unwrap();
    std::fs::create_dir(data.path().join("tine-data")).unwrap();
    let graph = data.path().join("graph with spaces");
    std::fs::create_dir_all(graph.join("pages")).unwrap();
    std::fs::create_dir_all(graph.join("logseq")).unwrap();
    std::fs::write(graph.join("pages/a.md"), "- original\n").unwrap();
    let cli = Cli {
        exe,
        graph,
        job: Job::new(),
    };
    (data, cli)
}

#[test]
fn windows_cli_round_trip_auto_starts_reads_writes_refuses_stale_and_stops() {
    let (_data, cli) = fixture();
    let before = cli.ok(&["page", "show", "a"]); // automatically starts serve
    assert_eq!(cli.ok(&["status"])["owner"], "serve");
    let handle = format!(
        "a/{}",
        before["page"]["blocks"][0]["handle"].as_str().unwrap()
    );
    assert_eq!(
        cli.ok(&["edit", &handle, "changed over a named pipe"])["kind"],
        "written"
    );
    let after = cli.ok(&["page", "show", "a"]);
    assert!(after["text"]
        .as_str()
        .unwrap()
        .contains("changed over a named pipe"));
    assert!(std::fs::read_to_string(cli.graph.join("pages/a.md"))
        .unwrap()
        .contains("changed over a named pipe"));
    let stale = cli.call(&["edit", &handle, "must not overwrite"]);
    assert_eq!(stale.status.code(), Some(6));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&stale.stderr).unwrap()["error"]["code"],
        "stale"
    );
    cli.stop();
    // The yield reply precedes lease release. A new command must discover or
    // start the successor rather than assume the reply itself is permission.
    let deadline = Instant::now() + Duration::from_secs(10);
    assert_eq!(cli.ok(&["page", "show", "a"])["text"], after["text"]);
    assert!(Instant::now() < deadline);
    cli.stop();
}

#[test]
fn windows_cli_guard_kills_auto_started_serve_on_assertion_failure() {
    let (_data, cli) = fixture();
    let pid = cli.ok(&["status"])["pid"].as_u64().unwrap() as u32;
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    assert!(!raw.is_null(), "{}", io::Error::last_os_error());
    let owner = unsafe { OwnedHandle::from_raw_handle(raw) };
    assert!(std::panic::catch_unwind(move || {
        let _cli = cli;
        panic!("simulated journey assertion failure");
    })
    .is_err());
    assert_eq!(
        unsafe { WaitForSingleObject(owner.as_raw_handle(), 5_000) },
        WAIT_OBJECT_0,
        "cleanup retained detached serve {pid}"
    );
}
