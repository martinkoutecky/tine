#![cfg(feature = "test-faults")]
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tine_store::{FaultPoint, OpenOptions, Store};

fn store(root: &Path) -> Store {
    Store::open(root, OpenOptions::default()).unwrap().0
}

#[test]
fn k1_merge_race_worker() {
    let Ok(root) = std::env::var("TINE_K1_MERGE_ROOT") else {
        return;
    };
    let graph = store(Path::new(&root));
    if let Ok(boundary) = std::env::var("TINE_K1_KILL_BOUNDARY") {
        graph.inject_fault(FaultPoint::AbortAfterStep(boundary.parse().unwrap()));
    }
    let result = tine_graph_features::pages::merge_pages(&graph, None, "pages/A.md", "pages/B.md");
    eprintln!("merge worker result: {result:?}");
}

fn run_race(kill: Option<usize>, alternate_target: bool) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir(root.join(dir)).unwrap();
    }
    fs::write(root.join("pages/A.md"), b"- keep\n").unwrap();
    fs::write(root.join("pages/B.md"), b"- survivor\n- keep\n").unwrap();
    let target = if alternate_target {
        let alias = root.join("assets/race-target.md");
        fs::hard_link(root.join("pages/B.md"), &alias).unwrap();
        alias
    } else {
        root.join("pages/B.md")
    };
    let barrier = root.join("barrier");
    let stdout = root.join("worker.stdout");
    let stderr = root.join("worker.stderr");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "k1_merge_race_worker", "--nocapture"])
        .env("TINE_K1_MERGE_ROOT", root)
        .env("TINE_K1_RACE_TARGET", target)
        .env("TINE_K1_RACE_BARRIER", &barrier)
        .stdout(Stdio::from(fs::File::create(&stdout).unwrap()))
        .stderr(Stdio::from(fs::File::create(&stderr).unwrap()));
    if let Some(boundary) = kill {
        command.env("TINE_K1_KILL_BOUNDARY", boundary.to_string());
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !root.join("barrier.ready").exists() {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "worker never reached apply after preflight\nstdout:\n{}\nstderr:\n{}",
                fs::read_to_string(&stdout).unwrap(),
                fs::read_to_string(&stderr).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Honest external editor, separate process from the Store and its locks.
    fs::write(root.join("pages/B.md"), b"- survivor\n").unwrap();
    fs::write(root.join("barrier.resume"), b"resume").unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "merge worker timed out\nstdout:\n{}\nstderr:\n{}",
                fs::read_to_string(&stdout).unwrap(),
                fs::read_to_string(&stderr).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        status.success(),
        kill.is_none(),
        "kill injection must actually terminate the merge worker"
    );
    let graph = store(root);
    let source_live = fs::read(root.join("pages/A.md")).is_ok_and(|bytes| bytes == b"- keep\n");
    let survivor_has_payload = fs::read(root.join("pages/B.md"))
        .unwrap()
        .windows(6)
        .any(|bytes| bytes == b"- keep");
    assert!(
        source_live || survivor_has_payload,
        "I-2: raced merge retired the only live payload; exemplar Transaction::apply"
    );
    graph.close();
}

#[test]
fn k1_merge_external_editor_race_never_silently_retires_payload() {
    run_race(None, false);
}

#[test]
fn k1_merge_external_editor_race_kill_and_reopen_keeps_payload_live() {
    for boundary in 0..2 {
        run_race(Some(boundary), false);
    }
}

#[test]
fn k1_merge_race_barrier_matches_the_file_under_an_alternate_path() {
    run_race(None, true);
}
