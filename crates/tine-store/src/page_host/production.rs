//! Unwired production phase adapter over the audited publication primitives.
//!
//! The two physical censuses retain observed bytes and successful metadata
//! witnesses, not logical drafts. Only drafts::scan selects logical records.
//! Reconstruct this adapter after a process crash to enumerate actual survivors.
use super::io::{DraftStatus, ErrorKind, HostIo, IoFailure, IoResult, MoveResult, Phase, Witness};
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
    /// Names renamed or unlinked since the last directory sync: the only
    /// entries a sync can move into the durable census (D-10).
    unsynced: std::collections::BTreeSet<String>,
    changes: Vec<(String, Option<Vec<u8>>)>,
    paths: BTreeMap<String, PathBuf>,
    /// `HostIo::take_stamp`'s stamps, per page key until taken.
    stamps: BTreeMap<String, crate::watch::Stamp>,
    /// Each page key's current graph-relative spelling (STEP3 §2): a key
    /// resolved through a case alias at registration, or moved by the alias
    /// spelling move (Q4). The one table (B1); the store's held index reads
    /// it while this host runs.
    spellings: std::sync::Arc<crate::model::entry_identity::Spellings>,
    quarantines: BTreeMap<String, (PathBuf, u8)>,
    directories: BTreeMap<PathBuf, durability::DirectoryCreation>,
    pub(super) launch_warnings: Vec<(PathBuf, io::ErrorKind)>,
    /// The graph whose self-write markers a save sets before its file step
    /// (STEP3 §5): the watcher's echo handling for paths it still owns.
    pub(super) marks: Option<std::sync::Arc<crate::model::Graph>>,
    /// Draft I/O is down (plan v3 §4, B-Q1): every draft effect fails
    /// without touching the filesystem until a re-probe succeeds.
    down: Option<Down>,
    /// Unreadable vehicles launch could not quarantine: left in place, and
    /// every later effect on one but Retry's quarantine fails untouched (§4).
    untouchable: std::collections::BTreeSet<String>,
    #[cfg(any(test, feature = "test-faults"))]
    pub faults: BTreeMap<super::io::Phase, std::collections::VecDeque<io::ErrorKind>>,
    /// Abort the process just before the phase's call number `n` (0-based).
    #[cfg(any(test, feature = "test-faults"))]
    abort: Option<(super::io::Phase, usize)>,
}

impl ProductionIo {
    /// `trash` is the existing typed transaction-trash directory chosen by the
    /// store (pages/journals/conflicts). Step 3 supplies that classification.
    /// Graph/page keys and graph ID are already resolved by the store boundary.
    /// Never fails (B-Q1): a drafts directory that cannot be created or
    /// listed leaves draft I/O down, the census unknown (M1).
    pub(super) fn attach(graph: &Path, app_data: &Path, graph_id: &str, trash: &Path) -> Self {
        let drafts = app_data.join("drafts-v2").join(graph_id);
        let listing = list(&drafts);
        let mut io = Self {
            graph: graph.to_path_buf(),
            drafts,
            trash: trash.to_path_buf(),
            page_temps: BTreeMap::new(),
            draft_temps: BTreeMap::new(),
            creates: BTreeMap::new(),
            draft_payloads: BTreeMap::new(),
            durable: BTreeMap::new(),
            readable: BTreeMap::new(),
            unsynced: Default::default(),
            changes: vec![],
            paths: BTreeMap::new(),
            stamps: BTreeMap::new(),
            spellings: Default::default(),
            quarantines: BTreeMap::new(),
            directories: BTreeMap::new(),
            launch_warnings: vec![],
            marks: None,
            down: None,
            untouchable: Default::default(),
            #[cfg(any(test, feature = "test-faults"))]
            faults: ATTACH_FAULTS.with(|faults| faults.take()),
            #[cfg(any(test, feature = "test-faults"))]
            abort: ATTACH_ABORT.with(|abort| abort.take()),
        };
        match listing {
            // A listed vehicle is readable, not yet durable: a process crash
            // can leave a rename without its directory sync. Launch's sync
            // makes the census durable before anything retires (M2).
            Ok(found) => {
                for (name, (path, bytes)) in found {
                    io.paths.insert(name.clone(), path);
                    io.readable.insert(name.clone(), bytes);
                    io.unsynced.insert(name);
                }
            }
            Err(error) => io.down = Some(Down::Unlisted(error.to_string())),
        }
        io
    }

    /// Every draft effect starts here (§4): none runs while draft I/O is
    /// down, or on a vehicle launch could not quarantine.
    fn draft_effect(&mut self, phase: Phase, name: Option<&str>) -> IoResult<()> {
        if self.down.is_some() || name.is_some_and(|name| self.untouchable.contains(name)) {
            return Err(IoFailure {
                kind: ErrorKind::Io,
                completed: false,
                operation: None,
                os_error: None,
            });
        }
        self.before(phase)
    }

    fn before(&mut self, phase: super::io::Phase) -> IoResult<()> {
        #[cfg(any(test, feature = "test-faults"))]
        if let Some((at, n)) = &mut self.abort {
            if *at == phase {
                if *n == 0 {
                    std::process::abort();
                }
                *n -= 1;
            }
        }
        #[cfg(any(test, feature = "test-faults"))]
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
        #[cfg(feature = "test-faults")]
        crate::cost_counters::fsync();
        durability::sync_directory_witness(dir)
            .map(witness)
            .map_err(sync_failure)
    }

    /// The page's file, through its key's current spelling.
    /// Record `page`'s latest pre-read stamp; none drops an older one.
    fn stamp(&mut self, page: &str, stamp: Option<crate::watch::Stamp>) {
        match stamp {
            Some(stamp) => self.stamps.insert(page.into(), stamp),
            None => self.stamps.remove(page),
        };
    }

    fn page_path(&self, key: &str) -> PathBuf {
        self.graph.join(self.spellings.spelling(key))
    }

    fn draft_path(&self, name: &str) -> PathBuf {
        self.paths
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.drafts.join(name))
    }

    /// The custody directory, created with the strict app-data recipe.
    fn custody(&self) -> io::Result<PathBuf> {
        let dir = self.drafts.join(CUSTODY);
        durability::create_dir_all_with_sync(&dir, durability::sync_private_directory)?;
        Ok(dir)
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

/// Why draft I/O is down (plan v3 §4).
enum Down {
    /// The drafts directory could not be created or listed: the census is
    /// unknown, so launch recovers nothing and reads no vehicle (M1).
    Unlisted(String),
    /// Launch could not sync the recovered census (M2).
    Unsynced,
}

/// Create the drafts directory with the strict app-data recipe and read
/// every vehicle in it, by name. An error is never an empty listing.
fn list(drafts: &Path) -> io::Result<BTreeMap<String, (PathBuf, Vec<u8>)>> {
    // The custody directory is not created here: a malformed one must not
    // stop the graph from opening (REVIEW-2b-r2 V1). The listing creates it.
    durability::create_dir_all_with_sync(drafts, durability::sync_private_directory)?;
    // Also cover a directory chain left readable by an interrupted earlier
    // creation attempt.
    for dir in drafts.ancestors().filter(|dir| !dir.as_os_str().is_empty()) {
        durability::sync_private_directory(dir)?;
    }
    let mut found = BTreeMap::new();
    for entry in fs::read_dir(drafts)? {
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
        // path as a checksum/format failure. Enumeration errors propagate.
        let bytes = fs::read(entry.path()).unwrap_or_default();
        found.insert(name, (entry.path(), bytes));
    }
    Ok(found)
}

/// `dir` and each of its ancestors up to and including the graph root.
fn chain<'a>(graph: &'a Path, dir: &'a Path) -> impl Iterator<Item = &'a Path> {
    dir.ancestors()
        .take_while(move |dir| dir.starts_with(graph))
}

/// Unlink; an already absent file is the requested state.
fn remove_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
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
    let step = crate::directory_durability::failure_step(&error);
    IoFailure {
        kind: if error.kind() == io::ErrorKind::AlreadyExists {
            ErrorKind::Collision
        } else {
            ErrorKind::Io
        },
        completed: false,
        operation: step.map(|(operation, _)| operation),
        os_error: step.map_or(error.raw_os_error(), |(_, os_error)| os_error),
    }
}

/// Read `path` with the metadata of the read's own handle taken before the
/// bytes, and their revision (GH #623; `transaction/publication.rs`).
fn read_observed(path: &Path) -> io::Result<(Vec<u8>, Option<crate::watch::Stamp>)> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let stamp = file
        .metadata()
        .ok()
        .and_then(|metadata| crate::watch::stamp_from_metadata(&metadata));
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    #[cfg(feature = "test-faults")]
    crate::cost_counters::full_read();
    let rev = crate::FileRev::from_bytes(&bytes);
    Ok((bytes, stamp.map(|stamp| stamp.with_rev(Some(rev)))))
}

/// A synced temporary file for `path`. The host's writes and syncs are
/// counted where the transaction's are (`cost_counters`, I-25, Q-P2b-2).
fn temp(path: &Path, bytes: &[u8]) -> io::Result<PreparedWrite> {
    PreparedWrite::with_hooks(
        path,
        bytes,
        || {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::wrote(bytes.len());
        },
        || {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::fsync();
        },
    )
}

fn sync_failure(error: io::Error) -> IoFailure {
    let completed = durability::is_directory_sync_failure(&error);
    IoFailure {
        completed,
        ..failure(error)
    }
}

impl ProductionIo {
    /// The key moved to `spelling` (a case alias or the alias spelling
    /// move), if any.
    pub(super) fn respelled(&self, spelling: &str) -> Option<String> {
        self.spellings.respelled(spelling)
    }

    /// The spelling table, shared with the store's held index (B1).
    pub(super) fn spellings(&self) -> &std::sync::Arc<crate::model::entry_identity::Spellings> {
        &self.spellings
    }
}

impl super::Host<ProductionIo> {
    /// The registered key whose current spelling is `spelling` (§2), by
    /// lookup (A-R5, D-10): a key moved to that spelling, or the key
    /// spelled as itself.
    pub(super) fn key_spelled(&self, spelling: &str) -> Option<String> {
        if let Some(key) = self.fs.respelled(spelling) {
            return Some(key);
        }
        (self.keys.contains(spelling) && self.fs.spelling(spelling) == spelling)
            .then(|| spelling.into())
    }

    /// The key for a new entry spelled `spelling` (REVIEW-3a4 #1): the key
    /// spelled there, else the spelling itself unless a live key of that
    /// name now spells another entry (a respelled key), in which case a
    /// fresh key no path can equal. Keys are opaque; each entry keeps its
    /// own buffer, lock and custody.
    pub(super) fn key_for(&self, spelling: &str) -> String {
        if let Some(key) = self.key_spelled(spelling) {
            return key;
        }
        if !self.keys.contains(spelling) {
            return spelling.into();
        }
        (1..)
            .map(|n| fresh_key(spelling, n))
            .find(|key| !self.keys.contains(key))
            .expect("an unused key")
    }

    /// The registered keys whose spelling's leaf folds to `fold`, with their
    /// spellings: [`crate::model::Graph::identify`]'s candidates (B1).
    pub(super) fn candidates(&self, fold: &str) -> Vec<(String, String)> {
        let keys = &self.keys;
        self.fs.spellings.candidates(fold, |key| keys.contains(key))
    }
}

/// The `n`th fresh key for `spelling`: NUL never occurs in a path, so the
/// key equals no spelling and no other entry's key.
fn fresh_key(spelling: &str, n: u64) -> String {
    format!("{spelling}\u{0}{n}")
}

/// The spelling a recovered key was created for: a fresh key's base.
pub(super) fn key_base(key: &str) -> &str {
    key.split('\u{0}').next().unwrap_or(key)
}

// `HostIo::spelling` calls on this thread (A-R5's counting test).
#[cfg(test)]
thread_local! {
    pub(super) static SPELLING_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// The plants the next `attach` on this thread starts with (`super::faults`).
/// The error a planted fault fails with: the adapter's own I/O error kind.
#[cfg(feature = "test-faults")]
pub type FaultKind = io::ErrorKind;
#[cfg(any(test, feature = "test-faults"))]
thread_local! {
    /// Faults: a binding's launch then fails as the filesystem would (R2's
    /// launch tests), and a host's later phases likewise (Q-P2b-2).
    pub(super) static ATTACH_FAULTS: std::cell::RefCell<
        BTreeMap<super::io::Phase, std::collections::VecDeque<io::ErrorKind>>,
    > = const { std::cell::RefCell::new(BTreeMap::new()) };
    /// A process abort before a phase's numbered call (Q-P2b-2).
    pub(super) static ATTACH_ABORT: std::cell::Cell<Option<(super::io::Phase, usize)>> =
        const { std::cell::Cell::new(None) };
}

impl HostIo for ProductionIo {
    fn spelling(&self, key: &str) -> String {
        #[cfg(test)]
        SPELLING_LOOKUPS.with(|n| n.set(n.get() + 1));
        self.spellings.spelling(key)
    }

    fn spell(&mut self, key: &str, spelling: &str) {
        self.spellings.spell(key, spelling);
    }

    fn graph_launch(&mut self, pages: &std::collections::BTreeSet<String>) {
        // Every existing ancestor up to the graph root: an interrupted nested
        // page-directory creation leaves entries no later leaf sync covers.
        // The same holds for the trash chain: a crash between a deletion's
        // mkdirs and their parent syncs loses the retained obligation, and the
        // next deletion's creation sees the entries and owes nothing
        // (REVIEW-2b-r2 R1). A trash directory that does not exist is skipped.
        let mut directories = std::collections::BTreeSet::from([self.graph.clone()]);
        for page in pages {
            let path = self.page_path(page);
            directories.extend(chain(&self.graph, path.parent().unwrap()).map(Path::to_path_buf));
        }
        directories.extend(
            chain(&self.graph, &self.trash)
                .filter(|dir| dir.try_exists().unwrap_or(true))
                .map(Path::to_path_buf),
        );
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
        match read_observed(&self.page_path(page)) {
            Ok((bytes, stamp)) => {
                self.creates.insert(page.into(), false);
                self.stamp(page, stamp);
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
        let target = self.page_path(page);
        self.graph_directory(target.parent().unwrap())?;
        let bytes = bytes.as_deref().ok_or(IoFailure {
            kind: ErrorKind::Io,
            completed: false,
            operation: None,
            os_error: None,
        })?;
        if let Some(graph) = &self.marks {
            graph.transaction_note_page(&target, bytes);
        }
        let prepared = temp(&target, bytes).map_err(failure)?;
        self.page_temps.insert(page.into(), prepared);
        Ok(())
    }

    fn page_rename(&mut self, page: &str) -> IoResult<()> {
        self.before(super::io::Phase::PageRename)?;
        let prepared = self.page_temps.remove(page).ok_or(IoFailure {
            kind: ErrorKind::Io,
            completed: false,
            operation: None,
            os_error: None,
        })?;
        prepared
            .publish(self.creates.get(page).copied().unwrap_or(false))
            .map_err(failure)?;
        // The publication read (GH #623): the bytes now at the path with
        // their handle's stamp, instead of the watcher's open and hash. A
        // failed read leaves the watcher its own stamp.
        let stamp = read_observed(&self.page_path(page))
            .ok()
            .and_then(|(_, s)| s);
        self.stamp(page, stamp);
        Ok(())
    }

    fn take_stamp(&mut self, page: &str) -> Option<crate::watch::Stamp> {
        self.stamps.remove(page)
    }

    fn page_sync(&mut self, page: &str) -> IoResult<Witness> {
        let path = self.page_path(page);
        self.graph_sync(super::io::Phase::PageSync, path.parent().unwrap())
    }

    fn page_twin(&mut self, page: &str) -> IoResult<Option<String>> {
        let twin =
            crate::transaction::alternate_extension_twin(&self.page_path(page)).map_err(failure)?;
        Ok(twin.map(|path| {
            let relative = path.strip_prefix(&self.graph).unwrap_or(&path);
            relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        }))
    }

    fn trash_move(&mut self, page: &str, payload: &str) -> MoveResult {
        let result = (|| {
            self.before(super::io::Phase::TrashMove)?;
            let source = self.page_path(page);
            if !source.try_exists().map_err(failure)? {
                return Ok(None);
            }
            self.graph_directory(&self.trash.clone())?;
            if let Some(graph) = &self.marks {
                graph.transaction_note_delete(&source);
            }
            let target = self.trash.join(payload);
            crate::no_replace::move_file_noreplace(&source, &target).map_err(failure)?;
            // Read the actual moved bytes, including an R1 external replacement
            // between the guard and move; never claim the guard's stale bytes.
            fs::read(target)
                .map(|bytes| Some(bytes.into()))
                .map_err(|_| IoFailure {
                    kind: ErrorKind::Io,
                    completed: true,
                    operation: None,
                    os_error: None,
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
        #[cfg(feature = "test-faults")]
        crate::cost_counters::fsync();
        match crate::atomic_file::sync_file_bytes(&self.trash.join(payload)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Witness::Durable),
            Err(error) => return Err(failure(error)),
        }
        chain(&self.graph, &self.trash).try_fold(Witness::Durable, |result, dir| {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::fsync();
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
        let dir = self.custody().map_err(failure)?;
        temp(&dir.join(name), bytes)
            .and_then(|prepared| prepared.publish(true))
            .and_then(|()| {
                #[cfg(feature = "test-faults")]
                crate::cost_counters::fsync();
                durability::sync_private_directory(&dir)
            })
            .map_err(failure)
    }

    fn custody_retire(&mut self, name: &str) -> IoResult<()> {
        self.before(super::io::Phase::CustodyRetire)?;
        let dir = self.drafts.join(CUSTODY);
        remove_present(&dir.join(name)).map_err(failure)?;
        #[cfg(feature = "test-faults")]
        crate::cost_counters::fsync();
        durability::sync_private_directory(&dir).map_err(failure)
    }

    fn custody_markers(&mut self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let report = |error: io::Error| format!("{CUSTODY}: {error}");
        self.before(super::io::Phase::CustodyList)
            .map_err(|_| report(io::Error::other("injected listing failure")))?;
        // A non-directory here was already quarantined by launch's draft scan
        // (it is no draft); the strict creation then succeeds.
        let dir = self.custody().map_err(report)?;
        let (mut markers, mut temps) = (vec![], false);
        for entry in fs::read_dir(&dir).map_err(report)? {
            let entry = entry.map_err(report)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp") {
                // V2: an unpublished marker temp was never installed, so no
                // move followed it and no custody is owed. Best effort: one
                // that survives is reclaimed at the next listing. A second
                // instance's in-flight temp would make its deletion fail and
                // retry, never lose bytes (F7, step 3).
                temps |= remove_present(&entry.path()).is_ok();
            } else {
                // Unreadable means malformed: quarantined, never acted on.
                markers.push((name, fs::read(entry.path()).unwrap_or_default()));
            }
        }
        if temps {
            let _ = durability::sync_private_directory(&dir);
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

    fn draft_changes(&mut self) -> Vec<(String, Option<Vec<u8>>)> {
        std::mem::take(&mut self.changes)
    }

    fn draft_temp(&mut self, name: &str, bytes: &[u8]) -> IoResult<()> {
        self.draft_effect(Phase::DraftTemp, Some(name))?;
        self.draft_temps.insert(
            name.into(),
            temp(&self.drafts.join(name), bytes).map_err(failure)?,
        );
        self.draft_payloads.insert(name.into(), bytes.to_vec());
        Ok(())
    }

    fn draft_rename(&mut self, name: &str) -> IoResult<()> {
        if let Err(error) = self.draft_effect(Phase::DraftRename, Some(name)) {
            self.draft_temps.remove(name);
            self.draft_payloads.remove(name);
            return Err(error);
        }
        let prepared = self.draft_temps.remove(name).ok_or(IoFailure {
            kind: ErrorKind::Io,
            completed: false,
            operation: None,
            os_error: None,
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
        self.unsynced.insert(name.into());
        Ok(())
    }

    fn draft_unlink(&mut self, name: &str) -> IoResult<()> {
        self.draft_effect(Phase::DraftUnlink, Some(name))?;
        self.draft_temps.remove(name);
        self.draft_payloads.remove(name);
        remove_present(&self.draft_path(name)).map_err(failure)?;
        self.readable.remove(name);
        self.unsynced.insert(name.into());
        // NotFound is only readable absence. Vehicle still requires DraftSync.
        Ok(())
    }

    fn draft_sync(&mut self) -> IoResult<Witness> {
        self.draft_effect(Phase::DraftSync, None)?;
        #[cfg(feature = "test-faults")]
        crate::cost_counters::fsync();
        durability::sync_private_directory(&self.drafts).map_err(sync_failure)?;
        // Only entries changed since the last sync differ between the two
        // censuses; every payload is not copied again (D-10).
        for name in std::mem::take(&mut self.unsynced) {
            let bytes = self.readable.get(&name).cloned();
            if bytes.is_none() {
                self.paths.remove(&name);
            }
            if self.durable.get(&name) != bytes.as_ref() {
                match &bytes {
                    Some(bytes) => self.durable.insert(name.clone(), bytes.clone()),
                    None => self.durable.remove(&name),
                };
                self.changes.push((name, bytes));
            }
        }
        Ok(Witness::Durable)
    }

    fn quarantine(&mut self, name: &str) -> IoResult<()> {
        // The one effect a vehicle left in place still takes: Retry's.
        self.draft_effect(Phase::Quarantine, None)?;
        let result = (|| {
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
            durability::sync_private_directory(source.parent().unwrap()).map_err(failure)
        })();
        if result.is_err() {
            // Launch goes on without it (§4): left in place, never touched.
            self.untouchable.insert(name.into());
            return result;
        }
        self.readable.remove(name);
        if self.durable.remove(name).is_some() {
            self.changes.push((name.into(), None));
        }
        self.unsynced.remove(name);
        self.paths.remove(name);
        self.quarantines.remove(name);
        self.untouchable.remove(name);
        Ok(())
    }

    fn draft_status(&self) -> DraftStatus {
        let drafts = self.drafts.display();
        DraftStatus {
            unavailable: self.down.as_ref().map(|down| match down {
                Down::Unlisted(error) => format!("{drafts}: {error}"),
                Down::Unsynced => format!("{drafts}: the recovered drafts could not be synced"),
            }),
            unreadable: self.untouchable.iter().cloned().collect(),
            unsaved: vec![],
        }
    }

    fn drafts_unsynced(&mut self) {
        self.down.get_or_insert(Down::Unsynced);
    }

    fn drafts_reprobe(&mut self) -> Result<(), String> {
        match self.down.take() {
            None => {}
            Some(Down::Unsynced) => {
                if !matches!(self.draft_sync(), Ok(Witness::Durable)) {
                    self.down = Some(Down::Unsynced);
                }
            }
            // The census was never known. Only a listing that finds no
            // vehicle can bring draft I/O up in place; vehicles found now
            // are left untouched for the next launch to recover.
            Some(Down::Unlisted(_)) => match list(&self.drafts) {
                Ok(found) if found.is_empty() => {}
                Ok(found) => {
                    self.down = Some(Down::Unlisted(format!(
                        "{} draft file(s) from an earlier session are recovered when Tine restarts",
                        found.len()
                    )))
                }
                Err(error) => self.down = Some(Down::Unlisted(error.to_string())),
            },
        }
        self.draft_status().unavailable.map_or(Ok(()), Err)
    }
}
