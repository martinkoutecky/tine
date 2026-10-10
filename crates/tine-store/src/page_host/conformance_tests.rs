//! Hand-written traces through the conformance driver (STEP2 §8, STEP3
//! review counterexamples).
use super::*;

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
    assert_eq!(d.host.advance_save(0), Disposition::Waiting);
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
    for oracle in fixture["oracles"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["mutant"] == "none")
    {
        let profile = oracle["profile"].as_str().unwrap();
        for (name, program) in fixture["scenarios"].as_object().unwrap() {
            let mut driver = Driver::new(profile, 3);
            let outcome = driver.program(program.as_array().unwrap());
            assert_eq!(
                outcome.map_or_else(|e| e, |_| "pass"),
                oracle["outcomes"][name],
                "{profile}/{name}"
            );
            barriers += driver.barriers;
            actions += driver.actions;
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 636);
    eprintln!("host scenarios: {comparisons} outcomes / {actions} actions / {barriers} barriers");
}

fn replay(fixture: &Value) -> (usize, usize, usize) {
    assert_eq!(fixture["model_sha256"], Oracle::model_sha());
    let mut traces = 0;
    let mut actions = 0;
    let mut barriers = 0;
    for (ti, trace) in fixture["traces"].as_array().unwrap().iter().enumerate() {
        if trace["mutant"].as_str().unwrap_or("none") != "none" {
            continue;
        }
        let mut d = Driver::new(
            trace["profile"].as_str().unwrap(),
            trace["pages"].as_u64().unwrap_or(3) as usize,
        );
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
                    eprintln!("failure capsule: HEAD b0c0f3b4c + lane diff; profile {:?}; trace {ti} {:?}/{si}/{action}; host/model conformance",trace["profile"],trace["name"]);
                    std::panic::resume_unwind(failure)
                });
            assert!(result, "trace {:?}/{si}/{name}", trace["name"]);
        }
        traces += 1;
        actions += d.actions;
        barriers += d.barriers;
    }
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
    assert_eq!(traces, 40);
    eprintln!("short host witnesses: {traces} traces / {actions} actions / {barriers} barriers");
}
