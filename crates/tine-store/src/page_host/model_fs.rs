//! Faultable files, with independent readable and durable directory entries.
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
}

impl ModelFs {
    pub fn external(&mut self, page: &str, bytes: Text, durable: bool) {
        let key = format!("graph/{page}");
        set(&mut self.files, &key, bytes.clone());
        if durable {
            set(&mut self.stable, &key, bytes);
        }
        *self.epochs.entry(page.into()).or_default() += 1;
    }

    pub fn inject(&mut self, phase: Phase, faults: impl IntoIterator<Item = Fault>) {
        self.faults.entry(phase).or_default().extend(faults);
    }

    /// Process crashes preserve readable names; only volatile temps disappear.
    pub fn crash(&mut self) {
        self.files.retain(|key, _| !key.starts_with("temp/"));
        self.faults.clear();
    }

    /// Choose which readable graph paths survive, including their trash. Draft
    /// metadata is strong and reverts to its last directory-sync witness.
    pub fn power(&mut self, keep: &BTreeSet<String>, keep_draft_directory: bool) {
        let readable = self.files.clone();
        self.files = self.stable.clone();
        for page in keep {
            let graph = format!("graph/{page}");
            set(&mut self.files, &graph, readable.get(&graph).cloned());
            let prefix = format!("trash/{page}/");
            self.files.retain(|key, _| !key.starts_with(&prefix));
            self.files.extend(
                readable
                    .iter()
                    .filter(|(key, _)| key.starts_with(&prefix))
                    .map(|(key, bytes)| (key.clone(), bytes.clone())),
            );
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
            set(&mut fs.files, &format!("graph/{page}"), bytes);
            Ok(())
        })
    }

    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        if self.weak_graph {
            self.calls.push(Phase::PageSync);
            return Ok(Witness::Unsupported);
        }
        self.run(Phase::PageSync, |fs| {
            let key = format!("graph/{page}");
            set(&mut fs.stable, &key, fs.files.get(&key).cloned());
            Ok(Witness::Durable)
        })
    }

    fn trash_move(&mut self, page: &str, name: &str) -> MoveResult {
        let removed = self.files.get(&format!("graph/{page}")).cloned();
        let result = self.run(Phase::TrashMove, |fs| {
            let key = format!("trash/{page}/{name}");
            if fs.files.contains_key(&key) {
                return Err(IoFailure {
                    kind: ErrorKind::Collision,
                    completed: false,
                });
            }
            let bytes = fs.files.remove(&format!("graph/{page}"));
            set(&mut fs.files, &key, bytes.clone());
            Ok(())
        });
        let completed = result.is_ok() || result.as_ref().is_err_and(|e| e.completed);
        MoveResult {
            removed: if completed { removed } else { None },
            result,
        }
    }

    fn trash_sync(&mut self, page: &str) -> IoResult<Witness> {
        if self.weak_graph {
            self.calls.push(Phase::TrashSync);
            return Ok(Witness::Unsupported);
        }
        self.run(Phase::TrashSync, |fs| {
            fs.sync_prefix(&format!("trash/{page}/"));
            Ok(Witness::Durable)
        })
    }

    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)> {
        let files = if durable { &self.stable } else { &self.files };
        files
            .iter()
            .filter_map(|(key, bytes)| {
                key.strip_prefix("draft/")
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
