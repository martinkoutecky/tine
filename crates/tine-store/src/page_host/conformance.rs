//! Observed-state driver: the oracle supplies successors, never client input,
//! host fields, physical draft selection or guarantee history.
use super::*;
use crate::page_state::conformance::Oracle;
use crate::PageId;
use io::Phase;
use model_fs::{Fault, ModelFs};
use serde_json::{json, Value};

#[path = "native_conformance_tests.rs"]
mod native_conformance;

/// Test physical environment; the native implementation independently checks
/// the real paths after every production I/O phase. Power stays ModelFs-only.
trait ConformanceIo: HostIo {
    fn native_create_refinement() -> bool {
        false
    }
    fn fresh(profile: &str, count: usize) -> Self;
    fn physical(&self) -> &ModelFs;
    fn physical_mut(&mut self) -> &mut ModelFs;
    fn external(&mut self, page: &str, bytes: Text, durable: bool);
    fn crash(&mut self);
    fn power(&mut self, keep: &BTreeSet<String>, keep_drafts: bool);
    fn fork(&self) -> Option<Self>
    where
        Self: Sized;
}

impl ConformanceIo for ModelFs {
    fn fresh(profile: &str, count: usize) -> Self {
        let mut fs = Self {
            weak_graph: matches!(profile, "weak" | "all"),
            ..Self::default()
        };
        for (p, t) in [(0, 1), (1, 2)].into_iter().filter(|(p, _)| *p < count) {
            fs.external(&key(p), text(t), true);
        }
        fs.epochs.clear();
        fs
    }
    fn physical(&self) -> &ModelFs {
        self
    }
    fn physical_mut(&mut self) -> &mut ModelFs {
        self
    }
    fn external(&mut self, page: &str, bytes: Text, durable: bool) {
        self.external(page, bytes, durable);
    }
    fn crash(&mut self) {
        self.crash();
    }
    fn power(&mut self, keep: &BTreeSet<String>, keep_drafts: bool) {
        self.power(keep, keep_drafts);
    }
    fn fork(&self) -> Option<Self> {
        Some(self.clone())
    }
}

#[path = "conformance_values.rs"]
mod values;
use values::*;

#[derive(Clone, Serialize)]
struct Window {
    on: bool,
    text: i64,
    bv: i64,
    pend: bool,
    sent: bool,
    obs: i64,
    conf: bool,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            on: false,
            text: ABSENT,
            bv: -1,
            pend: false,
            sent: false,
            obs: ABSENT,
            conf: false,
        }
    }
}

struct Driver<F: ConformanceIo = ModelFs> {
    host: Host<F>,
    oracle: Oracle,
    windows: Vec<Window>,
    pending_ids: Vec<Option<u64>>,
    /// The raw version the window held at each admitted Discard, by
    /// (generation, id): the model's `bv`, which the host's Discard does not
    /// carry (`upDiscard` never reads it).
    discard_versions: BTreeMap<(u64, u64), u64>,
    observed: Value,
    versions: BTreeMap<(u64, u64), i64>,
    record_versions: BTreeMap<u64, i64>,
    counter: i64,
    profile: String,
    barriers: usize,
    actions: usize,
    twin_failures: usize,
    occupied_refusals: usize,
    queued_refusals: usize,
    effective_renames: usize,
    save_kind_refinements: usize,
    refinement_schedule: bool,
    twin_suffix: bool,
    stepped: bool,
    prepare_operations: bool,
    effect: Option<(String, Vec<Value>, u64, u64)>,
    /// Trash keys whose custody a save escaped (R-STORAGE-ERROR).
    escaped: BTreeMap<String, i64>,
    /// The next saveFail lands after this many system calls of its HostIo
    /// call instead of before the call (REVIEW-2b-r2 R1).
    cut: Option<usize>,
}

impl Clone for Driver<ModelFs> {
    fn clone(&self) -> Self {
        self.fork().unwrap()
    }
}

impl Driver<ModelFs> {
    fn new(profile: &str, count: usize) -> Self {
        Self::new_with_io(profile, count)
    }
}

impl<F: ConformanceIo> Driver<F> {
    fn new_with_io(profile: &str, count: usize) -> Self {
        let fs = F::fresh(profile, count);
        let oracle = Oracle::new(profile, count);
        let observed = oracle.state(); // Only the model's fixed init state.
        Self {
            // STEP3 §2: keys register dynamically, as the binding opens them.
            host: Host::new(fs, BTreeMap::new()),
            oracle,
            windows: vec![Window::default(); count],
            pending_ids: vec![None; count],
            discard_versions: BTreeMap::new(),
            observed,
            versions: BTreeMap::new(),
            record_versions: BTreeMap::new(),
            counter: 0,
            profile: profile.into(),
            barriers: 0,
            actions: 0,
            twin_failures: 0,
            occupied_refusals: 0,
            queued_refusals: 0,
            effective_renames: 0,
            save_kind_refinements: 0,
            refinement_schedule: false,
            twin_suffix: false,
            stepped: false,
            prepare_operations: false,
            effect: None,
            escaped: BTreeMap::new(),
            cut: None,
        }
    }

    fn fork(&self) -> Option<Self> {
        let h = &self.host;
        Some(Self {
            host: Host {
                custody: h.custody.clone(),
                retire: h.retire.clone(),
                custody_errors: h.custody_errors.clone(),
                custody_unknown: h.custody_unknown.clone(),
                vehicle_debt: h.vehicle_debt.clone(),
                fs: h.fs.fork()?,
                keys: h.keys.clone(),
                locks: h.locks.clone(),
                lock_ownership: h.lock_ownership.clone(),
                held: h.held.clone(),
                lock_request: h.lock_request.clone(),
                contended: h.contended.clone(),
                drafts: h.drafts.clone(),
                vehicles_of: h.vehicles_of.clone(),
                durable: h.durable.clone(),
                live_ops: h.live_ops.clone(),
                changed: h.changed.clone(),
                pages: h.pages.clone(),
                queue: h.queue.clone(),
                applying: h.applying.clone(),
                outbox: h.outbox.clone(),
                subscriptions: h.subscriptions.clone(),
                events: h.events.clone(),
                job: h.job.clone(),
                worker: h.worker.clone(),
                retained: h.retained.clone(),
                order: h.order.clone(),
                version: h.version,
                wseq: h.wseq,
                incarnation: h.incarnation,
                generation: h.generation,
                last_admitted: h.last_admitted,
                last_applied: h.last_applied,
                admission_open: h.admission_open,
                switch_confirmation: h.switch_confirmation,
                alive: h.alive,
                #[cfg(test)]
                test: Default::default(),
            },
            oracle: self.oracle.clone(),
            windows: self.windows.clone(),
            pending_ids: self.pending_ids.clone(),
            discard_versions: self.discard_versions.clone(),
            observed: self.observed.clone(),
            versions: self.versions.clone(),
            record_versions: self.record_versions.clone(),
            counter: self.counter,
            profile: self.profile.clone(),
            barriers: self.barriers,
            actions: self.actions,
            twin_failures: self.twin_failures,
            occupied_refusals: self.occupied_refusals,
            queued_refusals: self.queued_refusals,
            effective_renames: self.effective_renames,
            save_kind_refinements: self.save_kind_refinements,
            refinement_schedule: self.refinement_schedule,
            twin_suffix: self.twin_suffix,
            stepped: self.stepped,
            prepare_operations: self.prepare_operations,
            effect: self.effect.clone(),
            escaped: self.escaped.clone(),
            cut: self.cut,
        })
    }

    fn version(&self, raw: u64) -> i64 {
        if raw == 0 {
            0
        } else {
            self.versions[&(self.host.incarnation, raw)]
        }
    }

    fn raw(&self, version: i64) -> u64 {
        if version <= 0 {
            return 0;
        }
        self.versions
            .iter()
            .find_map(|(&(inc, raw), &v)| {
                (inc == self.host.incarnation && v == version).then_some(raw)
            })
            .unwrap()
    }

    /// STEP2-DESIGN §2's version abstraction, as STEP3 §2 refines it: an
    /// order- and equality-preserving map from `(incarnation, raw)` to model
    /// versions, learned where the host first shows a version (a held page,
    /// its mail, the job) from the model's value in the same position. Host
    /// ranks count registered keys while the model reserves `PAGES.size()`
    /// (L524/L547), so allocation gaps differ and literal numbering cannot be
    /// compared once keys register dynamically (STEP3-REVIEW-1 F8). Every
    /// shown version must map, no model version may have two host versions,
    /// and the map must be strictly increasing; a version a host step shows
    /// that the model did not allocate there fails one of these.
    fn register_versions(&mut self, name: &str, args: &[Value], previous_inc: u64) {
        let model = self.oracle.next(name, args).expect("enabled").state();
        self.learn_versions(&model, previous_inc, name);
    }

    /// A physical step the model sees as a stutter learns against its
    /// current state.
    fn learn_versions(&mut self, model: &Value, previous_inc: u64, name: &str) {
        if previous_inc != self.host.incarnation {
            let surviving: BTreeSet<_> = drafts::scan(self.host.fs.draft_files(false))
                .files
                .values()
                .flatten()
                .map(|r| r.wseq)
                .collect();
            self.record_versions
                .retain(|seq, _| surviving.contains(seq));
        }
        let inc = self.host.incarnation;
        let mut shown: Vec<(u64, &Value)> = vec![];
        for p in 0..self.windows.len() {
            if let Some(pg) = self.host.pages.get(&key(p)) {
                shown.push((pg.version, &model["s"]["pages"][p]["ver"]));
            }
            if let Some(m) = self.host.outbox.get(&key(p)) {
                if let Some(pg) = &m.page {
                    shown.push((pg.version, &model["s"]["mb"][p]["ver"]));
                }
                if let Some(a) = &m.answer {
                    shown.push((a.version, &model["s"]["mb"][p]["ack"]));
                }
            }
        }
        if let Some(j) = &self.host.job {
            shown.push((j.version, &model["s"]["job"]["ver"]));
        }
        for (raw, value) in shown {
            let v = value.as_i64().expect("model version");
            if raw == 0 || self.versions.contains_key(&(inc, raw)) {
                continue;
            }
            assert!(
                !self.versions.values().any(|&mapped| mapped == v),
                "{name}: model version {v} already has a host version"
            );
            self.versions.insert((inc, raw), v);
        }
        let mut last = 0;
        for (&(i, raw), &v) in &self.versions {
            assert!(
                v > last,
                "{name}: version map not order-preserving at ({i}, {raw})"
            );
            last = v;
        }
        self.counter = model["g"]["vc"].as_i64().expect("model vc");
        for records in drafts::scan(self.host.fs.draft_files(false)).files.values() {
            for r in records {
                if !self.record_versions.contains_key(&r.wseq) {
                    if let Some(&v) = self.versions.get(&(inc, r.version)) {
                        self.record_versions.insert(r.wseq, v);
                    }
                }
            }
        }
    }

    fn abstract_state(&self) -> Value {
        let drafts = self.host.logical_drafts();
        let pages: Vec<_> = (0..self.windows.len())
            .map(|p| {
                self.host.pages.get(&key(p)).map_or_else(
                    || {
                        json!({"held":false,"buf":ABSENT,"base":ABSENT,"ver":0,
                    "typed":false,"risk":false,"conflict":false,"obs":ABSENT})
                    },
                    |pg| {
                        json!({"held":true,"buf":label(&pg.buf),"base":base(&pg.base),
                    "ver":self.version(pg.version),"typed":pg.typed,"risk":pg.risk,
                    "conflict":pg.conflict,"obs":pg.obs.as_ref().map_or(NONE,label)})
                    },
                )
            })
            .collect();
        let records: Vec<_> = (0..self.windows.len())
            .map(|p| {
                drafts.get(&key(p)).map_or_else(
                    || json!({"bytes":NONE,"base":NONE,"ver":0}),
                    |r| {
                        json!({"bytes":label(&r.bytes),"base":base(&r.base),
                    "ver":self.record_versions[&r.wseq]})
                    },
                )
            })
            .collect();
        let mail: Vec<_> = (0..self.windows.len())
            .map(|p| {
                self.host.outbox.get(&key(p)).map_or_else(
                    || {
                        json!({"on":false,"held":false,"ver":0,"text":ABSENT,
                    "obs":ABSENT,"conf":false,"ack":-1,"took":false})
                    },
                    |m| {
                        json!({"on":true,"held":m.page.is_some(),
                    "ver":m.page.as_ref().map_or(0,|p| self.version(p.version)),
                    "text":m.page.as_ref().map_or(ABSENT,|p| label(&p.buf)),
                    "obs":m.page.as_ref().map_or(ABSENT,|p| p.obs.as_ref().map_or(NONE,label)),
                    "conf":m.page.as_ref().is_some_and(|p|p.conflict),
                    "ack":m.answer.as_ref().map_or(-1,|a|self.version(a.version)),
                    "took":m.answer.as_ref().is_some_and(|a|a.took)})
                    },
                )
            })
            .collect();
        let up: Vec<_> = self
            .host
            .abstract_queue()
            .iter()
            .map(|r| {
                let p = index(&r.page);
                let mut u = json!({"kind":"open","p":p,"q":0,"t":0,"t2":0,
                "bv":0,"bv2":0,"ro":NONE,"cur":r.generation==self.host.generation});
                match &r.kind {
                    RequestKind::Open => {}
                    RequestKind::Close => u["kind"] = json!("close"),
                    RequestKind::Discard => {
                        u["kind"] = json!("discard");
                        let version = self.discard_versions[&(r.generation, r.id)];
                        u["bv"] = json!(self.version(version));
                    }
                    RequestKind::Submit {
                        bytes,
                        version,
                        resolve,
                    } => {
                        u["kind"] = json!("submit");
                        u["t"] = json!(label(bytes));
                        u["bv"] = json!(self.version(*version));
                        u["ro"] = json!(resolve.as_ref().map_or(NONE, label));
                    }
                    RequestKind::Move {
                        receiver,
                        source_text,
                        receiver_text,
                        source_version,
                        receiver_version,
                    } => {
                        u["kind"] = json!("op");
                        u["q"] = json!(index(receiver));
                        u["t"] = json!(label(source_text));
                        u["t2"] = json!(label(receiver_text));
                        u["bv"] = json!(self.version(*source_version));
                        u["bv2"] = json!(self.version(*receiver_version));
                    }
                }
                u
            })
            .collect();
        let job = self.host.job.as_ref().map_or_else(
            || json!({"on":false,"p":0,"bytes":0,"base":0,"ver":0,"phase":0,"ep":0}),
            |j| json!({"on":true,"p":index(&j.page),
                "bytes":label(&j.bytes),"base":base(&j.base),"ver":self.version(j.version),
                "phase":match j.phase { SavePhase::Custody|SavePhase::Temp|SavePhase::Check=>1,
                    SavePhase::Marker|SavePhase::Rename=>2,SavePhase::TrashSync|SavePhase::DirectorySync=>3 },"ep":j.epoch})
        );
        let disk = |files: &BTreeMap<String, Arc<[u8]>>| -> Vec<_> {
            (0..self.windows.len())
                .map(|p| label(&files.get(&format!("graph/{}", key(p))).cloned()))
                .collect()
        };
        let trash = |files| self.trash_labels(files);
        let gates: Vec<_> = self
            .host
            .order
            .view()
            .map(|(dst, src, versions, open)| {
                let vers: Vec<_> = (0..self.windows.len())
                    .map(|p| versions.get(&key(p)).map_or(NONE, |v| self.version(*v)))
                    .collect();
                let open: BTreeSet<_> = open.iter().map(|k| index(k)).collect();
                json!({"dst":index(dst),"src":index(src),"vers":vers,"open":open})
            })
            .collect();
        json!({"alive":self.host.alive,"claimed":self.claims(),"gates":gates,"disk":disk(&self.host.fs.physical().files),
            "stable":disk(&self.host.fs.physical().stable),"drafts":records,"pages":pages,
            "mb":mail,"up":up,"job":job,"w":self.windows,
            "trash":trash(&self.host.fs.physical().files),"trashStable":trash(&self.host.fs.physical().stable)})
    }

    fn trash_labels(&self, files: &BTreeMap<String, Arc<[u8]>>) -> Vec<BTreeSet<i64>> {
        (0..self.windows.len())
            .map(|p| {
                files
                    .iter()
                    .filter(|(k, _)| k.starts_with(&format!("trash/{}/", key(p))))
                    .map(|(_, b)| label(&Some(b.clone())))
                    .collect()
            })
            .collect()
    }

    /// Every power outcome ModelFs permits must keep the oracle's trash (L604,
    /// L695), unless the missing bytes are a named A4 residual. Anything else
    /// is a structural hole in A4: this panics and the lane stops.
    fn refine_power_cuts(&self, keep: &BTreeSet<String>, expected: &Value) {
        let fs = self.host.fs.physical();
        for cut in fs.cuts(keep) {
            let mut after = fs.clone();
            after.power_cut(keep, false, &cut);
            let actual = self.trash_labels(&after.files);
            for (p, labels) in expected.as_array().unwrap().iter().enumerate() {
                for t in labels.as_array().unwrap() {
                    let t = t.as_i64().unwrap();
                    if actual[p].contains(&t) {
                        continue;
                    }
                    let residual = fs.files.iter().any(|(k, b)| {
                        k.starts_with(&format!("trash/{}/", key(p)))
                            && label(&Some(b.clone())) == t
                            && (cut.lose.contains(k) || self.escaped.contains_key(k))
                    });
                    assert!(
                        residual,
                        "A4 hole: power keep={keep:?} cut={cut:?} loses trash {t} of page {p}"
                    );
                }
            }
        }
    }

    fn compare(&mut self, context: &str) {
        let actual = self.abstract_state();
        let expected = self.oracle.state();
        for (name, value) in actual.as_object().unwrap() {
            if matches!(name.as_str(), "trash" | "trashStable") {
                // The declared trash inclusion simulation permits a copy
                // synced before the source directory has been synced.
                // R-STORAGE-ERROR: an escaped payload's stability claim is
                // not kept, so a power cut may lose it; nothing else is exempt.
                let escaped = |p: usize, t: &Value| {
                    self.escaped.iter().any(|(k, label)| {
                        k.starts_with(&format!("trash/{}/", key(p))) && t.as_i64() == Some(*label)
                    })
                };
                for (p, set) in expected["s"][name].as_array().unwrap().iter().enumerate() {
                    for t in set.as_array().unwrap() {
                        assert!(
                            value[p].as_array().unwrap().contains(t) || escaped(p, t),
                            "{context}/{name}/{p}"
                        );
                    }
                }
            } else {
                assert_eq!(value, &expected["s"][name], "{context}/{name}");
            }
        }
        self.barriers += 1;
    }

    fn admit(&mut self, p: usize, kind: RequestKind) -> u64 {
        let id = self.host.last_admitted + 1;
        if !matches!(kind, RequestKind::Close) {
            self.pending_ids[p] = Some(id);
        }
        if let RequestKind::Move { receiver, .. } = &kind {
            let q = index(receiver);
            self.pending_ids[q] = Some(id);
        }
        assert_eq!(
            self.host.admit(Request {
                id,
                generation: self.host.generation,
                page: key(p),
                kind
            }),
            Disposition::Applied
        );
        id
    }

    /// The binding registers a key when the window opens it or an operation
    /// names it (STEP3 §2), in trace order, not key order.
    fn register(&mut self, p: usize) {
        self.host
            .register(key(p), &key(p), Arc::new(Mutex::new(())));
    }

    /// A process fault ends the binding: the next one registers afresh,
    /// its recovered keys first (§2).
    fn fresh_binding(&mut self) {
        self.host.keys = Keys::default();
        self.host.locks.clear();
    }

    fn receive(&mut self, p: usize) {
        let mail = self.host.receive(&key(p)).unwrap();
        assert_eq!(mail.generation, self.host.generation);
        let w = &self.windows[p];
        let ack = mail.answer.as_ref().is_some_and(|answer| {
            answer.generation == self.host.generation && Some(answer.id) == self.pending_ids[p]
        }) && w.sent;
        if ack {
            self.pending_ids[p] = None;
        }
        let mut nw = w.clone();
        let pg = mail.page.as_ref();
        let obs = pg.map_or(ABSENT, |p| p.obs.as_ref().map_or(NONE, label));
        let conf = pg.is_some_and(|p| p.conflict);
        if ack && pg.is_none() {
            nw = Window::default();
        } else if ack && w.pend {
            nw.sent = false;
            if mail.answer.as_ref().unwrap().took {
                nw.bv = self.version(mail.answer.unwrap().version);
            }
            nw.obs = obs;
            nw.conf = conf;
        } else if ack {
            let pg = pg.unwrap();
            nw = Window {
                on: true,
                text: label(&pg.buf),
                bv: self.version(pg.version),
                pend: false,
                sent: false,
                obs,
                conf,
            };
        } else if w.on && !w.sent && !w.pend {
            if let Some(pg) = pg {
                nw.text = label(&pg.buf);
                nw.bv = self.version(pg.version);
            }
            nw.obs = obs;
            nw.conf = conf;
        } else if w.on {
            nw.obs = obs;
            nw.conf = conf;
        }
        self.windows[p] = nw;
    }

    fn drain(&mut self, name: &str, args: &[Value], old_inc: u64, old_version: u64) {
        if self.stepped {
            assert!(self.effect.is_none());
            self.effect = Some((name.into(), args.to_vec(), old_inc, old_version));
            return;
        }
        let mut applied = false;
        for _ in 0..200 {
            let Some(worker) = &self.host.worker else {
                assert!(applied);
                return;
            };
            let application = applies_next(worker);
            self.host.advance_draft();
            if application {
                assert!(!applied);
                self.register_versions(name, args, old_inc);
                self.finish(name, args);
                applied = true;
            } else {
                self.compare(&format!("{name}/physical"));
            }
        }
        panic!("{name}: nonterminal draft effect");
    }

    /// Physical claimant paths are durable external files, separate from the
    /// canonical page path. Occupancy is read from the filesystem witness.
    fn claims(&self) -> Vec<usize> {
        (0..self.windows.len())
            .filter(|&p| {
                self.host
                    .fs
                    .physical()
                    .files
                    .contains_key(&format!("graph/{p:04}.org"))
            })
            .collect()
    }

    /// The binding's C3 predicate on a native projection of readable files.
    /// The oracle never supplies occupancy or the transaction's answer.
    fn slot_refusal(
        &self,
        eff: &BTreeMap<usize, i64>,
        plan: &BTreeMap<usize, i64>,
    ) -> Option<ApplyRefusal> {
        let creations: Vec<_> = eff
            .iter()
            .filter(|(p, t)| **t != ABSENT && plan.get(p) == Some(&ABSENT))
            .map(|(&p, _)| (crate::FileId::from(format!("pages/{}", key(p))), None, None))
            .collect();
        if creations.is_empty() {
            return None;
        }
        let root = tempfile::tempdir().unwrap();
        native_conformance::write_pages(self.host.fs.physical(), &root.path().join("pages"));
        let store = crate::Store::open(root.path(), Default::default())
            .unwrap()
            .0;
        let refusal = binding::creation_refusal(&store, &creations);
        store.close();
        refusal
    }

    fn rename_operation(&mut self, name: &str, args: &[Value]) -> Disposition {
        let p = args[0].as_u64().unwrap() as usize;
        let q = args[1].as_u64().unwrap() as usize;
        let refs: BTreeSet<usize> = if name == "opRename" {
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
        let rt: BTreeMap<usize, i64> = if name == "opRename" {
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
                .collect()
        } else {
            (0..3).map(|i| (i, args[5 + i].as_i64().unwrap())).collect()
        };
        let refkeys = refs.iter().map(|&r| key(r)).collect();
        self.host
            .rename_with(&key(p), &key(q), &refkeys, |bytes, k, moving| {
                let r = k.trim_end_matches(".md").parse::<usize>().unwrap();
                Ok(if !moving && refs.contains(&r) {
                    text(rt[&r])
                } else {
                    bytes.clone()
                })
            })
    }

    fn apply_admission(&mut self, name: &str, args: &[Value]) -> Result<Disposition, ApplyRefusal> {
        let (eff, plan, dst, src) = apply_args(name, args);
        let keys = eff.keys().map(|&p| key(p)).collect();
        let unsaved = binding::unsaved(&self.host, &keys);
        // Compare the complete production admission set with the independent
        // oracle, not merely the first page selected for the public refusal.
        let expected: BTreeSet<_> = eff
            .keys()
            .filter(|&&p| {
                let page = &self.oracle.state()["s"]["pages"][p];
                page["held"] == true
                    && (page["typed"] == true
                        || page["risk"] == true
                        || page["buf"] != page["base"])
            })
            .map(|&p| key(p))
            .collect();
        let actual: BTreeSet<_> = unsaved.keys().cloned().collect();
        assert!(
            expected.is_subset(&actual),
            "{name}: production omitted oracle-dirty page"
        );
        for &p in eff.keys() {
            if actual.contains(&key(p)) && !expected.contains(&key(p)) {
                let state = &self.oracle.state()["s"];
                assert!(
                    state["w"][p]["pend"] == true
                        || state["w"][p]["sent"] == true
                        || state["up"].as_array().unwrap().iter().any(|u| {
                            u["p"] == json!(p) || (u["kind"] == "op" && u["q"] == json!(p))
                        }),
                    "{name}: production unsaved page outside dirty-or-pending predicate"
                );
            }
        }
        if let Some(page) = unsaved.into_values().next() {
            return Err(ApplyRefusal::Unsaved(page));
        }
        if let Some(refusal) = self.slot_refusal(&eff, &plan) {
            return Err(refusal);
        }
        let effects = eff
            .iter()
            .map(|(&p, &t)| (key(p), (text(t), text(plan[&p]))))
            .collect();
        let ends = (dst != NONE).then(|| (key(dst as usize), key(src as usize)));
        let moved = ends.as_ref().map(|(d, s)| (d.as_str(), s.as_str()));
        match self.host.apply(&effects, moved) {
            Disposition::Refused | Disposition::Disabled => Err(ApplyRefusal::Changed),
            Disposition::Waiting => Err(ApplyRefusal::Busy),
            disposition => Ok(disposition),
        }
    }

    // The window side is the existing synthetic adapter, not native frontend
    // conformance. Submit only when that adapter's current window permits it.
    fn submit_input(&mut self, p: usize, resolve: Option<i64>) -> bool {
        let w = self.windows[p].clone();
        if !self.host.alive || !w.on || w.sent || if resolve.is_some() { !w.conf } else { !w.pend }
        {
            return false;
        }
        let t = resolve.unwrap_or(w.text);
        self.windows[p].text = t;
        self.windows[p].pend = false;
        self.windows[p].sent = true;
        self.admit(
            p,
            RequestKind::Submit {
                bytes: text(t),
                version: self.raw(w.bv),
                // NONE is the model's absent resolution token, not bytes.
                resolve: resolve.and_then(|_| (w.obs != NONE).then(|| text(w.obs))),
            },
        );
        true
    }

    fn edit_input(&mut self, p: usize, text: i64) -> bool {
        if !self.host.alive || !self.windows[p].on || self.windows[p].text == text {
            return false;
        }
        self.windows[p].text = text;
        self.windows[p].pend = true;
        true
    }

    fn move_input(&mut self, p: usize, q: usize, ts: i64, td: i64) -> bool {
        let a = &self.windows[p];
        let Some(b) = self.windows.get(q) else {
            return false;
        };
        if !self.host.alive
            || p == q
            || !a.on
            || !b.on
            || a.pend
            || a.sent
            || b.pend
            || b.sent
            || ts == a.text
            || td == b.text
        {
            return false;
        }
        let vs = self.raw(a.bv);
        let vd = self.raw(b.bv);
        self.windows[p].text = ts;
        self.windows[q].text = td;
        self.windows[p].sent = true;
        self.windows[q].sent = true;
        self.admit(
            p,
            RequestKind::Move {
                receiver: key(q),
                source_text: text(ts),
                receiver_text: text(td),
                source_version: vs,
                receiver_version: vd,
            },
        );
        true
    }

    /// The host never starts a save the model forbids (s3.2: a gated page's
    /// save waits). Traces schedule only enabled model steps, so without
    /// this a host that ignored its order would replay them unchanged.
    fn saves_start_only_where_the_model_may(&self) {
        for p in 0..self.windows.len() {
            let Some(mut probe) = self.fork() else {
                return;
            };
            if probe.host.start_save(&key(p)) == Disposition::Pending {
                assert!(
                    ["flush", "flushDel"]
                        .iter()
                        .any(|name| self.oracle.next(name, &[json!(p)]).is_some()),
                    "the host starts a save of page {p} the model forbids"
                );
            }
        }
    }

    /// Earlier trash custody before a save is internal to the flush step
    /// (A4 rule 4): Custody and Temp are both abstract phase 1.
    fn custody_stutter(&mut self) {
        if self.host.job.as_ref().unwrap().phase == SavePhase::Custody {
            self.host.advance_save();
        }
    }

    fn program(&mut self, program: &[Value]) -> Result<(), &'static str> {
        for op in program {
            match op[0].as_str().unwrap() {
                "init" => *self = Self::new_with_io(&self.profile, self.windows.len()),
                "expect" => {
                    if !self
                        .oracle
                        .eval_observed(&op[1], &self.observed)
                        .as_bool()
                        .unwrap()
                    {
                        return Err("assertion");
                    }
                }
                "if" => {
                    let branch = if self
                        .oracle
                        .eval_observed(&op[1], &self.observed)
                        .as_bool()
                        .unwrap()
                    {
                        2
                    } else {
                        3
                    };
                    self.program(op[branch].as_array().unwrap())?;
                }
                "fail" => {
                    let Some(mut probe) = self.fork() else {
                        if op[1].as_array().is_some_and(|ops| ops.len() == 1)
                            && op[1][0][0] == "action"
                            && matches!(
                                op[1][0][1].as_str(),
                                Some("opApply" | "planCreate" | "opDelete" | "opRename")
                            )
                        {
                            let before = self.abstract_state();
                            let outcome = self.program(op[1].as_array().unwrap());
                            if outcome == Err("effective-rename-refinement") {
                                return outcome;
                            }
                            if outcome != Err("disabled") {
                                return Err("false");
                            }
                            assert_eq!(
                                self.abstract_state(),
                                before,
                                "native refusal changed state"
                            );
                            continue;
                        }
                        return Err("unsupported-native-probe");
                    };
                    let outcome = probe.program(op[1].as_array().unwrap());
                    self.effective_renames = probe.effective_renames;
                    if outcome.is_ok() {
                        return Err("false");
                    }
                }
                "action" => {
                    let twin_failures = self.twin_failures;
                    let effective_renames = self.effective_renames;
                    let args: Vec<_> = op[2]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|e| self.oracle.eval_observed(e, &self.observed))
                        .collect();
                    if F::native_create_refinement()
                        && op[1] == "rename"
                        && self.host.job.as_ref().is_some_and(|job| {
                            job.bytes.is_some()
                                && job.base == Base::Known(None)
                                && self
                                    .host
                                    .fs
                                    .physical()
                                    .files
                                    .contains_key(&format!("graph/{}", job.page))
                        })
                    {
                        // The design explicitly refines contested creates into
                        // saveFail; the unchanged replace-model expectations
                        // are checked separately by the native no-replace test.
                        return Err("native-create-refinement");
                    }
                    if !self.step(op[1].as_str().unwrap(), &args) {
                        return Err("disabled");
                    }
                    if self.twin_failures > twin_failures {
                        // The complete saveFail successor was compared. The
                        // scripted publication suffix belongs to another choice.
                        return Err("twin-saveFail-refinement");
                    }
                    if self.effective_renames > effective_renames {
                        return Err("effective-rename-refinement");
                    }
                }
                _ => panic!("unsupported program opcode {op}"),
            }
        }
        Ok(())
    }
}

#[path = "scheduler.rs"]
mod scheduler;
#[path = "conformance_step.rs"]
mod step;
#[path = "conformance_tests.rs"]
mod tests;

fn run(d: &mut Driver, actions: &[(&str, Value)]) {
    for (name, args) in actions {
        let preparing = d.prepare_operations;
        d.prepare_operations = true;
        assert!(d.step(name, args.as_array().unwrap()), "{name}/{args}");
        d.prepare_operations = preparing;
    }
}

/// Whether the worker's next advance applies its effect: a removal only at
/// its last vehicle step, after the copies an operation vehicle it retires
/// owes its other pages (RENAME-COST L2); anything else at its first
/// vehicle's terminal. A failed copy is rewritten first (`advance_draft`).
pub(super) fn applies_next(worker: &DraftWorker) -> bool {
    let removal = matches!(
        worker.application,
        Some(Application::Removal | Application::Retire)
    );
    let retry = worker.retry_copy && worker.task.stage == Stage::Absent;
    worker.application.is_some()
        && matches!(worker.task.stage, Stage::Present | Stage::Absent)
        && !(retry && worker.task.bytes.is_some())
        && (worker.remaining.is_empty() || !removal && worker.task.bytes.is_some())
}
