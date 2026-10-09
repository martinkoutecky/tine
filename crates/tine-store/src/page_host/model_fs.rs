//! Faultable files, with independent readable and durable directory entries.
//! Trash payload data, trash names, the trash directory chain and source names
//! persist independently at a power cut, constrained only by single-move
//! atomicity and A4 rule 5. Each HostIo call is a sequence of system calls, and
//! `Fault::Cut` lands between any two of them (REVIEW-2b-r2 R1).
use super::io::{ErrorKind, HostIo, IoFailure, IoResult, MoveResult, Phase, Witness};
use super::Text;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

const CUSTODY: &str = "draft/trash-custody/";

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
    Unsupported,
    Collision,
    /// The call's first `k` system calls complete and the next one fails, as
    /// an error there or a process or power cut between the two.
    Cut(usize),
}

#[derive(Clone, Debug, Default)]
pub(super) struct ModelFs {
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
    /// System calls left before an injected `Fault::Cut` fails the next one.
    pub budget: Option<usize>,
}

/// One power outcome for readable trash names that are not yet durable.
#[derive(Clone, Debug, Default)]
pub(super) struct Cut {
    /// Unforced names that survive anyway (a surplus copy).
    pub survive: BTreeSet<String>,
    /// Surviving names whose unflushed external data is lost (R-UNFLUSHED).
    pub lose: BTreeSet<String>,
}

impl ModelFs {
    pub fn external(&mut self, page: &str, bytes: Text, durable: bool) {
        let key = format!("graph/{page}");
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
            let unflushed: Vec<_> = forced
                .iter()
                .chain(&survive)
                .filter(|key| self.volatile.contains(*key))
                .cloned()
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
        self.publications.clear();
        self.owed.clear();
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
        let fault = self.faults.get_mut(&phase).and_then(VecDeque::pop_front);
        match fault {
            Some(Fault::Before | Fault::Unsupported) => Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
            }),
            Some(Fault::Collision) => Err(IoFailure {
                kind: ErrorKind::Collision,
                completed: false,
            }),
            Some(Fault::Cut(calls)) => {
                self.budget = Some(calls);
                let result = effect(self);
                let landed = self.budget.take().is_none();
                assert!(landed, "{phase:?} makes at most {calls} system calls here");
                result
            }
            other => {
                let result = effect(self)?;
                if matches!(other, Some(Fault::After)) {
                    Err(IoFailure {
                        kind: ErrorKind::Io,
                        completed: true,
                    })
                } else {
                    Ok(result)
                }
            }
        }
    }

    /// One system call of the current HostIo call; fails once a cut's budget
    /// is spent.
    fn call(&mut self) -> IoResult<()> {
        match &mut self.budget {
            Some(0) => {
                self.budget = None;
                Err(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                })
            }
            Some(left) => {
                *left -= 1;
                Ok(())
            }
            None => Ok(()),
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
        if !self.weak_graph {
            set(&mut self.stable, key, self.files.get(key).cloned());
        }
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
    /// The graph launch's best-effort source-directory syncs: every page's
    /// latest namespace operation, and every existing trash chain entry,
    /// becomes durable where the witness is. Trash payload names are left
    /// alone: making them durable here would also claim their data.
    fn graph_launch(&mut self, pages: &BTreeSet<String>) {
        if !self.weak_graph {
            self.sourced.extend(pages.iter().cloned());
            for dir in CHAIN {
                self.sync_entry(dir);
            }
        }
    }

    fn page_finish(&mut self, page: &str) {
        self.files.remove(&format!("temp/page/{page}"));
    }

    fn read_page(&mut self, page: &str) -> IoResult<Text> {
        self.run(Phase::Read, |fs| {
            Ok(fs.files.get(&format!("graph/{page}")).cloned())
        })
    }

    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()> {
        self.run(Phase::PageTemp, |fs| {
            set(&mut fs.files, &format!("temp/page/{page}"), bytes.clone());
            Ok(())
        })
    }

    fn page_rename(&mut self, page: &str) -> IoResult<()> {
        self.run(Phase::PageRename, |fs| {
            let bytes = fs.files.remove(&format!("temp/page/{page}"));
            fs.publications.insert(page.into(), bytes.clone());
            set(&mut fs.files, &format!("graph/{page}"), bytes);
            fs.volatile.remove(&format!("graph/{page}"));
            fs.issue(page, None);
            Ok(())
        })
    }

    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        self.run(Phase::PageSync, |fs| {
            if fs.weak_graph {
                return Ok(Witness::Unsupported);
            }
            let key = format!("graph/{page}");
            let current = fs.files.get(&key).cloned();
            // Model dirSync L421: stable advances only when the path still
            // contains the job's synced payload. External durable writes have
            // already advanced stable in external().
            if fs.publications.get(page) == Some(&current) {
                set(&mut fs.stable, &key, current);
            }
            Ok(Witness::Durable)
        })
    }

    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult {
        let removed = self.files.get(&format!("graph/{page}")).cloned();
        let result = self.run(Phase::TrashMove, |fs| {
            let source = format!("graph/{page}");
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
                fs.files.insert(dir.into(), Arc::from(&b""[..]));
                fs.order.insert(dir.into(), fs.next_order());
                fs.owed.push(dir.into());
            }
            while let Some(dir) = fs.owed.first().cloned() {
                fs.call()?;
                fs.sync_entry(&dir);
                fs.owed.remove(0);
            }
            fs.call()?;
            let key = format!("trash/{page}/{payload}");
            if fs.files.contains_key(&key) {
                return Err(IoFailure {
                    kind: ErrorKind::Collision,
                    completed: false,
                });
            }
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
            let key = format!("trash/{page}/{payload}");
            if !fs.files.contains_key(&key) {
                return Ok(Witness::Durable); // R-PURGE: nothing left to keep
            }
            fs.call()?;
            fs.volatile.remove(&key); // (a) is a file sync, real on every filesystem
                                      // (b) `pages/` (the payload's name), then each ancestor's sync
                                      // makes its child chain entry durable, up to the graph root.
            fs.call()?;
            fs.sync_entry(&key);
            for dir in CHAIN.iter().rev() {
                fs.call()?;
                fs.sync_entry(dir);
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

    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.run(Phase::DraftTemp, |fs| {
            fs.files
                .insert(format!("temp/draft/{name}"), Arc::from(bytes));
            Ok(())
        })
    }

    fn draft_rename(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::DraftRename, |fs| {
            let key = format!("draft/{name}");
            if fs.files.contains_key(&key) {
                return Err(IoFailure {
                    kind: ErrorKind::Collision,
                    completed: false,
                });
            }
            let bytes = fs
                .files
                .remove(&format!("temp/draft/{name}"))
                .ok_or(IoFailure {
                    kind: ErrorKind::Io,
                    completed: false,
                })?;
            fs.files.insert(key, bytes);
            Ok(())
        })
    }

    fn draft_unlink(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::DraftUnlink, |fs| {
            // Missing is readable absence only. The worker still must sync.
            fs.files.remove(&format!("draft/{name}"));
            Ok(())
        })
    }

    fn draft_sync(&mut self) -> IoResult<Witness> {
        self.run(Phase::DraftSync, |fs| {
            fs.sync_dir("draft/");
            Ok(Witness::Durable)
        })
    }

    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::Quarantine, |fs| {
            let source = format!("draft/{name}");
            if let Some(bytes) = fs.files.remove(&source) {
                fs.files.insert(format!("unreadable/{name}"), bytes);
            }
            fs.sync_dir("unreadable/");
            fs.sync_dir(&source[..=source.rfind('/').unwrap()]);
            Ok(())
        })
    }
}
