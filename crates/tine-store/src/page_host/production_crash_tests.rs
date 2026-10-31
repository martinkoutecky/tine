//! G7: real process termination at four ProductionIo boundaries. These are
//! process-crash survivor checks on the test volume, not power-loss claims.
use super::*;
use std::process::Command;
use std::time::{Duration, Instant};

const WORKER: &str = "page_host::production_tests::production_crash_tests::g7_crash_worker";

#[test]
fn g7_crash_worker() {
    let Some(scratch) = std::env::var_os("TINE_G7_SCRATCH") else {
        return;
    };
    let cut = std::env::var("TINE_G7_CUT").unwrap();
    let phase = match cut.as_str() {
        "durable-unapplied" => (Phase::PageTemp, 0),
        "marker-before-move" => (Phase::TrashMove, 0),
        "trash-before-sync" => (Phase::TrashSync, 0),
        // First sync publishes the draft; the second follows its unlink.
        "draft-unlinked" => (Phase::DraftSync, 1),
        _ => panic!("unknown crash cut"),
    };
    super::super::production::ATTACH_ABORT.with(|at| at.set(Some(phase)));
    let mut f = Fixture::new();
    fs::write(
        Path::new(&scratch).join("root"),
        f.root.path().to_str().unwrap(),
    )
    .unwrap();
    if cut == "marker-before-move" || cut == "trash-before-sync" {
        f.send("a.md", RequestKind::Open);
        assert_eq!(f.host.delete("a.md"), Disposition::Pending);
        f.drain();
    } else {
        f.edit("a.md", "irreplaceable G7 edit");
        f.host.pages.get_mut("a.md").unwrap().risk = true;
        assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
        f.drain();
    }
    assert!(!f.host.fs.draft_files(true).is_empty());
    f.save("a.md");
    if cut == "draft-unlinked" {
        assert_eq!(f.host.begin_draft_retiring("a.md"), Disposition::Pending);
        f.drain();
    }
    panic!("G7 {cut}: the crash boundary was not reached");
}

#[test]
fn g7_production_process_crashes_preserve_relaunch_survivors() {
    for cut in [
        "durable-unapplied",
        "marker-before-move",
        "trash-before-sync",
        "draft-unlinked",
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let output = scratch.path().join("worker.log");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", WORKER, "--nocapture"])
            .env("TINE_G7_SCRATCH", scratch.path())
            .env("TINE_G7_CUT", cut)
            .env("TINE_HOST_FS_ROOT", scratch.path())
            .stdout(fs::File::create(&output).unwrap())
            .stderr(fs::File::options().append(true).open(&output).unwrap())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("G7 {cut}: worker timed out");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let log = fs::read_to_string(&output).unwrap();
        // A panic exits 101, so it cannot impersonate a reached crash seam.
        assert!(
            !status.success() && status.code() != Some(101),
            "G7 {cut}: {status}: {log}"
        );
        let root = PathBuf::from(fs::read_to_string(scratch.path().join("root")).unwrap());
        let graph = root.join("graph");
        let trash = graph.join("logseq/.tine-trash/pages");
        let io = ProductionIo::attach(&graph, &root.join("app"), "test-graph", &trash);
        let drafts_before = drafts::scan(io.draft_files(false)).logical;
        match cut {
            "durable-unapplied" => {
                assert_eq!(fs::read(graph.join("a.md")).unwrap(), b"A");
                assert_eq!(drafts_before["a.md"].bytes, text("irreplaceable G7 edit"));
            }
            "marker-before-move" => {
                assert_eq!(fs::read(graph.join("a.md")).unwrap(), b"A");
                assert_eq!(
                    fs::read_dir(root.join("app/drafts-v2/test-graph/trash-custody"))
                        .unwrap()
                        .count(),
                    1
                );
            }
            "trash-before-sync" => {
                assert!(!graph.join("a.md").exists());
                assert!(fs::read_dir(&trash)
                    .unwrap()
                    .any(|entry| fs::read(entry.unwrap().path()).is_ok_and(|bytes| bytes == b"A")));
                assert_eq!(
                    fs::read_dir(root.join("app/drafts-v2/test-graph/trash-custody"))
                        .unwrap()
                        .count(),
                    1
                );
            }
            "draft-unlinked" => {
                assert_eq!(
                    fs::read(graph.join("a.md")).unwrap(),
                    b"irreplaceable G7 edit"
                );
                assert!(drafts_before.is_empty());
            }
            _ => unreachable!(),
        }
        let locks = ["a.md", "b.md", "c.md"]
            .into_iter()
            .map(|key| (key.into(), Arc::new(Mutex::new(()))))
            .collect();
        let mut host = Host::new(io, locks);
        host.stop();
        assert!(matches!(
            host.launch(),
            Disposition::Applied | Disposition::Pending
        ));
        for _ in 0..100 {
            if host.worker.is_none() {
                break;
            }
            host.advance_draft();
        }
        assert!(host.worker.is_none());
        if cut == "durable-unapplied" {
            assert_eq!(host.pages["a.md"].buf, text("irreplaceable G7 edit"));
            assert!(host.pages["a.md"].risk);
        } else if cut == "draft-unlinked" {
            assert!(host
                .pages
                .get("a.md")
                .is_none_or(|page| page.buf == text("irreplaceable G7 edit")));
            assert_eq!(
                fs::read(graph.join("a.md")).unwrap(),
                b"irreplaceable G7 edit"
            );
        } else {
            assert!(host.custody.is_empty());
            assert_eq!(host.pages["a.md"].buf, None);
            assert_eq!(
                drafts::scan(host.fs.draft_files(true)).logical["a.md"].bytes,
                None
            );
        }
        eprintln!(
            "G7 {cut}: process aborted at its phase; physical survivors and cold relaunch verified"
        );
    }
}
