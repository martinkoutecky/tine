//! Independent physical steps; blocked host actions stutter, while faults
//! resolve pending effects from surviving files before the abstract fault.
use super::*;

impl Driver {
    fn stepped(profile: &str) -> Self {
        let mut d = Self::new(profile, 3);
        d.stepped = true;
        d.prepare_operations = true;
        d
    }

    fn tick_draft(&mut self) {
        let worker = self.host.worker.as_ref().unwrap();
        let application = worker.application.is_some()
            && !matches!(worker.application, Some(Application::Representation))
            && matches!(worker.task.stage, Stage::Present | Stage::Absent)
            && (worker.task.bytes.is_some() || worker.remaining.is_empty());
        let present = worker.task.stage == Stage::Present;
        let removal = matches!(worker.application, Some(Application::Removal(_)));
        self.host.advance_draft();
        if application {
            let (name, mut args, inc, ver) = self.effect.take().unwrap();
            self.register_versions(inc, ver);
            if present || removal || name == "deliverUp" {
                if name == "deliverUp" {
                    args[0] = json!(present);
                }
                self.finish(&name, &args);
            } else {
                self.compare("failed fresh install stutters");
            }
        } else {
            self.compare("physical draft barrier");
        }
    }

    fn fault(&mut self, power: bool, keep_drafts: bool) {
        let mut survivor = self.host.fs.clone();
        if power {
            survivor.power(&BTreeSet::new(), keep_drafts);
        } else {
            survivor.crash();
        }
        if self.effect.is_some() {
            let worker = self.host.worker.as_ref().unwrap();
            let present = survivor
                .files
                .contains_key(&format!("draft/{}", worker.task.name));
            let removal = match &worker.application {
                Some(Application::Removal(p)) => !drafts::scan(survivor.draft_files(false))
                    .logical
                    .contains_key(p),
                _ => false,
            };
            let new_state = worker.task.bytes.is_some() && present;
            if new_state || removal {
                // The abstraction linearizes a surviving pending effect just
                // before the fault. No real phase is advanced or I/O retried.
                let physical = self.host.fs.clone();
                self.host.fs.stable.retain(|k, _| !k.starts_with("draft/"));
                self.host.fs.stable.extend(
                    survivor
                        .files
                        .iter()
                        .filter(|(k, _)| k.starts_with("draft/"))
                        .map(|(k, v)| (k.clone(), v.clone())),
                );
                let worker = self.host.worker.as_mut().unwrap();
                worker.task.stage = if new_state {
                    Stage::Present
                } else {
                    Stage::Absent
                };
                if removal {
                    worker.remaining.clear();
                }
                self.tick_draft();
                self.host.fs = physical;
            } else {
                self.effect = None;
            }
        }
        // Apply the selected physical directory outcome; graph keep=false.
        self.host.fs = survivor;
        self.host.stop();
        self.windows.fill(Window::default());
        self.pending_ids.fill(None);
        self.finish(
            if power { "power" } else { "crash" },
            if power {
                &[json!(false), json!(false)]
            } else {
                &[]
            },
        );
    }

    // Dispositions are fixed by the host contract, not chosen from the oracle.
    fn attempt(&mut self, name: &str, args: &[Value]) -> bool {
        let p = args.first().and_then(Value::as_u64).unwrap_or(0) as usize;
        // One host start_save API represents the model's two payload kinds.
        // A mismatched model spelling has no separate backend action to probe.
        if matches!(name, "flush" | "flushDel")
            && self
                .host
                .pages
                .get(&key(p))
                .is_some_and(|pg| pg.buf.is_some() != (name == "flush"))
        {
            return false;
        }
        let blocked = match name {
            "deliverUp" => self.host.abstract_queue().first().is_some_and(|r|
                    self.host.busy(&r.page) || self.host.allocator_busy()
                    || matches!(&r.kind, RequestKind::Move { receiver, .. } if self.host.busy(receiver))),
            "observe" => self.host.pages.contains_key(&key(p))
                && (self.host.busy(&key(p)) || self.host.allocator_busy()),
            "load" => !self.host.pages.contains_key(&key(p))
                && (self.host.busy(&key(p)) || self.host.allocator_busy()),
            "flush" | "flushDel" => self.host.busy(&key(p)),
            "check" | "rename" | "dirSync" | "saveFail" => self.host.job.as_ref()
                .is_some_and(|j| self.host.worker.as_ref().is_some_and(|w| w.pages.contains(&j.page))
                    || name == "check" && j.phase == SavePhase::Check && self.host.allocator_busy()
                    && j.base != Base::Known(self.host.fs.files.get(&format!("graph/{}", j.page)).cloned())),
            "draftSync" | "opRename" => self.host.worker.is_some(),
            "opDelete" => self.host.worker.is_some() || self.host.busy(&key(p)),
            "switchReq" => self.host.worker.is_some(),
            "switchFin" => self.host.worker.is_some() || self.host.applying.is_some()
                || !self.host.queue.is_empty() || self.host.job.is_some()
                || self.host.outbox.values().any(|m| m.answer.is_some()),
            _ => false,
        };
        if blocked {
            match name {
                "deliverUp" => {
                    if self.host.applying.is_some() {
                        assert_eq!(self.host.apply_request(), Disposition::Waiting);
                    } else {
                        assert_eq!(self.host.dequeue(), Disposition::Waiting);
                    }
                }
                "observe" => assert_eq!(self.host.observe(&key(p)), Disposition::Waiting),
                "load" => assert_eq!(self.host.load(&key(p)), Disposition::Waiting),
                "flush" | "flushDel" => {
                    let expected = if self.host.job.is_some() {
                        Disposition::Disabled
                    } else {
                        Disposition::Waiting
                    };
                    assert_eq!(self.host.start_save(&key(p)), expected);
                }
                "draftSync" => assert_eq!(self.host.begin_draft(&key(p)), Disposition::Waiting),
                "opDelete" => assert_eq!(self.host.delete(&key(p)), Disposition::Waiting),
                "switchReq" => assert_eq!(self.host.switch_request(), Disposition::Waiting),
                "switchFin" => assert_eq!(self.host.switch_finish(), Disposition::Waiting),
                "check" | "rename" | "dirSync" | "saveFail" => {
                    assert_eq!(self.host.advance_save(0), Disposition::Waiting)
                }
                _ => {}
            }
            self.compare("blocked action retains custody");
            return false;
        }
        if self.oracle.next(name, args).is_none() {
            match name {
                "flush" | "flushDel" => {
                    assert_eq!(self.host.start_save(&key(p)), Disposition::Disabled)
                }
                "opDelete" if self.host.alive && !self.host.busy(&key(p)) => {
                    assert_eq!(self.host.delete_loaded(&key(p)), Disposition::Refused);
                }
                "opRename" if self.host.pages.len() == self.host.keys.len() => {
                    let q = args[1].as_u64().unwrap() as usize;
                    let refs: BTreeSet<usize> = serde_json::from_value(args[2].clone()).unwrap();
                    let refkeys = refs.iter().map(|&r| key(r)).collect();
                    let rt: BTreeMap<usize, i64> = args[3]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|pair| {
                            (
                                pair[0].as_u64().unwrap() as usize,
                                pair[1].as_i64().unwrap(),
                            )
                        })
                        .collect();
                    assert_eq!(
                        self.host
                            .rename_with(&key(p), &key(q), &refkeys, |bytes, k, moving| {
                                let r = k.trim_end_matches(".md").parse::<usize>().unwrap();
                                Ok(if !moving && refs.contains(&r) {
                                    text(rt[&r])
                                } else {
                                    bytes.clone()
                                })
                            }),
                        Disposition::Refused
                    );
                }
                "switchFin" => assert_eq!(self.host.switch_finish(), Disposition::Waiting),
                "observe" => assert_eq!(self.host.observe(&key(p)), Disposition::Disabled),
                "load" => assert_eq!(self.host.load(&key(p)), Disposition::Disabled),
                _ => return false, // Client actions have no backend call.
            }
            self.compare("disabled disposition");
            return false;
        }
        if name == "check"
            && self
                .host
                .job
                .as_ref()
                .is_some_and(|j| j.phase == SavePhase::Temp)
        {
            self.host.advance_save(0);
            self.compare("temp synced barrier");
            return true;
        }
        if name == "dirSync"
            && self
                .host
                .job
                .as_ref()
                .is_some_and(|j| j.phase == SavePhase::TrashSync)
        {
            self.host.advance_save(0);
            self.compare("trash directory sync barrier");
            return true;
        }
        if matches!(name, "crash" | "power") {
            self.fault(name == "power", false);
            true
        } else {
            self.step(name, args)
        }
    }

    fn settle(&mut self) {
        for _ in 0..300 {
            if self.host.worker.is_none() {
                return;
            }
            self.tick_draft();
        }
        panic!("working disk did not terminate pending effect");
    }
}

fn open(d: &mut Driver, p: usize) {
    run(
        d,
        &[
            ("wOpen", json!([p])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([p])),
        ],
    );
}

fn edit(d: &mut Driver, p: usize, t: i64) {
    run(
        d,
        &[
            ("wEdit", json!([p, t])),
            ("wSend", json!([p])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([p])),
        ],
    );
}

fn risk_draft(d: &mut Driver, p: usize) {
    if d.oracle.next("switchReq", &[]).is_some() {
        assert!(d.step("switchReq", &[]));
    }
    assert!(d.step("draftSync", &[json!(p)]));
    d.settle();
}

#[test]
fn failed_move_into_drafted_receiver_at_every_install_and_cleanup_cut() {
    for cut in 0..12 {
        for (power, keep) in [(false, false), (true, false), (true, true)] {
            let mut d = Driver::stepped("base");
            open(&mut d, 0);
            open(&mut d, 1);
            edit(&mut d, 1, 3);
            risk_draft(&mut d, 1);
            run(
                &mut d,
                &[("wOp", json!([0, 2, 1])), ("deliverUp", json!([true]))],
            );
            d.host.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
            for _ in 0..cut {
                if d.host.worker.is_some() {
                    d.tick_draft();
                }
            }
            d.fault(power, keep);
            assert!(d.step("launch", &[]));
            d.settle();
        }
    }
}

#[test]
fn failed_page_vehicle_newer_edits_and_every_fault_cut() {
    for cut in 0..10 {
        for (power, keep) in [(false, false), (true, false), (true, true)] {
            let mut d = Driver::stepped("base");
            open(&mut d, 0);
            edit(&mut d, 0, 2);
            run(
                &mut d,
                &[("switchReq", json!([])), ("draftSync", json!([0]))],
            );
            d.host.fs.inject(Phase::DraftSync, [Fault::Before; 3]);
            // Unsent local typing may proceed while the page snapshot is held.
            assert!(d.step("wEdit", &[json!(0), json!(3)]));
            for _ in 0..cut {
                if d.host.worker.is_some() {
                    d.tick_draft();
                }
            }
            d.fault(power, keep);
            assert!(d.step("launch", &[]));
            d.settle();
        }
    }
}

#[test]
fn durable_unapplied_operation_crash_and_power_and_allocator_wait() {
    for power in [false, true] {
        let mut d = Driver::stepped("base");
        open(&mut d, 1);
        open(&mut d, 2);
        run(&mut d, &[("opDelete", json!([0]))]);
        edit_local_submit(&mut d, 1, 3);
        assert!(!d.attempt("deliverUp", &[json!(true)]));
        for _ in 0..3 {
            d.tick_draft();
        }
        assert_eq!(d.host.worker.as_ref().unwrap().task.stage, Stage::Present);
        d.fault(power, false);
        assert!(d.step("launch", &[]));
        d.settle();
    }
}

fn edit_local_submit(d: &mut Driver, p: usize, t: i64) {
    run(d, &[("wEdit", json!([p, t])), ("wSend", json!([p]))]);
}

#[test]
fn equal_bytes_known_and_unknown_refresh_every_retirement_cut_both_orders() {
    for unknown in [false, true] {
        for cut in 0..10 {
            for (power, keep) in [(false, false), (true, false), (true, true)] {
                let mut d = Driver::stepped("base");
                open(&mut d, 0);
                if unknown {
                    run(
                        &mut d,
                        &[
                            ("wEdit", json!([0, 2])),
                            ("extWrite", json!([0, 3])),
                            ("observe", json!([0])),
                            ("wRecv", json!([0])),
                            ("wSend", json!([0])),
                            ("deliverUp", json!([true])),
                            ("wRecv", json!([0])),
                        ],
                    );
                    assert_eq!(d.host.pages[&key(0)].base, Base::Unknown);
                } else {
                    edit(&mut d, 0, 2);
                }
                risk_draft(&mut d, 0);
                let old = d.host.logical_drafts()[&key(0)].clone();
                run(
                    &mut d,
                    &[
                        ("extWrite", json!([0, 2])),
                        ("observe", json!([0])),
                        ("draftSync", json!([0])),
                    ],
                );
                for _ in 0..cut {
                    if d.host.worker.is_some() {
                        d.tick_draft();
                    }
                }
                let mut files = d.host.fs.draft_files(false);
                let a = drafts::scan(files.clone()).logical;
                files.reverse();
                assert_eq!(drafts::scan(files).logical, a);
                if let Some(new) = a.get(&key(0)).filter(|r| r.wseq > old.wseq) {
                    assert_eq!(old.version, new.version);
                    assert_eq!(new.base, Base::Known(text(2)));
                }
                d.fault(power, keep);
                assert!(d.step("launch", &[]));
                d.settle();
            }
        }
    }
}

#[test]
fn removal_between_each_oldest_first_unlink_and_sync() {
    for cut in 0..11 {
        for (power, keep) in [(false, false), (true, false), (true, true)] {
            let mut d = Driver::stepped("base");
            open(&mut d, 0);
            let mut old_files = Vec::new();
            for t in [2, 3, 2] {
                edit(&mut d, 0, t);
                risk_draft(&mut d, 0);
                old_files.extend(d.host.fs.draft_files(false));
            }
            // These are already acknowledged superseded representation files,
            // as can remain at retirement cuts; no new logical state is added.
            for (name, bytes) in old_files {
                let path = format!("draft/{name}");
                let bytes: Arc<[u8]> = Arc::from(bytes);
                d.host.fs.files.insert(path.clone(), bytes.clone());
                d.host.fs.stable.insert(path, bytes);
            }
            run(
                &mut d,
                &[
                    ("wDiscard", json!([0])),
                    ("deliverUp", json!([true])),
                    ("wRecv", json!([0])),
                    ("draftSync", json!([0])),
                ],
            );
            for _ in 0..cut {
                if d.host.worker.is_some() {
                    d.tick_draft();
                }
            }
            d.fault(power, keep);
            assert!(d.step("launch", &[]));
            d.settle();
        }
    }
}

#[test]
fn at_risk_move_source_keeps_old_draft_until_ordinary_refresh() {
    let mut d = Driver::stepped("base");
    open(&mut d, 0);
    open(&mut d, 1);
    edit(&mut d, 0, 3);
    risk_draft(&mut d, 0);
    let old = d.host.logical_drafts()[&key(0)].clone();
    run(
        &mut d,
        &[("wOp", json!([0, 2, 3])), ("deliverUp", json!([true]))],
    );
    d.settle();
    assert_eq!(d.host.logical_drafts()[&key(0)], old);
    assert!(d.step("draftSync", &[json!(0)]));
    d.settle();
    d.fault(true, false);
    assert!(d.step("launch", &[]));
    d.settle();
}

#[test]
fn one_post_unlink_sync_failure_completes_cleanup_retirement_removal_and_op_retirement() {
    for mode in 0..4 {
        let mut d = Driver::stepped("base");
        open(&mut d, 0);
        match mode {
            0 => {
                edit(&mut d, 0, 2);
                run(
                    &mut d,
                    &[("switchReq", json!([])), ("draftSync", json!([0]))],
                );
                d.host.fs.inject(Phase::DraftRename, [Fault::After]);
            }
            1 => {
                edit(&mut d, 0, 2);
                risk_draft(&mut d, 0);
                edit(&mut d, 0, 3);
                assert!(d.step("draftSync", &[json!(0)]));
            }
            2 => {
                edit(&mut d, 0, 2);
                risk_draft(&mut d, 0);
                run(
                    &mut d,
                    &[
                        ("wDiscard", json!([0])),
                        ("deliverUp", json!([true])),
                        ("wRecv", json!([0])),
                        ("draftSync", json!([0])),
                    ],
                );
            }
            _ => {
                assert!(d.step("opDelete", &[json!(0)]));
            }
        }
        for _ in 0..80 {
            let stage = d.host.worker.as_ref().unwrap().task.stage;
            if matches!(stage, Stage::UnlinkSync | Stage::CleanupSync) {
                break;
            }
            d.tick_draft();
        }
        let unlinks = d
            .host
            .fs
            .calls
            .iter()
            .filter(|p| **p == Phase::DraftUnlink)
            .count();
        d.host.fs.inject(Phase::DraftSync, [Fault::Before]);
        d.tick_draft();
        d.tick_draft();
        assert_eq!(
            d.host
                .fs
                .calls
                .iter()
                .filter(|p| **p == Phase::DraftUnlink)
                .count(),
            unlinks
        );
        d.settle();
        assert!(!d.host.allocator_busy());
        assert!(d.host.applying.is_none());
        d.fault(true, false);
        assert!(d.step("launch", &[]));
        d.settle();
    }
}

#[test]
fn grouped_rename_install_explosion_and_retirement_at_every_fault_cut() {
    for cut in 0..32 {
        for (power, keep) in [(false, false), (true, false), (true, true)] {
            let mut d = Driver::stepped("base");
            open(&mut d, 1);
            assert!(d.step(
                "opRename",
                &[json!(0), json!(2), json!([1]), json!([[1, 3]])]
            ));
            // Admissions and window-only typing remain possible under locks.
            edit_local_submit(&mut d, 1, 1);
            for _ in 0..cut {
                if d.host.worker.is_some() {
                    assert!(!d.attempt("deliverUp", &[json!(true)]));
                    d.tick_draft();
                }
            }
            d.fault(power, keep);
            assert!(d.step("launch", &[]));
            d.settle();
        }
    }
}

#[test]
fn switch_waits_for_unread_final_answer_and_busy_dispositions() {
    let mut d = Driver::stepped("base");
    open(&mut d, 0);
    edit_local_submit(&mut d, 0, 2);
    assert!(d.step("deliverUp", &[json!(true)]));
    assert!(!d.attempt("switchFin", &[]));
    assert!(d.step("wRecv", &[json!(0)]));
    assert!(d.step("flush", &[json!(0)]));
    edit_local_submit(&mut d, 0, 3);
    assert!(!d.attempt("deliverUp", &[json!(true)]));
    assert!(!d.attempt("observe", &[json!(0)]));
    assert!(!d.attempt("flush", &[json!(0)]));
    assert!(!d.attempt("switchFin", &[]));
    assert!(!d.attempt("opDelete", &[json!(0)]));
    assert!(d.attempt("saveFail", &[]));
    assert!(!d.attempt("opDelete", &[json!(0)]));
}

#[test]
#[cfg(test)]
fn scheduler_random_walks_with_faults_and_disabled_actions() {
    let mut steps = 0;
    let mut physical = 0;
    let mut disabled = 0;
    for profile in ["base", "R1", "weak", "all"] {
        for seed in 1u64..=12 {
            let mut rng = seed;
            let mut d = Driver::stepped(profile);
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
                        eprintln!("failure capsule: HEAD 4469437e + lane diff; {profile}/seed {seed}/turn {turn}/draft barrier; {steps} actions / {physical} physical / {disabled} disabled before failure; expected observed lockstep; frozen design/model mismatch");
                        std::panic::resume_unwind(e);
                    }
                    physical += 1;
                    continue;
                }
                let p = (rng as usize / 7) % 3;
                let t = 1 + (rng as i64 / 31).rem_euclid(3);
                let (name, args) = match (rng / 13) % 25 {
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
                    14 => ("extWriteD", json!([p, t, rng % 2 == 0])),
                    15 => ("windowCrash", json!([])),
                    16 => ("switchReq", json!([])),
                    17 => ("wDiscard", json!([p])),
                    18 => ("wClose", json!([p])),
                    19 => ("crash", json!([])),
                    20 => ("power", json!([false, false])),
                    21 => ("wOpTo", json!([p, (p + 1) % 3, t, 1 + t % 3])),
                    22 => ("load", json!([p])),
                    23 => ("wResolve", json!([p, t])),
                    _ => (
                        "opRename",
                        json!([p, (p + 1) % 3, [(p + 2) % 3], [[(p + 2) % 3, t]]]),
                    ),
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    d.attempt(name, args.as_array().unwrap())
                }));
                match result {
                    Ok(true) => steps += 1,
                    Ok(false) => disabled += 1,
                    Err(e) => {
                        eprintln!("failure capsule: HEAD 4469437e + lane diff; {profile}/seed {seed}/turn {turn}/{name}/{args}; expected observed lockstep; product or harness mismatch");
                        std::panic::resume_unwind(e);
                    }
                }
            }
            d.settle();
        }
    }
    eprintln!("scheduler 48 walks x 800 choices; seeds 1..12 per profile; {steps} actions / {physical} physical / {disabled} disabled");
}

// Former counterexamples now pass through the authorized s3.1 load boundary.
#[test]
fn loaded_delete_external_write_keeps_captured_base_and_blocks_publication() {
    let mut d = Driver::stepped("base");
    assert!(d.step("opDelete", &[json!(0)]));
    assert!(d.step("extWriteD", &[json!(0), json!(3), json!(true)]));
    d.settle();
    let actual = d.abstract_state();
    assert_eq!(actual["pages"][0]["base"], json!(1));
    assert_eq!(actual["drafts"][0]["base"], json!(1));
    assert_eq!(
        label(&d.host.fs.files.get(&format!("graph/{}", key(0))).cloned()),
        3
    );
    run(&mut d, &[("flushDel", json!([0])), ("check", json!([]))]);
    assert!(d.host.pages[&key(0)].conflict);
    assert!(d.host.job.is_none());
}

#[test]
fn loaded_rename_external_target_creation_recovers_and_blocks_publication() {
    let mut d = Driver::stepped("base");
    let args = [json!(0), json!(2), json!([]), json!([])];
    assert!(d.step("opRename", &args));
    for _ in 0..3 {
        d.tick_draft();
    }
    assert!(d.step("extWriteD", &[json!(2), json!(3), json!(true)]));
    assert!(d.oracle.next("opRename", &args).is_some());
    d.fault(true, false);
    assert!(d.step("launch", &[]));
    d.settle();
    assert_eq!(d.host.pages[&key(2)].buf, text(1));
    assert_eq!(d.host.pages[&key(2)].base, Base::Known(None));
    assert_eq!(
        label(&d.host.fs.files.get(&format!("graph/{}", key(2))).cloned()),
        3
    );
    assert!(d.attempt("flush", &[json!(2)]));
    assert!(d.attempt("check", &[]));
    assert!(d.attempt("check", &[]));
    assert!(d.host.pages[&key(2)].conflict);
    assert!(d.host.job.is_none());
}

#[test]
#[cfg(test)]
fn loaded_rename_preserves_unread_referrer_answer_until_application() {
    let mut d = Driver::stepped("base");
    // The CLI can rename a held clean referrer while its opening answer is
    // still unread. Mail receipt does not take the page's operation lock.
    run(
        &mut d,
        &[("wOpen", json!([1])), ("deliverUp", json!([true]))],
    );
    let args = [json!(0), json!(2), json!([1]), json!([[1, 3]])];
    assert!(d.step("opRename", &args));
    let external = [json!(2), json!(3), json!(true)];
    assert!(d.step("extWriteD", &external));
    assert!(d.oracle.next("opRename", &args).is_some());
    assert!(d.step("wRecv", &[json!(1)]));
    assert_eq!(d.windows[1].text, 2);
    assert_eq!(d.windows[1].bv, 1);
    d.settle();
    let actual = d.abstract_state();
    let expected = d.oracle.state();
    assert_eq!(actual["pages"], expected["s"]["pages"]);
    assert_eq!(actual["drafts"], expected["s"]["drafts"]);
    assert_eq!(actual["w"][1]["text"], json!(2));
    assert_eq!(actual["w"][1]["bv"], json!(1));
    assert_eq!(expected["s"]["w"][1]["text"], json!(2));
    assert_eq!(expected["s"]["w"][1]["bv"], json!(1));
    assert_eq!(actual["mb"][1]["on"], json!(true));
    assert_eq!(expected["s"]["mb"][1]["on"], json!(true));
    assert!(d.step("wRecv", &[json!(1)]));
    assert_eq!(d.windows[1].text, 3);
    assert_eq!(d.windows[1].bv, d.version(d.host.pages[&key(1)].version));
}

#[test]
#[cfg(test)]
fn loaded_delete_new_open_during_install_gets_a_modelled_push() {
    let mut d = Driver::stepped("base");
    assert!(d.step("opDelete", &[json!(0)]));
    assert!(d.step("wOpen", &[json!(0)]));
    d.settle();
    let actual = d.abstract_state();
    let expected = d.oracle.state();
    assert_eq!(actual["pages"], expected["s"]["pages"]);
    assert_eq!(actual["drafts"], expected["s"]["drafts"]);
    assert_eq!(actual["up"], expected["s"]["up"]);
    assert_eq!(actual["w"], expected["s"]["w"]);
    assert_eq!(actual["mb"][0]["on"], json!(true));
    assert_eq!(expected["s"]["mb"][0]["on"], json!(true));
    assert!(d.step("wRecv", &[json!(0)]));
    assert!(d.windows[0].sent);
    assert!(d.host.outbox.is_empty());
    run(
        &mut d,
        &[("deliverUp", json!([true])), ("wRecv", json!([0]))],
    );
    assert!(d.windows[0].on);
    assert!(!d.windows[0].sent);
}
