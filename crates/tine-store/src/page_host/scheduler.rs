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

    fn draft_retiring(&mut self, p: usize) {
        self.register(p);
        assert!(self.oracle.next("draftSync", &[json!(p)]).is_some());
        assert_eq!(
            self.host.begin_draft_retiring(&key(p)),
            Disposition::Pending
        );
        if matches!(
            self.host.worker.as_ref().unwrap().application,
            Some(Application::Retire)
        ) {
            self.whole_retires += 1;
        }
        self.compare("production draft entry pending");
        self.drain(
            "draftSync",
            &[json!(p)],
            self.host.incarnation,
            self.host.version,
        );
    }

    fn retirement_custody(&mut self) {
        let Some(worker) = self
            .host
            .worker
            .as_ref()
            .filter(|w| matches!(w.application, Some(Application::Retire)))
        else {
            return;
        };
        let pages = worker.pages.clone();
        assert_eq!(pages, worker.before.keys().cloned().collect());
        for op in self.host.live_ops.values() {
            let expected: BTreeSet<_> = op
                .versions
                .iter()
                .filter(|(k, v)| {
                    let page = &self.oracle.state()["s"]["pages"][index(k)];
                    page["risk"] == true
                        && page["conflict"] == false
                        && page["ver"] == self.version(**v)
                })
                .map(|(k, _)| k.clone())
                .collect();
            assert_eq!(op.owed, expected, "whole retirement publication debt");
        }
        let before = self.untouched();
        for page in pages {
            assert!(
                self.host.busy(&page),
                "whole retirement lost custody of {page}"
            );
            assert_eq!(self.host.observe(&page), Disposition::Waiting);
            assert_eq!(
                self.host.reserve(&BTreeSet::from([page])),
                Disposition::Waiting
            );
        }
        self.assert_untouched(&before, "whole retirement busy probes");
    }

    fn tick_draft(&mut self) {
        self.retirement_custody();
        let worker = self.host.worker.as_ref().unwrap();
        let application = !matches!(worker.application, Some(Application::Representation))
            && super::applies_next(worker);
        let present = worker.task.stage == Stage::Present;
        let removal = matches!(
            worker.application,
            Some(Application::Removal | Application::Retire)
        );
        let retired =
            matches!(worker.application, Some(Application::Retire)).then(|| worker.pages.clone());
        let inc = self.host.incarnation;
        self.host.advance_draft();
        let stutter = |d: &mut Self, context: &str| {
            let model = d.oracle.state();
            d.learn_versions(&model, inc, context);
            d.compare(context);
        };
        if application {
            let (name, mut args, inc, _) = self.effect.take().unwrap();
            if present || removal || name == "deliverUp" {
                if name == "deliverUp" {
                    args[0] = json!(present);
                }
                self.register_versions(&name, &args, inc);
                if let Some(pages) = retired {
                    assert_eq!(name, "draftSync");
                    // This terminal is atomic physically. Its virtual cuts
                    // remove exactly one record at a time; all other fields
                    // already have their observed terminal values. Check each
                    // against an enabled model step, including its ghosts.
                    let terminal = self.abstract_state();
                    let mut projected = terminal.clone();
                    projected["drafts"] = self.observed["s"]["drafts"].clone();
                    for page in pages {
                        let p = index(&page);
                        assert_eq!(self.oracle.state()["s"]["pages"][p]["risk"], false);
                        projected["drafts"][p] = terminal["drafts"][p].clone();
                        self.finish_projection("draftSync", &[json!(p)], projected.clone());
                    }
                    assert_eq!(
                        projected, terminal,
                        "whole retirement changed only its records"
                    );
                    self.compare("whole retirement physical terminal");
                } else {
                    self.finish(&name, &args);
                }
            } else {
                stutter(self, "failed fresh install stutters");
            }
        } else {
            stutter(self, "physical draft barrier");
        }
    }

    fn fault(&mut self, power: bool, keep_drafts: bool) {
        self.fault_keeping(power, keep_drafts, &BTreeSet::new());
    }

    fn fault_keeping(&mut self, power: bool, keep_drafts: bool, kept: &BTreeSet<usize>) {
        let keep = kept.iter().copied().map(key).collect();
        let mut survivor = self.host.fs.clone();
        if power {
            survivor.power(&keep, keep_drafts);
        } else {
            survivor.crash();
        }
        if self.effect.is_some() {
            let worker = self.host.worker.as_ref().unwrap();
            let present = survivor
                .files
                .contains_key(&format!("draft/{}", worker.task.name));
            let removal = match &worker.application {
                Some(Application::Removal | Application::Retire) => {
                    let logical = drafts::scan(survivor.draft_files(false)).logical;
                    !worker.pages.iter().any(|p| logical.contains_key(p))
                }
                _ => false,
            };
            // A removal's copies are not its effect: it takes effect only
            // when its page's record is gone (RENAME-COST L2).
            let removing = matches!(
                worker.application,
                Some(Application::Removal | Application::Retire)
            );
            let new_state = !removing && worker.task.bytes.is_some() && present;
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
                // The survivor projection changed the durable census. Keep
                // the host's existing index in step before observing it; no
                // I/O is retried and no production phase is advanced here.
                self.host.sync_drafts();
                self.tick_draft();
                self.host.fs = physical;
            } else {
                self.effect = None;
            }
        }
        // Apply the selected physical graph and draft directory outcomes.
        if power {
            let expected = self.oracle.next("powerK", &[json!(kept)]);
            self.refine_power_cuts(&keep, &expected.unwrap().state()["s"]["trash"]);
        }
        self.host.fs = survivor;
        self.host.stop();
        self.fresh_binding();
        self.windows.fill(Window::default());
        self.pending_ids.fill(None);
        let args = if power { vec![json!(kept)] } else { vec![] };
        self.finish(if power { "powerK" } else { "crash" }, &args);
    }

    // Dispositions are fixed by the host contract, not chosen from the oracle.
    fn attempt(&mut self, name: &str, args: &[Value]) -> bool {
        let p = args.first().and_then(Value::as_u64).unwrap_or(0) as usize;
        // The command registers what it names before the host decides (§2).
        for q in named(name, args) {
            self.register(q);
        }
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
            "draftSync" => self.host.worker.is_some(),
            "opRename" => self.host.worker.is_some() || self.host.allocator_busy(),
            "opApply" => self.host.worker.is_some() || self.host.allocator_busy(),
            "opDelete" => self.host.worker.is_some() || self.host.busy(&key(p))
                || self.host.allocator_busy() || self.host.order.touches(&BTreeSet::from([key(p)])),
            "switchReq" => self.host.worker.is_some(),
            "switchFin" => self.host.worker.is_some() || self.host.applying.is_some()
                || !self.host.queue.is_empty() || self.host.job.is_some()
                || self.host.outbox.values().any(|m| m.answer.is_some())
                || self.windows.iter().any(|w| w.pend || w.sent),
            _ => false,
        };
        if blocked {
            let before = matches!(name, "opApply" | "switchFin").then(|| self.untouched());
            let confirmation = (self.host.admission_open, self.host.switch_confirmation);
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
                "opRename" => assert_eq!(self.rename_operation(name, args), Disposition::Waiting),
                "opApply" => {
                    let (eff, plan, dst, src) = apply_args(name, args);
                    let effects = eff
                        .iter()
                        .map(|(&p, &t)| (key(p), (text(t), text(plan[&p]))))
                        .collect();
                    let ends = (dst != NONE).then(|| (key(dst as usize), key(src as usize)));
                    assert_eq!(
                        self.host.apply(
                            &effects,
                            ends.as_ref().map(|(d, s)| (d.as_str(), s.as_str()))
                        ),
                        Disposition::Waiting
                    );
                }
                "switchReq" => assert_eq!(self.host.switch_request(), Disposition::Waiting),
                "switchFin" => assert_eq!(self.host.switch_finish(), Disposition::Waiting),
                "check" | "rename" | "dirSync" | "saveFail" => {
                    assert_eq!(self.host.advance_save(), Disposition::Waiting)
                }
                _ => {}
            }
            if let Some(before) = before {
                self.assert_untouched(&before, "blocked action no-mutation disposition");
                assert_eq!(
                    (self.host.admission_open, self.host.switch_confirmation),
                    confirmation
                );
            }
            self.compare("blocked action retains custody");
            return false;
        }
        if name == "opApply" {
            // This adapter loads named pages and probes Changed/Unsaved/Busy
            // even when the oracle disables the operation. Its existing
            // occupied-name and queued-input stutters retain exact snapshots.
            let refusals = self.occupied_refusals + self.queued_refusals;
            let taken = self.step(name, args);
            self.compare("physical scheduler apply disposition");
            return taken && refusals == self.occupied_refusals + self.queued_refusals;
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
                    let before = self.untouched();
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
                    let result =
                        self.host
                            .rename_with(&key(p), &key(q), &refkeys, |bytes, k, moving| {
                                let r = k.trim_end_matches(".md").parse::<usize>().unwrap();
                                Ok(if !moving && refs.contains(&r) {
                                    text(rt[&r])
                                } else {
                                    bytes.clone()
                                })
                            });
                    let keys = named(name, args).into_iter().map(key).collect();
                    assert!(
                        result == Disposition::Refused
                            || result == Disposition::Waiting
                                && (self.host.order.touches(&keys)
                                    || keys.iter().any(|key| self.host.busy(key))),
                        "disabled rename without a refusal or physical busy predicate: {result:?}"
                    );
                    self.assert_untouched(&before, "disabled rename");
                }
                "switchFin" => {
                    let before = self.untouched();
                    let confirmation = (self.host.admission_open, self.host.switch_confirmation);
                    if self.host.switch_ready(self.host.last_admitted) == Disposition::Applied {
                        assert_eq!(self.host.switch_finish(), Disposition::Waiting);
                        self.host.switch_abort();
                    }
                    assert_eq!(
                        (self.host.admission_open, self.host.switch_confirmation),
                        confirmation
                    );
                    self.assert_untouched(&before, "disabled switch retains custody");
                }
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
            self.host.advance_save();
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
            self.host.advance_save();
            self.compare("trash directory sync barrier");
            return true;
        }
        if name == "draftSync" {
            self.draft_retiring(p);
            true
        } else if name == "powerK" {
            let keep = serde_json::from_value(args[0].clone()).unwrap();
            self.fault_keeping(true, false, &keep);
            true
        } else if matches!(name, "crash" | "power") {
            let keep = (0..2)
                .filter(|&p| args.get(p).is_some_and(|v| v == true))
                .collect();
            self.fault_keeping(name == "power", false, &keep);
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

#[test]
fn review_f2_external_recreation_before_and_after_trash_sync_then_power() {
    for before_trash_sync in [true, false] {
        let mut d = Driver::stepped("base");
        run(&mut d, &[("load", json!([0])), ("opDelete", json!([0]))]);
        d.settle();
        run(
            &mut d,
            &[
                ("flushDel", json!([0])),
                ("check", json!([])),
                ("rename", json!([])),
            ],
        );
        if !before_trash_sync {
            assert!(d.attempt("dirSync", &[json!(true)]));
            assert_eq!(d.host.job.as_ref().unwrap().phase, SavePhase::DirectorySync);
        }
        run(&mut d, &[("extWriteD", json!([0, 3, true]))]);
        if before_trash_sync {
            assert!(d.attempt("dirSync", &[json!(true)]));
        }
        assert!(d.attempt("dirSync", &[json!(true)]));
        assert_eq!(d.observed["g"]["delDurable"][0], json!([]));
        d.fault(true, false);
        assert_eq!(
            d.host.fs.files[&format!("graph/{}", key(0))],
            text(3).unwrap()
        );
        assert!(d
            .host
            .fs
            .files
            .iter()
            .any(|(k, v)| k.starts_with("trash/") && *v == text(1).unwrap()));
    }
}

/// Delete page 0 through its move; the marker is written, custody is owed.
fn delete_through_move(d: &mut Driver) {
    run(d, &[("load", json!([0])), ("opDelete", json!([0]))]);
    d.settle();
    run(
        d,
        &[
            ("flushDel", json!([0])),
            ("check", json!([])),
            ("rename", json!([])),
        ],
    );
    assert_eq!(d.host.job.as_ref().unwrap().phase, SavePhase::TrashSync);
    assert_eq!(d.host.custody[&key(0)].markers.len(), 1);
}

/// Physical entries in the custody directory, temps included (V2 census).
fn markers(d: &mut Driver) -> usize {
    d.host
        .fs
        .files
        .keys()
        .filter(|k| k.starts_with("draft/trash-custody/"))
        .count()
}

/// REVIEW-A2 B2 / A3 B2 / F9: a crash after the move and before custody (b);
/// launch redoes custody before the marker retires, and a recreation of the
/// page never renames before that custody completes.
#[test]
fn restart_redoes_custody_before_marker_retires_and_before_recreation() {
    for launch_fails in [false, true] {
        let mut d = Driver::stepped("base");
        delete_through_move(&mut d);
        d.fault(false, false); // L175: removal ghosts survive process crash
        if launch_fails {
            d.host.fs.inject(Phase::TrashSync, [Fault::Before]);
        }
        d.host.fs.calls.clear();
        run(&mut d, &[("launch", json!([]))]);
        let launch = d.host.fs.calls.clone();
        // Launch explodes the deletion's surviving operation vehicle.
        d.settle();
        assert_eq!(
            launch[..2],
            [Phase::CustodyList, Phase::TrashSync],
            "custody first"
        );
        assert_eq!(markers(&mut d), usize::from(launch_fails));
        assert_eq!(d.host.custody.contains_key(&key(0)), launch_fails);
        run(&mut d, &[("observe", json!([0]))]);
        d.settle();
        open(&mut d, 0);
        edit(&mut d, 0, 2);
        run(&mut d, &[("draftSync", json!([0]))]);
        d.settle();
        d.host.fs.calls.clear();
        run(&mut d, &[("flush", json!([0]))]);
        while d.host.job.as_ref().unwrap().phase == SavePhase::Temp {
            assert!(d.attempt("check", &[]));
        }
        run(&mut d, &[("check", json!([])), ("rename", json!([]))]);
        let calls = &d.host.fs.calls;
        let rename = calls.iter().position(|p| *p == Phase::PageRename).unwrap();
        assert!(
            !launch_fails || calls[..rename].contains(&Phase::TrashSync),
            "recreation renamed before custody: {calls:?}"
        );
        assert_eq!(markers(&mut d), 0);
        assert!(d.attempt("dirSync", &[json!(true)]));
        // Every permitted power outcome keeps the removed bytes (L695).
        d.fault(true, false);
        assert!(d
            .host
            .fs
            .files
            .iter()
            .any(|(key, bytes)| key.starts_with("trash/") && *bytes == text(1).unwrap()));
    }
}

/// F5 / R-STORAGE-ERROR: custody that keeps failing fails the save (model
/// saveFail) twice; the third filesystem error lets the save go ahead with a
/// sticky error, and only that named residual may lose the payload.
#[test]
fn failing_custody_fails_saves_until_the_three_failure_escape() {
    let mut d = Driver::stepped("base");
    delete_through_move(&mut d);
    d.fault(false, false);
    d.host.fs.inject(Phase::TrashSync, [Fault::Before]);
    run(&mut d, &[("launch", json!([]))]);
    // Launch explodes the deletion's surviving operation vehicle.
    d.settle();
    run(&mut d, &[("observe", json!([0]))]);
    d.settle();
    open(&mut d, 0);
    edit(&mut d, 0, 2);
    for attempt in 1..=3 {
        d.host.fs.inject(Phase::TrashSync, [Fault::Before]);
        assert_eq!(d.host.start_save(&key(0)), Disposition::Pending);
        d.finish("flush", &[json!(0)]);
        d.host.advance_save();
        if attempt < 3 {
            d.finish("saveFail", &[]);
            assert!(d.host.job.is_none());
        } else {
            d.compare("escaped custody is a stutter");
        }
    }
    assert!(d
        .host
        .events
        .iter()
        .any(|e| matches!(e, Event::CustodyError { .. })));
    while d.host.job.as_ref().unwrap().phase == SavePhase::Temp {
        assert!(d.attempt("check", &[]));
    }
    run(
        &mut d,
        &[
            ("check", json!([])),
            ("rename", json!([])),
            ("dirSync", json!([true])),
        ],
    );
    assert!(d.host.pages[&key(0)].clean());
    assert_eq!(markers(&mut d), 1, "the marker stays for the next launch");
    assert!(!d.escaped.is_empty());
    d.fault(true, false); // the escaped payload may be lost: R-STORAGE-ERROR only
    run(&mut d, &[("launch", json!([]))]);
    assert_eq!(markers(&mut d), 0);
}

/// REVIEW-A3: a trash collision leaves no unused marker and rewrites no
/// draft, at every physical cut, before and after a restart (F3).
#[test]
fn collision_retires_its_unused_marker_at_every_cut() {
    for cut in 0..4 {
        let mut d = Driver::stepped("base");
        run(&mut d, &[("load", json!([0])), ("opDelete", json!([0]))]);
        d.settle();
        if cut % 2 == 1 {
            d.fault(false, false);
            run(&mut d, &[("launch", json!([]))]);
            d.settle();
        }
        let drafts = d.host.fs.draft_files(true);
        run(&mut d, &[("flushDel", json!([0])), ("check", json!([]))]);
        d.host.advance_save();
        d.compare("marker is a stutter");
        assert_eq!(markers(&mut d), 1);
        d.host.fs.inject(Phase::TrashMove, [Fault::Collision]);
        d.host.advance_save();
        d.compare("collision leaves graph and abstract job unchanged");
        assert_eq!(d.host.job.as_ref().unwrap().phase, SavePhase::Marker);
        assert_eq!(markers(&mut d), 0, "unused marker retired");
        assert!(d.host.worker.is_none(), "no draft rewrite");
        assert_eq!(d.host.fs.draft_files(true), drafts);
        if cut >= 2 {
            d.fault(false, false);
            run(&mut d, &[("launch", json!([]))]);
            d.settle();
            run(&mut d, &[("flushDel", json!([0])), ("check", json!([]))]);
        }
        run(&mut d, &[("rename", json!([]))]);
        assert!(d.attempt("dirSync", &[json!(true)]));
        assert!(d.attempt("dirSync", &[json!(true)]));
        assert_eq!(markers(&mut d), 0);
        d.fault(true, false);
        assert!(d
            .host
            .fs
            .files
            .iter()
            .any(|(key, bytes)| key.starts_with("trash/") && *bytes == text(1).unwrap()));
    }
}

/// Discard has no trash barrier (A4 rule 4); the owed custody of an earlier
/// R1 payload is completed by launch, never by the draft removal.
#[test]
fn discard_has_no_trash_barrier_and_launch_completes_custody() {
    let mut d = Driver::stepped("R1");
    run(&mut d, &[("load", json!([0])), ("opDelete", json!([0]))]);
    d.settle();
    run(
        &mut d,
        &[
            ("flushDel", json!([0])),
            ("check", json!([])),
            ("extWriteD", json!([0, 3, false])),
            ("rename", json!([])),
        ],
    );
    d.host.fs.inject(Phase::TrashSync, [Fault::Before]);
    d.host.advance_save();
    d.finish("dirSync", &[json!(false)]);
    run(
        &mut d,
        &[
            ("extWriteD", json!([0, 2, true])),
            ("wOpen", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
            ("wDiscard", json!([0])),
            ("deliverUp", json!([true])),
            ("wRecv", json!([0])),
        ],
    );
    d.host.fs.calls.clear();
    run(&mut d, &[("draftSync", json!([0]))]);
    d.settle();
    assert!(!d.host.fs.calls.contains(&Phase::TrashSync));
    assert_eq!(markers(&mut d), 1);
    d.fault(false, false);
    run(&mut d, &[("launch", json!([]))]);
    assert_eq!(markers(&mut d), 0);
    open(&mut d, 0);
    edit(&mut d, 0, 1);
    run(&mut d, &[("flush", json!([0]))]);
    assert!(d.attempt("check", &[])); // temp barrier
    run(
        &mut d,
        &[
            ("check", json!([])),
            ("rename", json!([])),
            ("dirSync", json!([true])),
        ],
    );
    d.fault(true, false);
    assert!(d
        .host
        .fs
        .files
        .iter()
        .any(|(key, bytes)| key.starts_with("trash/") && *bytes == text(3).unwrap()));
}

/// REVIEW-2b F6: a speculative fork copies the custody state it may act on.
#[test]
fn speculative_fork_preserves_custody() {
    let mut d = Driver::stepped("base");
    delete_through_move(&mut d);
    let fork = d.fork().unwrap();
    assert_eq!(fork.host.custody, d.host.custody);
    assert_eq!(fork.escaped, d.escaped);
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
                // RENAME-COST L2: a refresh that supersedes the operation
                // vehicle's record retires the vehicle.
                assert!(d.step("opDelete", &[json!(0)]));
                d.settle();
                run(&mut d, &[("wRecv", json!([0]))]);
                edit(&mut d, 0, 2);
                assert!(d.step("draftSync", &[json!(0)]));
                for _ in 0..80 {
                    let w = d.host.worker.as_ref().unwrap();
                    if w.task.name.starts_with("op-") {
                        break;
                    }
                    d.tick_draft();
                }
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

#[path = "scheduler_retirement_tests.rs"]
mod retirement;
use retirement::published_group;

#[path = "scheduler_random_tests.rs"]
mod random;

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

/// The system calls inside each custody HostIo call of a deletion on a fresh
/// trash chain: marker temp, rename, directory sync; three mkdirs, three
/// parent syncs, the move; payload stat (no sync), `pages/` and three ancestor
/// syncs;
/// marker unlink and its directory sync (REVIEW-2b-r2 R1).
const CUSTODY_CALLS: [(Phase, usize); 4] = [
    (Phase::CustodyWrite, 3),
    (Phase::TrashMove, 7),
    (Phase::TrashSync, 4),
    (Phase::CustodyRetire, 2),
];

/// Drives page 0's deletion and fails its `phase` call after `k` system calls.
fn cut_deletion(d: &mut Driver, phase: Phase, k: usize) {
    run(d, &[("load", json!([0])), ("opDelete", json!([0]))]);
    d.settle();
    run(d, &[("flushDel", json!([0])), ("check", json!([]))]);
    match phase {
        Phase::CustodyWrite | Phase::TrashMove => {
            if phase == Phase::TrashMove {
                d.host.advance_save();
                d.compare("marker is a stutter");
            }
            d.cut = Some(k);
            run(d, &[("saveFail", json!([]))]);
        }
        Phase::TrashSync => {
            run(d, &[("rename", json!([]))]);
            // The process stops inside this call; nobody sees its outcome.
            d.host.fs.inject(phase, [Fault::Cut(k)]);
            let events = d.host.events.len();
            d.host.advance_save();
            d.host.events.truncate(events);
        }
        _ => {
            run(d, &[("rename", json!([]))]);
            assert!(d.attempt("dirSync", &[json!(true)]));
            d.host.fs.inject(phase, [Fault::Cut(k)]);
            assert!(d.attempt("dirSync", &[json!(true)]));
        }
    }
}

/// Every permitted power outcome, keeping or reverting page 0's source, must
/// keep the oracle's trash unless a named residual applies.
fn probe_power(d: &Driver) {
    for keep in [false, true] {
        let mut probe = d.clone();
        assert!(probe.step("power", &[json!(keep), json!(false)]));
    }
}

/// Finishes page 0's deletion, probing a power cut after every step.
fn finish_deletion(d: &mut Driver, context: &str) {
    let steps = [
        ("flushDel", json!([0])),
        ("check", json!([])),
        ("rename", json!([])),
        ("dirSync", json!([true])),
    ];
    for _ in 0..12 {
        let progressed = steps
            .iter()
            .any(|(name, args)| d.attempt(name, args.as_array().unwrap()));
        if !progressed {
            break;
        }
        probe_power(d);
    }
    assert!(d.host.job.is_none(), "{context}: deletion finished");
}

/// R1: cut between the system calls of every custody call of a deletion;
/// then lose power at once, or finish the deletion in the same process, or
/// crash, relaunch and finish it, probing a power cut after every later step.
#[test]
fn r1_every_constituent_cut_of_a_deletion_refines_the_oracle() {
    for profile in ["base", "weak"] {
        for (phase, calls) in CUSTODY_CALLS {
            for k in 0..calls {
                let context = format!("{profile} {phase:?} cut after {k} calls");
                // Name the failing cut in the panic (production-stderr guard I-5).
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut d = Driver::stepped(profile);
                    cut_deletion(&mut d, phase, k);
                    probe_power(&d);
                    if phase != Phase::TrashSync {
                        // No crash: the retained obligations are discharged first.
                        finish_deletion(&mut d.clone(), &format!("{context}, no crash"));
                    }
                    d.fault(false, false);
                    run(&mut d, &[("launch", json!([]))]);
                    d.settle();
                    probe_power(&d);
                    finish_deletion(&mut d, &context);
                }));
                if let Err(panic) = outcome {
                    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
                    panic!("{context}: {message}");
                }
            }
        }
    }
}

/// RENAME-COST review (2026-10-11): a crash while a removal copies its
/// operation siblings leaves the removal unapplied; the harness drops its
/// pending effect, and the next draft effect checks against the model.
#[test]
fn a_crash_during_a_removal_copy_leaves_the_removal_unapplied() {
    let mut d = Driver::stepped("base");
    run(&mut d, &[("opRename", json!([0, 2, [1], [[1, 3]]]))]);
    d.settle();
    assert!(d.attempt("flush", &[json!(2)]));
    while d.host.job.as_ref().unwrap().phase == SavePhase::Temp {
        assert!(d.attempt("check", &[]));
    }
    assert!(d.attempt("check", &[]));
    assert!(d.attempt("rename", &[]));
    assert!(d.attempt("dirSync", &[json!(true)]));
    run(&mut d, &[("draftSync", json!([2]))]);
    for _ in 0..3 {
        d.tick_draft();
    }
    assert_eq!(d.host.worker.as_ref().unwrap().task.stage, Stage::Present);
    d.fault(true, false);
    run(&mut d, &[("launch", json!([]))]);
    d.settle();
    run(&mut d, &[("draftSync", json!([0]))]);
    d.settle();
}
