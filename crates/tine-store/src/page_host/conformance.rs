//! Observed-state driver: the oracle supplies successors, never client input,
//! host fields, physical draft selection or guarantee history.
use super::*;
use crate::page_state::conformance::Oracle;
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

const ABSENT: i64 = -1;
const NONE: i64 = -2;
const UNKNOWN: i64 = -3;

fn key(p: usize) -> String {
    format!("{p:04}.md")
}

/// Pages a model action names through the host: the window's own page,
/// an operation's endpoints and referrers. External writes, faults and
/// whole-host actions name none.
fn named(name: &str, args: &[Value]) -> Vec<usize> {
    let at = |i: usize| args[i].as_u64().unwrap() as usize;
    let flag = |i: usize| {
        args[i]
            .as_bool()
            .unwrap_or_else(|| args[i].as_i64() == Some(1))
    };
    match name {
        "wOpen" | "wSend" | "wDiscard" | "wClose" | "wRecv" | "flush" | "observe" | "draftSync"
        | "wEdit" | "wResolve" | "opDelete" | "flushDel" | "load" => vec![at(0)],
        "wOp" => vec![at(0), 1 - at(0)],
        "wOpTo" => vec![at(0), at(1)],
        "opRename" => {
            let refs: Vec<usize> = serde_json::from_value(args[2].clone()).unwrap();
            [at(0), at(1)].into_iter().chain(refs).collect()
        }
        "opRenameRaw" | "opRenamePacked" => [at(0), at(1)]
            .into_iter()
            .chain((0..3).filter(|&i| flag(2 + i)))
            .collect(),
        _ => vec![],
    }
}

/// The model page a harness key names.
fn index(key: &str) -> usize {
    key.trim_end_matches(".md").parse().unwrap()
}

fn text(label: i64) -> Text {
    (label != ABSENT).then(|| Arc::from(label.to_string().into_bytes()))
}

fn label(bytes: &Text) -> i64 {
    bytes
        .as_ref()
        .map_or(ABSENT, |b| std::str::from_utf8(b).unwrap().parse().unwrap())
}

fn base(base: &Base) -> i64 {
    match base {
        Base::Known(bytes) => label(bytes),
        Base::Unknown => UNKNOWN,
    }
}

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
    observed: Value,
    versions: BTreeMap<(u64, u64), i64>,
    record_versions: BTreeMap<u64, i64>,
    counter: i64,
    profile: String,
    barriers: usize,
    actions: usize,
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
            observed,
            versions: BTreeMap::new(),
            record_versions: BTreeMap::new(),
            counter: 0,
            profile: profile.into(),
            barriers: 0,
            actions: 0,
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
                fs: h.fs.fork()?,
                keys: h.keys.clone(),
                locks: h.locks.clone(),
                lock_ownership: h.lock_ownership.clone(),
                held: h.held.clone(),
                lock_request: h.lock_request.clone(),
                contended: h.contended.clone(),
                drafts: h.drafts.clone(),
                pages: h.pages.clone(),
                queue: h.queue.clone(),
                applying: h.applying.clone(),
                outbox: h.outbox.clone(),
                subscriptions: h.subscriptions.clone(),
                events: h.events.clone(),
                job: h.job.clone(),
                worker: h.worker.clone(),
                retained: h.retained.clone(),
                version: h.version,
                wseq: h.wseq,
                incarnation: h.incarnation,
                generation: h.generation,
                last_admitted: h.last_admitted,
                last_applied: h.last_applied,
                admission_open: h.admission_open,
                switch_confirmation: h.switch_confirmation,
                alive: h.alive,
            },
            oracle: self.oracle.clone(),
            windows: self.windows.clone(),
            pending_ids: self.pending_ids.clone(),
            observed: self.observed.clone(),
            versions: self.versions.clone(),
            record_versions: self.record_versions.clone(),
            counter: self.counter,
            profile: self.profile.clone(),
            barriers: self.barriers,
            actions: self.actions,
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
                    RequestKind::Discard { version } => {
                        u["kind"] = json!("discard");
                        u["bv"] = json!(self.version(*version));
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
        json!({"alive":self.host.alive,"disk":disk(&self.host.fs.physical().files),
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

    fn admit(&mut self, p: usize, kind: RequestKind) {
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
            let application = worker.application.is_some()
                && matches!(worker.task.stage, Stage::Present | Stage::Absent)
                && (worker.task.bytes.is_some() || worker.remaining.is_empty());
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

    fn step(&mut self, name: &str, args: &[Value]) -> bool {
        if self.prepare_operations
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
            return false;
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
                self.windows[p].text = v(1);
                self.windows[p].pend = true;
            }
            "wSend" | "wResolve" => {
                let w = self.windows[p].clone();
                let t = if name == "wResolve" { v(1) } else { w.text };
                self.windows[p].text = t;
                self.windows[p].pend = false;
                self.windows[p].sent = true;
                self.admit(
                    p,
                    RequestKind::Submit {
                        bytes: text(t),
                        version: self.raw(w.bv),
                        resolve: (name == "wResolve").then(|| text(w.obs)),
                    },
                );
            }
            "wDiscard" => {
                self.windows[p].pend = false;
                self.windows[p].sent = true;
                self.admit(
                    p,
                    RequestKind::Discard {
                        version: self.raw(self.windows[p].bv),
                    },
                );
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
                let vs = self.raw(self.windows[p].bv);
                let vd = self.raw(self.windows[q].bv);
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
                assert_eq!(self.host.start_save(&key(p)), Disposition::Pending);
                self.custody_stutter();
                if name == "flush" && !self.stepped {
                    self.host.advance_save(0);
                }
            }
            "check" => {
                self.host.advance_save(0);
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
                    self.host.advance_save(ep);
                    self.compare("custody marker");
                }
                self.host.advance_save(ep);
            }
            "dirSync" => {
                if self.host.job.as_ref().unwrap().phase == SavePhase::TrashSync {
                    self.host.advance_save(0);
                    self.compare("trash sync");
                }
                if !b(0) {
                    // An explicit error wins over graph Unsupported too.
                    self.host
                        .fs
                        .physical_mut()
                        .inject(Phase::PageSync, [Fault::Before]);
                }
                self.host.advance_save(0);
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
                self.host.advance_save(0);
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
                let q = v(1) as usize;
                let refs: BTreeSet<usize> = if name == "opRename" {
                    serde_json::from_value(args[2].clone()).unwrap()
                } else {
                    (0..3)
                        .filter(|&i| b(2 + i) && (name == "opRenameRaw" || i != p && i != q))
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
                    (0..3).map(|i| (i, v(5 + i))).collect()
                };
                let refkeys = refs.iter().map(|&r| key(r)).collect();
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
                    Disposition::Pending
                );
                self.compare("rename pending");
                self.drain(name, args, old_inc, old_version);
                return true;
            }
            "switchReq" => {
                assert_eq!(self.host.switch_request(), Disposition::Applied);
            }
            "switchFin" => {
                assert_eq!(
                    self.host.switch_ready(self.host.last_admitted),
                    Disposition::Applied
                );
                assert_eq!(self.host.switch_finish(), Disposition::Applied);
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
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
                self.host.fs.crash();
                self.host.stop();
                self.fresh_binding();
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "power" | "powerK" | "powerKBits" => {
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
        match name {
            "wSend" | "wResolve" => {
                owed.insert((p, self.windows[p].text));
                if name == "wResolve" {
                    insert(&mut g["seen"][p], json!(before["s"]["w"][p]["obs"]));
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
                        let page = &actual["pages"][p];
                        if page["ver"] != before["s"]["pages"][p]["ver"] {
                            mine.insert(p);
                            let pr = &g["promise"][p];
                            let live = pr["on"] == true
                                && !(pr["saved"] == true
                                    && g["ext"][p].as_i64().unwrap() > pr["ep"].as_i64().unwrap());
                            if !live || pr["bytes"] != page["buf"] {
                                g["promise"][p]["on"] = json!(false);
                                g["promise"][p]["ver"] = page["ver"].clone();
                            }
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
            "opDelete" | "opRename" | "opRenamePacked" | "opRenameRaw" => {
                for i in 0..self.windows.len() {
                    if before["s"]["pages"][i] != actual["pages"][i] {
                        mine.insert(i);
                    }
                }
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
                g["ext"][p] = json!(self.host.fs.physical().epochs[&key(p)]);
                tag = "extWrite";
            }
            "crash" | "switchFin" | "power" | "powerK" | "powerKBits" => {
                owed.clear();
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
                    promise(
                        &mut g["promise"][p],
                        label(&bytes),
                        self.version(version),
                        true,
                        epoch,
                    );
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
        self.actions += 1;
    }

    /// Earlier trash custody before a save is internal to the flush step
    /// (A4 rule 4): Custody and Temp are both abstract phase 1.
    fn custody_stutter(&mut self) {
        if self.host.job.as_ref().unwrap().phase == SavePhase::Custody {
            self.host.advance_save(0);
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
                        return Err("unsupported-native-probe");
                    };
                    if probe.program(op[1].as_array().unwrap()).is_ok() {
                        return Err("false");
                    }
                }
                "action" => {
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

fn insert(set: &mut Value, value: Value) {
    let mut values = set.as_array().unwrap().clone();
    if !values.contains(&value) {
        values.push(value);
    }
    // All set values here are numbers, pairs or tags. Use numeric order for
    // pairs so crossing the diagnostic version bounds remains correct.
    values.sort_by(|a, b| match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_i64().cmp(&b.as_i64()),
        (Value::Array(a), Value::Array(b)) => a
            .iter()
            .map(|v| v.as_i64().unwrap())
            .cmp(b.iter().map(|v| v.as_i64().unwrap())),
        _ => a.to_string().cmp(&b.to_string()),
    });
    *set = json!(values);
}

fn promise(pr: &mut Value, bytes: i64, ver: i64, saved: bool, ep: u64) {
    let old = pr["ver"].as_i64().unwrap();
    if ver > old || (ver == old && pr["on"] == true && saved && pr["saved"] == false) {
        *pr = json!({"on":true,"bytes":bytes,"ver":ver,"saved":saved,"ep":ep});
    }
}

#[test]
#[cfg(test)]
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
    assert_eq!(comparisons, 572);
    eprintln!("host scenarios: {comparisons} outcomes / {actions} actions / {barriers} barriers");
}

#[cfg(test)]
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
#[cfg(test)]
fn committed_itf_traces_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/traces.json")).unwrap();
    let (traces, actions, barriers) = replay(&fixture);
    assert_eq!(traces, 32);
    eprintln!("host ITF: {traces} traces / {actions} actions / {barriers} barriers");
}

#[test]
#[cfg(test)]
fn committed_witnesses_through_host() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/s2/witnesses.json")).unwrap();
    let (traces, actions, barriers) = replay(&fixture);
    assert!(traces > 0);
    eprintln!("host witnesses: {traces} traces / {actions} actions / {barriers} barriers");
}

#[test]
#[cfg(test)]
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
    assert_eq!(traces, 38);
    eprintln!("short host witnesses: {traces} traces / {actions} actions / {barriers} barriers");
}
