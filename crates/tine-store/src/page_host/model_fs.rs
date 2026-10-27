//! Faultable files, with independent readable and durable directory entries.
//! Trash payload data, trash names, the trash directory chain and source names
//! persist independently at a power cut, constrained only by single-move
//! atomicity and A4 rule 5. Each HostIo call is a sequence of system calls, and
//! `Fault::Cut` lands between any two of them (REVIEW-2b-r2 R1).
use super::io::{DraftStatus, ErrorKind, HostIo, IoFailure, IoResult, MoveResult, Phase, Witness};
use super::Text;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

const CUSTODY: &str = "draft/trash-custody/";

#[path = "model_env.rs"]
mod environment;
pub(super) use environment::{Dir, DirKind, Env, EnvStep, Escape, RootState};

/// The trash directory chain's own entries, outermost first: `logseq` in the
/// graph root, `.tine-trash` in it, and `pages` in that. A payload is
/// reachable after a power cut only while every one of them survives.
const CHAIN: [&str; 3] = [
    "dir/logseq",
    "dir/logseq/.tine-trash",
    "dir/logseq/.tine-trash/pages",
];

#[derive(Clone, Copy, Debug)]
pub(super) enum Fault {
    Before,
    After,
    Collision,
    /// The call's first `k` system calls complete and the next one fails, as
    /// an error there or a process or power cut between the two.
    Cut(usize),
    /// A classified I/O error, before the phase or after k modeled calls.
    Errno(std::io::ErrorKind, Option<usize>),
    RootGone(usize),
    RootReplaced(usize),
}

#[derive(Clone, Debug, Default)]
pub(super) struct ModelFs {
    pub env: Env,
    pub escapes: Vec<Escape>,
    pub env_schedule: BTreeMap<Phase, (usize, EnvStep)>,
    pub env_fault: Option<(usize, EnvStep)>,
    pub detached: BTreeMap<String, Arc<[u8]>>,
    pub root_fault: Option<(usize, RootState)>,
    pub graph_io: bool,
    pub files: BTreeMap<String, Arc<[u8]>>,
    pub stable: BTreeMap<String, Arc<[u8]>>,
    pub faults: BTreeMap<Phase, VecDeque<Fault>>,
    pub calls: Vec<Phase>,
    pub weak_graph: bool,
    pub epochs: BTreeMap<String, u64>,
    /// Payload already synced by Tine's temp phase (or a deletion). A graph
    /// directory sync cannot promise unsynced bytes from an external writer.
    pub publications: BTreeMap<String, Text>,
    /// Readable files whose data an external writer has not flushed.
    pub volatile: BTreeSet<String>,
    /// Issue order of each readable graph or trash name's latest operation.
    pub order: BTreeMap<String, u64>,
    /// Pages whose latest namespace operation is the move to this trash key.
    pub moved: BTreeMap<String, String>,
    /// Pages whose latest namespace operation a source-directory sync made
    /// durable (graph launch); single-move atomicity keeps the destination.
    pub sourced: BTreeSet<String>,
    /// Trash chain entries created by this process whose parent sync is still
    /// owed (the adapter's retained `DirectoryCreation`; lost at a crash).
    pub owed: Vec<String>,
    pub(super) owed_identities: BTreeMap<String, Vec<(String, u64)>>,
    pub(super) replan_directories: bool,
    pub(super) publication_epochs: BTreeMap<String, u64>,
    /// System calls left before an injected `Fault::Cut` fails the next one.
    pub budget: Option<usize>,
    pub budget_kind: Option<std::io::ErrorKind>,
    pub observed_errno: Option<std::io::ErrorKind>,
    /// Durable draft entries as last reported by `draft_changes`.
    pub reported: BTreeMap<String, Arc<[u8]>>,
    /// Adapter memory (STEP3 §2), lost at a crash. A model entry is named by
    /// its key; a spelling only orders the driver's path locks.
    pub spellings: BTreeMap<String, String>,
    /// Scripted production-only owner refusals; conformance independently
    /// requires matching physical claimant bytes before accepting saveFail.
    pub owners: BTreeMap<String, String>,
    /// Draft I/O is down (M2): every draft effect fails untouched.
    pub down: bool,
    /// The adapter's trash-chain witness: every ancestor was synced durably by
    /// a `trash_sync` in this incarnation, so later ones sync only the trash
    /// directory (production `trash_chain`, GH3 step 2). Lost at a crash.
    pub chain_witnessed: bool,
}

/// One power outcome for readable trash names that are not yet durable.
#[derive(Clone, Debug, Default)]
pub(super) struct Cut {
    /// Unforced names that survive anyway (a surplus copy).
    pub survive: BTreeSet<String>,
    /// Trash names whose unflushed external data is lost (R-UNFLUSHED), even
    /// after their directory chain was synced: no custody phase syncs a
    /// payload's data (RO3 (a), Martin 2026-10-11).
    pub lose: BTreeSet<String>,
}

impl ModelFs {
    pub fn external(&mut self, page: &str, bytes: Text, durable: bool) {
        let key = format!("graph/{page}");
        if bytes.is_some() {
            self.env.seed_dirs(&self.env.parent(&key));
        }
        set(&mut self.files, &key, bytes.clone());
        if durable {
            set(&mut self.stable, &key, bytes);
            self.volatile.remove(&key);
        } else {
            self.volatile.insert(key.clone());
        }
        self.issue(page, None);
        *self.epochs.entry(page.into()).or_default() += 1;
    }

    /// One namespace operation on the graph filesystem, in issue order.
    fn issue(&mut self, page: &str, trash: Option<&str>) {
        let next = self.next_order();
        self.order.insert(format!("graph/{page}"), next);
        self.sourced.remove(page);
        if let Some(trash) = trash {
            self.order.insert(trash.into(), next);
            self.moved.insert(page.into(), trash.into());
        } else {
            self.moved.remove(page);
        }
    }

    fn next_order(&self) -> u64 {
        self.order.values().max().map_or(1, |n| n + 1)
    }

    /// Readable trash names and chain entries that a power cut must keep, and
    /// those it may drop. Single-move atomicity: a kept page (or one whose
    /// source directory was synced) whose latest operation is its move keeps
    /// the destination name. Rule 5 (only where the witness is Unsupported):
    /// a persisted namespace operation implies every earlier one.
    ///
    /// Rule 5's predicate is an overapproximation of ordered persistence: it
    /// forces what precedes kept operations, but then lets every subset of
    /// the remaining free names survive, including a later one without an
    /// earlier one. That admits more outcomes than ordered persistence, which
    /// is conservative for this check (REVIEW-2b-r2 R1).
    fn trash_outcomes(&self, keep: &BTreeSet<String>) -> (BTreeSet<String>, BTreeSet<String>) {
        let pending: BTreeSet<String> = self
            .files
            .iter()
            .filter(|(key, bytes)| {
                (key.starts_with("trash/") || key.starts_with("dir/"))
                    && self.stable.get(*key) != Some(bytes)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let mut forced: BTreeSet<String> = keep
            .iter()
            .chain(&self.sourced)
            .filter_map(|page| self.moved.get(page))
            .filter(|key| pending.contains(*key))
            .cloned()
            .collect();
        if self.weak_graph {
            let changed = keep
                .iter()
                .map(|page| format!("graph/{page}"))
                .filter(|key| self.files.get(key) != self.stable.get(key));
            let latest = changed
                .chain(forced.iter().cloned())
                .filter_map(|key| self.order.get(&key).copied())
                .max();
            if let Some(latest) = latest {
                forced.extend(
                    pending
                        .iter()
                        .filter(|key| self.order.get(*key).is_some_and(|n| *n < latest))
                        .cloned(),
                );
            }
        }
        let free = pending.difference(&forced).cloned().collect();
        (forced, free)
    }

    /// Every outcome a power cut permits, for an exhaustive refinement check.
    pub fn cuts(&self, keep: &BTreeSet<String>) -> Vec<Cut> {
        let (forced, free) = self.trash_outcomes(keep);
        let free: Vec<_> = free.into_iter().collect();
        let mut cuts = vec![];
        for mask in 0..1usize << free.len() {
            let survive: BTreeSet<_> = (0..free.len())
                .filter(|i| mask >> i & 1 == 1)
                .map(|i| free[i].clone())
                .collect();
            // A durable trash name keeps its writer's unflushed data volatile.
            let named = self
                .stable
                .keys()
                .filter(|key| key.starts_with("trash/") && self.files.contains_key(*key));
            let unflushed: Vec<_> = forced
                .iter()
                .chain(&survive)
                .chain(named)
                .filter(|key| self.volatile.contains(*key))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            for lost in 0..1usize << unflushed.len() {
                let lose = (0..unflushed.len())
                    .filter(|i| lost >> i & 1 == 1)
                    .map(|i| unflushed[i].clone())
                    .collect();
                cuts.push(Cut {
                    survive: survive.clone(),
                    lose,
                });
            }
        }
        cuts
    }

    pub fn inject(&mut self, phase: Phase, faults: impl IntoIterator<Item = Fault>) {
        self.faults.entry(phase).or_default().extend(faults);
    }

    /// Process crashes preserve readable names; only volatile temps disappear.
    pub fn crash(&mut self) {
        self.files.retain(|key, _| !key.starts_with("temp/"));
        self.faults.clear();
        self.env_schedule.clear();
        self.env_fault = None;
        self.root_fault = None;
        self.budget_kind = None;
        self.publications.clear();
        self.publication_epochs.clear();
        self.owed.clear();
        self.owed_identities.clear();
        self.replan_directories = false;
        self.spellings.clear();
        self.down = false;
        self.chain_witnessed = false;
    }

    /// The least surviving outcome: only forced trash names, with their data.
    pub fn power(&mut self, keep: &BTreeSet<String>, keep_draft_directory: bool) {
        self.power_cut(keep, keep_draft_directory, &Cut::default());
    }

    /// Choose which readable graph paths survive (current or stable, §2a rule
    /// 8) and which pending trash names survive. Draft metadata is strong and
    /// reverts to its last directory-sync witness.
    pub fn power_cut(&mut self, keep: &BTreeSet<String>, keep_draft_directory: bool, cut: &Cut) {
        let (forced, _) = self.trash_outcomes(keep);
        let readable = self.files.clone();
        self.files = self.stable.clone();
        for page in keep {
            let graph = format!("graph/{page}");
            set(&mut self.files, &graph, readable.get(&graph).cloned());
        }
        for key in forced.iter().chain(&cut.survive) {
            if !cut.lose.contains(key) {
                self.files.insert(key.clone(), readable[key].clone());
            }
        }
        // R-UNFLUSHED: a durable name does not make unflushed data durable.
        self.files.retain(|key, _| !cut.lose.contains(key));
        // A lost chain entry takes everything beneath it.
        if let Some(lost) = CHAIN.iter().position(|dir| !self.files.contains_key(*dir)) {
            self.files.retain(|key, _| {
                !key.starts_with("trash/") && !CHAIN[lost..].contains(&key.as_str())
            });
        }
        if keep_draft_directory {
            self.files.retain(|key, _| !child(key, "draft/"));
            self.files
                .extend(readable.into_iter().filter(|(key, _)| child(key, "draft/")));
        }
        self.stable = self.files.clone();
        self.volatile.clear();
        self.order.clear();
        self.moved.clear();
        self.sourced.clear();
        self.crash();
    }

    fn run<T>(
        &mut self,
        phase: Phase,
        effect: impl FnOnce(&mut Self) -> IoResult<T>,
    ) -> IoResult<T> {
        self.calls.push(phase);
        self.env_fault = self.env_schedule.remove(&phase);
        self.graph_io = matches!(
            phase,
            Phase::Read
                | Phase::PageTemp
                | Phase::PageRename
                | Phase::PageSync
                | Phase::TrashMove
                | Phase::TrashSync
        );
        let draft = matches!(
            phase,
            Phase::DraftTemp
                | Phase::DraftRename
                | Phase::DraftUnlink
                | Phase::DraftSync
                | Phase::Quarantine
        );
        if self.down && draft {
            return Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: None,
                os_error: None,
            });
        }
        let fault = self.faults.get_mut(&phase).and_then(VecDeque::pop_front);
        let result = match fault {
            Some(Fault::Errno(kind, at)) => {
                if let Some(calls) = at {
                    self.budget = Some(calls);
                    self.budget_kind = Some(kind);
                    let result = effect(self);
                    assert!(
                        self.budget.take().is_none(),
                        "errno fault did not land: {phase:?}/{calls}"
                    );
                    result
                } else {
                    self.observed_errno = Some(kind);
                    Err(IoFailure {
                        kind: ErrorKind::Io,
                        completed: false,
                        operation: None,
                        os_error: None,
                    })
                }
            }
            Some(Fault::RootGone(calls)) => {
                self.root_fault = Some((calls, RootState::Gone));
                let result = effect(self);
                assert!(
                    self.root_fault.take().is_none(),
                    "root fault did not land: {phase:?}/{calls}"
                );
                result
            }
            Some(Fault::RootReplaced(calls)) => {
                self.root_fault = Some((calls, RootState::Replaced(1)));
                let result = effect(self);
                assert!(
                    self.root_fault.take().is_none(),
                    "root fault did not land: {phase:?}/{calls}"
                );
                result
            }
            Some(Fault::Before) => Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: None,
                os_error: None,
            }),
            Some(Fault::Collision) => Err(IoFailure {
                kind: ErrorKind::Collision,
                completed: false,
                operation: None,
                os_error: None,
            }),
            Some(Fault::Cut(calls)) => {
                self.budget = Some(calls);
                let result = effect(self);
                let landed = self.budget.take().is_none();
                assert!(landed, "{phase:?} makes at most {calls} system calls here");
                result
            }
            other => effect(self).and_then(|result| {
                if matches!(other, Some(Fault::After)) {
                    Err(IoFailure {
                        kind: ErrorKind::Io,
                        completed: true,
                        operation: None,
                        os_error: None,
                    })
                } else {
                    Ok(result)
                }
            }),
        };
        assert!(
            self.env_fault.is_none(),
            "environment fault did not land: {phase:?}/{:?}",
            self.env_fault
        );
        result
    }

    /// One system call of the current HostIo call; fails once a cut's budget
    /// is spent.
    fn call(&mut self) -> IoResult<()> {
        self.root_step()?;
        match &mut self.budget {
            Some(0) => {
                self.budget = None;
                self.observed_errno =
                    Some(self.budget_kind.take().unwrap_or(std::io::ErrorKind::Other));
                Err(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                    operation: None,
                    os_error: None,
                })
            }
            Some(left) => {
                *left -= 1;
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Root faults can land between modeled syscalls. Existing graph entries
    /// belong to the detached epoch; only app-data drafts remain at this root.
    fn root_step(&mut self) -> IoResult<()> {
        match self.env_fault.take() {
            Some((0, step)) => self.apply_env(step),
            Some((left, step)) => self.env_fault = Some((left - 1, step)),
            None => {}
        }
        match self.root_fault {
            Some((0, next)) => {
                self.root_fault = None;
                self.apply_env(match next {
                    RootState::Gone => EnvStep::RootGone,
                    RootState::Replaced(_) => EnvStep::RootReplaced,
                    RootState::Bound => unreachable!(),
                });
            }
            Some((left, next)) => self.root_fault = Some((left - 1, next)),
            None => {}
        }
        if self.graph_io && self.env.root != RootState::Bound {
            Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: Some("GraphRootMoved"),
                os_error: None,
            })
        } else {
            Ok(())
        }
    }

    /// A directory sync covers its direct entries, never a child directory's.
    fn sync_dir(&mut self, dir: &str) {
        self.stable.retain(|key, _| !child(key, dir));
        self.stable.extend(
            self.files
                .iter()
                .filter(|(key, _)| child(key, dir))
                .map(|(key, bytes)| (key.clone(), bytes.clone())),
        );
    }

    /// A graph directory sync: durable where the witness is.
    fn sync_entry(&mut self, key: &str) {
        if !self.weak_graph && (self.files.contains_key(key) || self.stable.contains_key(key)) {
            self.record_mutation(key);
            set(&mut self.stable, key, self.files.get(key).cloned());
        }
    }

    /// The physical publication, separated from its adapter check so the
    /// escape oracle can be tested against an omitted-check mutant.
    pub fn publish_page(&mut self, page: &str) {
        let key = format!("graph/{page}");
        let temp = format!("temp/page/{page}");
        self.record_mutation(&temp);
        let bytes = self.files.remove(&temp);
        self.publications.insert(page.into(), bytes.clone());
        self.publication_epochs
            .insert(page.into(), self.epochs.get(page).copied().unwrap_or(0));
        self.record_mutation(&key);
        set(&mut self.files, &key, bytes);
        self.volatile.remove(&key);
        self.issue(page, None);
    }

    fn directory_identity(&self, entry: &str) -> Vec<(String, u64)> {
        let mut path = entry.strip_prefix("dir/").unwrap();
        let mut ids = Vec::new();
        // Production retains the entry and its immediate parent, not a
        // stronger identity guard on every ancestor (the root checks itself).
        for _ in 0..2 {
            if path.is_empty() {
                break;
            }
            if let Some(dir) = self.env.dirs.get(path) {
                ids.push((path.into(), dir.epoch));
            }
            path = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        }
        ids
    }

    fn owe_directory(&mut self, dir: String) {
        self.owed_identities
            .insert(dir.clone(), self.directory_identity(&dir));
        self.owed.push(dir);
    }

    fn check_creation_parent(&mut self, dir: &str) -> IoResult<()> {
        self.check_parent(dir)?;
        // Unlike containment, mkdir needs an existing immediate parent. The
        // model combines the native parent check and mkdir into one step.
        if !self.env.contains(&self.env.parent(dir), true) {
            self.replan_directories = true;
            return Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: None,
                os_error: None,
            });
        }
        Ok(())
    }

    fn retry_directories(&mut self) {
        if !self.replan_directories {
            return;
        }
        self.replan_directories = false;
        let old = std::mem::take(&mut self.owed);
        self.owed_identities.clear();
        let mut entries = BTreeSet::new();
        for dir in old {
            let mut path = dir.strip_prefix("dir/").unwrap();
            while !path.is_empty() {
                if self.env.dirs.contains_key(path) {
                    entries.insert(format!("dir/{path}"));
                }
                path = path.rsplit_once('/').map_or("", |(parent, _)| parent);
            }
        }
        for dir in entries {
            self.files
                .entry(dir.clone())
                .or_insert_with(|| Arc::from(&b""[..]));
            self.owe_directory(dir);
        }
    }

    fn settle_directories(&mut self) -> IoResult<()> {
        while let Some(dir) = self.owed.first().cloned() {
            self.call()?;
            self.check_parent(&dir)?;
            if self.owed_identities.get(&dir) != Some(&self.directory_identity(&dir)) {
                self.replan_directories = true;
                return Err(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                    operation: None,
                    os_error: None,
                });
            }
            self.sync_entry(&dir);
            self.owed.remove(0);
            self.owed_identities.remove(&dir);
        }
        Ok(())
    }
}

/// `key` names a direct entry of directory `dir` (which ends in `/`).
fn child(key: &str, dir: &str) -> bool {
    key.strip_prefix(dir)
        .is_some_and(|rest| !rest.contains('/'))
}

fn set(files: &mut BTreeMap<String, Arc<[u8]>>, key: &str, bytes: Text) {
    if let Some(bytes) = bytes {
        files.insert(key.into(), bytes);
    } else {
        files.remove(key);
    }
}

impl HostIo for ModelFs {
    fn spelling(&self, key: &str) -> String {
        self.spellings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.into())
    }

    fn spell(&mut self, key: &str, spelling: &str) {
        self.spellings.insert(key.into(), spelling.into());
        let parent = spelling.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
        let graph = format!("graph/{key}");
        self.env.parents.insert(graph.clone(), parent.clone());
        if self.files.contains_key(&graph) {
            self.env.seed_dirs(&parent);
        }
    }

    /// The graph launch's best-effort source-directory syncs: every page's
    /// latest namespace operation, and every existing trash chain entry,
    /// becomes durable where the witness is. Trash payload names are left
    /// alone: making them durable here would also claim their data.
    fn graph_launch(&mut self, pages: &BTreeSet<String>) {
        if !self.weak_graph && self.env.root == RootState::Bound {
            self.sourced.extend(pages.iter().cloned());
            for dir in CHAIN {
                if self.check_parent(dir).is_ok() {
                    self.sync_entry(dir);
                }
            }
        }
    }

    fn page_finish(&mut self, page: &str) {
        self.files.remove(&format!("temp/page/{page}"));
        self.publication_epochs.remove(page);
    }

    fn read_page(&mut self, page: &str) -> IoResult<Text> {
        self.run(Phase::Read, |fs| {
            fs.root_step()?;
            fs.check_parent(&format!("graph/{page}"))?;
            Ok(fs.alpha(page, false))
        })
    }

    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()> {
        self.run(Phase::PageTemp, |fs| {
            fs.root_step()?;
            fs.retry_directories();
            let key = format!("temp/page/{page}");
            fs.check_parent(&key)?;
            let parent = fs.env.parent(&key);
            // Same mkdir + retained parent-sync obligations as the trash
            // chain, now exercised by absent nested-page fixtures too.
            let mut current = String::new();
            for component in parent.split('/').filter(|c| !c.is_empty()) {
                if !current.is_empty() {
                    current.push('/');
                }
                current.push_str(component);
                if fs.env.dirs.contains_key(&current) {
                    continue;
                }
                fs.call()?;
                let dir = format!("dir/{current}");
                fs.check_creation_parent(&dir)?;
                if !fs.env.contains(&current, false) {
                    return Err(IoFailure {
                        kind: ErrorKind::Io,
                        completed: false,
                        operation: Some("WriteTargetEscapes"),
                        os_error: None,
                    });
                }
                fs.record_mutation(&dir);
                fs.env.dirs.insert(
                    current.clone(),
                    Dir {
                        epoch: 0,
                        kind: DirKind::Real,
                    },
                );
                fs.files.insert(dir.clone(), Arc::from(&b""[..]));
                fs.owe_directory(dir);
            }
            fs.settle_directories()?;
            if !parent.is_empty() {
                fs.call()?;
                fs.check_parent(&key)?;
            }
            fs.record_mutation(&key);
            set(&mut fs.files, &key, bytes.clone());
            Ok(())
        })
    }

    fn page_rename(&mut self, page: &str) -> IoResult<()> {
        self.run(Phase::PageRename, |fs| {
            fs.root_step()?;
            fs.check_parent(&format!("graph/{page}"))?;
            // A directory replacement can remove a prepared temp's epoch.
            // Native PreparedWrite cannot publish a missing temporary either.
            if !fs.files.contains_key(&format!("temp/page/{page}")) {
                return Err(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                    operation: None,
                    os_error: None,
                });
            }
            fs.publish_page(page);
            Ok(())
        })
    }

    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        self.run(Phase::PageSync, |fs| {
            fs.root_step()?;
            fs.check_parent(&format!("graph/{page}"))?;
            // Missing directories are I/O failure, never Unsupported.
            if !fs
                .env
                .contains(&fs.env.parent(&format!("graph/{page}")), true)
            {
                return Err(IoFailure {
                    kind: ErrorKind::Io,
                    completed: true,
                    operation: None,
                    os_error: None,
                });
            }
            let key = format!("graph/{page}");
            let current = fs.files.get(&key).cloned();
            if let Some(epoch) = fs.publication_epochs.get(page) {
                if fs.publications.get(page) != Some(&current)
                    || *epoch != fs.epochs.get(page).copied().unwrap_or(0)
                {
                    return Err(IoFailure {
                        kind: ErrorKind::Io,
                        completed: true,
                        operation: None,
                        os_error: None,
                    });
                }
            }
            if fs.weak_graph {
                return Ok(Witness::Unsupported);
            }
            // Model dirSync L421: stable advances only when the path still
            // contains the job's synced payload. External durable writes have
            // already advanced stable in external().
            if fs.publications.get(page) == Some(&current) {
                fs.record_mutation(&key);
                set(&mut fs.stable, &key, current);
            }
            Ok(Witness::Durable)
        })
    }

    fn page_twin(&mut self, page: &str) -> IoResult<Option<String>> {
        let twin = if let Some(stem) = page.strip_suffix(".md") {
            format!("{stem}.org")
        } else if let Some(stem) = page.strip_suffix(".org") {
            format!("{stem}.md")
        } else {
            return Ok(None);
        };
        Ok(self
            .files
            .contains_key(&format!("graph/{twin}"))
            .then_some(twin))
    }

    fn page_owner(
        &mut self,
        page: &str,
        _bytes: &Text,
        _source: Option<&str>,
    ) -> IoResult<Option<String>> {
        Ok(self.owners.get(page).cloned())
    }

    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult {
        let removed = self.files.get(&format!("graph/{page}")).cloned();
        let result = self.run(Phase::TrashMove, |fs| {
            fs.root_step()?;
            fs.retry_directories();
            let source = format!("graph/{page}");
            fs.check_parent(&source)?;
            fs.check_parent(&format!("trash/{page}/{payload}"))?;
            if !fs.files.contains_key(&source) {
                fs.publications.insert(page.into(), None);
                return Ok(());
            }
            // DirectoryCreation: mkdir each missing chain entry, then sync
            // each parent; the owed syncs are retained across a failure.
            for dir in CHAIN {
                if fs.files.contains_key(dir) {
                    continue;
                }
                fs.call()?;
                fs.check_creation_parent(dir)?;
                fs.record_mutation(dir);
                fs.env.dirs.insert(
                    dir.strip_prefix("dir/").unwrap().into(),
                    Dir {
                        epoch: 0,
                        kind: DirKind::Real,
                    },
                );
                fs.files.insert(dir.into(), Arc::from(&b""[..]));
                fs.order.insert(dir.into(), fs.next_order());
                fs.owe_directory(dir.into());
            }
            fs.settle_directories()?;
            fs.call()?;
            let key = format!("trash/{page}/{payload}");
            fs.check_parent(&source)?;
            fs.check_parent(&key)?;
            if fs.files.contains_key(&key) {
                return Err(IoFailure {
                    kind: ErrorKind::Collision,
                    completed: false,
                    operation: None,
                    os_error: None,
                });
            }
            fs.record_mutation(&source);
            fs.record_mutation(&key);
            let bytes = fs.files.remove(&source);
            fs.publications.insert(page.into(), None);
            set(&mut fs.files, &key, bytes);
            if fs.volatile.remove(&source) {
                fs.volatile.insert(key.clone());
            }
            fs.issue(page, Some(&key));
            Ok(())
        });
        let completed = result.is_ok() || result.as_ref().is_err_and(|e| e.completed);
        MoveResult {
            removed: if completed { removed } else { None },
            result,
        }
    }

    fn trash_sync(&mut self, page: &str, payload: &str) -> IoResult<Witness> {
        self.run(Phase::TrashSync, |fs| {
            fs.root_step()?;
            let key = format!("trash/{page}/{payload}");
            fs.check_parent(&key)?;
            if !fs.files.contains_key(&key) {
                return Ok(Witness::Durable); // R-PURGE: nothing left to keep
            }
            // RO3 (a) (Martin 2026-10-11): no payload data sync. Tine's own
            // saves already synced the bytes; another program's unflushed
            // bytes stay volatile (R-UNFLUSHED). `pages/` (the payload's
            // name), then each ancestor's sync makes its child chain entry
            // durable, up to the graph root.
            fs.call()?;
            fs.check_parent(&key)?;
            fs.sync_entry(&key);
            if !fs.chain_witnessed {
                for dir in CHAIN.iter().rev() {
                    fs.call()?;
                    fs.check_parent(dir)?;
                    fs.sync_entry(dir);
                }
                fs.chain_witnessed = !fs.weak_graph;
            }
            Ok(if fs.weak_graph {
                Witness::Unsupported
            } else {
                Witness::Durable
            })
        })
    }

    /// The audited new-file write: temp (written and file-synced), no-replace
    /// rename, then a sync of the custody directory.
    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.run(Phase::CustodyWrite, |fs| {
            // The production temp name, composed by the audited helper.
            let temp = crate::atomic_file::temp_path(std::path::Path::new(name), 0, "");
            let temp = format!("{CUSTODY}{}", temp.to_string_lossy());
            fs.call()?;
            fs.files.insert(temp.clone(), Arc::from(bytes));
            fs.call()?;
            fs.files.remove(&temp);
            fs.files
                .insert(format!("{CUSTODY}{name}"), Arc::from(bytes));
            fs.call()?;
            fs.sync_dir(CUSTODY);
            Ok(())
        })
    }

    /// Unlink, then sync the custody directory.
    fn custody_retire(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::CustodyRetire, |fs| {
            fs.call()?;
            fs.files.remove(&format!("{CUSTODY}{name}"));
            fs.call()?;
            fs.sync_dir(CUSTODY);
            Ok(())
        })
    }

    fn custody_markers(&mut self) -> Result<Vec<(String, Vec<u8>)>, String> {
        self.run(Phase::CustodyList, |fs| {
            fs.root_step()?;
            let temps: Vec<_> = fs
                .files
                .keys()
                .filter(|key| key.starts_with(CUSTODY) && key.ends_with(".tmp"))
                .cloned()
                .collect();
            if !temps.is_empty() {
                // Unlink each unpublished temp, then sync the custody directory.
                for key in temps {
                    fs.files.remove(&key);
                }
                fs.sync_dir(CUSTODY);
            }
            Ok(fs
                .files
                .iter()
                .filter_map(|(key, bytes)| {
                    key.strip_prefix(CUSTODY)
                        .map(|name| (name.into(), bytes.to_vec()))
                })
                .collect())
        })
        .map_err(|_| "trash-custody: injected listing failure".into())
    }

    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)> {
        let files = if durable { &self.stable } else { &self.files };
        files
            .iter()
            .filter_map(|(key, bytes)| {
                key.strip_prefix("draft/")
                    .filter(|name| !name.contains('/'))
                    .map(|name| (name.into(), bytes.to_vec()))
            })
            .collect()
    }

    fn draft_changes(&mut self) -> Vec<(String, Option<Vec<u8>>)> {
        let current: BTreeMap<String, Arc<[u8]>> = self
            .stable
            .iter()
            .filter_map(|(key, bytes)| {
                key.strip_prefix("draft/")
                    .filter(|name| !name.contains('/'))
                    .map(|name| (name.to_string(), bytes.clone()))
            })
            .collect();
        let mut changes = vec![];
        for name in current.keys().chain(self.reported.keys()) {
            let (now, before) = (current.get(name), self.reported.get(name));
            if now != before && !changes.iter().any(|(n, _)| n == name) {
                changes.push((name.clone(), now.map(|b| b.to_vec())));
            }
        }
        self.reported = current;
        changes
    }

    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.run(Phase::DraftTemp, |fs| {
            fs.root_step()?;
            fs.files
                .insert(format!("temp/draft/{name}"), Arc::from(bytes));
            Ok(())
        })
    }

    fn draft_rename(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::DraftRename, |fs| {
            fs.root_step()?;
            let key = format!("draft/{name}");
            if fs.files.contains_key(&key) {
                return Err(IoFailure {
                    kind: ErrorKind::Collision,
                    completed: false,
                    operation: None,
                    os_error: None,
                });
            }
            let bytes = fs
                .files
                .remove(&format!("temp/draft/{name}"))
                .ok_or(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                    operation: None,
                    os_error: None,
                })?;
            fs.files.insert(key, bytes);
            Ok(())
        })
    }

    fn draft_unlink(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::DraftUnlink, |fs| {
            fs.root_step()?;
            // Missing is readable absence only. The worker still must sync.
            fs.files.remove(&format!("draft/{name}"));
            Ok(())
        })
    }

    fn draft_sync(&mut self) -> IoResult<Witness> {
        self.run(Phase::DraftSync, |fs| {
            fs.root_step()?;
            fs.sync_dir("draft/");
            Ok(Witness::Durable)
        })
    }

    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::Quarantine, |fs| {
            fs.root_step()?;
            let source = format!("draft/{name}");
            if let Some(bytes) = fs.files.remove(&source) {
                fs.files.insert(format!("unreadable/{name}"), bytes);
            }
            fs.sync_dir("unreadable/");
            fs.sync_dir(&source[..=source.rfind('/').unwrap()]);
            Ok(())
        })
    }

    fn draft_status(&self) -> DraftStatus {
        DraftStatus {
            unavailable: self.down.then(|| "drafts unsynced".into()),
            unreadable: vec![],
            unsaved: vec![],
        }
    }

    fn drafts_unsynced(&mut self) {
        self.down = true;
    }

    fn drafts_reprobe(&mut self) -> Result<(), String> {
        self.down = false;
        self.down = !matches!(self.draft_sync(), Ok(Witness::Durable));
        self.draft_status().unavailable.map_or(Ok(()), Err)
    }
}
