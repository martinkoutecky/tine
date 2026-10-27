//! Test-side path resolution beside ModelFs's byte map. No directories enter
//! Quint: losing a containing directory projects to external ABSENT writes.
use super::*;

pub(crate) type DirKey = String;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RootState {
    #[default]
    Bound,
    Gone,
    Replaced(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirKind {
    Real,
    Symlink { out_of_root: bool },
}

#[derive(Clone, Debug)]
pub(crate) struct Dir {
    pub epoch: u64,
    pub kind: DirKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EnvStep {
    RootGone,
    RootReplaced,
    SymlinkSwap(DirKey),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Env {
    pub root: RootState,
    pub dirs: BTreeMap<DirKey, Dir>,
    /// Physical parent for each graph/page-temp/trash/directory key. Root is
    /// ""; absence from this map preserves the old flat fixtures.
    pub parents: BTreeMap<String, DirKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Escape {
    pub phase: Option<Phase>,
    pub key: String,
    pub parent: DirKey,
    pub root: RootState,
}

impl Env {
    pub fn parent(&self, key: &str) -> DirKey {
        self.parents.get(key).cloned().unwrap_or_else(|| {
            if let Some(dir) = key.strip_prefix("dir/") {
                dir.rsplit_once('/').map_or("", |(p, _)| p).into()
            } else if key.starts_with("trash/") {
                "logseq/.tine-trash/pages".into()
            } else if let Some(page) = key.strip_prefix("temp/page/") {
                self.parent(&format!("graph/{page}"))
            } else if let Some(page) = key.strip_prefix("graph/") {
                page.rsplit_once('/').map_or("", |(p, _)| p).into()
            } else {
                String::new()
            }
        })
    }

    /// Missing descendants are acceptable to a containment check, but not to
    /// α or a syscall that needs its parent. Root/ancestors are never created.
    pub fn contains(&self, dir: &str, require_present: bool) -> bool {
        if self.root != RootState::Bound
            || dir.starts_with('/')
            || dir
                .split('/')
                .any(|component| component == ".." || component == ".")
        {
            return false;
        }
        let mut current = dir;
        while !current.is_empty() {
            match self.dirs.get(current) {
                Some(Dir {
                    kind: DirKind::Real,
                    ..
                }) => {}
                Some(Dir {
                    kind: DirKind::Symlink { out_of_root: true },
                    ..
                }) => return false,
                // M2 schedules only out-of-root links. An in-root link has
                // no target identity in this environment and is excluded too.
                Some(_) => return false,
                None if require_present => return false,
                None => {}
            }
            current = current.rsplit_once('/').map_or("", |(p, _)| p);
        }
        true
    }

    pub fn seed_dirs(&mut self, dir: &str) {
        let mut current = String::new();
        for component in dir.split('/').filter(|c| !c.is_empty()) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(component);
            self.dirs.entry(current.clone()).or_insert(Dir {
                epoch: 0,
                kind: DirKind::Real,
            });
        }
    }
}

impl ModelFs {
    /// Executable α from design §1. The byte maps are physical evidence;
    /// inaccessible entries stay there, rather than being silently deleted.
    pub fn alpha(&self, page: &str, durable: bool) -> Text {
        let key = format!("graph/{page}");
        self.env
            .contains(&self.env.parent(&key), true)
            .then(|| {
                if durable { &self.stable } else { &self.files }
                    .get(&key)
                    .cloned()
            })
            .flatten()
    }

    pub fn inject_env(&mut self, phase: Phase, k: usize, step: EnvStep) {
        assert!(self.env_schedule.insert(phase, (k, step)).is_none());
    }

    pub fn apply_env(&mut self, step: EnvStep) {
        let pages: Vec<_> = self
            .files
            .keys()
            .chain(self.stable.keys())
            .filter_map(|key| key.strip_prefix("graph/").map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let before: BTreeMap<_, _> = pages
            .iter()
            .map(|page| (page.clone(), self.alpha(page, false)))
            .collect();
        match step {
            EnvStep::RootGone => self.env.root = RootState::Gone,
            EnvStep::RootReplaced => self.env.root = RootState::Replaced(1),
            EnvStep::SymlinkSwap(dir) => {
                assert!(!dir.is_empty(), "use C1 to replace the root");
                let epoch = self.env.dirs.get(&dir).map_or(1, |d| d.epoch + 1);
                self.env.dirs.insert(
                    dir,
                    Dir {
                        epoch,
                        kind: DirKind::Symlink { out_of_root: true },
                    },
                );
            }
        }
        if self.env.root != RootState::Bound {
            let keys: Vec<_> = self
                .files
                .keys()
                .filter(|key| graph_key(key))
                .cloned()
                .collect();
            for key in keys {
                self.detached
                    .insert(key.clone(), self.files.remove(&key).unwrap());
            }
            self.stable.retain(|key, _| !graph_key(key));
        }
        for page in pages {
            if before[&page] != self.alpha(&page, false) {
                *self.epochs.entry(page).or_default() += 1;
            }
        }
    }

    /// Observe a mutation independently of guard success. This deliberately
    /// does not refuse it: a missing adapter check must leave escape evidence.
    pub fn record_mutation(&mut self, key: &str) {
        if graph_key(key) {
            let parent = self.env.parent(key);
            if key == "dir/" || !self.env.contains(&parent, true) {
                self.escapes.push(Escape {
                    phase: self.calls.last().copied(),
                    key: key.into(),
                    parent,
                    root: self.env.root,
                });
            }
        }
    }

    pub fn assert_no_escapes(&self) {
        assert!(
            self.escapes.is_empty(),
            "M2 mutation escaped α: {:?}",
            self.escapes
        );
    }

    pub(super) fn check_parent(&self, key: &str) -> IoResult<()> {
        if self.env.contains(&self.env.parent(key), false) {
            Ok(())
        } else {
            Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: Some(if self.env.root == RootState::Bound {
                    "WriteTargetEscapes"
                } else {
                    "GraphRootMoved"
                }),
                os_error: None,
            })
        }
    }
}

pub(super) fn graph_key(key: &str) -> bool {
    ["graph/", "temp/page/", "trash/", "dir/"]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}
