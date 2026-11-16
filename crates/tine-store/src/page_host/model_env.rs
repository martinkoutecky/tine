//! Test-side path resolution beside ModelFs's byte map. No directories enter
//! Quint: losing a containing directory projects to external ABSENT writes.
use super::*;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

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
    DirRename(DirKey, DirKey),
    InodeChange(DirKey),
    /// An in-place writer has exposed only this prefix when Read runs.
    ExternalInPlace(String, Arc<[u8]>, usize),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Fold {
    #[default]
    None,
    /// Unicode default full case fold plus canonical normalization. A model
    /// folding volume, not an emulation of NTFS's one-character upcase table.
    Case,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Env {
    pub root: RootState,
    pub fold: Fold,
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
    /// Resolve physical spellings independently of logical page registration.
    /// Case models full Unicode case folding and normalization insensitivity.
    /// Production asks the actual directory resolver rather than using this
    /// key: sensitive volumes must keep their distinct physical names.
    pub fn case_alias(&self, page: &str) -> Option<String> {
        self.case_alias_in(page, false)
    }

    fn case_alias_in(&self, page: &str, durable: bool) -> Option<String> {
        if self.env.fold != Fold::Case {
            return None;
        }
        let spelling = self.spelling(page);
        let folded: String = spelling.nfd().case_fold().nfc().collect();
        (if durable { &self.stable } else { &self.files })
            .keys()
            .filter_map(|key| key.strip_prefix("graph/"))
            .find(|key| {
                *key != page
                    && self
                        .spelling(key)
                        .nfd()
                        .case_fold()
                        .nfc()
                        .eq(folded.chars())
            })
            .map(str::to_owned)
    }

    /// Executable α from design §1. The byte maps are physical evidence;
    /// inaccessible entries stay there, rather than being silently deleted.
    pub fn alpha(&self, page: &str, durable: bool) -> Text {
        let page = self
            .case_alias_in(page, durable)
            .unwrap_or_else(|| page.into());
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
            EnvStep::DirRename(from, to) => self.relocate_dir(&from, Some(&to)),
            EnvStep::InodeChange(dir) => self.relocate_dir(&dir, None),
            EnvStep::ExternalInPlace(page, bytes, prefix) => {
                assert!(prefix < bytes.len(), "C7 must expose an incomplete version");
                self.external(&page, Some(Arc::from(&bytes[..prefix])), false);
                // external already advanced the ghost write epoch. C1–C4's
                // namespace projection below must not count this write twice.
                return;
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

    /// Move physical entries, independently of the registered page keys.
    /// Recreating the old name never resurrects the old epoch's bytes. For C3
    /// the destination is readable through its new spelling; C4 keeps the
    /// removed epoch as detached evidence and installs an empty directory.
    fn relocate_dir(&mut self, from: &str, to: Option<&str>) {
        let under = |path: &str| path == from || path.starts_with(&format!("{from}/"));
        let epoch = self.env.dirs.get(from).expect("existing directory").epoch + 1;
        for durable in [false, true] {
            let entries = if durable { &self.stable } else { &self.files };
            let moves: Vec<_> = entries
                .iter()
                .filter(|(key, _)| graph_key(key))
                .filter_map(|(key, bytes)| {
                    let physical = if let Some(page) = key.strip_prefix("graph/") {
                        self.spelling(page)
                    } else if let Some(page) = key.strip_prefix("temp/page/") {
                        self.spelling(page)
                    } else if let Some(dir) = key.strip_prefix("dir/") {
                        dir.into()
                    } else {
                        self.env.parent(key)
                    };
                    under(&physical).then(|| (key.clone(), physical, bytes.clone()))
                })
                .collect();
            for (key, physical, bytes) in moves {
                let entries = if durable {
                    &mut self.stable
                } else {
                    &mut self.files
                };
                entries.remove(&key);
                if let Some(to) = to {
                    let physical = format!("{to}{}", &physical[from.len()..]);
                    let prefix = if key.starts_with("graph/") {
                        "graph/"
                    } else if key.starts_with("dir/") {
                        "dir/"
                    } else {
                        "temp/page/"
                    };
                    entries.insert(format!("{prefix}{physical}"), bytes);
                } else if !durable {
                    self.detached.insert(key, bytes);
                }
            }
        }
        let dirs: Vec<_> = self
            .env
            .dirs
            .keys()
            .filter(|dir| under(dir))
            .cloned()
            .collect();
        for dir in dirs {
            let mut entry = self.env.dirs.remove(&dir).unwrap();
            entry.epoch += 1;
            if let Some(to) = to {
                self.env
                    .dirs
                    .insert(format!("{to}{}", &dir[from.len()..]), entry);
            }
        }
        if to.is_none() {
            self.env.dirs.insert(
                from.into(),
                Dir {
                    epoch,
                    kind: DirKind::Real,
                },
            );
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
