//! F-M3S-1: opt-in acceptance on the actual backing volume, with real native
//! publication, directory sync and draft I/O. No native power loss is simulated.
//!
//! From the repository root, after sourcing scripts/env.sh:
//! `env TINE_STORAGE_ACCEPT_ROOT=/path/on/volume CARGO_TARGET_DIR=target cargo test -p tine-store --lib storage_acceptance_tests -- --ignored --nocapture --test-threads=1`
//!
//! The supplied parent must already exist. Each case creates and owns a fresh
//! empty subdirectory there; it never initializes or removes the parent itself.
//! A failed case also drops its handles and sandbox before resuming the panic.
use super::*;
use std::time::Duration;

fn on_volume(case: &str, run: impl FnOnce(Fixture) -> Fixture) {
    let parent = std::env::var_os("TINE_STORAGE_ACCEPT_ROOT")
        .expect("set TINE_STORAGE_ACCEPT_ROOT to an existing directory on the volume under test");
    let parent = fs::canonicalize(parent).expect("acceptance parent must exist");
    assert!(parent.is_dir(), "acceptance parent must be a directory");
    let root = tempfile::Builder::new()
        .prefix("tine-storage-accept-")
        .tempdir_in(&parent)
        .expect("create a fresh acceptance sandbox on the supplied volume");
    let sandbox = root.path().to_path_buf();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let f = run(Fixture::in_empty_root(root));
        let Fixture { host, root, .. } = f;
        drop(host);
        root.close()
            .expect("remove the complete acceptance sandbox");
    }));
    assert!(!sandbox.exists(), "acceptance sandbox was not cleaned up");
    eprintln!(
        "F-M3S-1 case={case} result={} volume={} sandbox_removed=true",
        if result.is_ok() { "PASS" } else { "FAIL" },
        parent.display()
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

/// Change metadata alone, portably and by more than a timestamp granule. Close
/// the handle before the next native phase (also works with Windows sharing).
fn touch_page(path: &Path) {
    let old = fs::metadata(path).unwrap().modified().unwrap();
    let changed = old + Duration::from_secs(60);
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(changed))
        .unwrap();
    assert_ne!(fs::metadata(path).unwrap().modified().unwrap(), old);
}

/// An adoption at Check has no SaveOutcome; let the next attempt retry through
/// the normal host pipeline. The disturbance lands after the publication read
/// and before directory sync, without an injected I/O error or adapter stub.
fn attempt(f: &mut Fixture, interfere: bool) -> Option<Outcome> {
    assert_eq!(f.host.start_save("a.md"), Disposition::Pending);
    let start = f.host.events.len();
    let mut touched = false;
    for _ in 0..15 {
        if interfere
            && f.host
                .job
                .as_ref()
                .is_some_and(|job| job.phase == SavePhase::DirectorySync)
        {
            assert!(!touched);
            let path = f.graph.join("a.md");
            let bytes = fs::read(&path).unwrap();
            touch_page(&path);
            assert_eq!(fs::read(path).unwrap(), bytes, "metadata-only disturbance");
            touched = true;
        }
        f.host.advance_save();
        if f.host.job.is_none() {
            let outcome = f.host.events[start..].iter().find_map(|event| {
                if let Event::SaveOutcome { outcome, .. } = event {
                    Some(*outcome)
                } else {
                    None
                }
            });
            if touched {
                assert_eq!(outcome, Some(Outcome::Uncertain));
                assert!(!f.host.events[start..]
                    .iter()
                    .any(|event| matches!(event, Event::Published { .. })));
                assert!(f.host.pages["a.md"].risk);
            }
            return outcome;
        }
    }
    panic!("native acceptance save did not finish");
}

#[test]
#[ignore = "F-M3S-1: requires TINE_STORAGE_ACCEPT_ROOT on the volume under acceptance"]
fn ordinary_saves_publish() {
    on_volume("ordinary_saves_publish", |mut f| {
        for round in 0..8 {
            let input = format!("- ordinary volume save {round}\n");
            f.edit("a.md", &input);
            assert_eq!(f.save("a.md"), Outcome::Published, "round={round}");
            assert!(f.host.pages["a.md"].clean());
            assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), input.as_bytes());
        }
        f
    });
}

#[test]
#[ignore = "F-M3S-1: requires TINE_STORAGE_ACCEPT_ROOT on the volume under acceptance"]
fn finite_metadata_interference_retries_to_published() {
    on_volume(
        "finite_metadata_interference_retries_to_published",
        |mut f| {
            let input = "- input survives finite metadata interference\n";
            f.edit("a.md", input);
            let mut uncertain = 0;
            for _ in 0..6 {
                // Also exercise metadata changes between consecutive save attempts.
                touch_page(&f.graph.join("a.md"));
                if let Some(outcome) = attempt(&mut f, true) {
                    assert_eq!(outcome, Outcome::Uncertain);
                    uncertain += 1;
                    if uncertain == 3 {
                        break;
                    }
                }
            }
            assert_eq!(uncertain, 3, "three finite native metadata disturbances");
            let published = (0..2).any(|_| attempt(&mut f, false) == Some(Outcome::Published));
            assert!(
                published,
                "a stable retry must publish after interference ends"
            );
            assert!(f.host.pages["a.md"].clean());
            assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), input.as_bytes());
            f
        },
    );
}

#[test]
#[ignore = "F-M3S-1: requires TINE_STORAGE_ACCEPT_ROOT on the volume under acceptance"]
fn uncertain_save_recovers_physical_draft_on_cold_reopen() {
    on_volume(
        "uncertain_save_recovers_physical_draft_on_cold_reopen",
        |mut f| {
            let input = "- cold recoverable volume input\n";
            f.edit("a.md", input);
            assert_eq!(attempt(&mut f, true), Some(Outcome::Uncertain));
            assert_eq!(f.host.begin_draft("a.md"), Disposition::Pending);
            f.drain();
            let mut f = f.cold_reopen();
            assert_eq!(f.host.pages["a.md"].buf, text(input));
            assert!(f.host.pages["a.md"].risk);
            assert_eq!(
                drafts::scan(draft_dir(&f).into_iter().collect()).logical["a.md"].bytes,
                text(input)
            );
            let published = (0..2).any(|_| attempt(&mut f, false) == Some(Outcome::Published));
            assert!(
                published,
                "recovered input must publish after a stable retry"
            );
            assert!(f.host.pages["a.md"].clean());
            assert_eq!(fs::read(f.graph.join("a.md")).unwrap(), input.as_bytes());
            f
        },
    );
}
