//! Faultable files, with independent readable and durable directory entries.
//! Trash payload data, trash names and source names persist independently at
//! a power cut, constrained only by single-move atomicity and A4 rule 5.
use super::io::{ErrorKind, HostIo, IoFailure, IoResult, MoveResult, Phase, Witness};
use super::Text;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub(super) enum Fault {
    Before,
    After,
    Unsupported,
    Collision,
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
        let next = self.order.values().max().map_or(1, |n| n + 1);
        self.order.insert(format!("graph/{page}"), next);
        if let Some(trash) = trash {
            self.order.insert(trash.into(), next);
            self.moved.insert(page.into(), trash.into());
        } else {
            self.moved.remove(page);
        }
    }

    /// Readable trash names that a power cut must keep, and those it may drop.
    /// Single-move atomicity: a kept page whose latest operation is its move
    /// keeps the destination. Rule 5 (only where the witness is Unsupported):
    /// a persisted namespace operation implies every earlier one.
    fn trash_outcomes(&self, keep: &BTreeSet<String>) -> (BTreeSet<String>, BTreeSet<String>) {
        let pending: BTreeSet<String> = self
            .files
            .iter()
            .filter(|(key, bytes)| {
                key.starts_with("trash/") && self.stable.get(*key) != Some(bytes)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let mut forced: BTreeSet<String> = keep
            .iter()
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
        if keep_draft_directory {
            self.files.retain(|key, _| !key.starts_with("draft/"));
            self.files.extend(
                readable
                    .into_iter()
                    .filter(|(key, _)| key.starts_with("draft/")),
            );
        }
        self.stable = self.files.clone();
        self.volatile.clear();
        self.order.clear();
        self.moved.clear();
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

    fn sync_prefix(&mut self, prefix: &str) {
        self.stable.retain(|key, _| !key.starts_with(prefix));
        self.stable.extend(
            self.files
                .iter()
                .filter(|(key, _)| key.starts_with(prefix))
                .map(|(key, bytes)| (key.clone(), bytes.clone())),
        );
    }
}

fn set(files: &mut BTreeMap<String, Arc<[u8]>>, key: &str, bytes: Text) {
    if let Some(bytes) = bytes {
        files.insert(key.into(), bytes);
    } else {
        files.remove(key);
    }
}

impl HostIo for ModelFs {
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
            fs.volatile.remove(&key); // (a) is a file sync, real on every filesystem
            if fs.weak_graph && fs.files.contains_key(&key) {
                return Ok(Witness::Unsupported);
            }
            if let Some(bytes) = fs.files.get(&key).cloned() {
                fs.stable.insert(key, bytes);
            }
            Ok(Witness::Durable)
        })
    }

    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.run(Phase::CustodyWrite, |fs| {
            let key = format!("draft/trash-custody/{name}");
            fs.files.insert(key.clone(), Arc::from(bytes));
            fs.stable.insert(key, Arc::from(bytes));
            Ok(())
        })
    }

    fn custody_retire(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::CustodyRetire, |fs| {
            let key = format!("draft/trash-custody/{name}");
            fs.files.remove(&key);
            fs.stable.remove(&key);
            Ok(())
        })
    }

    fn custody_markers(&mut self) -> IoResult<Vec<(String, Vec<u8>)>> {
        Ok(self
            .files
            .iter()
            .filter_map(|(key, bytes)| {
                key.strip_prefix("draft/trash-custody/")
                    .map(|name| (name.into(), bytes.to_vec()))
            })
            .collect())
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
            fs.sync_prefix("draft/");
            Ok(Witness::Durable)
        })
    }

    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        self.run(Phase::Quarantine, |fs| {
            if let Some(bytes) = fs.files.remove(&format!("draft/{name}")) {
                fs.files.insert(format!("unreadable/{name}"), bytes);
            }
            fs.sync_prefix("unreadable/");
            fs.sync_prefix("draft/");
            Ok(())
        })
    }
}
