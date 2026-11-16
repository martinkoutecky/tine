//! Whole-vehicle retirement refinement and physical recovery cuts.
use super::*;

/// Keep a three-page operation vehicle after all of its publications.
pub(super) fn published_group(profile: &str) -> Driver {
    let mut d = Driver::stepped(profile);
    run(&mut d, &[("opRename", json!([0, 2, [1], [[1, 3]]]))]);
    d.settle();
    for p in [2, 1, 0] {
        publish(&mut d, p);
    }
    assert!(d.host.live_ops.values().all(|op| op.owed.is_empty()));
    d
}

fn publish(d: &mut Driver, p: usize) {
    let flush = if d.host.pages[&key(p)].buf.is_none() {
        "flushDel"
    } else {
        "flush"
    };
    assert!(d.attempt(flush, &[json!(p)]));
    while d.host.job.as_ref().unwrap().phase == SavePhase::Temp {
        assert!(d.attempt("check", &[]));
    }
    assert!(d.attempt("check", &[]));
    assert!(d.attempt("rename", &[]));
    while d.host.job.is_some() {
        assert!(d.attempt("dirSync", &[json!(true)]));
    }
}

#[test]
fn whole_retirement_preserves_unpublished_siblings() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = Driver::stepped(profile);
        run(&mut d, &[("opRename", json!([0, 2, [1], [[1, 3]]]))]);
        d.settle();
        publish(&mut d, 2);
        assert_eq!(
            d.host.live_ops.values().next().unwrap().owed,
            BTreeSet::from([key(0), key(1)])
        );
        d.draft_retiring(2);
        assert_eq!(
            d.whole_retires, 0,
            "unpublished siblings forbid whole retirement"
        );
        // The permitted single removal copies siblings before the unlink.
        loop {
            let mut cut = d.clone();
            cut.fault(true, false);
            assert!(cut.step("launch", &[]));
            cut.settle();
            if d.host.worker.is_none() {
                break;
            }
            d.tick_draft();
        }
    }
}

#[test]
fn whole_vehicle_retirement_refines_individual_removals() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = published_group(profile);
        d.draft_retiring(0);
        assert!(matches!(
            d.host.worker.as_ref().unwrap().application,
            Some(Application::Retire)
        ));
        d.settle();
        assert!(d.host.logical_drafts().is_empty());
    }
}

#[test]
fn whole_vehicle_retirement_every_physical_fault_and_recovery_cut() {
    let mut cuts = 0;
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = published_group(profile);
        d.draft_retiring(0);
        loop {
            d.retirement_custody();
            for (power, drafts) in [(false, false), (true, false), (true, true)] {
                for mask in if power { 0..8 } else { 0..1 } {
                    let mut cut = d.clone();
                    let keep = (0..3).filter(|p| mask & (1 << p) != 0).collect();
                    cut.fault_keeping(power, drafts, &keep);
                    assert!(cut.step("launch", &[]));
                    cut.settle();
                    cut.compare("whole retirement recovered cut");
                    cuts += 1;
                }
            }
            let Some(worker) = &d.host.worker else { break };
            let phase = match worker.task.stage {
                Stage::Unlink => Some(Phase::DraftUnlink),
                Stage::UnlinkSync => Some(Phase::DraftSync),
                _ => None,
            };
            if let Some(phase) = phase {
                for fault in [Fault::Before, Fault::After] {
                    let mut failed = d.clone();
                    failed.host.fs.inject(phase, [fault]);
                    failed.tick_draft();
                    // Retry and crash are both checked from the failed cut.
                    let mut retry = failed.clone();
                    retry.settle();
                    for (power, drafts) in [(false, false), (true, false), (true, true)] {
                        let mut cut = failed.clone();
                        cut.fault(power, drafts);
                        assert!(cut.step("launch", &[]));
                        cut.settle();
                        cuts += 1;
                    }
                }
            }
            d.tick_draft();
        }
        assert_eq!(d.whole_retires, 1);
    }
    eprintln!("whole retirement: {cuts} fault/recovery cases across four profiles");
}
