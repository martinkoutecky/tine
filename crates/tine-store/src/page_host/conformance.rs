//! Observed-state driver: the oracle supplies successors, never client input,
//! host fields, physical draft selection or guarantee history.
use super::*;
use crate::page_state::conformance::Oracle;
use io::Phase;
use model_fs::{Fault, ModelFs};
use serde_json::{json, Value};

const ABSENT: i64 = -1;
const NONE: i64 = -2;
const UNKNOWN: i64 = -3;

fn key(p: usize) -> String {
    format!("{p:04}.md")
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

#[derive(Clone)]
struct Driver {
    host: Host<ModelFs>,
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
}

impl Driver {
    fn new(profile: &str, count: usize) -> Self {
        let locks = (0..count)
            .map(|p| (key(p), Arc::new(Mutex::new(()))))
            .collect();
        let mut fs = ModelFs {
            weak_graph: matches!(profile, "weak" | "all"),
            ..ModelFs::default()
        };
        for (p, t) in [(0, 1), (1, 2)].into_iter().filter(|(p, _)| *p < count) {
            fs.external(&key(p), text(t), true);
        }
        fs.epochs.clear();
        let oracle = Oracle::new(profile, count);
        let observed = oracle.state(); // Only the model's fixed init state.
        Self {
            host: Host::new(fs, locks),
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
        }
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

    fn register_versions(&mut self, previous_inc: u64, previous: u64) {
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
        let start = if previous_inc == self.host.incarnation {
            previous
        } else {
            // Launch seeds from every readable record and allocates for held
            // pages in key order; record tokens keep their earlier identity.
            drafts::scan(self.host.fs.draft_files(false)).max_version
        };
        for raw in start + 1..=self.host.version {
            self.counter += 1;
            self.versions
                .insert((self.host.incarnation, raw), self.counter);
        }
        for records in drafts::scan(self.host.fs.draft_files(false)).files.values() {
            for r in records {
                if !self.record_versions.contains_key(&r.wseq) {
                    if let Some(&v) = self.versions.get(&(self.host.incarnation, r.version)) {
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
                let p = self.host.keys.iter().position(|k| k == &r.page).unwrap();
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
                        u["q"] = json!(self.host.keys.iter().position(|k| k == receiver).unwrap());
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
            |j| json!({"on":true,"p":self.host.keys.iter().position(|k| k==&j.page).unwrap(),
                "bytes":label(&j.bytes),"base":base(&j.base),"ver":self.version(j.version),
                "phase":match j.phase { SavePhase::Temp|SavePhase::Check=>1,
                    SavePhase::Rename=>2,SavePhase::TrashSync|SavePhase::DirectorySync=>3 },"ep":j.epoch})
        );
        let disk = |files: &BTreeMap<String, Arc<[u8]>>| -> Vec<_> {
            (0..self.windows.len())
                .map(|p| label(&files.get(&format!("graph/{}", key(p))).cloned()))
                .collect()
        };
        let trash = |files: &BTreeMap<String, Arc<[u8]>>| -> Vec<_> {
            (0..self.windows.len())
                .map(|p| {
                    files
                        .iter()
                        .filter(|(k, _)| k.starts_with(&format!("trash/{}/", key(p))))
                        .map(|(_, b)| label(&Some(b.clone())))
                        .collect::<BTreeSet<_>>()
                })
                .collect()
        };
        json!({"alive":self.host.alive,"disk":disk(&self.host.fs.files),
            "stable":disk(&self.host.fs.stable),"drafts":records,"pages":pages,
            "mb":mail,"up":up,"job":job,"w":self.windows,
            "trash":trash(&self.host.fs.files),"trashStable":trash(&self.host.fs.stable)})
    }

    fn compare(&mut self, context: &str) {
        let actual = self.abstract_state();
        let expected = self.oracle.state();
        for (name, value) in actual.as_object().unwrap() {
            if matches!(name.as_str(), "trash" | "trashStable") {
                // The declared trash inclusion simulation permits a copy
                // synced before the source directory has been synced.
                for (p, set) in expected["s"][name].as_array().unwrap().iter().enumerate() {
                    for t in set.as_array().unwrap() {
                        assert!(
                            value[p].as_array().unwrap().contains(t),
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
            let q = self.host.keys.iter().position(|k| k == receiver).unwrap();
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
                self.register_versions(old_inc, old_version);
                self.finish(name, args);
                applied = true;
            } else {
                self.compare(&format!("{name}/physical"));
            }
        }
        panic!("{name}: nonterminal draft effect");
    }

    fn step(&mut self, name: &str, args: &[Value]) -> bool {
        let successor = self.oracle.next(name, args);
        if successor.is_none() {
            return false;
        }
        let p = args.first().and_then(Value::as_u64).unwrap_or(0) as usize;
        let v = |i: usize| args[i].as_i64().unwrap();
        let b = |i: usize| args[i].as_bool().unwrap_or_else(|| v(i) == 1);
        let old_inc = self.host.incarnation;
        let old_version = self.host.version;
        match name {
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
                let kind = self.host.queue.front().unwrap().kind.clone();
                if !b(0) {
                    let phase = if matches!(kind, RequestKind::Move { .. }) {
                        Phase::DraftTemp
                    } else {
                        Phase::Read
                    };
                    if !matches!(kind, RequestKind::Submit { .. } | RequestKind::Close) {
                        self.host.fs.inject(phase, [Fault::Before]);
                    }
                }
                assert_eq!(self.host.dequeue(), Disposition::Pending);
                self.compare("dequeue");
                self.host.apply_request();
                if self.host.worker.is_none() {
                    // rok belongs to this delivery; a held open or a stale
                    // move performs no read/install and consumes no fault.
                    self.host.fs.faults.clear();
                }
                if self.host.worker.is_some() {
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
                if name == "flush" {
                    self.host.advance_save(0);
                }
            }
            "check" => {
                self.host.advance_save(0);
            }
            "rename" => {
                let page = self.host.job.as_ref().unwrap().page.clone();
                let ep = self.host.fs.epochs.get(&page).copied().unwrap_or(0);
                self.host.advance_save(ep);
            }
            "dirSync" => {
                if self.host.job.as_ref().unwrap().phase == SavePhase::TrashSync {
                    self.host.advance_save(0);
                    self.compare("trash sync");
                }
                if !b(0) {
                    // An explicit error wins over graph Unsupported too.
                    self.host.fs.inject(Phase::PageSync, [Fault::Before]);
                }
                self.host.advance_save(0);
            }
            "saveFail" => {
                let phase = match self.host.job.as_ref().unwrap().phase {
                    SavePhase::Temp => Phase::PageTemp,
                    SavePhase::Check => Phase::Read,
                    SavePhase::Rename if self.host.job.as_ref().unwrap().bytes.is_some() => {
                        Phase::PageRename
                    }
                    SavePhase::Rename => Phase::TrashMove,
                    _ => panic!("late saveFail"),
                };
                self.host.fs.inject(phase, [Fault::Before]);
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
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "power" | "powerK" | "powerKBits" => {
                let keep: BTreeSet<usize> = match name {
                    "power" => (0..2).filter(|&i| b(i)).collect(),
                    "powerKBits" => (0..3).filter(|&i| b(i)).collect(),
                    _ => serde_json::from_value(args[0].clone()).unwrap(),
                };
                self.host
                    .fs
                    .power(&keep.into_iter().map(key).collect(), false);
                self.host.stop();
                self.windows.fill(Window::default());
                self.pending_ids.fill(None);
            }
            "launch" => {
                self.host.launch();
                self.register_versions(old_inc, old_version);
                self.finish(name, args);
                while self.host.worker.is_some() {
                    self.host.advance_draft();
                    self.compare("launch representation");
                }
                return true;
            }
            _ => panic!("unsupported host action {name}"),
        }
        self.register_versions(old_inc, old_version);
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
                g["ext"][p] = json!(self.host.fs.epochs[&key(p)]);
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
                    let p = self.host.keys.iter().position(|k| k == &page).unwrap();
                    insert(
                        &mut g["wrote"][p],
                        json!([label(&bytes), self.version(version)]),
                    );
                }
                Event::Draft(r) => {
                    let p = self.host.keys.iter().position(|k| k == &r.page).unwrap();
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
                    let p = self.host.keys.iter().position(|k| k == &page).unwrap();
                    promise(
                        &mut g["promise"][p],
                        label(&bytes),
                        self.version(version),
                        true,
                        epoch,
                    );
                }
                Event::Removed { page, bytes } => {
                    let p = self.host.keys.iter().position(|k| k == &page).unwrap();
                    insert(&mut g["removed"][p], json!(label(&bytes)));
                }
                Event::DeleteDurable { page, .. } => {
                    let p = self.host.keys.iter().position(|k| k == &page).unwrap();
                    g["delDurable"][p] = g["removed"][p].clone();
                }
                Event::OperationRead { page, base: read } => {
                    let p = self.host.keys.iter().position(|k| k == &page).unwrap();
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

    fn program(&mut self, program: &[Value]) -> Result<(), &'static str> {
        for op in program {
            match op[0].as_str().unwrap() {
                "init" => *self = Self::new(&self.profile, self.windows.len()),
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
                    let mut probe = self.clone();
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

fn run(d: &mut Driver, actions: &[(&str, Value)]) {
    for (name, args) in actions {
        assert!(d.step(name, args.as_array().unwrap()), "{name}/{args}");
    }
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
    assert_eq!(comparisons, 480);
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
