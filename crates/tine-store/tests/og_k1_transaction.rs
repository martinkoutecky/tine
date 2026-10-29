#![cfg(feature = "test-faults")]
//! K1 (I-2): a merge raced by an honest external editor in another process,
//! and killed mid-way, never retires the only live payload. The merge is a
//! host operation (host-only-writer S1): the editor rewrites the survivor
//! after the merge planned and before the host admits it (another
//! reservation holds the survivor meanwhile), and a kill lands at one of
//! the host's phases; the relaunched host completes the operation.
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tine_store::host_faults::{abort_before, Phase};
use tine_store::{OpenOptions, PageHost, PageId, Store};

/// The host phases a killed merge stops before, by boundary.
const KILLS: [Phase; 2] = [Phase::PageRename, Phase::TrashMove];

fn store(root: &Path) -> std::sync::Arc<Store> {
    std::sync::Arc::new(Store::open(root, OpenOptions::default()).unwrap().0)
}

fn wait_for(path: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !Path::new(path).exists() {
        assert!(
            Instant::now() < deadline,
            "external-editor race barrier timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn k1_merge_race_worker() {
    let _integration_guard = crate::integration_lock();
    let Ok(root) = std::env::var("TINE_K1_MERGE_ROOT") else {
        return;
    };
    let app_data = std::env::var("TINE_K1_APP_DATA").unwrap();
    let barrier = std::env::var("TINE_K1_RACE_BARRIER").unwrap();
    let graph = store(Path::new(&root));
    graph.whole_graph().unwrap();
    if let Ok(boundary) = std::env::var("TINE_K1_KILL_BOUNDARY") {
        abort_before(KILLS[boundary.parse::<usize>().unwrap()], 0);
    }
    let host = PageHost::start_for_tests(&graph, Path::new(&app_data)).unwrap();
    let held = host.pause_for_tests(&[PageId::from("pages/B.md")]);
    std::thread::scope(|scope| {
        let merge = scope.spawn(|| {
            tine_graph_features::pages::merge_pages(&graph, &host, "pages/A.md", "pages/B.md")
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !merge.is_finished(),
            "the merge wrote under another reservation"
        );
        fs::write(format!("{barrier}.ready"), b"planned").unwrap();
        wait_for(&format!("{barrier}.resume"));
        drop(held);
        eprintln!("merge worker result: {:?}", merge.join().unwrap());
    });
}

fn run_race(kill: Option<usize>) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("graph");
    let app_data = scratch.path().join("app");
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    fs::write(root.join("pages/A.md"), b"- keep\n").unwrap();
    fs::write(root.join("pages/B.md"), b"- survivor\n- keep\n").unwrap();
    let barrier = root.join("barrier");
    let stdout = root.join("worker.stdout");
    let stderr = root.join("worker.stderr");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "og_k1_transaction::k1_merge_race_worker",
            "--nocapture",
        ])
        .env("TINE_K1_MERGE_ROOT", &root)
        .env("TINE_K1_APP_DATA", &app_data)
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
                "worker never planned the merge\nstdout:\n{}\nstderr:\n{}",
                fs::read_to_string(&stdout).unwrap(),
                fs::read_to_string(&stderr).unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Honest external editor, separate process from the Store and its host.
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
        "kill injection must actually terminate the merge worker\nstderr:\n{}",
        fs::read_to_string(&stderr).unwrap()
    );
    let live = || {
        let source_live = fs::read(root.join("pages/A.md")).is_ok_and(|bytes| bytes == b"- keep\n");
        let survivor_has_payload = fs::read(root.join("pages/B.md"))
            .unwrap()
            .windows(6)
            .any(|bytes| bytes == b"- keep");
        source_live || survivor_has_payload
    };
    let trashed = || {
        fs::read_dir(root.join("logseq/.tine-trash/pages"))
            .into_iter()
            .flatten()
            .any(|entry| fs::read(entry.unwrap().path()).unwrap() == b"- keep\n")
    };
    assert!(
        live() || trashed(),
        "I-2: raced merge lost the only payload; exemplar pages::merge_pages"
    );
    // The relaunched host completes a killed operation forward.
    let graph = store(&root);
    let host = PageHost::start_for_tests(&graph, &app_data).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !live() {
        assert!(
            Instant::now() < deadline,
            "I-2: raced merge retired the only live payload; exemplar pages::merge_pages"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(host);
    graph.close();
}

#[test]
fn k1_merge_external_editor_race_never_silently_retires_payload() {
    let _integration_guard = crate::integration_lock();
    run_race(None);
}

#[test]
fn k1_merge_external_editor_race_kill_and_reopen_keeps_payload_live() {
    let _integration_guard = crate::integration_lock();
    for boundary in 0..KILLS.len() {
        run_race(Some(boundary));
    }
}
