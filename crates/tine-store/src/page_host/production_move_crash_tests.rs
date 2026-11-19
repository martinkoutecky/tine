//! KC2SE: the day move's writes (rename-merge-domain contract, "Day move"),
//! killed at every ProductionIo boundary they reach. The window's sequence:
//! the own-alias helper's save of the day (its alias line released), then one
//! two-page host move (the day keeps its header and a `Moved to` block, the
//! page receives the notes), then each page's save. A real process abort at
//! each boundary, then a cold relaunch on the same app data that saves what
//! the host restored: no note is lost, each file is whole, and the outcome is
//! the move done, not done (after the alias save alone: the documented
//! helper state, the day without its alias line and all its notes), or the
//! duplicate SPEC-s2 §1 accepts for an interrupted move, never a loss.
use super::*;
use std::process::Command;
use std::time::{Duration, Instant};

const WORKER: &str =
    "page_host::production_tests::production_move_crash_tests::day_move_crash_worker";
const DAY: &str = "alias:: Trip\n- day note\n- TODO call\n  - child note\n";
const RELEASED: &str = "- day note\n- TODO call\n  - child note\n";
const STUB: &str = "alias:: Trip\n- Moved to [[Trip]]\n";
const PAGE: &str = "- trip plan\n";
const MOVED: &str = "- trip plan\n- day note\n- TODO call\n  - child note\n";
const NOTES: [&str; 3] = ["day note", "TODO call", "child note"];
const PHASES: [Phase; 7] = [
    Phase::DraftTemp,
    Phase::DraftRename,
    Phase::DraftSync,
    Phase::DraftUnlink,
    Phase::PageTemp,
    Phase::PageRename,
    Phase::PageSync,
];

fn bytes(text: &str) -> Text {
    Some(Arc::from(text.as_bytes()))
}

#[test]
fn day_move_crash_worker() {
    let Some(scratch) = std::env::var_os("TINE_KC2SE_SCRATCH") else {
        return;
    };
    let phase = PHASES[std::env::var("TINE_KC2SE_PHASE")
        .unwrap()
        .parse::<usize>()
        .unwrap()];
    let n = std::env::var("TINE_KC2SE_N").unwrap().parse().unwrap();
    super::super::production::ATTACH_ABORT.with(|at| at.set(Some((phase, n))));
    let mut f = Fixture::new();
    fs::write(
        Path::new(&scratch).join("root"),
        f.root.path().to_str().unwrap(),
    )
    .unwrap();
    fs::write(f.graph.join("a.md"), DAY).unwrap();
    fs::write(f.graph.join("b.md"), PAGE).unwrap();
    // The own-alias helper: the day's alias line released, saved.
    f.edit("a.md", RELEASED);
    assert_eq!(f.save("a.md"), Outcome::Published);
    // The move: one two-page host operation, then each page's save.
    f.send("b.md", RequestKind::Open);
    let source_version = f.host.pages["a.md"].version;
    let receiver_version = f.host.pages["b.md"].version;
    f.send(
        "a.md",
        RequestKind::Move {
            receiver: "b.md".into(),
            source_text: bytes(STUB),
            receiver_text: bytes(MOVED),
            source_version,
            receiver_version,
        },
    );
    f.drain();
    for key in ["b.md", "a.md"] {
        if f.host.start_save(key) == Disposition::Pending {
            for _ in 0..15 {
                f.host.advance_save();
                if f.host.job.is_none() {
                    break;
                }
            }
        }
    }
    f.drain();
    // Exit 3: this boundary is not on the move's path (101 is a failure).
    std::process::exit(3);
}

#[test]
fn a_day_move_killed_at_each_write_boundary_reopens_with_every_note() {
    let mut reached = Vec::new();
    let mut outcomes = Vec::new();
    for (p, phase) in PHASES.iter().enumerate() {
        for n in 0..4usize {
            let scratch = tempfile::tempdir().unwrap();
            let output = scratch.path().join("worker.log");
            let mut child = Command::new(crate::test_binary::own_test_binary())
                .args(["--exact", WORKER, "--nocapture"])
                .env("TINE_KC2SE_SCRATCH", scratch.path())
                .env("TINE_KC2SE_PHASE", p.to_string())
                .env("TINE_KC2SE_N", n.to_string())
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
                    panic!("{phase:?} {n}: worker timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let log = fs::read_to_string(&output).unwrap();
            assert!(
                !status.success() && status.code() != Some(101),
                "{phase:?} {n}: {status}: {log}"
            );
            let root = PathBuf::from(fs::read_to_string(scratch.path().join("root")).unwrap());
            let graph = root.join("graph");
            let read = |key: &str| fs::read_to_string(graph.join(key)).unwrap();
            if status.code() == Some(3) {
                // Not reached: the whole sequence ran, so it must be done.
                assert_eq!((read("a.md"), read("b.md")), (STUB.into(), MOVED.into()));
                continue;
            }
            reached.push(format!("{phase:?}#{n}"));
            // Every file the kill left is whole bytes of one of its states.
            assert!(
                [DAY, RELEASED, STUB].contains(&read("a.md").as_str()),
                "{phase:?} {n}"
            );
            assert!(
                [PAGE, MOVED].contains(&read("b.md").as_str()),
                "{phase:?} {n}"
            );
            // Cold relaunch on the same app data; save what it restored.
            let trash = graph.join("logseq/.tine-trash/pages");
            let io = ProductionIo::attach(&graph, &root.join("app"), "test-graph", &trash);
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
            for key in ["b.md", "a.md"] {
                if host.start_save(key) == Disposition::Pending {
                    for _ in 0..15 {
                        host.advance_save();
                        if host.job.is_none() {
                            break;
                        }
                    }
                }
            }
            let (day, page) = (read("a.md"), read("b.md"));
            for note in NOTES {
                assert!(
                    day.contains(note) || page.contains(note),
                    "{phase:?} {n}: “{note}” lost on reopen:\n{day}\n---\n{page}"
                );
            }
            // Done, not done, or the duplicate SPEC-s2 accepts for an
            // interrupted move (the receiver's draft recovered, the day not
            // yet saved): never a page between its states.
            let before = [DAY, RELEASED].contains(&day.as_str());
            assert!(
                (day == STUB && page == MOVED) || (before && (page == PAGE || page == MOVED)),
                "{phase:?} {n}: a partial move survived reopen:\n{day}\n---\n{page}"
            );
            outcomes.push(format!(
                "{phase:?}#{n}: {}",
                match (day == STUB, page == MOVED) {
                    (true, _) => "moved",
                    (false, true) => "duplicate (receiver recovered, day kept its notes)",
                    _ if day == RELEASED => "not moved, alias released (helper state)",
                    _ => "not moved",
                }
            ));
            drop(host);
        }
    }
    eprintln!("KC2SE boundaries reached: {reached:?}");
    for outcome in &outcomes {
        eprintln!("KC2SE {outcome}");
    }
    assert!(
        reached.len() >= 6,
        "too few boundaries reached: {reached:?}"
    );
}
