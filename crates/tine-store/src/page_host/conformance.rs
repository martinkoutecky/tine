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
        std::fs::create_dir(root.path().join("pages")).unwrap();
        for (path, bytes) in &self.host.fs.physical().files {
            if let Some(page) = path.strip_prefix("graph/") {
                if page.ends_with(".md") || page.ends_with(".org") {
                    std::fs::write(root.path().join("pages").join(page), bytes).unwrap();
                }
            }
        }
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

    fn step(&mut self, name: &str, args: &[Value]) -> bool {
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

    fn finish(&mut self, name: &str, args: &[Value]) {
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
