use super::*;
use serde_json::{json, Value};

#[cfg(test)]
#[test]
fn review_h444_ended_source_promise_advances_restoration_watermark() {
    let mut x = init(config("base", "none"));
    for action in [
        Action::WOpen(0),
        Action::DeliverUp(true),
        Action::WRecv(0),
        Action::Load(2),
        Action::OpRename(0, 2, BTreeSet::new(), std::collections::BTreeMap::new()),
        Action::ExtWriteD(0, 2, true),
        Action::WDiscard(0),
        Action::DeliverUp(true),
        Action::WRecv(0),
        Action::ExtWriteD(0, ABSENT, true),
        Action::Observe(0),
        Action::WOpen(2),
        Action::DeliverUp(true),
        Action::WRecv(2),
        Action::WDiscard(2),
    ] {
        x = step(&x, action.clone()).unwrap_or_else(|| panic!("disabled: {action:?}"));
        assert!(checked(&x));
    }
    assert!(x.s.pages[0].held && x.s.pages[0].buf == ABSENT);
    assert!(!x.g.promise[0].on);
    assert_eq!(x.g.promise[0].bytes, ABSENT);
    let before = x.g.promise[0].ver;
    let after = step(&x, Action::DeliverUp(true)).unwrap();
    assert_eq!(after.g.promise[0].ver, after.s.pages[0].ver);
    assert!(after.g.promise[0].ver > before);
    eprintln!(
        "H444 source watermark: {before} -> {}, source ver {}, vc {}",
        after.g.promise[0].ver, after.s.pages[0].ver, after.g.vc
    );
    assert!(checked(&after));
}

fn config(profile: &str, mutant: &str) -> Config {
    Config {
        pages: 3,
        r1: profile == "R1" || profile == "all",
        weak: profile == "weak" || profile == "all",
        mutant: mutant.into(),
    }
}

#[test]
fn review_real_text_predicate_excludes_unknown_observation_sentinel() {
    for profile in ["base", "R1", "weak", "all"] {
        let mut x = init(config(profile, "none"));
        // Restore leaves obs=NONE. Publication and a later rename preserve it;
        // typing against the pre-rename window version then raises conflict.
        for action in [
            Action::WOpen(0),
            Action::DeliverUp(true),
            Action::WRecv(0),
            Action::WEdit(0, 2),
            Action::WSend(0),
            Action::DeliverUp(true),
            Action::WRecv(0),
            Action::SwitchReq,
            Action::DraftSync(0),
            Action::Crash,
            Action::Launch,
            Action::WOpen(0),
            Action::DeliverUp(true),
            Action::WRecv(0),
            Action::Flush(0),
            Action::Check,
            Action::Rename,
            Action::DirSync(true),
            Action::Load(2),
            Action::OpRename(0, 2, BTreeSet::new(), std::collections::BTreeMap::new()),
            Action::WEdit(0, 3),
            Action::WSend(0),
            Action::DeliverUp(true),
            Action::WRecv(0),
        ] {
            x = step(&x, action.clone()).unwrap_or_else(|| panic!("disabled: {action:?}"));
            assert!(checked(&x), "{profile}: unhealthy after {action:?}");
        }
        assert_eq!(x.s.w[0].obs, NONE);
        assert!(x.s.w[0].conf && !x.s.w[0].sent);
        assert!(!x.g.seen[0].contains(&NONE));
        let after = step(&x, Action::WResolve(0, 1)).unwrap();
        assert!(checked(&after));
        assert_eq!(after.g.seen[0], x.g.seen[0], "NONE is not seen text");
    }
}

#[test]
fn claim_and_preflight_disabled_choices() {
    let initial = init(config("base", "none"));
    assert!(step(&initial, Action::PlanCreate(0)).is_none());
    assert!(step(&initial, Action::PlanCreate(2)).is_some());
    assert!(step(&initial, Action::ExtClaim(2, false)).is_none());
    let claimed = step(&initial, Action::ExtClaim(2, true)).unwrap();
    assert!(step(&claimed, Action::ExtClaim(2, true)).is_none());
    assert!(step(&claimed, Action::PlanCreate(2)).is_none());
    let cleared = step(&claimed, Action::ExtClaim(2, false)).unwrap();
    assert_eq!(cleared, initial);
}

#[test]
fn cancelling_discard_waits_for_a_locked_rename_source() {
    // Extend the model's rename-cancel-by-discard witness with MDF's early
    // source save. This fault exposes deliverUp's partner lock independently
    // of the healthy gate that normally prevents the source save from starting.
    let mut x = init(config("base", "MDF"));
    for action in [
        Action::Load(0),
        Action::Load(1),
        Action::WOpen(2),
        Action::DeliverUp(true),
        Action::WRecv(2),
        Action::OpRename(
            0,
            2,
            BTreeSet::from([1]),
            std::collections::BTreeMap::from([(1, 3)]),
        ),
        Action::WRecv(2),
        Action::FlushDel(0),
    ] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    // Closing the destination does not revert its partner. Only Discard must
    // wait for that partner's save; a close request can still be processed.
    let closing = step(&x, Action::WClose(2)).unwrap();
    let closed = step(&closing, Action::DeliverUp(true)).unwrap();
    assert_eq!(closed.s.job, x.s.job);
    assert_eq!(closed.s.gates, x.s.gates);
    assert_eq!(closed.s.pages, x.s.pages);
    x = step(&x, Action::WDiscard(2)).unwrap();
    assert_eq!(partner(&x.s, 2), Some(0));
    assert!(locked(&x, 0));
    assert!(step(&x, Action::DeliverUp(true)).is_none());
    assert!(step(&x, Action::DeliverUp(false)).is_none());
    // Once the failed save releases its lock, the same queued Discard cancels
    // the order and restores the source, as in the original model witness.
    let unlocked = step(&x, Action::SaveFail).unwrap();
    let cancelled = step(&unlocked, Action::DeliverUp(true)).unwrap();
    assert_eq!(cancelled.s.pages[0].buf, 1);
    assert_eq!(cancelled.s.pages[2].buf, ABSENT);
    assert!(cancelled.s.gates.is_empty());
    assert!(cancelled.g.obl.is_empty());
}

#[test]
fn unheld_push_keeps_a_reopening_window_request() {
    // Reorder the clean-close-reopen model witness: the next open is sent
    // before the backend retires the closed page. Its unheld push is not an
    // acknowledgement of the new request and must not clear that request.
    let mut x = init(config("base", "none"));
    for action in [
        Action::WOpen(0),
        Action::DeliverUp(true),
        Action::WRecv(0),
        Action::WClose(0),
        Action::WOpen(0),
        Action::DeliverUp(true),
    ] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    assert!(!x.s.pages[0].held);
    assert!(!x.s.mb[0].held);
    assert_eq!(x.s.mb[0].ack, -1);
    assert!(x.s.w[0].sent);
    let pushed = step(&x, Action::WRecv(0)).unwrap();
    assert_eq!(pushed.s.w[0], x.s.w[0]);
    let opened = step(&pushed, Action::DeliverUp(true)).unwrap();
    let shown = step(&opened, Action::WRecv(0)).unwrap();
    assert!(shown.s.w[0].on);
    assert!(!shown.s.w[0].sent);
    assert_eq!(shown.s.w[0].text, 1);
    assert!(checked(&shown));
}

#[test]
fn switch_request_protects_dirty_pages_and_keeps_clean_siblings() {
    // The switch witness's OPEN/EDIT prefix, with a separately loaded clean
    // sibling. Quint switchReq marks held dirty pages, not every held page.
    let mut x = init(config("base", "none"));
    for action in [
        Action::Load(1),
        Action::WOpen(0),
        Action::DeliverUp(true),
        Action::WRecv(0),
        Action::WEdit(0, 2),
        Action::WSend(0),
        Action::DeliverUp(true),
    ] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    let requested = step(&x, Action::SwitchReq).unwrap();
    assert!(requested.s.pages[0].risk);
    assert_eq!(requested.s.pages[1], x.s.pages[1]);
    assert_eq!(requested.s.pages[2], x.s.pages[2]);
    assert!(step(&requested, Action::DraftSync(1)).is_none());
    let drafted = step(&requested, Action::DraftSync(0)).unwrap();
    assert_eq!(drafted.s.drafts[0].bytes, 2);
    assert!(checked(&drafted));
}

#[test]
fn rename_refuses_a_dirty_absent_destination_until_its_delete_publishes() {
    // Combine the model's delete and held-absent-destination witnesses. The
    // pending deletion is ABSENT but dirty, so D4 still refuses the rename.
    let mut x = init(config("base", "none"));
    for action in [Action::Load(0), Action::Load(1), Action::OpDelete(1)] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    let rename = || Action::OpRename(0, 1, BTreeSet::new(), std::collections::BTreeMap::new());
    assert_eq!(op_cur(&x.s, 1), ABSENT);
    assert!(dirty(&x.s.pages[1]));
    assert!(step(&x, rename()).is_none());
    for action in [
        Action::FlushDel(1),
        Action::Check,
        Action::Rename,
        Action::DirSync(true),
    ] {
        x = step(&x, action).unwrap();
    }
    assert!(clean(&x.s.pages[1]));
    let renamed = step(&x, rename()).unwrap();
    assert_eq!(renamed.s.pages[1].buf, 1);
    assert_eq!(renamed.s.pages[0].buf, ABSENT);
    assert!(checked(&renamed));
}

#[test]
fn moving_a_held_snapshot_after_external_deletion_does_not_claim_an_existing_file() {
    // The model admits operations on the clean held snapshot. If an external
    // writer has already removed its file, renameResolves has no original
    // source file to track; the operation instead protects bytes in its drafts.
    let mut x = init(config("base", "none"));
    for action in [
        Action::Load(0),
        Action::Load(2),
        Action::ExtWriteD(0, ABSENT, true),
    ] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    for action in [
        Action::OpRename(0, 2, BTreeSet::new(), std::collections::BTreeMap::new()),
        Action::OpApply(
            std::collections::BTreeMap::from([(0, ABSENT), (2, 1)]),
            std::collections::BTreeMap::from([(0, 1), (2, ABSENT)]),
            2,
            0,
        ),
    ] {
        let moved = step(&x, action).unwrap();
        assert!(moved.g.renamed.is_empty());
        assert_eq!(moved.s.drafts[2].bytes, 1);
        assert!(rename_resolves(&moved));
        assert!(checked(&moved));
    }
}

#[test]
fn host_oracle_profiles_keep_r1_and_weak_independent() {
    // The save-phases/R1 and weak-power witnesses distinguish the two axes:
    // only R1 admits an external write in the checked window, and only a
    // strong profile makes a successful directory sync update stable bytes.
    for (profile, race_allowed, stable) in [
        ("base", false, 2),
        ("R1", true, 2),
        ("weak", false, 1),
        ("all", true, 1),
    ] {
        let mut oracle = super::conformance::Oracle::new(profile, 3);
        for (name, args) in [
            ("wOpen", vec![json!(0)]),
            ("deliverUp", vec![json!(1)]),
            ("wRecv", vec![json!(0)]),
            ("wEdit", vec![json!(0), json!(2)]),
            ("wSend", vec![json!(0)]),
            ("deliverUp", vec![json!(1)]),
            ("flush", vec![json!(0)]),
            ("check", vec![]),
        ] {
            oracle = oracle.next(name, &args).unwrap();
        }
        let raced = oracle.next("extWriteD", &[json!(0), json!(3), json!(0)]);
        assert_eq!(
            raced.is_some(),
            race_allowed,
            "{profile} checked-window race"
        );
        if let Some(raced) = raced {
            oracle = raced;
        }
        oracle = oracle.next("rename", &[]).unwrap();
        oracle = oracle.next("dirSync", &[json!(1)]).unwrap();
        assert_eq!(
            oracle.state()["s"]["stable"][0],
            json!(stable),
            "{profile} durability"
        );
    }
}

#[test]
fn host_operation_refuses_invalid_move_shapes_in_the_model_domains() {
    // Frozen opApply's move-shape guards, with the step driver's PAGES-valued
    // endpoints, finite effect texts and complete current-byte plan. These are
    // refused choices, so successful scenario replays alone cannot pin them.
    let mut x = init(config("base", "none"));
    assert!(step(
        &x,
        Action::OpApply(
            std::collections::BTreeMap::from([(0, 3)]),
            std::collections::BTreeMap::from([(0, 1)]),
            NONE,
            NONE,
        ),
    )
    .is_none());
    for p in 0..3 {
        x = step(&x, Action::Load(p)).unwrap();
    }
    let plan = std::collections::BTreeMap::from([(0, 1), (1, 2), (2, ABSENT)]);
    for (name, entries, dst, src) in [
        ("empty effects", vec![], NONE, NONE),
        ("missing destination", vec![(0, ABSENT)], 2, 0),
        ("missing source", vec![(2, 1)], 2, 0),
        ("source not deleted", vec![(0, 3), (2, 1)], 2, 0),
        ("destination deleted", vec![(0, ABSENT), (2, ABSENT)], 2, 0),
        ("absent source", vec![(2, ABSENT), (0, 3)], 0, 2),
    ] {
        let eff = entries.into_iter().collect();
        assert!(
            step(&x, Action::OpApply(eff, plan.clone(), dst, src)).is_none(),
            "{name}"
        );
    }
    let moved = step(
        &x,
        Action::OpApply(
            std::collections::BTreeMap::from([(0, ABSENT), (2, 1)]),
            plan.clone(),
            2,
            0,
        ),
    )
    .unwrap();
    assert!(checked(&moved));
    let edited = step(
        &x,
        Action::OpApply(std::collections::BTreeMap::from([(1, 3)]), plan, NONE, NONE),
    )
    .unwrap();
    assert!(checked(&edited));
}

#[test]
fn operation_cleanliness_faults_do_not_bypass_save_locks() {
    // MAC and MRN2 deliberately permit dirty operation pages/referrers. Extend
    // those model scenarios with the save witness's active job: opFree remains
    // an independent admission obligation even under either fault.
    for mutant in ["MAC", "MRN2"] {
        let mut x = init(config("base", mutant));
        for action in [
            Action::Load(1),
            Action::Load(2),
            Action::WOpen(0),
            Action::DeliverUp(true),
            Action::WRecv(0),
            Action::WEdit(0, 2),
            Action::WSend(0),
            Action::DeliverUp(true),
            Action::Flush(0),
        ] {
            x = step(&x, action).expect("model witness prefix is enabled");
        }
        let operation = if mutant == "MAC" {
            Action::OpApply(
                std::collections::BTreeMap::from([(0, 3)]),
                std::collections::BTreeMap::from([(0, 2)]),
                NONE,
                NONE,
            )
        } else {
            Action::OpRename(
                1,
                2,
                BTreeSet::from([0]),
                std::collections::BTreeMap::from([(0, 3)]),
            )
        };
        assert!(step(&x, operation.clone()).is_none(), "{mutant} save lock");
        let unlocked = step(&x, Action::SaveFail).unwrap();
        assert!(
            step(&unlocked, operation).is_some(),
            "{mutant} unlocked control"
        );
    }
}

#[test]
fn discard_after_destination_publication_keeps_the_source_deletion_and_order() {
    // Extend rename-order-gates at the destination's Published witness while
    // its referrer is still outstanding. This destination is no longer a
    // cancelling partner; Discard must preserve the remaining source order.
    let mut x = init(config("base", "none"));
    for action in [
        Action::Load(0),
        Action::Load(1),
        Action::WOpen(2),
        Action::DeliverUp(true),
        Action::WRecv(2),
        Action::OpRename(
            0,
            2,
            BTreeSet::from([1]),
            std::collections::BTreeMap::from([(1, 3)]),
        ),
        Action::WRecv(2),
        Action::Flush(2),
        Action::Check,
        Action::Rename,
        Action::DirSync(true),
    ] {
        x = step(&x, action).expect("model witness prefix is enabled");
    }
    assert_eq!(partner(&x.s, 2), None);
    let queued = step(&x, Action::WDiscard(2)).unwrap();
    let discarded = step(&queued, Action::DeliverUp(true)).unwrap();
    assert_eq!(discarded.s.pages[0], x.s.pages[0]);
    assert_eq!(discarded.s.gates, x.s.gates);
    assert_eq!(discarded.g.obl, x.g.obl);
    assert!(step(&discarded, Action::FlushDel(0)).is_none());
    assert!(checked(&discarded));
}

#[test]
fn cancelling_a_rename_keeps_an_absent_source_protected_with_or_without_its_draft() {
    // Extend the cancellation witness with an external source deletion. The
    // source still holds ABSENT: that equality alone preserves risk even under
    // MRN5's missing draft. MDE deliberately drops this protection instead.
    for (mutant, outside) in [
        ("none", ABSENT),
        ("MRN5", ABSENT),
        ("MDE", ABSENT),
        ("none", 3),
    ] {
        let mut x = init(config("base", mutant));
        for action in [
            Action::Load(0),
            Action::Load(1),
            Action::WOpen(2),
            Action::DeliverUp(true),
            Action::WRecv(2),
            Action::OpRename(
                0,
                2,
                BTreeSet::from([1]),
                std::collections::BTreeMap::from([(1, 3)]),
            ),
            Action::WRecv(2),
        ] {
            x = step(&x, action).expect("model witness prefix is enabled");
        }
        x = step(&x, Action::ExtWriteD(0, outside, true)).unwrap();
        if outside != ABSENT {
            x = step(&x, Action::Observe(0)).unwrap();
            assert!(x.s.pages[0].conflict);
        }
        x = step(&x, Action::WDiscard(2)).unwrap();
        x = step(&x, Action::DeliverUp(true)).unwrap();
        assert_eq!(x.s.pages[0].buf, outside);
        assert_eq!(x.s.pages[0].base, outside);
        assert_eq!(x.s.pages[0].obs, outside);
        assert!(!x.s.pages[0].conflict);
        assert_eq!(
            x.s.pages[0].risk,
            outside == ABSENT && mutant != "MDE",
            "{mutant} protection"
        );
        assert_eq!(
            x.g.promise[0].on,
            outside == ABSENT && mutant != "MRN5",
            "{mutant} draft promise"
        );
    }
}

#[test]
fn cancelling_after_faulty_early_source_publication_respects_its_saved_promise() {
    // MDF permits publishing the source before the destination. Cancel after
    // that fault: restoring an already clean source must keep it clean, and an
    // external replacement ends its saved promise even if ABSENT returns.
    for replaced in [false, true] {
        let mut x = init(config("base", "MDF"));
        for action in [
            Action::Load(0),
            Action::Load(1),
            Action::WOpen(2),
            Action::DeliverUp(true),
            Action::WRecv(2),
            Action::OpRename(
                0,
                2,
                BTreeSet::from([1]),
                std::collections::BTreeMap::from([(1, 3)]),
            ),
            Action::WRecv(2),
            Action::FlushDel(0),
            Action::Check,
            Action::Rename,
            Action::DirSync(true),
        ] {
            x = step(&x, action).expect("model witness prefix is enabled");
        }
        assert!(!rename_resolves(&x));
        if replaced {
            x = step(&x, Action::ExtWriteD(0, 1, true)).unwrap();
            x = step(&x, Action::ExtWriteD(0, ABSENT, true)).unwrap();
        }
        assert!(clean(&x.s.pages[0]));
        let queued = step(&x, Action::WDiscard(2)).unwrap();
        let cancelled = step(&queued, Action::DeliverUp(true)).unwrap();
        assert!(!cancelled.s.pages[0].risk);
        assert_eq!(
            cancelled.g.promise[0].on, !replaced,
            "saved promise replacement"
        );
        // A later save cannot erase the already recorded ordering failure.
        let saving_referrer = step(&cancelled, Action::Flush(1)).unwrap();
        assert!(!order_holds(&saving_referrer));
    }
}

#[test]
fn editing_either_endpoint_after_a_completed_rename_retires_its_ghost_pair() {
    let mut x = init(config("base", "none"));
    for action in [
        Action::Load(0),
        Action::Load(2),
        Action::OpRename(0, 2, BTreeSet::new(), std::collections::BTreeMap::new()),
        Action::Flush(2),
        Action::Check,
        Action::Rename,
        Action::DirSync(true),
        Action::FlushDel(0),
        Action::Check,
        Action::Rename,
        Action::DirSync(true),
    ] {
        x = step(&x, action).expect("model rename witness is enabled");
    }
    assert_eq!(x.g.renamed, BTreeSet::from([(0, 2)]));
    for (p, base) in [(0, ABSENT), (2, 1)] {
        let edited = step(
            &x,
            Action::OpApply(
                std::collections::BTreeMap::from([(p, 3)]),
                std::collections::BTreeMap::from([(p, base)]),
                NONE,
                NONE,
            ),
        )
        .unwrap();
        assert!(edited.g.renamed.is_empty(), "endpoint {p}");
        assert!(checked(&edited));
    }
}

#[test]
fn discarding_the_source_releases_its_obligations_without_cancelling_the_destination() {
    let mut x = init(config("base", "none"));
    for action in [
        Action::Load(1),
        Action::WOpen(0),
        Action::DeliverUp(true),
        Action::WRecv(0),
        Action::WOpen(2),
        Action::DeliverUp(true),
        Action::WRecv(2),
        Action::OpRename(
            0,
            2,
            BTreeSet::from([1]),
            std::collections::BTreeMap::from([(1, 3)]),
        ),
        Action::WRecv(0),
        Action::WDiscard(0),
        Action::DeliverUp(true),
    ] {
        x = step(&x, action).expect("model cancellation witness is enabled");
    }
    assert!(x.g.obl.iter().all(|o| o.0 != 0 && o.1 != 0));
    assert!(!x.g.obl.is_empty());
    assert!(!x.s.gates.is_empty());
    assert!(!gated(&x, 0));
    // The retained source can now take and save fresh window input, although
    // the destination/referrer part of the earlier order is still pending.
    for action in [
        Action::WRecv(0),
        Action::WEdit(0, 3),
        Action::WSend(0),
        Action::DeliverUp(true),
        Action::Flush(0),
    ] {
        x = step(&x, action).unwrap();
    }
    assert!(order_holds(&x));
    assert!(checked(&x));
}

#[test]
fn cancelling_one_of_two_disjoint_renames_keeps_the_other_order() {
    // The model's parameterized witnesses permit five paths. Two independent
    // no-referrer renames can coexist; cancellation releases only its own order.
    let mut cfg = config("base", "none");
    cfg.pages = 5;
    let mut x = init(cfg);
    for action in [
        Action::Load(0),
        Action::Load(1),
        Action::Load(3),
        Action::WOpen(2),
        Action::DeliverUp(true),
        Action::WRecv(2),
        Action::OpRename(0, 2, BTreeSet::new(), std::collections::BTreeMap::new()),
        Action::OpRename(1, 3, BTreeSet::new(), std::collections::BTreeMap::new()),
        Action::WRecv(2),
    ] {
        x = step(&x, action).expect("parameterized model witness is enabled");
    }
    let other: Obligations = x.g.obl.iter().copied().filter(|o| o.3 == 3).collect();
    assert!(!other.is_empty());
    let queued = step(&x, Action::WDiscard(2)).unwrap();
    let cancelled = step(&queued, Action::DeliverUp(true)).unwrap();
    assert_eq!(cancelled.g.obl, other);
    assert_eq!(cancelled.s.gates.len(), 1);
    assert_eq!(cancelled.s.gates[0].dst, 3);
    assert!(step(&cancelled, Action::FlushDel(1)).is_none());
    assert!(checked(&cancelled));
}

// Evaluate the original scenario predicates, including intermediate expects.
// Unsupported syntax is a harness error, never a passing assertion.
fn substitute(e: &Value, name: &str, value: &Value) -> Value {
    if e.is_array() && e[0] == "id" && e[1] == name {
        return json!(["id", value.to_string()]);
    }
    match e {
        Value::Array(a) => json!(a
            .iter()
            .map(|e| substitute(e, name, value))
            .collect::<Vec<_>>()),
        _ => e.clone(),
    }
}

pub(super) fn eval(e: &Value, x: &State) -> Value {
    let tag = e[0].as_str().unwrap();
    match tag {
        "id" => {
            let n = e[1].as_str().unwrap();
            match n {
                "s" => json!(x.s),
                "g" => json!(x.g),
                "true" => json!(true),
                "false" => json!(false),
                "ok" => json!(checked(x)),
                "guarantee" => json!(guarantee(x)),
                "orderHolds" => json!(order_holds(x)),
                "renameResolves" => json!(rename_resolves(x)),
                "canSwitch" => json!(can_switch(x)),
                "ABSENT" => json!(ABSENT),
                "NONE" => json!(NONE),
                "NOPAGE" => json!(nopage()),
                "NODRAFT" => json!(nodraft()),
                "MUTANT" => json!(x.config.mutant),
                "PAGES" => json!((0..x.s.pages.len()).collect::<Vec<_>>()),
                "UNKNOWN" => json!(UNKNOWN),
                // scenarios-s33.inc: `val cur = PAGES.mapBy(p => opCur(s, p))`.
                "cur" => json!((0..x.s.pages.len())
                    .map(|p| json!([p, op_cur(&x.s, p)]))
                    .collect::<Vec<_>>()),
                _ if n.starts_with('"') => json!(n.trim_matches('"')),
                _ => json!(n.parse::<i64>().expect("unsupported identifier")),
            }
        }
        "neg" => json!(-eval(&e[1], x).as_i64().unwrap()),
        "dot" => eval(&e[1], x)[e[2].as_str().unwrap()].clone(),
        "if" => eval(
            if eval(&e[1], x).as_bool().unwrap() {
                &e[2]
            } else {
                &e[3]
            },
            x,
        ),
        "bin" => {
            let a = eval(&e[2], x);
            let b = eval(&e[3], x);
            match e[1].as_str().unwrap() {
                "and" => json!(a.as_bool().unwrap() && b.as_bool().unwrap()),
                "or" => json!(a.as_bool().unwrap() || b.as_bool().unwrap()),
                "==" => json!(a == b),
                "!=" => json!(a != b),
                ">" => json!(a.as_i64().unwrap() > b.as_i64().unwrap()),
                "<" => json!(a.as_i64().unwrap() < b.as_i64().unwrap()),
                ">=" => json!(a.as_i64().unwrap() >= b.as_i64().unwrap()),
                "<=" => json!(a.as_i64().unwrap() <= b.as_i64().unwrap()),
                "+" => json!(a.as_i64().unwrap() + b.as_i64().unwrap()),
                "-" => json!(a.as_i64().unwrap() - b.as_i64().unwrap()),
                "->" => json!([a, b]),
                op => panic!("unsupported operator {op}"),
            }
        }
        "call" => {
            let f = &e[1];
            let args = e[2].as_array().unwrap();
            if f[0] == "id" && (f[1] == "Set" || f[1] == "Map" || f[1] == "List") {
                let mut values: Vec<_> = args.iter().map(|e| eval(e, x)).collect();
                if f[1] == "Set" {
                    values.sort_by_key(|v| v.to_string());
                    values.dedup();
                }
                return json!(values);
            }
            if f[0] == "dot" {
                let method = f[2].as_str().unwrap();
                if method == "forall" {
                    let domain = eval(&f[1], x);
                    let lambda = &args[0];
                    assert_eq!(lambda[1], "=>");
                    return json!(domain.as_array().unwrap().iter().all(|v| {
                        eval(
                            &substitute(&lambda[3], lambda[2][1].as_str().unwrap(), v),
                            x,
                        )
                        .as_bool()
                        .unwrap()
                    }));
                }
                if method == "mapBy" {
                    // A map as its (key, value) pairs, as `Map(..)` evaluates.
                    let domain = eval(&f[1], x);
                    let lambda = &args[0];
                    assert_eq!(lambda[1], "=>");
                    let name = lambda[2][1].as_str().unwrap();
                    return json!(domain
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| json!([v, eval(&substitute(&lambda[3], name, v), x)]))
                        .collect::<Vec<_>>());
                }
                if method == "get" {
                    return eval(&f[1], x)[eval(&args[0], x).as_u64().unwrap() as usize].clone();
                }
                if method == "contains" && f[1][1] == "RACES" {
                    return json!(match eval(&args[0], x).as_str().unwrap() {
                        "R1" => x.config.r1,
                        "weak" => x.config.weak,
                        "crash" | "power" => true,
                        race => panic!("unknown race {race}"),
                    });
                }
                if method == "contains" {
                    return json!(eval(&f[1], x)
                        .as_array()
                        .unwrap()
                        .contains(&eval(&args[0], x)));
                }
                panic!("unsupported method {method}");
            }
            let a = eval(&args[0], x);
            match f[1].as_str().unwrap() {
                "not" => json!(!a.as_bool().unwrap()),
                "clean" => json!(clean(&serde_json::from_value(a).unwrap())),
                "dirty" => json!(dirty(&serde_json::from_value(a).unwrap())),
                "draftEntry" => json!(draft_entry(&serde_json::from_value(a).unwrap())),
                "opClean" => json!(op_clean(&x.s, eval(&args[1], x).as_u64().unwrap() as usize)),
                "opCur" => json!(op_cur(&x.s, eval(&args[1], x).as_u64().unwrap() as usize)),
                function => panic!("unsupported function {function}"),
            }
        }
        _ => panic!("unsupported AST {e}"),
    }
}

pub(super) fn action(name: &str, args: &[Value]) -> Action {
    let p = || args[0].as_u64().unwrap() as usize;
    let v = |i: usize| args[i].as_i64().unwrap();
    let b = |i: usize| args[i].as_bool().unwrap_or_else(|| v(i) == 1);
    match name {
        "wOpen" => Action::WOpen(p()),
        "wEdit" => Action::WEdit(p(), v(1)),
        "wSend" => Action::WSend(p()),
        "wResolve" => Action::WResolve(p(), v(1)),
        "wDiscard" => Action::WDiscard(p()),
        "wOp" => Action::WOp(p(), v(1), v(2)),
        "wOpTo" => Action::WOpTo(p(), v(1) as usize, v(2), v(3)),
        "opDelete" => Action::OpDelete(p()),
        "load" => Action::Load(p()),
        "flushDel" => Action::FlushDel(p()),
        "opRename" => Action::OpRename(
            p(),
            v(1) as usize,
            serde_json::from_value(args[2].clone()).unwrap(),
            args[3]
                .as_array()
                .unwrap()
                .iter()
                .map(|pair| {
                    (
                        pair[0].as_u64().unwrap() as usize,
                        pair[1].as_i64().unwrap(),
                    )
                })
                .collect(),
        ),
        "opApply" => {
            let map = |i: usize| -> std::collections::BTreeMap<usize, Text> {
                args[i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|pair| {
                        (
                            pair[0].as_u64().unwrap() as usize,
                            pair[1].as_i64().unwrap(),
                        )
                    })
                    .collect()
            };
            Action::OpApply(map(0), map(1), v(2), v(3))
        }
        // The trace driver's encoding: which pages change (bits), their new
        // texts, the bytes each page holds (the plan), then (dst, src).
        "opApplyPacked" => Action::OpApply(
            (0..3).filter(|&i| b(i)).map(|i| (i, v(3 + i))).collect(),
            (0..3).map(|i| (i, v(6 + i))).collect(),
            v(9),
            v(10),
        ),
        "powerK" => Action::PowerK(serde_json::from_value(args[0].clone()).unwrap()),
        "powerKBits" => Action::PowerK((0..3).filter(|&i| b(i)).collect()),
        "opRenamePacked" => Action::OpRename(
            p(),
            v(1) as usize,
            (0..3)
                .filter(|&i| i != p() && i != v(1) as usize && b(2 + i))
                .collect(),
            (0..3).map(|i| (i, v(5 + i))).collect(),
        ),
        "opRenameRaw" => Action::OpRename(
            p(),
            v(1) as usize,
            (0..3).filter(|&i| b(2 + i)).collect(),
            (0..3).map(|i| (i, v(5 + i))).collect(),
        ),
        "wClose" => Action::WClose(p()),
        "wRecv" => Action::WRecv(p()),
        "deliverUp" => Action::DeliverUp(b(0)),
        "observe" => Action::Observe(p()),
        "flush" => Action::Flush(p()),
        "check" => Action::Check,
        "rename" => Action::Rename,
        "dirSync" => Action::DirSync(b(0)),
        "saveFail" => Action::SaveFail,
        "draftSync" => Action::DraftSync(p()),
        "switchReq" => Action::SwitchReq,
        "switchFin" => Action::SwitchFin,
        "planCreate" => Action::PlanCreate(p()),
        "extClaim" => Action::ExtClaim(p(), b(1)),
        "extWrite" => Action::ExtWriteD(p(), v(1), true),
        "extWriteD" => Action::ExtWriteD(p(), v(1), b(2)),
        "crash" => Action::Crash,
        "windowCrash" => Action::WindowCrash,
        "power" => Action::Power(b(0), b(1)),
        "launch" => Action::Launch,
        _ => panic!("unknown action {name}"),
    }
}

#[test]
fn s31_load_is_one_unheld_path_and_keeps_the_loaded_snapshot() {
    let x = init(config("base", "none"));
    let loaded = step(&x, Action::Load(0)).unwrap();
    assert_eq!(
        loaded.s.pages[0],
        Page {
            held: true,
            buf: 1,
            base: 1,
            obs: 1,
            ver: 1,
            ..nopage()
        }
    );
    assert!(step(&loaded, Action::Load(0)).is_none());
    let external = step(&loaded, Action::ExtWriteD(0, 3, true)).unwrap();
    assert_eq!(external.s.pages[0], loaded.s.pages[0]);
    let absent = step(&external, Action::Load(2)).unwrap();
    assert_eq!(
        absent.s.pages[2],
        Page {
            held: true,
            buf: ABSENT,
            base: ABSENT,
            obs: ABSENT,
            ver: 2,
            ..nopage()
        }
    );
    assert!(guarantee(&absent));
    assert!(step(&step(&absent, Action::Crash).unwrap(), Action::Load(1)).is_none());
}

#[test]
fn s31_delete_and_rename_refuse_missing_held_pages() {
    let x = init(config("base", "none"));
    assert!(step(&x, Action::OpDelete(0)).is_none());
    assert!(step(&step(&x, Action::Load(0)).unwrap(), Action::OpDelete(0)).is_some());
    let refs = BTreeSet::from([1]);
    let rt = std::collections::BTreeMap::from([(1, 3)]);
    for missing in 0..3 {
        let mut partial = x.clone();
        for p in (0..3).filter(|p| *p != missing) {
            partial = step(&partial, Action::Load(p)).unwrap();
        }
        assert!(step(&partial, Action::OpRename(0, 2, refs.clone(), rt.clone())).is_none());
        partial = step(&partial, Action::Load(missing)).unwrap();
        assert!(step(&partial, Action::OpRename(0, 2, refs.clone(), rt.clone())).is_some());
    }
    let mut refs_only = step(&x, Action::Load(2)).unwrap();
    refs_only = step(&refs_only, Action::Load(1)).unwrap();
    assert!(step(&refs_only, Action::OpRename(2, 0, refs, rt)).is_some());
    assert!(!refs_only.s.pages[0].held);
}

fn program(ops: &[Value], x: &mut State) -> Result<(), &'static str> {
    for op in ops {
        match op[0].as_str().unwrap() {
            "init" => *x = init(x.config.clone()),
            "expect" => {
                if !eval(&op[1], x).as_bool().unwrap() {
                    return Err("assertion");
                }
            }
            "if" => program(
                op[if eval(&op[1], x).as_bool().unwrap() {
                    2
                } else {
                    3
                }]
                .as_array()
                .unwrap(),
                x,
            )?,
            "fail" => {
                let mut probe = x.clone();
                if program(op[1].as_array().unwrap(), &mut probe).is_ok() {
                    return Err("false");
                }
            }
            "action" => {
                let args: Vec<_> = op[2]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| eval(e, x))
                    .collect();
                *x = step(x, action(op[1].as_str().unwrap(), &args)).ok_or("disabled")?;
            }
            _ => panic!("unknown instruction {op}"),
        }
    }
    Ok(())
}

fn scenarios() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/s2/scenarios.json")).unwrap()
}

#[test]
fn scenario_outcomes_equal_quint_in_all_profiles_and_mutants() {
    let fixture = scenarios();
    assert_eq!(fixture["model_sha256"], MODEL_SHA);
    assert_eq!(
        fixture["scenario_sha256"],
        "9200a28404f60177fa3dadda6e3b932f8a866c6bb7a2110854fcd7e181abe6b5"
    );
    let scenarios = fixture["scenarios"].as_object().unwrap();
    assert_eq!(scenarios.len(), 194);
    assert_eq!(fixture["oracles"].as_array().unwrap().len(), 42);
    let mut comparisons = 0;
    for oracle in fixture["oracles"].as_array().unwrap() {
        let profile = oracle["profile"].as_str().unwrap();
        let mutant = oracle["mutant"].as_str().unwrap();
        for (name, ops) in scenarios {
            let mut x = init(config(profile, mutant));
            let result = program(ops.as_array().unwrap(), &mut x);
            let status = result.err().unwrap_or("pass");
            assert_eq!(
                status,
                oracle["outcomes"][name].as_str().unwrap(),
                "{profile}/{mutant}/{name}: bad={:?}",
                x.g.bad
            );
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 8148);
}

#[test]
#[cfg(test)]
fn every_declared_mutant_is_caught() {
    let fixture = scenarios();
    let mut count = 0;
    for oracle in fixture["oracles"].as_array().unwrap() {
        let mutant = oracle["mutant"].as_str().unwrap();
        if mutant == "none" {
            continue;
        }
        let mut x = init(config(oracle["profile"].as_str().unwrap(), mutant));
        let result = program(
            fixture["scenarios"][format!("m{mutant}")]
                .as_array()
                .unwrap(),
            &mut x,
        );
        assert_eq!(result, Err("assertion"), "{mutant}");
        // MDE's own scenario checks risk before the later power-loss suffix.
        // Run that suffix to demonstrate the actual A violation as well.
        if mutant == "MDE" {
            if x.s.drafts[0] != draft_entry(&x.s.pages[0]) {
                x = draft_sync(&x, 0).unwrap();
            }
            x = power(&x, false, true).unwrap();
        }
        if mutant != "MRN5" && mutant != "MRH" {
            assert!(!checked(&x), "{mutant} failed only a scenario predicate");
        }
        eprintln!(
            "{mutant}: caught; A={} B={} Bprime={} C={} G={} accepted={} order={} resolves={} bad={:?}",
            no_loss(&x),
            clause_b(&x),
            clause_b_prime(&x),
            clause_c(&x),
            clause_g(&x),
            accepted(&x),
            order_holds(&x),
            rename_resolves(&x),
            x.g.bad
        );
        count += 1;
    }
    assert_eq!(count, 38);
}

fn replay_traces(fixture: &Value) -> (usize, usize) {
    assert_eq!(fixture["model_sha256"], MODEL_SHA);
    let mut states = 0;
    let traces = fixture["traces"].as_array().unwrap();
    for (ti, trace) in traces.iter().enumerate() {
        let profile = trace["profile"].as_str().unwrap();
        let mutant = trace["mutant"].as_str().unwrap_or("none");
        let mut cfg = config(profile, mutant);
        cfg.pages = trace["pages"].as_u64().unwrap_or(3) as usize;
        // The host-facing wrapper must replay the same Quint states, including
        // R1/weak profile selection; checking only State misses its constructor.
        let mut oracle =
            (mutant == "none").then(|| super::conformance::Oracle::new(profile, cfg.pages));
        let mut x = init(cfg);
        let mut expected = Value::Null;
        for (si, entry) in trace["states"].as_array().unwrap().iter().enumerate() {
            let recorded = if let Some(index) = entry["a"].as_u64() {
                &fixture["actions"][index as usize]
            } else {
                &entry["action"]
            };
            let name = recorded["name"].as_str().unwrap();
            if si == 0 {
                assert_eq!(name, "init");
            } else {
                x = step(&x, action(name, recorded["args"].as_array().unwrap()))
                    .unwrap_or_else(|| panic!("disabled {profile}/{ti}/{si}/{name}"));
                if let Some(current) = oracle.as_mut() {
                    *current = current
                        .next(name, recorded["args"].as_array().unwrap())
                        .unwrap_or_else(|| panic!("oracle disabled {profile}/{ti}/{si}/{name}"));
                }
            }
            if entry["state"].is_object() {
                expected = entry["state"].clone();
            } else {
                for change in entry["delta"].as_array().unwrap() {
                    let mut target = &mut expected;
                    let path = &fixture["paths"][change[0].as_u64().unwrap() as usize];
                    for key in path.as_array().unwrap() {
                        target = if let Some(key) = key.as_str() {
                            &mut target[key]
                        } else {
                            &mut target[key.as_u64().unwrap() as usize]
                        };
                    }
                    if change[2].as_bool() == Some(true) {
                        target
                            .as_array_mut()
                            .unwrap()
                            .extend(change[1].as_array().unwrap().iter().cloned());
                    } else {
                        *target = change[1].clone();
                    }
                }
            }
            // All Sys and Ghost fields, including inactive padding, queue order,
            // durable bytes, promises, seen/wrote/owed, and transition tags.
            assert_eq!(json!(x), expected, "{profile}/trace {ti}/step {si}/{name}");
            if let Some(oracle) = &oracle {
                assert_eq!(
                    oracle.state(),
                    expected,
                    "host oracle {profile}/trace {ti}/step {si}/{name}"
                );
            }
            if mutant == "none" {
                assert!(checked(&x), "{profile}/{ti}/{si}");
            }
            let bad = expected["g"]["bad"].as_array().unwrap();
            let has = |tag: &str| bad.iter().any(|v| v.as_str() == Some(tag));
            assert_eq!(clause_b(&x), !has("B-unsaved-replaced"));
            assert_eq!(clause_c(&x), !has("C-overwrote-external"));
            assert_eq!(clause_g(&x), !has("G-overwrote-unseen"));
            assert_eq!(
                clause_b_prime(&x),
                !has("B'-window-replaced") && !has("B'-submit-not-taken") && accepted(&x)
            );
            if entry["predicates"].is_object() {
                assert_eq!(no_loss(&x), entry["predicates"]["loss"].as_bool().unwrap());
                assert_eq!(
                    accepted(&x),
                    entry["predicates"]["accepted"].as_bool().unwrap()
                );
                assert_eq!(within(&x), entry["predicates"]["within"].as_bool().unwrap());
                assert_eq!(trashed(&x), entry["predicates"]["trash"].as_bool().unwrap());
                assert_eq!(
                    guarantee(&x),
                    entry["predicates"]["guarantee"].as_bool().unwrap()
                );
                assert_eq!(
                    order_holds(&x),
                    entry["predicates"]["order"].as_bool().unwrap()
                );
                assert_eq!(
                    rename_resolves(&x),
                    entry["predicates"]["resolves"].as_bool().unwrap()
                );
            }
            if let Some(expected) = entry["within"].as_bool() {
                assert_eq!(within(&x), expected);
            }
            if let Some(choices) = entry["enabled"].as_array() {
                for (ci, allowed) in choices.iter().enumerate() {
                    let choice = &fixture["choices"][ci];
                    let name = choice["name"].as_str().unwrap();
                    let selected = action(name, choice["args"].as_array().unwrap());
                    assert_eq!(
                        step(&x, selected).is_some(),
                        allowed.as_bool().unwrap(),
                        "{profile}/trace {ti}/step {si}/guard {choice}"
                    );
                }
            }
            states += 1;
        }
    }
    (traces.len(), states)
}

#[test]
fn committed_random_traces_match_every_state() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/traces.json")).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert_eq!(traces, 32);
    assert!(states >= traces);
}

#[test]
fn targeted_quint_witnesses_match_states_and_disabled_choices() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert!(traces > 0 && states > traces);
}

#[test]
#[cfg(test)]
fn generated_random_traces_match_every_state() {
    // Optional larger local corpus; normal CI always runs the committed sample.
    let Ok(path) = std::env::var("S3_TRACE_FIXTURE") else {
        return;
    };
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (traces, states) = replay_traces(&fixture);
    assert!(traces >= 256);
    eprintln!("s3 full corpus: {traces} traces / {states} full states matched");
}
