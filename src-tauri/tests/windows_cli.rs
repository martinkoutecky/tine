//! Literal CLI journey on Windows. Uses the real per-account coordination
//! namespace, a fresh temporary graph, and portable app data beside a copied
//! executable. Hosted Windows CI is the runtime proof for this test.
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Cli {
    exe: PathBuf,
    graph: PathBuf,
}

impl Cli {
    fn call(&self, args: &[&str]) -> Output {
        Command::new(&self.exe)
            .args(args)
            .arg("--graph")
            .arg(&self.graph)
            .arg("--json")
            .env_remove("TINE_RUN")
            .env_remove("TINE_AUTHOR")
            .output()
            .unwrap()
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
        // `serve` takes its graph positionally; --json belongs to graph verbs.
        let output = Command::new(&self.exe)
            .arg("serve")
            .arg(&self.graph)
            .arg("--stop")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        // Best effort cleanup also when an assertion fails: no detached serve
        // should retain a test's portable executable or temporary graph.
        let _ = Command::new(&self.exe)
            .arg("serve")
            .arg(&self.graph)
            .arg("--stop")
            .output();
    }
}

#[test]
fn windows_cli_round_trip_auto_starts_reads_writes_refuses_stale_and_stops() {
    let data = tempfile::tempdir().unwrap();
    let exe = data.path().join("tine.exe");
    std::fs::copy(env!("CARGO_BIN_EXE_tine"), &exe).unwrap();
    std::fs::create_dir(data.path().join("tine-data")).unwrap();
    let graph = data.path().join("graph with spaces");
    std::fs::create_dir_all(graph.join("pages")).unwrap();
    std::fs::create_dir_all(graph.join("logseq")).unwrap();
    std::fs::write(graph.join("pages/a.md"), "- original\n").unwrap();
    let cli = Cli { exe, graph };
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
