//! The step relation: one model action replayed on the production host,
//! then the observed successor compared with the oracle's (`finish`).
use super::*;

impl<F: ConformanceIo> Driver<F> {
    pub(super) fn step(&mut self, name: &str, args: &[Value]) -> bool {
        let prepare_operations = self.prepare_operations
            || (self.refinement_schedule
                && (self.queued_refusals > 0 || self.occupied_refusals > 0));
        if prepare_operations && self.host.alive && matches!(name, "opApply" | "opApplyPacked") {
            for p in apply_args(name, args).0.into_keys() {
                if !self.host.pages.contains_key(&key(p)) {
                    assert!(self.step("load", &[json!(p)]));
                }
            }
        }
        if prepare_operations
            && self.host.alive
            && matches!(
                name,
                "opDelete" | "opRename" | "opRenameRaw" | "opRenamePacked"
            )
        {
            let p = args[0].as_u64().unwrap() as usize;
            if !self.host.pages.contains_key(&key(p)) {
                assert!(self.step("load", &[json!(p)]));
            }
            if name != "opDelete" {
                let q = args[1].as_u64().unwrap() as usize;
                let mut needed: BTreeSet<usize> = if name == "opRename" {
                    serde_json::from_value(args[2].clone()).unwrap()
                } else {
                    (0..3)
                        .filter(|&i| {
                            args[2 + i]
                                .as_bool()
                                .unwrap_or_else(|| args[2 + i].as_i64() == Some(1))
                                && (name == "opRenameRaw" || i != p && i != q)
                        })
                        .collect()
                };
                if self.host.pages[&key(p)].buf.is_some() {
                    needed.insert(q);
                }
                for r in needed {
                    if !self.host.pages.contains_key(&key(r)) {
                        assert!(self.step("load", &[json!(r)]));
                    }
                }
            }
        }
        let successor = self.oracle.next(name, args);
        if successor.is_none() {
            let refused_schedule = self.refinement_schedule
                && (self.queued_refusals > 0 || self.occupied_refusals > 0);
            if refused_schedule && matches!(name, "flush" | "flushDel") {
                // The host's start_save chooses payload versus deletion from
                // its current buffer; a scripted label can become obsolete.
                let current_kind = if name == "flush" { "flushDel" } else { "flush" };
                if self.oracle.next(current_kind, args).is_some() {
                    assert!(self.step(current_kind, args));
                    self.save_kind_refinements += 1;
                    return true;
                }
            }
            if refused_schedule
                && matches!(
                    name,
                    "flush"
                        | "flushDel"
                        | "wRecv"
                        | "observe"
                        | "wSend"
                        | "wResolve"
                        | "deliverUp"
                        | "wEdit"
                        | "draftSync"
                        | "wOp"
                        | "wOpTo"
                )
            {
                for q in named(name, args) {
                    self.register(q);
                }
                let before = self.abstract_state();
                let files = self.host.fs.physical().files.clone();
                let stable = self.host.fs.physical().stable.clone();
                let version = self.host.version;
                let versions = self.versions.clone();
                let page = key(args[0].as_u64().unwrap_or(0) as usize);
                if matches!(name, "wOp" | "wOpTo") {
                    let p = args[0].as_u64().unwrap() as usize;
                    let (q, ts, td) = if name == "wOp" {
                        (
                            1usize.checked_sub(p).unwrap_or(self.windows.len()),
                            args[1].as_i64().unwrap(),
                            args[2].as_i64().unwrap(),
                        )
                    } else {
                        (
                            args[1].as_u64().unwrap() as usize,
                            args[2].as_i64().unwrap(),
                            args[3].as_i64().unwrap(),
                        )
                    };
                    assert!(
                        !self.move_input(p, q, ts, td),
                        "window adapter accepted oracle-disabled move"
                    );
                } else if name == "wEdit" {
                    assert!(
                        !self.edit_input(
                            args[0].as_u64().unwrap() as usize,
                            args[1].as_i64().unwrap()
                        ),
                        "window adapter accepted oracle-disabled edit"
                    );
                } else if matches!(name, "wSend" | "wResolve") {
                    assert!(
                        !self.submit_input(
                            args[0].as_u64().unwrap() as usize,
                            (name == "wResolve").then(|| args[1].as_i64().unwrap())
                        ),
                        "window adapter accepted oracle-disabled submit"
                    );
                } else if name == "deliverUp" {
                    assert_eq!(self.host.dequeue(), Disposition::Disabled);
                } else if name == "wRecv" {
                    assert!(
                        self.host.receive(&page).is_none(),
                        "disabled receive returned mail"
                    );
                } else {
                    let result = if name == "observe" {
                        self.host.observe(&page)
                    } else if name == "draftSync" {
                        self.host.begin_draft(&page)
                    } else {
                        self.host.start_save(&page)
                    };
                    assert!(
                        matches!(result, Disposition::Disabled | Disposition::Waiting),
                        "{name}: host accepted disabled save after admission stutter: {result:?}"
                    );
                }
                assert_eq!(self.abstract_state(), before);
                assert_eq!(self.host.fs.physical().files, files);
                assert_eq!(self.host.fs.physical().stable, stable);
                assert_eq!(self.host.version, version);
                assert_eq!(self.versions, versions);
                self.compare("refused operation save schedule stutter");
                self.actions += 1;
                return true;
            }
            if name == "opRename" {
                // R5/A-W1: oracle refs describe actual effects, whereas the
                // public host input supplies candidates. Independently derive
                // the effective set from oracle buffers and the opaque rewrite.
                let refs: BTreeSet<usize> = serde_json::from_value(args[2].clone()).unwrap();
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
                let state = &self.oracle.state()["s"];
                let effective: BTreeSet<_> = refs
                    .iter()
                    .copied()
                    .filter(|&r| {
                        let page = &state["pages"][r];
                        let dirty = page["typed"] == true
                            || page["risk"] == true
                            || page["buf"] != page["base"];
                        let busy = state["job"]["on"] == true && state["job"]["p"] == json!(r);
                        !(page["held"] == true && (dirty || busy) && page["buf"] == json!(rt[&r]))
                    })
                    .collect();
                let mapped = [
                    args[0].clone(),
                    args[1].clone(),
                    json!(effective),
                    json!(rt
                        .iter()
                        .filter(|(r, _)| effective.contains(r))
                        .map(|(r, t)| json!([r, t]))
                        .collect::<Vec<_>>()),
                ];
                if effective != refs && self.oracle.next("opRename", &mapped).is_some() {
                    let version = self.host.version;
                    assert_eq!(self.rename_operation(name, args), Disposition::Pending);
                    self.compare("effective rename pending");
                    self.drain("opRename", &mapped, self.host.incarnation, version);
                    self.effective_renames += 1;
                    return true;
                }
            }
            if (self.twin_suffix || refused_schedule)
                && self.host.job.is_none()
                && matches!(name, "check" | "rename" | "dirSync" | "saveFail")
            {
                let before = self.abstract_state();
                let files = self.host.fs.physical().files.clone();
                let stable = self.host.fs.physical().stable.clone();
                let version = self.host.version;
                let versions = self.versions.clone();
                assert_eq!(self.host.advance_save(), Disposition::Disabled);
                assert_eq!(self.abstract_state(), before);
                assert_eq!(self.host.fs.physical().files, files);
                assert_eq!(self.host.fs.physical().stable, stable);
                assert_eq!(self.host.version, version);
                assert_eq!(self.versions, versions);
                self.compare("terminated twin save suffix stutter");
                self.actions += 1;
                return true;
            }
            if matches!(name, "opApply" | "opApplyPacked") {
                let before = self.abstract_state();
                let version = self.host.version;
                let files = self.host.fs.physical().files.clone();
                let stable = self.host.fs.physical().stable.clone();
                let versions = self.versions.clone();
                let (eff, plan, dst, src) = apply_args(name, args);
                let oracle = self.oracle.state();
                let dirty = eff.keys().find(|&&p| {
                    let page = &oracle["s"]["pages"][p];
                    page["held"] == true
                        && (page["typed"] == true
                            || page["risk"] == true
                            || page["buf"] != page["base"])
                });
                let expected = if let Some(&p) = dirty {
                    ApplyRefusal::Unsaved(PageId::from(key(p)))
                } else if eff
                    .keys()
                    .any(|p| plan.get(p) != Some(&label(&self.host.pages[&key(*p)].buf)))
                    || eff.iter().any(|(p, t)| {
                        *t != ABSENT && plan.get(p) == Some(&ABSENT) && self.claims().contains(p)
                    })
                {
                    ApplyRefusal::Changed
                } else if self
                    .host
                    .order
                    .touches(&eff.keys().map(|&p| key(p)).collect())
                    || self.host.job.is_some()
                {
                    ApplyRefusal::Busy
                } else {
                    panic!("unclassified opApply refusal: {eff:?} {plan:?} {dst}/{src}");
                };
                let result = self.apply_admission(name, args);
                if matches!(result, Err(ApplyRefusal::Unsaved(_))) {
                    // The entire observed set already passed the independent
                    // dirty-or-queued predicate, including every dirty page.
                    // Its first spelling may be pending before a dirty sibling.
                } else {
                    assert_eq!(
                        result,
                        Err(expected),
                        "{name}: host must refuse oracle-disabled input"
                    );
                }
                assert_eq!(
                    self.abstract_state(),
                    before,
                    "{name}: refusal changed host state"
                );
                assert_eq!(self.host.version, version);
                assert_eq!(self.host.fs.physical().files, files);
                assert_eq!(self.host.fs.physical().stable, stable);
                assert_eq!(self.versions, versions);
            } else if matches!(
                name,
                "opDelete" | "opRename" | "opRenamePacked" | "opRenameRaw"
            ) {
                let before = self.abstract_state();
                let files = self.host.fs.physical().files.clone();
                let stable = self.host.fs.physical().stable.clone();
                let version = self.host.version;
                let versions = self.versions.clone();
                let result = if name == "opDelete" {
                    self.host
                        .delete_loaded(&key(args[0].as_u64().unwrap() as usize))
                } else {
                    self.rename_operation(name, args)
                };
                assert!(
                    matches!(
                        result,
                        Disposition::Refused | Disposition::Waiting | Disposition::Disabled
                    ),
                    "{name}: host accepted oracle-disabled operation: {result:?}"
                );
                assert_eq!(
                    self.abstract_state(),
                    before,
                    "{name}: refusal changed host state"
                );
                assert_eq!(self.host.fs.physical().files, files);
                assert_eq!(self.host.fs.physical().stable, stable);
                assert_eq!(self.host.version, version);
                assert_eq!(self.versions, versions);
                self.compare("operation refusal stutter");
            } else if name == "planCreate" {
                let p = args[0].as_u64().unwrap() as usize;
                assert_eq!(
                    self.slot_refusal(&BTreeMap::from([(p, 3)]), &BTreeMap::from([(p, ABSENT)])),
                    Some(ApplyRefusal::Changed)
                );
            }
            if refused_schedule
                && matches!(
                    name,
                    "opApply"
                        | "opApplyPacked"
                        | "opDelete"
                        | "opRename"
                        | "opRenamePacked"
                        | "opRenameRaw"
                )
            {
                self.actions += 1;
                return true;
            }
            return false;
        }
        if matches!(name, "opApply" | "opApplyPacked") {
            let before = self.abstract_state();
            let files = self.host.fs.physical().files.clone();
            let stable = self.host.fs.physical().stable.clone();
            let version = self.host.version;
            let versions = self.versions.clone();
            let result = self.apply_admission(name, args);
            if let Err(refusal) = result {
                let (eff, plan, _, _) = apply_args(name, args);
                if matches!(refusal, ApplyRefusal::Unsaved(_)) {
                    // apply_admission independently checked the entire set against
                    // oracle dirty-or-pending input, including every dirty page.
                    self.queued_refusals += 1;
                } else {
                    assert_eq!(refusal, ApplyRefusal::Changed, "{name}: unexpected refusal");
                    assert!(
                        eff.iter().any(|(p, t)| {
                            *t != ABSENT
                                && plan.get(p) == Some(&ABSENT)
                                && (files.contains_key(&format!("graph/{}", key(*p)))
                                    || self.claims().contains(p))
                        }),
                        "{name}: refusal without physical expected-ABSENT occupancy"
                    );
                    let oracle = self.oracle.state();
                    assert!(
                        eff.iter().any(|(p, t)| {
                            *t != ABSENT
                                && plan.get(p) == Some(&ABSENT)
                                && (oracle["s"]["disk"][*p] != json!(ABSENT)
                                    || oracle["s"]["claimed"]
                                        .as_array()
                                        .unwrap()
                                        .contains(&json!(p)))
                        }),
                        "{name}: occupied refusal outside oracle occupancy predicate"
                    );
                    self.occupied_refusals += 1;
                }
                assert_eq!(
                    self.abstract_state(),
                    before,
                    "{name}: refusal changed state"
                );
                assert_eq!(self.host.fs.physical().files, files);
                assert_eq!(self.host.fs.physical().stable, stable);
                assert_eq!(self.host.version, version);
                assert_eq!(self.versions, versions);
                self.compare("admission refusal stutter");
                self.actions += 1;
                return true;
            }
            assert_eq!(result, Ok(Disposition::Pending));
            self.compare("apply pending");
            self.drain(name, args, self.host.incarnation, version);
            return true;
        }
        for q in named(name, args) {
            self.register(q);
        }
        let p = args.first().and_then(Value::as_u64).unwrap_or(0) as usize;
        let v = |i: usize| args[i].as_i64().unwrap();
        let b = |i: usize| args[i].as_bool().unwrap_or_else(|| v(i) == 1);
        let old_inc = self.host.incarnation;
        let old_version = self.host.version;
        match name {
            "load" => {
                assert_eq!(self.host.load(&key(p)), Disposition::Applied);
            }
            "wOpen" => {
                self.windows[p].sent = true;
                self.admit(p, RequestKind::Open);
            }
            "wEdit" => {
                assert!(self.edit_input(p, v(1)));
            }
            "wSend" | "wResolve" => {
                assert!(self.submit_input(p, (name == "wResolve").then(|| v(1))));
            }
            "wDiscard" => {
                self.windows[p].pend = false;
                self.windows[p].sent = true;
                let version = self.raw(self.windows[p].bv);
                let id = self.admit(p, RequestKind::Discard);
                (self.discard_versions).insert((self.host.generation, id), version);
            }
            "wClose" => {
                self.windows[p] = Window::default();
                self.admit(p, RequestKind::Close);
            }
            "wOp" | "wOpTo" => {
                let (q, ts, td) = if name == "wOp" {
                    (1 - p, v(1), v(2))
                } else {
                    (v(1) as usize, v(2), v(3))
                };
                assert!(self.move_input(p, q, ts, td));
            }
            "wRecv" => self.receive(p),
            "deliverUp" => {
                let kind = self.host.abstract_queue()[0].kind.clone();
                if !b(0) {
                    let phase = if matches!(kind, RequestKind::Move { .. }) {
                        Phase::DraftTemp
                    } else {
                        Phase::Read
                    };
                    if !matches!(kind, RequestKind::Submit { .. } | RequestKind::Close) {
                        self.host.fs.physical_mut().inject(phase, [Fault::Before]);
                    }
                }
                if self.host.applying.is_none() {
                    assert_eq!(self.host.dequeue(), Disposition::Pending);
                }
                self.compare("dequeue");
                self.host.apply_request();
                if self.host.worker.is_none() {
                    // rok belongs to this delivery; a held open or a stale
                    // move performs no read/install and consumes no fault.
                    self.host.fs.physical_mut().faults.clear();
                }
                if self.host.applying.is_some() && self.host.worker.is_some() {
                    self.compare("install pending");
                    self.drain(name, args, old_inc, old_version);
                    return true;
                }
            }
            "observe" => {
                assert_eq!(self.host.observe(&key(p)), Disposition::Applied);
            }
            "flush" | "flushDel" => {
                self.twin_suffix = false;
                assert_eq!(self.host.start_save(&key(p)), Disposition::Pending);
                self.custody_stutter();
                if name == "flush" && !self.stepped {
                    self.host.advance_save();
                }
            }
            "check" => {
                let twin = self.host.job.as_ref().is_some_and(|job| {
                    job.base == Base::Known(None)
                        && job.bytes.is_some()
                        && !self
                            .host
                            .fs
                            .physical()
                            .files
                            .contains_key(&format!("graph/{}", job.page))
                        && self.claims().contains(&index(&job.page))
                });
                self.host.advance_save();
                if twin && self.host.job.is_none() {
                    assert!(
                        self.oracle.next("saveFail", &[]).is_some(),
                        "twin failure must be an enabled model step"
                    );
                    self.register_versions("saveFail", &[], old_inc);
                    self.finish("saveFail", &[]);
                    self.twin_failures += 1;
                    self.twin_suffix = true;
                    return true;
                }
            }
            "rename" => {
                let page = self.host.job.as_ref().unwrap().page.clone();
                let ep = self
                    .host
                    .fs
                    .physical()
                    .epochs
                    .get(&page)
                    .copied()
                    .unwrap_or(0);
                if self.host.job.as_ref().unwrap().phase == SavePhase::Marker {
                    self.host.advance_save_at(ep);
                    self.compare("custody marker");
                }
                self.host.advance_save_at(ep);
            }
            "dirSync" => {
                if self.host.job.as_ref().unwrap().phase == SavePhase::TrashSync {
                    self.host.advance_save();
                    self.compare("trash sync");
                }
                if !b(0) {
                    // An explicit error wins over graph Unsupported too.
                    self.host
                        .fs
                        .physical_mut()
                        .inject(Phase::PageSync, [Fault::Before]);
                }
                self.host.advance_save();
            }
            "saveFail" => {
                let phase = match self.host.job.as_ref().unwrap().phase {
                    SavePhase::Custody => Phase::TrashSync,
                    SavePhase::Marker => Phase::CustodyWrite,
                    SavePhase::Temp => Phase::PageTemp,
                    SavePhase::Check => Phase::Read,
                    SavePhase::Rename if self.host.job.as_ref().unwrap().bytes.is_some() => {
                        Phase::PageRename
                    }
                    SavePhase::Rename => Phase::TrashMove,
                    _ => panic!("late saveFail"),
                };
                let fault = self.cut.take().map_or(Fault::Before, Fault::Cut);
                self.host.fs.physical_mut().inject(phase, [fault]);
                self.host.advance_save();
            }
            "draftSync" => {
                assert_eq!(self.host.begin_draft(&key(p)), Disposition::Pending);
                self.compare("draft pending");
                self.drain(name, args, old_inc, old_version);
                return true;
            }
            "opDelete" => {
                assert_eq!(self.host.delete(&key(p)), Disposition::Pending);
                self.compare("delete pending");
                self.drain(name, args, old_inc, old_version);
                return true;
            }
            // R5 (A-W1): the model's `refs` is the implementation's choice of
            // referrers, which the spec leaves open. The concrete operation
            // refines `opRename(src, dst, E, rt|E)` for its EFFECTIVE set E
            // (`Host::rename_refs`): the caller's candidates, less a held one
            // that is dirty or busy and whose rewrite leaves its buffer
            // unchanged (untouched: no draft, version or write), plus each
            // held buffer the rewrite changes. Endpoints and changed refs keep
            // every model guard (clean, free, version checks, receiver-first
            // drafting); a dirty or busy ref whose bytes change still refuses
            // or waits. An empty E with an absent source is the model's
            // `all3 != Set()` guard failing: a no-op, never an applied step.
            // Enabled traces have only clean, free refs, so E is the trace's
            // own `refs` here; the newly enabled concrete schedules are pinned
            // by `mutation_tests` (R3a) against that effective-set step.
            "opRename" | "opRenamePacked" | "opRenameRaw" => {
                assert_eq!(self.rename_operation(name, args), Disposition::Pending);
                self.compare("rename pending");
                self.drain(name, args, old_inc, old_version);
                return true;
            }
            "switchReq" => {
                assert_eq!(self.host.switch_request(), Disposition::Applied);
            }
            "switchFin" => {
                self.twin_suffix = false;
                assert_eq!(
                    self.host.switch_ready(self.host.last_admitted),
                    Disposition::Applied
                );
                assert_eq!(self.host.switch_finish(), Disposition::Applied);
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "planCreate" => {
                assert_eq!(
                    self.slot_refusal(&BTreeMap::from([(p, 3)]), &BTreeMap::from([(p, ABSENT)])),
                    None
                );
            }
            "extClaim" => {
                self.host.fs.external(
                    &format!("{p:04}.org"),
                    b(1).then(|| Arc::from(&b"claimant"[..])),
                    true,
                );
            }
            "extWrite" | "extWriteD" => {
                self.host
                    .fs
                    .external(&key(p), text(v(1)), name == "extWrite" || b(2));
            }
            "windowCrash" => {
                self.host.window_crash();
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "crash" => {
                self.twin_suffix = false;
                self.host.fs.crash();
                self.host.stop();
                self.fresh_binding();
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "power" | "powerK" | "powerKBits" => {
                self.twin_suffix = false;
                let keep: BTreeSet<usize> = match name {
                    "power" => (0..2).filter(|&i| b(i)).collect(),
                    "powerKBits" => (0..3).filter(|&i| b(i)).collect(),
                    _ => serde_json::from_value(args[0].clone()).unwrap(),
                };
                let keep = keep.into_iter().map(key).collect();
                self.refine_power_cuts(&keep, &successor.as_ref().unwrap().state()["s"]["trash"]);
                self.host.fs.power(&keep, false);
                self.host.stop();
                self.fresh_binding();
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            // Declared deviations of draft I/O that fails at launch (plan v3
            // §4, B-Q1); the model's launch (storage-s3.qnt:609-619) always
            // lists and syncs. Neither is replayed here.
            // - M1: a drafts directory that cannot be created or listed
            //   recovers nothing, where the model makes every draft its
            //   buffer; no vehicle is read or touched, so a later launch
            //   recovers them (`production_tests::b_q1_a_drafts_directory_
            //   that_cannot_be_created..`, `b_q1_an_unlistable_drafts_
            //   directory..`).
            // - M2: a census launch cannot sync keeps its buffers at risk but
            //   retires nothing until a Retry's sync, where the model retires
            //   at once; while draft I/O is down every draft effect fails
            //   untouched, the model's failed draft write (:446-455), so a
            //   switch waits as `canSwitch` (:462-467) says (`tests::m2_an_
            //   unsynced_launch..`, `production_tests::b_q1_m2_..`). Its known
            //   vehicles stay cleanup debt until that sync: a stop aborts
            //   naming their pages, where the model has none left
            //   (REVIEW-3b-P1 R2, `production_tests::review_p1_m2_..`).
            "launch" => {
                for recovered in self.host.recovered_keys() {
                    self.register(index(&recovered));
                }
                self.host.launch();
                self.register_versions(name, args, old_inc);
                self.finish(name, args);
                while !self.stepped && self.host.worker.is_some() {
                    self.host.advance_draft();
                    self.compare("launch representation");
                }
                return true;
            }
            _ => panic!("unsupported host action {name}"),
        }
        self.register_versions(name, args, old_inc);
        self.finish(name, args);
        true
    }

    pub(super) fn finish(&mut self, name: &str, args: &[Value]) {
        let before = self.observed.clone();
        let actual = self.abstract_state();
        let mut after = json!({"s":actual,"g":before["g"]});
        let actual = after["s"].clone();
        let g = &mut after["g"];
        g["vc"] = json!(self.counter);
        let p = args.first().and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut tag = name;
        let mut mine = BTreeSet::<usize>::new();
        let pairset = |value: &Value| -> BTreeSet<(usize, i64)> {
            serde_json::from_value(value.clone()).unwrap()
        };
        let mut owed = pairset(&g["owed"]);
        // s3.2 order ghost: obligations a full rename records, retired by
        // the dependency's Published or a Discard; a save start checks them.
        let mut obl: BTreeSet<(usize, usize, i64, usize)> =
            serde_json::from_value(g["obl"].clone()).unwrap();
        let mut renamed: BTreeSet<(usize, usize)> =
            serde_json::from_value(g["renamed"].clone()).unwrap();
        let mut order_ok = g["orderOk"].as_bool().unwrap();
        match name {
            "wSend" | "wResolve" => {
                owed.insert((p, self.windows[p].text));
                if name == "wResolve" {
                    let obs = before["s"]["w"][p]["obs"].as_i64().unwrap();
                    // Restore can leave NONE through publication and a stale
                    // submit. Observation sentinels are not seen text.
                    if [ABSENT, 1, 2, 3].contains(&obs) {
                        insert(&mut g["seen"][p], json!(obs));
                    }
                }
            }
            "wOp" | "wOpTo" => {
                let q = if name == "wOp" {
                    1 - p
                } else {
                    args[1].as_u64().unwrap() as usize
                };
                owed.extend([(p, self.windows[p].text), (q, self.windows[q].text)]);
                tag = "wOp";
            }
            "wRecv" => {
                if before["s"]["mb"][p]["ack"].as_i64().unwrap() >= 0
                    && before["s"]["w"][p]["sent"] == true
                {
                    tag = "wAck";
                }
            }
            "deliverUp" => {
                let u = &before["s"]["up"][0];
                let p = u["p"].as_u64().unwrap() as usize;
                let kind = u["kind"].as_str().unwrap();
                match kind {
                    "submit" => {
                        let resolve = u["ro"].as_i64().unwrap() != NONE;
                        let stale = !resolve && u["bv"] != before["s"]["pages"][p]["ver"];
                        owed.remove(&(p, u["t"].as_i64().unwrap()));
                        if !stale {
                            mine.insert(p);
                        }
                        tag = if resolve {
                            "uResolve"
                        } else if stale {
                            "uStale"
                        } else {
                            "uSubmit"
                        };
                    }
                    "op" => {
                        let q = u["q"].as_u64().unwrap() as usize;
                        owed.remove(&(p, u["t"].as_i64().unwrap()));
                        owed.remove(&(q, u["t2"].as_i64().unwrap()));
                        let took = actual["pages"][p]["ver"] != before["s"]["pages"][p]["ver"];
                        if took {
                            mine.extend([p, q]);
                            tag = "uOp";
                        } else {
                            tag = "uOpRefused";
                        }
                    }
                    "discard" => {
                        // A running rename's unwitnessed destination: its
                        // Discard may also revert the source.
                        let partner = before["s"]["gates"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|gt| {
                                gt["dst"] == p && gt["open"].as_array().unwrap().contains(&json!(p))
                            })
                            .map(|gt| gt["src"].as_u64().unwrap() as usize);
                        if actual["pages"][p]["ver"] != before["s"]["pages"][p]["ver"] {
                            for p in [Some(p), partner].into_iter().flatten() {
                                let page = &actual["pages"][p];
                                if page["ver"] == before["s"]["pages"][p]["ver"] {
                                    continue;
                                }
                                mine.insert(p);
                                let pr = &g["promise"][p];
                                let live = pr["on"] == true
                                    && !(pr["saved"] == true
                                        && g["ext"][p].as_i64().unwrap()
                                            > pr["ep"].as_i64().unwrap());
                                if !live || pr["bytes"] != page["buf"] {
                                    g["promise"][p]["on"] = json!(false);
                                    g["promise"][p]["ver"] = page["ver"].clone();
                                }
                            }
                            let cancels = obl.iter().any(|o| o.1 == p && o.3 == p);
                            obl.retain(|o| !(o.0 == p || o.1 == p || (cancels && o.3 == p)));
                            tag = "uDiscard";
                        } else {
                            tag = "uDiscardFail";
                        }
                    }
                    "open" => tag = "uOpen",
                    "close" => tag = "uClose",
                    _ => panic!("unknown request"),
                }
            }
            "opDelete" | "opRename" | "opRenamePacked" | "opRenameRaw" | "opApply"
            | "opApplyPacked" => {
                for i in 0..self.windows.len() {
                    if before["s"]["pages"][i] != actual["pages"][i] {
                        mine.insert(i);
                    }
                }
                // The pages whose rename history the step ends, and the
                // (src, dst) of the order it may install: an apply's are its
                // effect pages (the model's `unrename(ps)`) and its move.
                let (ends, moved) = match name {
                    "opDelete" => (vec![p], None),
                    "opApply" | "opApplyPacked" => {
                        let (eff, _, dst, src) = apply_args(name, args);
                        let moved = (dst != NONE).then_some((src as usize, dst as usize));
                        (eff.into_keys().collect(), moved)
                    }
                    _ => {
                        let q = args[1].as_u64().unwrap() as usize;
                        (vec![p, q], Some((p, q)))
                    }
                };
                renamed.retain(|r| !(ends.contains(&r.0) || ends.contains(&r.1)));
                // A full rename or a moving apply installed its order: the
                // obligations follow the host's own gate, so a missing or
                // misversioned gate diverges from the model's ghost.
                let gates = actual["gates"].as_array().unwrap();
                let grew = gates.len() > before["s"]["gates"].as_array().unwrap().len();
                if let Some((src, dst)) = moved.filter(|_| grew) {
                    let gt = gates.last().unwrap();
                    assert_eq!(
                        (gt["src"].as_u64(), gt["dst"].as_u64()),
                        (Some(src as u64), Some(dst as u64))
                    );
                    let ver = |k: usize| gt["vers"][k].as_i64().unwrap();
                    let open: BTreeSet<usize> = serde_json::from_value(gt["open"].clone()).unwrap();
                    for &r in open.iter().filter(|&&r| r != src && r != dst) {
                        obl.insert((r, dst, ver(dst), dst));
                        obl.insert((src, r, ver(r), dst));
                    }
                    obl.insert((src, dst, ver(dst), dst));
                    if before["s"]["disk"][src] != ABSENT {
                        renamed.insert((src, dst));
                    }
                }
            }
            "flush" | "flushDel" => {
                order_ok = order_ok && !obl.iter().any(|o| o.0 == p);
            }
            "check" => {
                g["guard"] = json!(self.host.job.is_some());
            }
            "rename" => {
                let j = &before["s"]["job"];
                let p = j["p"].as_u64().unwrap() as usize;
                if g["guard"] != true
                    || (before["s"]["disk"][p] != j["base"]
                        && !matches!(self.profile.as_str(), "R1" | "all"))
                {
                    insert(&mut g["bad"], json!("C-overwrote-external"));
                }
                let seen = g["seen"][p].as_array().unwrap().contains(&j["base"]);
                let wrote = g["wrote"][p]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w[0] == j["base"]);
                let read = g["opRead"][p].as_array().unwrap().contains(&j["base"]);
                if g["guard"] == true && j["bytes"] != j["base"] && !seen && !wrote && !read {
                    insert(&mut g["bad"], json!("G-overwrote-unseen"));
                }
                g["guard"] = json!(false);
            }
            "saveFail" => g["guard"] = json!(false),
            "extWrite" | "extWriteD" => {
                renamed.retain(|r| r.0 != p && r.1 != p);
                g["ext"][p] = json!(self.host.fs.physical().epochs[&key(p)]);
                tag = "extWrite";
            }
            "crash" | "switchFin" | "power" | "powerK" | "powerKBits" => {
                owed.clear();
                obl.clear();
                renamed.clear();
                g["guard"] = json!(false);
                if name.starts_with("power") {
                    tag = "power";
                    for i in 0..self.windows.len() {
                        let kept = match name {
                            "power" => {
                                i < 2
                                    && args[i]
                                        .as_bool()
                                        .unwrap_or_else(|| args[i].as_i64() == Some(1))
                            }
                            "powerKBits" => args[i].as_i64() == Some(1),
                            _ => args[0].as_array().unwrap().contains(&json!(i)),
                        };
                        if !kept {
                            g["removed"][i] = g["delDurable"][i].clone();
                        }
                        let pr = &g["promise"][i];
                        if matches!(self.profile.as_str(), "weak" | "all")
                            && pr["on"] == true
                            && pr["saved"] == true
                            && actual["disk"][i] != pr["bytes"]
                            && actual["drafts"][i]["bytes"] != pr["bytes"]
                        {
                            g["promise"][i]["on"] = json!(false);
                        }
                    }
                }
            }
            _ => {}
        }
        for event in std::mem::take(&mut self.host.events) {
            match event {
                Event::Renamed {
                    page,
                    bytes,
                    version,
                } => {
                    let p = index(&page);
                    insert(
                        &mut g["wrote"][p],
                        json!([label(&bytes), self.version(version)]),
                    );
                }
                Event::Draft(r) => {
                    let p = index(&r.page);
                    let v = self.record_versions[&r.wseq];
                    insert(&mut g["wrote"][p], json!([label(&r.bytes), v]));
                    promise(&mut g["promise"][p], label(&r.bytes), v, false, 0);
                }
                Event::Published {
                    page,
                    bytes,
                    version,
                    epoch,
                } => {
                    let p = index(&page);
                    let v = self.version(version);
                    obl.retain(|o| !(o.1 == p && v >= o.2));
                    promise(&mut g["promise"][p], label(&bytes), v, true, epoch);
                }
                Event::Removed { page, bytes } => {
                    let p = index(&page);
                    insert(&mut g["removed"][p], json!(label(&bytes)));
                }
                Event::DeleteDurable { page, .. } => {
                    let p = index(&page);
                    // The physical witness may leave surplus durable trash.
                    // Model dirSync L431/L439 certifies deletion only while
                    // its path still contains the job's absent payload. This
                    // is a test-side projection, never another host path read.
                    if !self
                        .host
                        .fs
                        .physical()
                        .files
                        .contains_key(&format!("graph/{page}"))
                    {
                        g["delDurable"][p] = g["removed"][p].clone();
                    }
                }
                Event::CustodyError { page, payload } => {
                    let k = format!("trash/{page}/{payload}");
                    if let Some(bytes) = self.host.fs.physical().files.get(&k) {
                        self.escaped.insert(k, label(&Some(bytes.clone())));
                    }
                }
                Event::OperationRead { page, base: read } => {
                    let p = index(&page);
                    insert(&mut g["opRead"][p], json!(base(&read)));
                }
                _ => {}
            }
        }
        g["mine"] = json!(mine);
        g["owed"] = json!(owed);
        g["obl"] = json!(obl);
        g["renamed"] = json!(renamed);
        g["orderOk"] = json!(order_ok);
        let instrumented = self.oracle.observed_commit(&before, &after, tag);
        // Keep the actual outbox; instrumentation is allowed to evaluate
        // guarantee predicates, never to manufacture a host push.
        after["g"] = instrumented["g"].clone();
        self.observed = after;
        self.oracle = self.oracle.next(name, args).unwrap();
        self.compare(name);
        assert_eq!(
            self.observed["g"],
            self.oracle.state()["g"],
            "{name}/observed ghosts"
        );
        assert!(
            self.oracle.observed_guarantee(&self.observed),
            "{name}/guarantee"
        );
        self.saves_start_only_where_the_model_may();
        self.actions += 1;
    }
}
