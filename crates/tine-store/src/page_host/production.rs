//! Unwired production phase adapter over the audited publication primitives.
//!
//! The two physical censuses retain observed bytes and successful metadata
//! witnesses, not logical drafts. Only drafts::scan selects logical records.
//! Reconstruct this adapter after a process crash to enumerate actual survivors.
use super::io::{ErrorKind, HostIo, IoFailure, IoResult, MoveResult, Witness};
use super::Text;
use crate::atomic_file::PreparedWrite;
use crate::directory_durability::{self as durability, DirectoryWitness};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) struct ProductionIo {
    graph: PathBuf,
    drafts: PathBuf,
    trash: PathBuf,
    page_temps: BTreeMap<String, PreparedWrite>,
    draft_temps: BTreeMap<String, PreparedWrite>,
    draft_payloads: BTreeMap<String, Vec<u8>>,
    creates: BTreeMap<String, bool>,
    readable: BTreeMap<String, Vec<u8>>,
    durable: BTreeMap<String, Vec<u8>>,
    paths: BTreeMap<String, PathBuf>,
    quarantines: BTreeMap<String, (PathBuf, u8)>,
    directories: BTreeMap<PathBuf, durability::DirectoryCreation>,
    pub(super) launch_warnings: Vec<(PathBuf, io::ErrorKind)>,
    #[cfg(test)]
    pub faults: BTreeMap<super::io::Phase, std::collections::VecDeque<io::ErrorKind>>,
}

impl ProductionIo {
    /// `trash` is the existing typed transaction-trash directory chosen by the
    /// store (pages/journals/conflicts). Step 3 supplies that classification.
    /// Graph/page keys and graph ID are already resolved by the store boundary.
    pub(super) fn new(
        graph: &Path,
        app_data: &Path,
        graph_id: &str,
        trash: &Path,
    ) -> io::Result<Self> {
        let drafts = app_data.join("drafts-v2").join(graph_id);
        let custody = drafts.join(CUSTODY);
        durability::create_dir_all_with_sync(&custody, durability::sync_private_directory)?;
        // Also cover a directory chain left readable by an interrupted earlier
        // creation attempt. Constructor failure is retryable, never weak success.
        for dir in custody
            .ancestors()
            .filter(|dir| !dir.as_os_str().is_empty())
        {
            durability::sync_private_directory(dir)?;
        }
        let mut readable = BTreeMap::new();
        let mut paths = BTreeMap::new();
        for entry in fs::read_dir(&drafts)? {
            let entry = entry?;
            if ((entry.file_name() == "unreadable" || entry.file_name() == CUSTODY)
                && entry.file_type()?.is_dir())
                || entry.file_name().to_string_lossy().ends_with(".tmp")
            {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .unwrap_or_else(|_| format!("invalid-name-{}", uuid::Uuid::new_v4().simple()));
            // An unreadable file goes through the same preserving quarantine
            // path as a checksum/format failure. Enumeration errors propagate;
            // they must never masquerade as an empty store.
            let bytes = fs::read(entry.path()).unwrap_or_default();
            readable.insert(name.clone(), bytes);
            paths.insert(name, entry.path());
        }
        Ok(Self {
            graph: graph.to_path_buf(),
            drafts,
            trash: trash.to_path_buf(),
            page_temps: BTreeMap::new(),
            draft_temps: BTreeMap::new(),
            creates: BTreeMap::new(),
            draft_payloads: BTreeMap::new(),
            durable: readable.clone(),
            readable,
            paths,
            quarantines: BTreeMap::new(),
            directories: BTreeMap::new(),
            launch_warnings: vec![],
            #[cfg(test)]
            faults: BTreeMap::new(),
        })
    }

    fn before(&mut self, phase: super::io::Phase) -> IoResult<()> {
        #[cfg(test)]
        if let Some(kind) = self
            .faults
            .get_mut(&phase)
            .and_then(|queue| queue.pop_front())
        {
            return Err(failure(io::Error::new(kind, "injected phase failure")));
        }
        let _ = phase;
        Ok(())
    }

    fn graph_sync(&mut self, phase: super::io::Phase, dir: &Path) -> IoResult<Witness> {
        self.before(phase)?;
        durability::sync_directory_witness(dir)
            .map(witness)
            .map_err(sync_failure)
    }

    fn draft_path(&self, name: &str) -> PathBuf {
        self.paths
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.drafts.join(name))
    }

    fn graph_directory(&mut self, dir: &Path) -> IoResult<()> {
        self.directories
            .entry(dir.to_path_buf())
            .or_insert_with(|| durability::DirectoryCreation::new(dir))
            .finish(durability::sync_directory_entry)
            .map_err(failure)?;
        self.directories.remove(dir);
        Ok(())
    }
}

/// A4 custody markers: only this device's unfinished deletions (D-10).
const CUSTODY: &str = "trash-custody";

/// Unlink; an already absent file is the requested state.
fn remove_present(path: &Path) -> IoResult<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(failure(error)),
        _ => Ok(()),
    }
}

fn witness(value: DirectoryWitness) -> Witness {
    match value {
        DirectoryWitness::Durable => Witness::Durable,
        DirectoryWitness::Unsupported => Witness::Unsupported,
    }
}

fn failure(error: io::Error) -> IoFailure {
    IoFailure {
        kind: if error.kind() == io::ErrorKind::AlreadyExists {
            ErrorKind::Collision
        } else {
            ErrorKind::Io
        },
        completed: false,
    }
}

fn sync_failure(error: io::Error) -> IoFailure {
    let completed = durability::is_directory_sync_failure(&error);
    IoFailure {
        completed,
        ..failure(error)
    }
}

impl HostIo for ProductionIo {
    fn graph_launch(&mut self, pages: &std::collections::BTreeSet<String>) {
        // Every existing ancestor up to the graph root: an interrupted nested
        // page-directory creation leaves entries no later leaf sync covers.
        let mut directories = std::collections::BTreeSet::from([self.graph.clone()]);
        for page in pages {
            let path = self.graph.join(page);
            directories.extend(
                path.ancestors()
                    .skip(1)
                    .take_while(|dir| dir.starts_with(&self.graph))
                    .map(Path::to_path_buf),
            );
        }
        self.launch_warnings.clear();
        for directory in directories {
            if let Err(error) = durability::sync_directory_witness(&directory) {
                self.launch_warnings.push((directory, error.kind()));
            }
        }
    }

    fn page_finish(&mut self, page: &str) {
        self.page_temps.remove(page);
        self.creates.remove(page);
    }

    fn read_page(&mut self, page: &str) -> IoResult<Text> {
        self.before(super::io::Phase::Read)?;
        match fs::read(self.graph.join(page)) {
            Ok(bytes) => {
                self.creates.insert(page.into(), false);
                Ok(Some(bytes.into()))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.creates.insert(page.into(), true);
                Ok(None)
            }
            Err(error) => Err(failure(error)),
        }
    }

    fn page_temp(&mut self, page: &str, bytes: &Text) -> IoResult<()> {
        self.before(super::io::Phase::PageTemp)?;
        let target = self.graph.join(page);
        self.graph_directory(target.parent().unwrap())?;
        let prepared = PreparedWrite::new(
            &target,
            bytes.as_deref().ok_or(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
            })?,
        )
        .map_err(failure)?;
        self.page_temps.insert(page.into(), prepared);
        Ok(())
    }

    fn page_rename(&mut self, page: &str) -> IoResult<()> {
        self.before(super::io::Phase::PageRename)?;
        let prepared = self.page_temps.remove(page).ok_or(IoFailure {
            kind: ErrorKind::Io,
            completed: false,
        })?;
        prepared
            .publish(self.creates.get(page).copied().unwrap_or(false))
            .map_err(failure)
    }

    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        let path = self.graph.join(page);
        self.graph_sync(super::io::Phase::PageSync, path.parent().unwrap())
    }

    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult {
        let result = (|| {
            self.before(super::io::Phase::TrashMove)?;
            let source = self.graph.join(page);
            if !source.try_exists().map_err(failure)? {
                return Ok(None);
            }
            self.graph_directory(&self.trash.clone())?;
            let target = self.trash.join(payload);
            crate::no_replace::move_file_noreplace(&source, &target).map_err(failure)?;
            // Read the actual moved bytes, including an R1 external replacement
            // between the guard and move; never claim the guard's stale bytes.
            fs::read(target)
                .map(|bytes| Some(bytes.into()))
                .map_err(|_| IoFailure {
                    kind: ErrorKind::Io,
                    completed: true,
                })
        })();
        match result {
            Ok(removed) => MoveResult {
                removed,
                result: Ok(()),
            },
            Err(error) => MoveResult {
                removed: None,
                result: Err(error),
            },
        }
    }

    fn trash_sync(&mut self, _page: &str, payload: &str) -> IoResult<Witness> {
        self.before(super::io::Phase::TrashSync)?;
        // The recorded basename locates the payload; the trash is never listed.
        match crate::atomic_file::sync_file_bytes(&self.trash.join(payload)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Witness::Durable),
            Err(error) => return Err(failure(error)),
        }
        self.trash
            .ancestors()
            .take_while(|dir| dir.starts_with(&self.graph))
            .try_fold(Witness::Durable, |result, dir| {
                let synced = durability::sync_directory_witness(dir).map_err(sync_failure)?;
                let synced = witness(synced);
                Ok(if synced == Witness::Unsupported {
                    synced
                } else {
                    result
                })
            })
    }

    fn custody_write(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.before(super::io::Phase::CustodyWrite)?;
        let dir = self.drafts.join(CUSTODY);
        PreparedWrite::new(&dir.join(name), bytes)
            .and_then(|prepared| prepared.publish(true))
            .and_then(|()| durability::sync_private_directory(&dir))
            .map_err(failure)
    }

    fn custody_retire(&mut self, name: &str) -> IoResult<()> {
        self.before(super::io::Phase::CustodyRetire)?;
        let dir = self.drafts.join(CUSTODY);
        remove_present(&dir.join(name))?;
        durability::sync_private_directory(&dir).map_err(failure)
    }

    fn custody_markers(&mut self) -> IoResult<Vec<(String, Vec<u8>)>> {
        let mut markers = vec![];
        for entry in fs::read_dir(self.drafts.join(CUSTODY)).map_err(failure)? {
            let entry = entry.map_err(failure)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".tmp") {
                // Unreadable means malformed: quarantined, never acted on.
                markers.push((name, fs::read(entry.path()).unwrap_or_default()));
            }
        }
        Ok(markers)
    }

    fn draft_files(&self, durable: bool) -> Vec<(String, Vec<u8>)> {
        let census = if durable {
            &self.durable
        } else {
            &self.readable
        };
        census
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect()
    }

    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.before(super::io::Phase::DraftTemp)?;
        self.draft_temps.insert(
            name.into(),
            PreparedWrite::new(&self.drafts.join(name), bytes).map_err(failure)?,
        );
        self.draft_payloads.insert(name.into(), bytes.to_vec());
        Ok(())
    }

    fn draft_rename(&mut self, name: &str) -> IoResult<()> {
        if let Err(error) = self.before(super::io::Phase::DraftRename) {
            self.draft_temps.remove(name);
            self.draft_payloads.remove(name);
            return Err(error);
        }
        let prepared = self.draft_temps.remove(name).ok_or(IoFailure {
            kind: ErrorKind::Io,
            completed: false,
        })?;
        if let Err(error) = prepared.publish(true) {
            self.draft_payloads.remove(name);
            return Err(failure(error));
        }
        let target = self.drafts.join(name);
        let bytes = self
            .draft_payloads
            .remove(name)
            .expect("prepared draft payload");
        self.paths.insert(name.into(), target);
        self.readable.insert(name.into(), bytes);
        Ok(())
    }

    fn draft_unlink(&mut self, name: &str) -> IoResult<()> {
        self.before(super::io::Phase::DraftUnlink)?;
        self.draft_temps.remove(name);
        self.draft_payloads.remove(name);
        remove_present(&self.draft_path(name))?;
        self.readable.remove(name);
        // NotFound is only readable absence. Vehicle still requires DraftSync.
        Ok(())
    }

    fn draft_sync(&mut self) -> IoResult<Witness> {
        self.before(super::io::Phase::DraftSync)?;
        durability::sync_private_directory(&self.drafts).map_err(sync_failure)?;
        self.durable.clone_from(&self.readable);
        self.paths
            .retain(|name, _| self.readable.contains_key(name));
        Ok(Witness::Durable)
    }

    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        self.before(super::io::Phase::Quarantine)?;
        let unreadable = self.drafts.join("unreadable");
        durability::create_dir_all_with_sync(&unreadable, durability::sync_private_directory)
            .map_err(failure)?;
        if !self.quarantines.contains_key(name) {
            let source = self.draft_path(name);
            loop {
                let target = unreadable.join(crate::atomic_file::prefixed_name(
                    &format!("{}-", uuid::Uuid::new_v4().simple()),
                    &source.file_name().unwrap().to_string_lossy(),
                ));
                match crate::no_replace::move_file_noreplace(&source, &target) {
                    Ok(()) => {
                        self.quarantines.insert(name.into(), (target, 0));
                        break;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(failure(error)),
                }
            }
        }
        let (_, phase) = self.quarantines.get_mut(name).unwrap();
        if *phase == 0 {
            durability::sync_private_directory(&unreadable).map_err(failure)?;
            *phase = 1;
        }
        let source = self.draft_path(name);
        durability::sync_private_directory(source.parent().unwrap()).map_err(failure)?;
        self.readable.remove(name);
        self.durable.remove(name);
        self.paths.remove(name);
        self.quarantines.remove(name);
        Ok(())
    }
}
