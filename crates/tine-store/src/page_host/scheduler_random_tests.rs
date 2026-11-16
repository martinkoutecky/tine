//! Seeded action/disposition coverage under the physical scheduler.
use super::*;

#[test]
fn scheduler_random_walks_with_faults_and_disabled_actions() {
    let mut steps = 0;
    let mut physical = 0;
    let mut disabled = 0;
    let mut whole_retires = 0;
    let mut coverage = BTreeMap::<String, usize>::new();
    for profile in ["base", "R1", "weak", "all"] {
        for seed in 1u64..=12 {
            let mut rng = seed;
            let mut d = if seed % 3 == 0 {
                let mut grouped = published_group(profile);
                grouped.draft_retiring(0);
                grouped
            } else {
                Driver::stepped(profile)
            };
            if seed == 1 {
                // A counter alone cannot prove that a claimant protects a
                // creation. Seed that admission boundary, then leave the
                // successful apply pending for the random physical schedule.
                assert!(d.attempt("load", &[json!(2)]));
                assert!(d.attempt("extClaim", &[json!(2), json!(true)]));
                let apply = json!([[[2, 3]], [[2, ABSENT]], NONE, NONE]);
                assert!(!d.attempt("opApply", apply.as_array().unwrap()));
                *coverage.entry("claim/create/refused".into()).or_default() += 1;
                assert!(d.attempt("extClaim", &[json!(2), json!(false)]));
                assert!(d.attempt("opApply", apply.as_array().unwrap()));
            }
            let mut captured = [1, 2, ABSENT];
            for turn in 0..800 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                if !d.host.alive {
                    d.step("launch", &[]);
                    steps += 1;
                    continue;
                }
                if rng % 5 == 0
                    && d.host.applying.is_none()
                    && !d.host.queue.is_empty()
                    && d.host.dequeue() == Disposition::Pending
                {
                    // Dequeue preserves abstract queue custody. Admission,
                    // window crash and external writes can interleave before
                    // the later deliverUp application.
                    d.compare("dequeued request barrier");
                    physical += 1;
                    continue;
                }
                if d.host.worker.is_some() && rng % 3 != 0 {
                    if rng % 19 == 0 {
                        let w = d.host.worker.as_ref().unwrap();
                        let phase = match w.task.stage {
                            Stage::Temp => Some(Phase::DraftTemp),
                            Stage::Rename => Some(Phase::DraftRename),
                            Stage::Unlink | Stage::CleanupUnlink => Some(Phase::DraftUnlink),
                            Stage::Sync | Stage::UnlinkSync | Stage::CleanupSync => {
                                Some(Phase::DraftSync)
                            }
                            _ => None,
                        };
                        if let Some(phase) = phase {
                            d.host.fs.inject(
                                phase,
                                [if rng % 2 == 0 {
                                    Fault::After
                                } else {
                                    Fault::Before
                                }],
                            );
                        }
                    }
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.tick_draft()));
                    if let Err(e) = result {
                        eprintln!("failure capsule: HEAD 5da9833371d15ccd6ba89238f09841c750d47a1d + lane diff; {profile}/seed {seed}/turn {turn}/draft barrier; {steps} actions / {physical} physical / {disabled} disabled before failure; expected observed lockstep; ambiguous host/adapter mismatch");
                        std::panic::resume_unwind(e);
                    }
                    physical += 1;
                    continue;
                }
                let p = (rng as usize / 7) % 3;
                let t = 1 + (rng as i64 / 31).rem_euclid(3);
                let mut choice = (rng / 13) % 29;
                // Apply includes the real binding predicates (including a
                // native Store projection for creates), so sample it less
                // often than inexpensive client/environment steps.
                if matches!(choice, 25 | 26) && rng & 0x600 != 0 {
                    choice = (rng / 113) % 25;
                }
                let current: Vec<_> = if rng % 127 == 2 || matches!(choice, 25 | 26) {
                    (0..3)
                        .map(|p| {
                            if let Some(page) = d.host.pages.get(&key(p)) {
                                label(&page.buf)
                            } else {
                                label(&d.host.fs.files.get(&format!("graph/{}", key(p))).cloned())
                            }
                        })
                        .collect()
                } else {
                    captured.to_vec()
                };
                // A genuine older capture can go stale across edits, deletes,
                // observations, recovery and other operations.
                if rng % 127 == 2 {
                    captured.copy_from_slice(&current);
                }
                let payload = if rng & 0x100 == 0 { ABSENT } else { t };
                let (name, args) = match choice {
                    0 => ("wOpen", json!([p])),
                    1 => ("wEdit", json!([p, t])),
                    2 => ("wSend", json!([p])),
                    3 => ("wRecv", json!([p])),
                    4 => ("deliverUp", json!([true])),
                    5 => ("observe", json!([p])),
                    6 => ("flush", json!([p])),
                    7 => ("flushDel", json!([p])),
                    8 => ("check", json!([])),
                    9 => ("rename", json!([])),
                    10 => ("dirSync", json!([rng % 2 == 0])),
                    11 => ("saveFail", json!([])),
                    12 => ("draftSync", json!([p])),
                    13 => ("opDelete", json!([p])),
                    14 => ("extWriteD", json!([p, payload, rng % 2 == 0])),
                    15 => ("windowCrash", json!([])),
                    16 => ("switchReq", json!([])),
                    17 => ("wDiscard", json!([p])),
                    18 => ("wClose", json!([p])),
                    19 => ("crash", json!([])),
                    20 => (
                        "powerK",
                        json!([(0..3)
                            .filter(|p| (rng / 97) & (1 << p) != 0)
                            .collect::<BTreeSet<_>>()]),
                    ),
                    21 => ("wOpTo", json!([p, (p + 1) % 3, t, 1 + t % 3])),
                    22 => ("load", json!([p])),
                    23 => ("wResolve", json!([p, t])),
                    24 => (
                        "opRename",
                        json!([p, (p + 1) % 3, [(p + 2) % 3], [[(p + 2) % 3, t]]]),
                    ),
                    25 => (
                        "opApply",
                        json!([[[p, payload]], [[p, current[p]]], NONE, NONE]),
                    ),
                    26 => (
                        "opApply",
                        json!([[[p, payload]], [[p, captured[p]]], NONE, NONE]),
                    ),
                    27 => ("extClaim", json!([p, rng % 2 == 0])),
                    _ => ("switchFin", json!([])),
                };
                let branch = match name {
                    "opApply" => Some(format!(
                        "apply/{}/{}",
                        if args[1][0][1] == current[p] {
                            "fresh"
                        } else {
                            "stale"
                        },
                        if payload == ABSENT {
                            "absent"
                        } else {
                            "present"
                        }
                    )),
                    "extClaim" => Some(format!("claim/{}", args[1])),
                    "switchFin" => Some("switchFin".into()),
                    "extWriteD" => Some(format!(
                        "external/{}/{}",
                        if payload == ABSENT {
                            "delete"
                        } else {
                            "present"
                        },
                        args[2]
                    )),
                    "powerK" => Some(format!("power/{}", (rng / 97) & 7)),
                    _ => None,
                };
                if name == "powerK" {
                    for p in 0..3 {
                        let path = format!("graph/{}", key(p));
                        if d.host.fs.files.get(&path) != d.host.fs.stable.get(&path) {
                            let kept = args[0].as_array().unwrap().contains(&json!(p));
                            *coverage
                                .entry(format!(
                                    "power/{}-change/taken",
                                    if kept { "kept" } else { "reverted" }
                                ))
                                .or_default() += 1;
                        }
                    }
                }
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    d.attempt(name, args.as_array().unwrap())
                }));
                match result {
                    Ok(taken) => {
                        if taken {
                            steps += 1
                        } else {
                            disabled += 1
                        }
                        if let Some(branch) = branch {
                            *coverage
                                .entry(format!(
                                    "{branch}/{}",
                                    if taken { "taken" } else { "refused" }
                                ))
                                .or_default() += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("failure capsule: HEAD 5da9833371d15ccd6ba89238f09841c750d47a1d + lane diff; {profile}/seed {seed}/turn {turn}/{name}/{args}; expected observed lockstep; ambiguous host/adapter mismatch");
                        std::panic::resume_unwind(e);
                    }
                }
            }
            d.settle();
            whole_retires += d.whole_retires;
        }
    }
    eprintln!("scheduler 48 walks x 800 choices; seeds 1..12 per profile; {steps} actions / {physical} physical / {disabled} disabled");
    eprintln!(
        "whole retirement branches taken: {whole_retires}; branch dispositions: {coverage:?}"
    );
    assert!(whole_retires > 0, "whole retirement branch never taken");
    for branch in [
        "apply/fresh/absent/taken",
        "apply/fresh/present/taken",
        "apply/stale/absent/refused",
        "apply/stale/present/refused",
        "claim/true/taken",
        "claim/false/taken",
        "claim/create/refused",
        "switchFin/taken",
        "switchFin/refused",
        "external/delete/true/taken",
        "external/delete/false/taken",
        "power/kept-change/taken",
        "power/reverted-change/taken",
    ] {
        assert!(
            coverage.get(branch).copied().unwrap_or(0) > 0,
            "missing scheduler branch {branch}"
        );
    }
    for mask in 0..8 {
        assert!(
            coverage
                .get(&format!("power/{mask}/taken"))
                .copied()
                .unwrap_or(0)
                > 0
        );
    }
}
