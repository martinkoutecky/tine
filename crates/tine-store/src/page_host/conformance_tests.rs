//! Hand-written traces through the conformance driver (STEP2 §8, STEP3
//! review counterexamples).
use super::*;

fn review_dirty_delete_prefix() -> Driver {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 3])),
            ("wSend", json!([0])),
            ("deliverUp", json!([true])),
        ],
    );
    d
}

#[test]
fn review_disabled_delete_step_preserves_state() {
    let mut d = review_dirty_delete_prefix();
    let before = d.abstract_state();
    assert!(!d.step("opDelete", &[json!(0)]));
    assert_eq!(d.abstract_state(), before);
}

#[test]
fn review_direct_dirty_delete_must_refuse() {
    let mut d = review_dirty_delete_prefix();
    assert_eq!(d.host.delete(&key(0)), Disposition::Refused);
}

#[test]
fn review_dirty_apply_observes_every_unsaved_page() {
    let mut d = review_dirty_delete_prefix();
    run(
        &mut d,
        &[
            ("wOpen", json!([1])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([1])),
            ("wEdit", json!([1, 1])),
            ("wSend", json!([1])),
            ("deliverUp", json!([true])),
        ],
    );
    assert!(!d.step(
        "opApply",
        &[
            json!([[0, 2], [1, 3]]),
            json!([[0, 3], [1, 1]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
}

#[test]
fn review_queued_submit_refusal_is_checked_stutter() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 3])),
            ("wSend", json!([0])),
        ],
    );
    let args = [json!([[0, 2]]), json!([[0, 1]]), json!(NONE), json!(NONE)];
    assert!(d.oracle.next("opApply", &args).is_some());
    assert!(d.host.pages[&key(0)].clean());
    assert_eq!(
        binding::unsaved(&d.host, &BTreeSet::from([key(0)])),
        BTreeMap::from([(key(0), PageId::from(key(0)))])
    );
    assert!(d.claims().is_empty());
    assert!(d
        .host
        .fs
        .physical()
        .files
        .contains_key(&format!("graph/{}", key(0))));
    let before = d.oracle.state().clone();
    assert!(d.step("opApply", &args));
    assert_eq!(d.oracle.state(), before);
    assert_eq!(d.queued_refusals, 1);
}

#[test]
fn review_pending_spelling_precedes_dirty_sibling_in_unsaved_set() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wOpen", json!([1])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([1])),
            ("wEdit", json!([1, 1])),
            ("wSend", json!([1])),
            ("deliverUp", json!([true])),
            ("wEdit", json!([0, 3])),
            ("wSend", json!([0])),
        ],
    );
    assert!(!d.step(
        "opApply",
        &[
            json!([[0, 2], [1, 3]]),
            json!([[0, 1], [1, 1]]),
            json!(NONE),
            json!(NONE),
        ]
    ));
    assert_eq!(
        binding::unsaved(&d.host, &BTreeSet::from([key(0), key(1)])),
        BTreeMap::from([
            (key(0), PageId::from(key(0))),
            (key(1), PageId::from(key(1)))
        ])
    );
}

#[test]
#[should_panic(expected = "outside dirty-or-pending predicate")]
fn queued_refusal_mutant_without_oracle_input_is_rejected() {
    let mut d = Driver::new("base", 3);
    run(&mut d, &[("load", json!([0]))]);
    // Host-only mutant: an invented admitted input forces production Unsaved
    // although the independently observed oracle has no pending input.
    d.admit(
        0,
        RequestKind::Submit {
            bytes: text(3),
            version: d.host.pages[&key(0)].version,
            resolve: None,
        },
    );
    d.step(
        "opApply",
        &[json!([[0, 2]]), json!([[0, 1]]), json!(NONE), json!(NONE)],
    );
}

#[test]
fn review_disabled_rename_observes_dirty_source_and_occupied_target() {
    let mut d = review_dirty_delete_prefix();
    assert!(d.step("load", &[json!(2)]));
    assert!(!d.step("opRename", &[json!(0), json!(2), json!([]), json!([])]));
    let mut d = Driver::new("base", 3);
    run(&mut d, &[("load", json!([0])), ("load", json!([1]))]);
    assert!(!d.step("opRename", &[json!(0), json!(1), json!([]), json!([])]));
}

#[test]
fn review_h444_history_through_actual_host() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("load", json!([2])),
            ("opRename", json!([0, 2, [], []])),
            ("extWriteD", json!([0, 2, true])),
            ("wDiscard", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("extWriteD", json!([0, ABSENT, true])),
            ("observe", json!([0])),
            ("wOpen", json!([2])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([2])),
            ("wDiscard", json!([2])),
            ("deliverUp", json!([true])),
        ],
    );
    assert_eq!(d.observed["g"]["promise"][0]["ver"], json!(9));
}

#[test]
fn review_unknown_observation_resolution_through_actual_host() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = Driver::new(profile, 3);
        run(
            &mut d,
            &[
                ("wOpen", json!([0])),
                ("deliverUp", json!([true])),
                ("wRecv", json!([0])),
                ("wEdit", json!([0, 2])),
                ("wSend", json!([0])),
                ("deliverUp", json!([true])),
                ("wRecv", json!([0])),
                ("switchReq", json!([])),
                ("draftSync", json!([0])),
                ("crash", json!([])),
                ("launch", json!([])),
                ("wOpen", json!([0])),
                ("deliverUp", json!([true])),
                ("wRecv", json!([0])),
                ("flush", json!([0])),
                ("check", json!([])),
                ("rename", json!([])),
                ("dirSync", json!([true])),
                ("load", json!([2])),
                ("opRename", json!([0, 2, [], []])),
                ("wEdit", json!([0, 3])),
                ("wSend", json!([0])),
                ("deliverUp", json!([true])),
                ("wRecv", json!([0])),
            ],
        );
        assert_eq!(d.observed["s"]["w"][0]["obs"], json!(NONE));
        let seen = d.observed["g"]["seen"][0].clone();
        run(&mut d, &[("wResolve", json!([0, 1]))]);
        assert_eq!(d.observed["g"]["seen"][0], seen);
        run(
            &mut d,
            &[("deliverUp", json!([true])), ("wRecv", json!([0]))],
        );
        assert_eq!(d.observed["s"]["pages"][0]["buf"], json!(1));
        assert_eq!(d.observed["s"]["pages"][0]["base"], json!(UNKNOWN));
    }
}

/// STEP3 §2 / §14: keys register in trace order. Lexically earlier keys
/// joining after the first allocations, and across a crash, still refine the
/// model under the order-preserving version map (STEP3-REVIEW-1 F8).
#[test]
fn lexically_earlier_keys_register_between_operations_and_across_a_crash() {
    // Four model pages, three registered at the rename: host ranks leave
    // fewer gaps than the model's PAGES.size() reservation (L524, L547).
    let mut d = Driver::new("base", 4);
    let keys = |d: &Driver| d.host.keys.iter().map(|k| index(k)).collect::<Vec<_>>();
    run(
        &mut d,
        &[
            ("wOpen", json!([2])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([2])),
        ],
    );
    assert_eq!(keys(&d), [2]);
    run(
        &mut d,
        &[("opDelete", json!([1])), ("flushDel", json!([1]))],
    );
    assert_eq!(keys(&d), [1, 2]);
    // A references-only rename allocates by rank over all three keys.
    run(&mut d, &[("opRename", json!([1, 0, [2], [[2, 3]]]))]);
    assert_eq!(keys(&d), [0, 1, 2]);
    assert!(d.host.pages[&key(2)].risk);
    run(&mut d, &[("crash", json!([])), ("launch", json!([]))]);
    // The next binding registered its recovered keys only, the rewritten
    // referrer among them; the window then opens a lexically earlier key.
    assert!(
        keys(&d).contains(&2) && !keys(&d).contains(&0),
        "{:?}",
        keys(&d)
    );
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
        ],
    );
    assert!(keys(&d).contains(&0));
}

#[test]
fn draft_during_save_holds_publication_until_snapshot_applies() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 2])),
            ("wSend", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("switchReq", json!([])),
            ("flush", json!([0])),
        ],
    );
    assert_eq!(d.host.begin_draft(&key(0)), Disposition::Pending);
    assert_eq!(d.host.advance_save(), Disposition::Waiting);
    d.compare("save waits for snapshot");
    let inc = d.host.incarnation;
    let version = d.host.version;
    d.drain("draftSync", &[json!(0)], inc, version);
    run(
        &mut d,
        &[
            ("check", json!([])),
            ("rename", json!([])),
            ("dirSync", json!([true])),
        ],
    );
}

#[test]
fn refs_only_rename_does_not_reserve_absent_sources_save_job() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("opDelete", json!([0])),
            ("flushDel", json!([0])),
            ("opRename", json!([0, 2, [1], [[1, 3]]])),
        ],
    );
    assert_eq!(d.host.job.as_ref().unwrap().page, key(0));
}

#[test]
fn switch_risk_marking_is_allowed_while_final_barrier_waits_for_save() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 2])),
            ("wSend", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("flush", json!([0])),
            ("switchReq", json!([])),
        ],
    );
    assert!(d.host.pages[&key(0)].risk);
    assert_eq!(
        d.host.switch_ready(d.host.last_admitted),
        Disposition::Applied
    );
    assert!(!d.host.can_switch());
    assert_eq!(d.host.switch_finish(), Disposition::Waiting);
}

#[test]
fn failed_open_stops_pushes_after_its_answer_is_consumed() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([false])),
            ("wRecv", json!([0])),
            ("opDelete", json!([0])),
        ],
    );
    assert!(!d.host.outbox.contains_key(&key(0)));
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([false])),
            ("opDelete", json!([0])),
            ("wRecv", json!([0])),
        ],
    );
    assert!(d.windows[0].on);
}

#[test]
fn queued_close_cannot_remove_a_new_open_subscription() {
    let mut d = Driver::new("base", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wClose", json!([0])),
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("extWrite", json!([0, 3])),
            ("observe", json!([0])),
            ("wRecv", json!([0])),
        ],
    );
    assert_eq!(d.windows[0].text, 3);
}

#[test]
fn directory_sync_does_not_make_external_unsynced_payload_durable() {
    let mut d = Driver::new("R1", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 2])),
            ("wSend", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("flush", json!([0])),
            ("check", json!([])),
            ("extWriteD", json!([0, 3, false])),
            ("rename", json!([])),
            ("extWriteD", json!([0, 3, false])),
            ("dirSync", json!([true])),
            ("power", json!([false, false])),
        ],
    );
    assert_eq!(
        label(&d.host.fs.files.get(&format!("graph/{}", key(0))).cloned()),
        1
    );
}

#[test]
fn m3_identity_refinement_requires_the_adapter_to_refuse() {
    let mut d = Driver::new("R1", 3);
    run(
        &mut d,
        &[
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wEdit", json!([0, 2])),
            ("wSend", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("flush", json!([0])),
            ("check", json!([])),
            ("rename", json!([])),
            ("extWriteD", json!([0, 3, false])),
        ],
    );
    // Negative control: make the adapter accept the external file as its
    // publication. The independently held job epoch still requires Uncertain.
    // If the driver injected the expected failure, this mutant would pass.
    let page = key(0);
    d.host
        .fs
        .publication_epochs
        .insert(page.clone(), d.host.fs.epochs[&page]);
    d.host.fs.publications.insert(
        page.clone(),
        d.host.fs.files.get(&format!("graph/{page}")).cloned(),
    );
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || d.step("dirSync", &[json!(true)])
    ))
    .is_err());
}

#[test]
fn all_s3_scenarios_in_four_profiles() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/scenarios.json")).unwrap();
    assert_eq!(fixture["model_sha256"], Oracle::model_sha());
    let mut comparisons = 0;
    let mut barriers = 0;
    let mut actions = 0;
    let mut twins = 0;
    let mut occupied = 0;
    let mut queued = 0;
    let mut effective = 0;
    let mut mismatches = Vec::new();
    for oracle in fixture["oracles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["mutant"] == "none")
    {
        let profile = oracle["profile"].as_str().unwrap();
        for (name, program) in fixture["scenarios"].as_object().unwrap() {
            let mut driver = Driver::new(profile, 3);
            let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                driver.program(program.as_array().unwrap())
            })) {
                Ok(outcome) => outcome,
                Err(failure) => {
                    let message = failure
                        .downcast_ref::<String>()
                        .map_or("host replay panicked", String::as_str);
                    mismatches.push(format!("{profile}/{name}: {message}"));
                    continue;
                }
            };
            if outcome != Err("twin-saveFail-refinement") {
                let observed = outcome.map_or_else(|e| e, |_| "pass");
                let expected = &oracle["outcomes"][name];
                if expected != observed {
                    mismatches.push(format!(
                        "{profile}/{name}: expected {expected}, observed {observed}"
                    ));
                }
            }
            twins += driver.twin_failures;
            occupied += driver.occupied_refusals;
            queued += driver.queued_refusals;
            effective += driver.effective_renames;
            barriers += driver.barriers;
            actions += driver.actions;
            comparisons += 1;
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} host divergences\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert_eq!(comparisons, 824);
    assert_eq!(
        (actions, barriers, twins, occupied, queued, effective),
        (8162, 11446, 4, 0, 0, 8)
    );
    eprintln!("host scenarios: {comparisons} outcomes / {actions} actions / {barriers} barriers / {twins} twin saveFail / {occupied} occupied stutters / {queued} queued stutters / {effective} effective renames");
}

#[test]
fn paired_rename_reopens_after_the_source_move() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/scenarios.json")).unwrap();
    let program = fixture["scenarios"]["ownRenameInterrupted"]
        .as_array()
        .unwrap();
    for profile in ["base", "R1", "weak", "all"] {
        let mut driver = Driver::new(profile, 3);
        assert_eq!(driver.program(program), Ok(()), "{profile}");
    }
}

#[test]
#[should_panic(expected = "check/job")]
fn s35_refinement_rejects_save_failure_without_twin() {
    let mut d = Driver::new("base", 3);
    assert!(d.step("load", &[json!(2)]));
    assert!(d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
    assert!(d.step("flush", &[json!(2)]));
    // A host failing this check without a physical twin cannot use the
    // saveFail alternative merely because saveFail is model-enabled.
    d.host.fs.inject(Phase::Read, [Fault::Before]);
    d.step("check", &[]);
}

#[test]
#[should_panic(expected = "refusal without physical expected-ABSENT occupancy")]
fn s35_refinement_rejects_refusal_without_occupancy() {
    let mut d = Driver::new("base", 3);
    assert!(d.step("load", &[json!(2)]));
    // Corrupt the host registration to force its Changed refusal, while
    // the model still enables this clean absent-create and disk is empty.
    d.host.keys = Keys::default();
    d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE),
        ],
    );
}

#[test]
fn s35_creation_publishes_without_twin() {
    let mut d = Driver::new("base", 3);
    creation_control(&mut d, false);
    assert!(d.host.pages[&key(2)].clean());
    assert_eq!(d.twin_failures, 0);
}

pub(super) fn creation_control<F: ConformanceIo>(d: &mut Driver<F>, twin: bool) {
    assert!(d.step("load", &[json!(2)]));
    assert!(d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
    if twin {
        assert!(d.step("extClaim", &[json!(2), json!(true)]));
    }
    assert!(d.step("flush", &[json!(2)]));
    assert!(d.step("check", &[]));
    if twin {
        assert_eq!(d.twin_failures, 1);
        assert!(!d.host.pages[&key(2)].clean());
        assert_eq!(label(&d.host.pages[&key(2)].buf), 3);
        assert!(!d.host.fs.physical().files.contains_key("graph/0002.md"));
    } else {
        assert!(d.step("rename", &[]));
        assert!(d.step("dirSync", &[json!(true)]));
        assert_eq!(
            label(&d.host.fs.physical().files.get("graph/0002.md").cloned()),
            3
        );
    }
}

#[test]
fn s35_physical_twin_takes_save_fail() {
    creation_control(&mut Driver::new("base", 3), true);
}

#[test]
fn s35_occupied_create_is_stutter_and_trace_continues() {
    let mut d = Driver::new("base", 3);
    assert!(d.step("load", &[json!(2)]));
    assert!(d.step("extWrite", &[json!(2), json!(1)]));
    assert!(d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
    assert_eq!(d.occupied_refusals, 1);
    assert_eq!(label(&d.host.pages[&key(2)].buf), ABSENT);
    assert!(d.step("observe", &[json!(2)]));
    assert_eq!(label(&d.host.pages[&key(2)].buf), 1);
}

#[test]
fn s35_slot_refusal_calls_binding_admission() {
    let mut d = Driver::new("base", 3);
    assert!(d.step("load", &[json!(2)]));
    assert!(d.step("extClaim", &[json!(2), json!(true)]));
    assert!(!d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
    assert!(d.step("extClaim", &[json!(2), json!(false)]));
    assert!(d.step(
        "opApply",
        &[
            json!([[2, 3]]),
            json!([[2, ABSENT]]),
            json!(NONE),
            json!(NONE)
        ]
    ));
}

#[test]
fn s35_stale_plan_calls_host_and_refuses() {
    let mut d = Driver::new("base", 3);
    assert!(d.step("load", &[json!(1)]));
    assert!(d.step("extWrite", &[json!(1), json!(3)]));
    assert!(d.step("observe", &[json!(1)]));
    assert!(!d.step(
        "opApply",
        &[json!([[1, 1]]), json!([[1, 2]]), json!(NONE), json!(NONE)]
    ));
}

#[test]
fn s35_discard_keeps_retired_source_unheld() {
    let mut d = Driver::new("base", 3);
    for p in 0..3 {
        assert!(d.step("load", &[json!(p)]));
    }
    assert!(d.step(
        "opRename",
        &[json!(0), json!(2), json!([1]), json!([[1, 3]])]
    ));
    for (name, args) in [
        ("wOpen", vec![json!(0)]),
        ("deliverUp", vec![json!(true)]),
        ("wRecv", vec![json!(0)]),
        ("wDiscard", vec![json!(0)]),
        ("deliverUp", vec![json!(true)]),
        ("wRecv", vec![json!(0)]),
        ("wClose", vec![json!(0)]),
        ("deliverUp", vec![json!(true)]),
        ("wOpen", vec![json!(2)]),
        ("deliverUp", vec![json!(true)]),
        ("wRecv", vec![json!(2)]),
        ("wDiscard", vec![json!(2)]),
        ("deliverUp", vec![json!(true)]),
    ] {
        assert!(d.step(name, &args), "{name}");
    }
    assert!(!d.host.pages.contains_key(&key(0)));
}

/// Counts a replay checked, ratcheted by each caller (F9): scripted steps,
/// host actions and barriers, and every declared alternative or stutter.
#[derive(Debug, Default, PartialEq)]
struct Replayed {
    traces: usize,
    scripted: usize,
    actions: usize,
    barriers: usize,
    twins: usize,
    occupied: usize,
    queued: usize,
    effective: usize,
    save_kinds: usize,
    stutters: BTreeMap<String, usize>,
    /// Traces ended at a lifecycle divergence, and the scripted actions left.
    truncated: Vec<String>,
    unreplayed: usize,
}

fn replay(fixture: &Value, mutant: Option<(&'static str, RefusalMutant)>) -> Replayed {
    assert_eq!(fixture["model_sha256"], Oracle::model_sha());
    let mut r = Replayed::default();
    for (ti, trace) in fixture["traces"].as_array().unwrap().iter().enumerate() {
        if trace["mutant"].as_str().unwrap_or("none") != "none" {
            continue;
        }
        let mut d = Driver::new(
            trace["profile"].as_str().unwrap(),
            trace["pages"].as_u64().unwrap_or(3) as usize,
        );
        d.refinement_schedule = true;
        d.refusal_mutant = mutant;
        let states = trace["states"].as_array().unwrap();
        for (si, entry) in states.iter().enumerate() {
            let action = if let Some(index) = entry["a"].as_u64() {
                &fixture["actions"][index as usize]
            } else {
                &entry["action"]
            };
            let name = action["name"].as_str().unwrap();
            if si == 0 {
                assert_eq!(name, "init");
                continue;
            }
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||
                d.step(name,action["args"].as_array().unwrap()))).unwrap_or_else(|failure| {
                    eprintln!("failure capsule: tested revision = this checkout's HEAD; profile {:?}; trace {ti} {:?}/{si}/{action}; expected enabled model step; observed host/model mismatch; classification=product-or-model divergence",trace["profile"],trace["name"]);
                    std::panic::resume_unwind(failure)
                });
            assert!(
                result,
                "trace {ti} profile {:?} name {:?}/{si}/{name}; twin={} occupied={} queued={} effective={}; action={action}; observed={}",
                trace["profile"], trace["name"], d.twin_failures,
                d.occupied_refusals, d.queued_refusals, d.effective_renames,
                d.abstract_state()
            );
            r.scripted += 1;
            if let Some(action) = d.truncated.take() {
                r.truncated.push(format!(
                    "truncated/{action}@{ti}:{}/{}",
                    trace["profile"].as_str().unwrap_or_default(),
                    trace["name"].as_str().unwrap_or("-")
                ));
                r.unreplayed += states.len() - 1 - si;
                break;
            }
        }
        r.traces += 1;
        r.actions += d.actions;
        r.barriers += d.barriers;
        r.twins += d.twin_failures;
        r.occupied += d.occupied_refusals;
        r.queued += d.queued_refusals;
        r.effective += d.effective_renames;
        r.save_kinds += d.save_kind_refinements;
        for (kind, n) in d.stutters {
            *r.stutters.entry(kind).or_default() += n;
        }
    }
    eprintln!("host replay: {r:?}");
    r
}

/// Ratchet (F9): a committed replay's counts change only by editing here.
fn ratchet(r: &Replayed, expected: &str) {
    assert_eq!(format!("{r:?}"), expected, "replay counts moved");
}

#[test]
fn committed_itf_traces_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/traces.json")).unwrap();
    ratchet(&replay(&fixture, None), "Replayed { traces: 32, scripted: 1280, actions: 1281, barriers: 2259, twins: 1, occupied: 0, queued: 4, effective: 0, save_kinds: 0, stutters: {\"refusal-continuation/check\": 3, \"refusal-continuation/dirSync\": 1, \"refusal-continuation/flushDel\": 3, \"refusal-continuation/observe\": 1, \"refusal-continuation/opRenamePacked\": 1, \"refusal-continuation/rename\": 2, \"refusal-continuation/wEdit\": 2, \"refusal-continuation/wRecv\": 2, \"refusal-continuation/wSend\": 1, \"twin-suffix/dirSync\": 1, \"twin-suffix/rename\": 1}, truncated: [], unreplayed: 0 }");
}

fn frozen_twin_then_delete() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/s2/audit-1019-twin-then-delete.json"
    ))
    .unwrap()
}

// Audit reproducer (F4): a valid Quint trace continues after the permitted
// twin saveFail refinement; its scripted delete of the page that save would
// have published is now Unsaved, and Host must refuse it without mutation.
#[test]
fn audit_1019_twin_then_delete_host_replay() {
    ratchet(&replay(&frozen_twin_then_delete(), None), "Replayed { traces: 1, scripted: 29, actions: 29, barriers: 37, twins: 1, occupied: 0, queued: 0, effective: 0, save_kinds: 0, stutters: {\"twin-continuation/opDelete\": 1, \"twin-suffix/dirSync\": 1, \"twin-suffix/rename\": 1}, truncated: [], unreplayed: 0 }");
}

/// Every compared component, as a deliberately mutating refusal changes it.
const MUTANTS: [(RefusalMutant, &str); 5] = [
    (RefusalMutant::State, "host state"),
    (RefusalMutant::Files, "files"),
    (RefusalMutant::Stable, "stable files"),
    (RefusalMutant::Version, "the host version"),
    (RefusalMutant::Versions, "recorded versions"),
];

/// A refusal of `action` that changes any compared component fails replay.
fn mutating_refusal_fails(fixture: &Value, action: &'static str) {
    for (mutant, component) in MUTANTS {
        let failure = std::panic::catch_unwind(|| replay(fixture, Some((action, mutant))))
            .expect_err("a mutating refusal replayed");
        let message = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or_default();
        assert!(
            message.contains(&format!("{action}: refusal changed {component}")),
            "{mutant:?}: {message}"
        );
    }
}

// The twin continuation keeps the complete no-mutation comparison.
#[test]
fn twin_continuation_rejects_a_mutating_refusal() {
    mutating_refusal_fails(&frozen_twin_then_delete(), "opDelete");
}

fn frozen_continuation_prefixes() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/s2/lane-1020-continuation-prefixes.json"
    ))
    .unwrap()
}

// Two valid Quint prefixes from the 10/20 lane corpus that reach a scripted
// action the occupied/queued-refined oracle disables: a no-op external delete
// (stutter, Host not called) and a refused switch (trace ends there, counted).
#[test]
fn continuation_prefixes_replay_with_counted_truncation() {
    ratchet(&replay(&frozen_continuation_prefixes(), None), "Replayed { traces: 2, scripted: 67, actions: 66, barriers: 97, twins: 0, occupied: 1, queued: 2, effective: 0, save_kinds: 0, stutters: {\"refusal-continuation/check\": 1, \"refusal-continuation/dirSync\": 1, \"refusal-continuation/draftSync\": 1, \"refusal-continuation/extWriteD\": 1, \"refusal-continuation/flush\": 1, \"refusal-continuation/rename\": 1, \"refusal-continuation/wRecv\": 2}, truncated: [\"truncated/switchFin@1:R1/lane-1020-refused-switch\"], unreplayed: 0 }");
}

#[test]
fn noop_external_write_rejects_a_mutation() {
    mutating_refusal_fails(&frozen_continuation_prefixes(), "extWriteD");
}

#[test]
fn truncation_rejects_a_mutating_refused_switch() {
    mutating_refusal_fails(&frozen_continuation_prefixes(), "switchFin");
}

// Without a prior refinement a disabled scripted action still fails replay,
// even on a random-trace (`refinement_schedule`) driver.
#[test]
fn disabled_action_without_refinement_fails_replay() {
    let mut d = Driver::new("base", 3);
    d.refinement_schedule = true;
    assert!(d.step("load", &[json!(1)]));
    assert!(d.step("extWrite", &[json!(1), json!(3)]));
    assert!(d.step("observe", &[json!(1)]));
    assert!(!d.step(
        "opApply",
        &[json!([[1, 1]]), json!([[1, 2]]), json!(NONE), json!(NONE)]
    ));
    assert!(!d.step("opDelete", &[json!(2)]));
    assert!(d.stutters.is_empty());
}

/// A generated corpus must be at least this large (the generator's default
/// four profiles x 64 traces); any larger corpus replays in full.
const MIN_GENERATED_TRACES: usize = 256;
/// At most this share of a generated corpus's scripted actions may replay as
/// refusal stutters after a refinement (1.2% for the 10/19 corpus, 1.4% for the
/// committed sample); more means the replay checks less than the corpus claims.
const MAX_STUTTER_PERCENT: usize = 3;
/// At most this share of generated traces may end at a lifecycle divergence.
const MAX_TRUNCATED_PERCENT: usize = 3;

#[test]
#[ignore = "optional generated corpus: set S3_TRACE_FIXTURE=<decoded traces.json>, run with --ignored"]
fn generated_itf_traces_through_host() {
    let path = std::env::var_os("S3_TRACE_FIXTURE")
        .expect("NOT RUN: S3_TRACE_FIXTURE must name a decoded generated corpus");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let declared = fixture["traces"].as_array().unwrap().len();
    assert!(
        declared >= MIN_GENERATED_TRACES,
        "corpus has {declared} traces, below the declared minimum {MIN_GENERATED_TRACES}"
    );
    let r = std::panic::catch_unwind(|| replay(&fixture, None)).unwrap_or_else(|_| {
        // Report every stopping trace, not only the first.
        let stops: Vec<_> = (fixture["traces"].as_array().unwrap().iter().enumerate())
            .filter(|(_, trace)| {
                let one = json!({"model_sha256": fixture["model_sha256"], "traces": [trace]});
                std::panic::catch_unwind(|| replay(&one, None)).is_err()
            })
            .map(|(i, trace)| format!("{i}:{}/{}", trace["profile"], trace["name"]))
            .collect();
        panic!("{} generated traces stop: {stops:?}", stops.len())
    });
    assert_eq!(r.traces, declared, "every generated trace replays");
    let stutters: usize = r.stutters.values().sum();
    assert!(
        stutters * 100 <= r.scripted * MAX_STUTTER_PERCENT,
        "{stutters} of {} scripted actions replayed as stutters",
        r.scripted
    );
    assert!(
        r.truncated.len() * 100 <= r.traces * MAX_TRUNCATED_PERCENT,
        "{} of {} traces truncated: {:?}",
        r.truncated.len(),
        r.traces,
        r.truncated
    );
    eprintln!("full host ITF: seeds {}; {r:?}", fixture["seeds"]);
}

#[test]
fn committed_witnesses_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    ratchet(&replay(&fixture, None), "Replayed { traces: 44, scripted: 8513, actions: 8513, barriers: 10733, twins: 0, occupied: 0, queued: 0, effective: 0, save_kinds: 0, stutters: {}, truncated: [], unreplayed: 0 }");
}

#[test]
fn short_witnesses_through_host() {
    // Keep the diagnostic-bound traces in the ordinary full replay. Mutation
    // sweeps use this subset to avoid repeating 8,000 counter-only actions.
    let mut fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    fixture["traces"]
        .as_array_mut()
        .unwrap()
        .retain(|trace| trace["states"].as_array().unwrap().len() <= 100);
    ratchet(&replay(&fixture, None), "Replayed { traces: 41, scripted: 504, actions: 504, barriers: 722, twins: 0, occupied: 0, queued: 0, effective: 0, save_kinds: 0, stutters: {}, truncated: [], unreplayed: 0 }");
}

/// The directed paired-recovery vocabulary of the model's
/// `scenarios-paired-recovery.inc`, as driver steps (each compared with the
/// oracle): `admitted`, `writeMember`, `removeSource`, `openPairPage` and
/// `takeDisk` expand exactly as there.
fn paired(d: &mut Driver, macro_name: &str, p: usize) {
    let save = |del: &str| {
        vec![
            (del.to_owned(), json!([p])),
            ("check".into(), json!([])),
            ("rename".into(), json!([])),
            ("dirSync".into(), json!([true])),
        ]
    };
    let steps: Vec<(String, Value)> = match macro_name {
        "admitted" => vec![
            ("load".into(), json!([0])),
            ("load".into(), json!([1])),
            ("load".into(), json!([2])),
            ("opRename".into(), json!([0, 2, [1], [[1, 3]]])),
        ],
        "writeMember" => save("flush"),
        "removeSource" => save("flushDel"),
        "openPairPage" => vec![
            ("wOpen".into(), json!([p])),
            ("deliverUp".into(), json!([true])),
            ("wRecv".into(), json!([p])),
        ],
        "takeDisk" => vec![
            ("wDiscard".into(), json!([p])),
            ("deliverUp".into(), json!([true])),
            ("wRecv".into(), json!([p])),
        ],
        other => panic!("no paired macro {other}"),
    };
    for (name, args) in steps {
        assert!(
            d.step(&name, args.as_array().unwrap()),
            "{macro_name}/{name}/{args}"
        );
    }
}

fn checked_members(profile: &str) -> Driver {
    let mut d = Driver::new(profile, 3);
    paired(&mut d, "admitted", 0);
    paired(&mut d, "writeMember", 2);
    paired(&mut d, "writeMember", 1);
    d
}

/// Model `movedAndReplaced`: the source moved, then the receiver replaced
/// before the post-check could witness it.
fn moved_and_replaced(profile: &str) -> Driver {
    let mut d = checked_members(profile);
    run(
        &mut d,
        &[
            ("flushDel", json!([0])),
            ("check", json!([])),
            ("rename", json!([])),
            ("extWrite", json!([2, 3])),
            ("dirSync", json!([true])),
        ],
    );
    d
}

/// Capsule `ownPairVolatileReceiverIsNotCertified` (item 5): the receiver
/// is replaced durably, then written back to the image volatilely. In a
/// strong profile the source's pre-check holds (no durability witness for
/// the bytes it compares); a weak graph gives no witness to wait for.
#[test]
fn s35_volatile_matching_receiver_is_not_certified() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = checked_members(profile);
        run(
            &mut d,
            &[
                ("extWrite", json!([2, 3])),
                ("extWriteD", json!([2, 1, false])),
                ("flushDel", json!([0])),
                ("check", json!([])),
            ],
        );
        let s = d.abstract_state();
        assert_eq!(s["disk"][0], json!(1), "{profile}");
        if profile.contains("weak") || profile == "all" {
            assert_eq!(s["job"]["phase"], json!(2), "{profile}");
        } else {
            assert_eq!(s["pairs"][0]["phase"], json!("held"), "{profile}");
            assert_eq!(s["job"]["on"], json!(false), "{profile}");
        }
    }
}

/// Capsule `ownPairRestoresSourceAfterRacedReceiver` (item 4): Use Disk on
/// a receiver replaced after the source moved records the choice, keeps
/// the source's admitted preimage selectable across relaunch, and a guarded
/// restoration writes it back over the absent slot.
#[test]
fn s35_restores_source_after_raced_receiver() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = moved_and_replaced(profile);
        paired(&mut d, "openPairPage", 2);
        paired(&mut d, "takeDisk", 2);
        run(&mut d, &[("crash", json!([])), ("launch", json!([]))]);
        let s = d.abstract_state();
        assert_eq!(s["pairs"][0]["phase"], json!("restore"), "{profile}");
        assert_eq!(s["pairs"][0]["decision"], json!("cancelled"), "{profile}");
        run(
            &mut d,
            &[("restoreSource", json!([0])), ("draftSync", json!([0]))],
        );
        paired(&mut d, "writeMember", 0);
        let s = d.abstract_state();
        assert_eq!(
            (s["disk"][0].clone(), s["disk"][2].clone()),
            (json!(1), json!(3)),
            "{profile}"
        );
    }
}

/// The model's `ownPairOccupiedSourceRetainsPreimage` and
/// `ownPairCannotRestoreOverClaimant`: restoration never writes over an
/// occupied source slot or a claimant.
#[test]
fn s35_restoration_refuses_occupied_source() {
    let mut d = moved_and_replaced("base");
    run(&mut d, &[("extWrite", json!([0, 2]))]);
    paired(&mut d, "openPairPage", 2);
    paired(&mut d, "takeDisk", 2);
    run(&mut d, &[("crash", json!([])), ("launch", json!([]))]);
    let s = d.abstract_state();
    assert_eq!(s["pairs"][0]["phase"], json!("retained"));
    assert_eq!(s["pairs"][0]["records"][0]["base"], json!(1));
    assert!(!d.step("restoreSource", &[json!(0)]));

    let mut d = moved_and_replaced("base");
    paired(&mut d, "openPairPage", 2);
    paired(&mut d, "takeDisk", 2);
    run(
        &mut d,
        &[
            ("extClaim", json!([0, true])),
            ("crash", json!([])),
            ("launch", json!([])),
        ],
    );
    assert!(!d.step("restoreSource", &[json!(0)]));
}

/// Item 1 across relaunch: Use Disk on the source of an admitted pair is a
/// durable `sourceDisk` decision, so relaunch never reinstalls the
/// source's deletion and the source keeps its disk text.
#[test]
fn s35_source_use_disk_survives_relaunch() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut d = Driver::new(profile, 3);
        paired(&mut d, "admitted", 0);
        paired(&mut d, "openPairPage", 0);
        paired(&mut d, "takeDisk", 0);
        run(&mut d, &[("crash", json!([])), ("launch", json!([]))]);
        let s = d.abstract_state();
        assert_eq!(s["pairs"][0]["decision"], json!("sourceDisk"), "{profile}");
        assert_eq!(s["disk"][0], json!(1), "{profile}");
        assert!(!s["pages"][0]["held"].as_bool().unwrap(), "{profile}");
    }
}

/// Call 21 (item 3): with draft I/O down (an unowned host keeps it down for
/// good), a paired source's removal takes no destructive step; the model's
/// `flushDel` needs `canDraft`.
#[test]
fn s35_paired_removal_waits_while_drafts_are_down() {
    let mut d = checked_members("base");
    d.host.fs.drafts_unsynced();
    assert_eq!(d.host.start_save(&key(0)), Disposition::Disabled);
    assert!(d.host.job.is_none());
    assert_eq!(
        label(
            &d.host
                .fs
                .physical()
                .files
                .get(&format!("graph/{}", key(0)))
                .cloned()
        ),
        1
    );
}
