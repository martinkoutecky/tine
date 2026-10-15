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
    for oracle in fixture["oracles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["mutant"] == "none")
    {
        let profile = oracle["profile"].as_str().unwrap();
        for (name, program) in fixture["scenarios"].as_object().unwrap() {
            let mut driver = Driver::new(profile, 3);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                driver.program(program.as_array().unwrap())
            }))
            .unwrap_or_else(|failure| {
                eprintln!("failure capsule: {profile}/{name}; program={program}");
                std::panic::resume_unwind(failure)
            });
            if outcome != Err("twin-saveFail-refinement") {
                assert_eq!(
                    outcome.map_or_else(|e| e, |_| "pass"),
                    oracle["outcomes"][name],
                    "{profile}/{name}"
                );
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
    assert_eq!(comparisons, 776);
    eprintln!("host scenarios: {comparisons} outcomes / {actions} actions / {barriers} barriers / {twins} twin saveFail / {occupied} occupied stutters / {queued} queued stutters / {effective} effective renames");
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

fn replay(fixture: &Value) -> (usize, usize, usize) {
    assert_eq!(fixture["model_sha256"], Oracle::model_sha());
    let mut traces = 0;
    let mut actions = 0;
    let mut barriers = 0;
    let mut twins = 0;
    let mut occupied = 0;
    let mut queued = 0;
    let mut effective = 0;
    let mut save_kinds = 0;
    for (ti, trace) in fixture["traces"].as_array().unwrap().iter().enumerate() {
        if trace["mutant"].as_str().unwrap_or("none") != "none" {
            continue;
        }
        let mut d = Driver::new(
            trace["profile"].as_str().unwrap(),
            trace["pages"].as_u64().unwrap_or(3) as usize,
        );
        d.refinement_schedule = true;
        for (si, entry) in trace["states"].as_array().unwrap().iter().enumerate() {
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
                    eprintln!("failure capsule: HEAD 1b040e6c7a20 + lane diff; profile {:?}; trace {ti} {:?}/{si}/{action}; expected enabled model step; observed host/model mismatch; classification=product-or-model divergence",trace["profile"],trace["name"]);
                    std::panic::resume_unwind(failure)
                });
            assert!(result, "trace {:?}/{si}/{name}", trace["name"]);
        }
        traces += 1;
        actions += d.actions;
        barriers += d.barriers;
        twins += d.twin_failures;
        occupied += d.occupied_refusals;
        queued += d.queued_refusals;
        effective += d.effective_renames;
        save_kinds += d.save_kind_refinements;
    }
    eprintln!("host replay alternatives: {twins} twin saveFail / {occupied} occupied stutters / {queued} queued stutters / {effective} effective renames / {save_kinds} save-kind refinements");
    (traces, actions, barriers)
}

#[test]
fn committed_itf_traces_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/traces.json")).unwrap();
    let (traces, actions, barriers) = replay(&fixture);
    assert_eq!(traces, 32);
    eprintln!("host ITF: {traces} traces / {actions} actions / {barriers} barriers");
}

#[test]
fn generated_itf_traces_through_host() {
    if let Some(path) = std::env::var_os("S3_TRACE_FIXTURE") {
        let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(fixture["model_sha256"], Oracle::model_sha());
        let (traces, actions, barriers) = replay(&fixture);
        assert_eq!(traces, 256);
        eprintln!("full host ITF: {traces} traces / {actions} actions / {barriers} barriers");
    }
}

#[test]
fn committed_witnesses_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    let (traces, actions, barriers) = replay(&fixture);
    assert!(traces > 0);
    eprintln!("host witnesses: {traces} traces / {actions} actions / {barriers} barriers");
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
    let (traces, actions, barriers) = replay(&fixture);
    assert_eq!(traces, 41);
    eprintln!("short host witnesses: {traces} traces / {actions} actions / {barriers} barriers");
}
