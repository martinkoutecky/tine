//! Installed derived page state and the pages a host holds (A-H2, invariant
//! (G); REVIEW-AH2-AW1-plan R1/R2). This module alone writes the page cache,
//! its path index, revisions, anchors, file times, error bookkeeping, the
//! generation-keyed listing and claimant memos, and the held map. Their
//! fields are private here, so every write is one of the functions below.
//!
//! **(G).** For a page a host holds, every installed row about it comes from
//! its owner's last published bytes, and its name and document come from the
//! same bytes. The one identity rule ([`Graph::identify`], B1) decides a
//! path's authority inside the critical section that installs it:
//! - not held (`New`): the caller's disk-derived row;
//! - held, with indexed bytes: a row this module derives from those bytes
//!   (the owner's R8 document travels with its own publication, E104);
//! - held and unindexed, indexed absent, `Unknown` or `Outside`: no row, and
//!   any current row is retired.
//!
//! **Critical section.** Every install takes the cache lock (write for
//! rows; at least read for error collections) and then reads the held map:
//! lock order cache → held → collection, authority decided inside it. A
//! whole build decides at the held epoch it checks again under the lock.
//! Holds, releases and respellings are one ownership transition
//! ([`Graph::transition`], REVIEW-3a4): under the cache lock it changes the
//! held map, bumps the held epoch and settles every row and error whose
//! authority it moves (the keys' spellings and their colliding leaves, which
//! become or stop being `Unknown`); an owner's absent publication settles
//! its path the same way. The settled paths go to the next snapshot capture,
//! which the transition's caller publishes under the writer
//! ([`crate::Store::publish_retired`]), so no view acquired after it shows a
//! pre-transition row. A disk disappearance never removes a held page's
//! row. File times are file metadata, not content: every installed row
//! records its file's time (E109).
//!
//! `cache_gen` (a counter), the self-write markers and the launch
//! observations live outside this module: they are not derived state about
//! a page. Uninstalled computed answers (a direct read's disk name, a walk)
//! are not constrained here; the constraint applies where they are
//! installed.

use super::entry_identity::{fold_leaf, Identity, Memo, Spellings};
use super::page_identity::{list_graph_pages, list_graph_pages_kind, listed_entry, page_claimants};
use super::*;
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// What a held key's owner last published since the hold began: `None`
/// until the first publication, `Some(None)` for no file. The bytes are the
/// owner's buffer, shared, not copied.
pub(crate) type Indexed = Option<Option<Arc<[u8]>>>;

/// The held pages, by host key (B1): a key is the page's identity, and its
/// spelling (the host's [`Spellings`], attached while a host runs) is only
/// where its I/O goes. Changed only here, under the store writer and the
/// cache lock.
#[derive(Default)]
pub(crate) struct HeldPages {
    keys: RwLock<HashMap<String, Indexed>>,
    /// The running host's spelling table; a fresh empty one with no host.
    spellings: RwLock<Arc<Spellings>>,
    /// Bumped by every hold, release and respelling: identities decided
    /// before a bump are stale.
    epoch: AtomicU64,
    /// Paths whose rows a transition settled, for the next snapshot
    /// capture (E105, REVIEW-3a4 #2).
    retired: Mutex<BTreeSet<PathBuf>>,
    /// Per held key, the paths withheld as `Unknown` with it a candidate:
    /// its release rereads them (REVIEW-3a4 #3). Lock order keys → this.
    withheld: Mutex<HashMap<String, BTreeSet<PathBuf>>>,
    /// Per held key, the disk row its hold retired (R1), until the owner's
    /// first publication: reused when those bytes are the row's, so the
    /// claim parses nothing again (GH #623). Bounded by the held keys.
    claims: Mutex<HashMap<String, Row>>,
}

impl HeldPages {
    pub(crate) fn spellings(&self) -> Arc<Spellings> {
        Arc::clone(&self.spellings.read().unwrap())
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.keys.read().unwrap().is_empty()
    }

    /// The held keys whose spelling's leaf folds to `fold`, with their
    /// spellings ([`Graph::identify`]'s candidates).
    pub(super) fn candidates(&self, fold: &str) -> Vec<(String, String)> {
        let keys = self.keys.read().unwrap();
        if keys.is_empty() {
            return Vec::new();
        }
        self.spellings()
            .candidates(fold, |key| keys.contains_key(key))
    }

    /// What `key`'s owner indexed; None when it is not held.
    pub(super) fn indexed_of(&self, key: &str) -> Option<Indexed> {
        self.keys.read().unwrap().get(key).cloned()
    }

    /// Every held key whose owner indexed bytes.
    fn published(&self) -> Vec<(String, Arc<[u8]>)> {
        let keys = self.keys.read().unwrap();
        keys.iter()
            .filter_map(|(key, indexed)| Some((key.clone(), indexed.clone()??)))
            .collect()
    }

    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    /// `path` is withheld as `Unknown` with `candidates`: each one still
    /// held rereads it on release. Atomic with a release (keys → withheld).
    fn withhold(&self, candidates: &[String], path: &Path) {
        let keys = self.keys.read().unwrap();
        let mut withheld = self.withheld.lock().unwrap();
        for key in candidates.iter().filter(|key| keys.contains_key(*key)) {
            withheld
                .entry(key.clone())
                .or_default()
                .insert(path.to_path_buf());
        }
    }
}

/// The installed derived page state (fields private: (G) above).
pub(crate) struct Derived {
    /// In-memory cache of every parsed page, keyed implicitly by position.
    /// Built once on the first whole-graph question and kept in sync by
    /// edits, so search, backlinks and `{{query}}` scan memory. `None` = not
    /// built. `Arc<Document>`, so a snapshot or a copy-on-write is an O(1)
    /// refcount bump.
    cache: RwLock<Option<Arc<Pages>>>,
    /// Exact-path index into stable cache slots. Whole-graph iteration keeps
    /// the initial page order, with new pages appended and removed slots
    /// omitted. `None` rebuilds it from the cache on the next lookup.
    cache_index: RwLock<Option<PageCacheIndex>>,
    /// File times observed while publishing the cache. Readers clone the
    /// table with their graph view, so later disk edits cannot alter it.
    observed_mtimes: RwLock<Arc<SharedMap<String, std::time::SystemTime>>>,
    /// Graph-relative paths of pages the latest whole-graph build skipped
    /// because their parse panicked, so a parser gap never degrades search
    /// invisibly.
    page_index_failures: RwLock<Vec<String>>,
    unreadable_pages: RwLock<Arc<Vec<(crate::store::FileId, String)>>>,
    discovery_errors: RwLock<Vec<(crate::FileId, crate::IoError)>>,
    /// Memoized `list_pages()`, keyed by `cache_gen` (which every page
    /// create, delete and rename bumps), so quick-switch and `[[` complete
    /// without re-walking the graph on every keystroke.
    page_list_cache: RwLock<Option<(u64, Arc<Vec<PageEntry>>)>>,
    /// Memoized exact `find_entry(name, kind)` claimants, keyed by
    /// `cache_gen`. It keeps `find_entry`'s duplicate selection: the
    /// date-stem file first, else the first match of the walk.
    find_entry_cache: RwLock<Option<(u64, FindEntryIndex)>>,
    /// The launch pass's complete effective-name listing (duplicate journal
    /// days included), keyed by `cache_gen`: the first publication's name
    /// index is built from it instead of a second walk.
    launch_listing: RwLock<Option<(u64, Certified)>>,
    /// `path → content_rev` of the bytes each cached document was parsed
    /// from. Invariant: an entry exists IFF the page is cached, and
    /// `disk_revs[path] == content_rev(disk bytes)` ⟹ the cached doc
    /// reflects disk. A missing or mismatched entry only costs a reparse.
    disk_revs: RwLock<HashMap<PathBuf, String>>,
    /// The cached pages whose bytes carry a column-0 VCS anchor line
    /// (`tine_core::concord_queue::has_vcs_anchor`), observed from the SAME
    /// bytes as `disk_revs[path]`. A cached page with no entry here has no
    /// anchor line ([`Graph::vcs_anchor_state`]). A superset is harmless; a
    /// subset would hide a conflicted page. Checkpointed (FORMAT 6).
    vcs_anchored: RwLock<HashSet<PathBuf>>,
}

impl Default for Derived {
    fn default() -> Self {
        Derived {
            cache: RwLock::new(None),
            cache_index: RwLock::new(None),
            observed_mtimes: RwLock::new(Arc::new(SharedMap::new())),
            page_index_failures: RwLock::new(Vec::new()),
            unreadable_pages: RwLock::new(Arc::new(Vec::new())),
            discovery_errors: RwLock::new(Vec::new()),
            page_list_cache: RwLock::new(None),
            find_entry_cache: RwLock::new(None),
            launch_listing: RwLock::new(None),
            disk_revs: RwLock::new(HashMap::new()),
            vcs_anchored: RwLock::new(HashSet::new()),
        }
    }
}

/// A page listing every row of which has passed (G) (R2): rows that a held
/// or unknown identity names are dropped, and each held key whose owner
/// indexed bytes is listed once, named from those bytes. Built only by
/// [`Graph::certify`]; every installed listing and claimant index is one.
#[derive(Clone, Default)]
pub(crate) struct Certified(Vec<PageEntry>);

/// Where the row for one path comes from, decided under the cache lock.
enum Authority {
    /// No host holds it: the caller's row.
    Disk,
    /// Held, with these indexed bytes.
    Owner(String, Arc<[u8]>),
    /// Held and unindexed or absent, unknown, or outside: no row.
    Withheld,
}

pub(super) type Row = (PageEntry, Arc<Document>, DiskObs);

/// A row derived from an owner's bytes, bound to them.
struct Owned {
    key: String,
    bytes: Arc<[u8]>,
    row: Option<Row>,
}

/// What a snapshot names a changed path (R2).
pub(crate) enum Named {
    Entry(PageEntry),
    /// Held and not installed, or gone: the previous name is removed.
    Absent,
    /// Not graph text.
    NotAPage,
}

/// Copy `saved`'s runtime identities onto `into`, a tree of equal content.
fn keep_runtime_ids(into: &mut [DocBlock], saved: &[DocBlock]) {
    for (block, saved) in into.iter_mut().zip(saved) {
        if !saved.uuid.is_empty() {
            block.uuid.clone_from(&saved.uuid);
        }
        keep_runtime_ids(&mut block.children, &saved.children);
    }
}

impl Graph {
    /// The authority for `path`'s row now ((G) above), within the pass
    /// whose memo is `memo`.
    fn authority_in(&self, memo: &mut Memo, path: &Path) -> Authority {
        match self.held_identity_in(memo, path) {
            Identity::New => Authority::Disk,
            Identity::Key(key) => match self.held.indexed_of(&key) {
                Some(Some(Some(bytes))) => Authority::Owner(key, bytes),
                _ => Authority::Withheld,
            },
            Identity::Unknown { candidates, .. } => {
                self.held.withhold(&candidates, path);
                Authority::Withheld
            }
            Identity::Outside => Authority::Withheld,
        }
    }

    fn authority(&self, path: &Path) -> Authority {
        self.authority_in(&mut Memo::default(), path)
    }

    /// Whether `path`'s installed rows come from its file: no host holds a
    /// page it names. With nothing held, true at no cost.
    pub(crate) fn disk_sourced(&self, path: &Path) -> bool {
        matches!(self.authority(path), Authority::Disk)
    }

    /// [`Self::disk_sourced`] within the pass whose memo is `memo`.
    pub(crate) fn disk_sourced_in(&self, memo: &mut Memo, path: &Path) -> bool {
        matches!(self.authority_in(memo, path), Authority::Disk)
    }

    /// The watcher left `path` to `candidates`' owners (`Unknown`): each
    /// one's release rereads it.
    pub(crate) fn note_withheld(&self, candidates: &[String], path: &Path) {
        self.held.withhold(candidates, path);
    }

    /// `key`'s listing entry named from its owner's `bytes`, at the key's
    /// spelling; None when that path is no cacheable page or the bytes are
    /// not a page's.
    pub(super) fn owned_entry(&self, key: &str, bytes: &[u8]) -> Option<PageEntry> {
        let path = self.root.join(self.held.spellings().spelling(key));
        let config = self.current_config();
        if path_is_sync_conflict(&path) || !graph_text_eligible(&self.root, &path, &config) {
            return None;
        }
        let (format, journals) = (self.current_journal_format(), self.journals_path());
        let mut entry = listed_entry(self, (&format, &journals, config.file_name_format), path)?;
        if let Some(date) = entry
            .date_key
            .map(tine_core::date::JournalDate::from_ordinal)
        {
            if entry.kind == PageKind::Journal && self.is_shadow_journal(&entry.path, date) {
                return None;
            }
        }
        validate_parse_bytes_for_path(bytes, &entry.path).ok()?;
        let content = std::str::from_utf8(bytes).ok()?;
        if let Ok(name) = self.name_from(&entry, content) {
            entry.name = name;
        }
        Some(entry)
    }

    /// The row `key`'s owner bytes give: its entry named from `bytes`, and
    /// its document and observations from the same bytes. `saved`, when
    /// given, is the owner's Document for them (R8): only its runtime
    /// identities are kept, and only when its content is exactly what
    /// `bytes` parse to; its content never reaches the row (REVIEW-3a4 #5,
    /// E126).
    pub(super) fn owned_row(
        &self,
        key: &str,
        bytes: &[u8],
        saved: Option<&Document>,
    ) -> Option<Row> {
        let entry = self.owned_entry(key, bytes)?;
        let content = std::str::from_utf8(bytes).ok()?;
        let (mut doc, disk) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parse_page_content(&entry, content)
        }))
        .ok()?;
        if let Some(saved) = saved.filter(|saved| **saved == doc) {
            keep_runtime_ids(&mut doc.roots, &saved.roots);
        }
        Some((entry, Arc::new(doc), disk))
    }

    // ---- reads ----------------------------------------------------------

    /// The built page cache, if any. Never builds.
    pub(crate) fn peek_pages(&self) -> Option<Arc<Pages>> {
        self.derived.cache.read().unwrap().as_ref().map(Arc::clone)
    }

    pub(crate) fn cache_built(&self) -> bool {
        self.derived.cache.read().unwrap().is_some()
    }

    /// `f` over the cached row for `path` (None when not cached), under the
    /// cache read lock; None when the cache is not built.
    pub(super) fn with_cached<T>(
        &self,
        path: &Path,
        f: impl FnOnce(Option<&(PageEntry, Arc<Document>)>) -> T,
    ) -> Option<T> {
        let guard = self.derived.cache.read().unwrap();
        let pages = guard.as_ref()?;
        let slot = self.cached_page_index_for_path(pages, path);
        Some(f(slot.and_then(|slot| pages.get(slot))))
    }

    /// Locate a page in the cache by its resolved physical path. Callers
    /// hold the cache lock; only the companion index is (re)built here, from
    /// that immutable root (lock order cache → cache_index).
    fn cached_page_index_for_path(&self, pages: &Pages, path: &Path) -> Option<usize> {
        if let Some(index) = self.derived.cache_index.read().unwrap().as_ref() {
            return index.by_path.get(path).copied();
        }
        let mut guard = self.derived.cache_index.write().unwrap();
        if guard.is_none() {
            #[cfg(test)]
            count_cache_linear_scan(pages.len());
            *guard = Some(build_page_cache_index(pages));
        }
        guard
            .as_ref()
            .and_then(|index| index.by_path.get(path).copied())
    }

    pub(crate) fn observed_page_mtimes(&self) -> Arc<SharedMap<String, std::time::SystemTime>> {
        Arc::clone(&self.derived.observed_mtimes.read().unwrap())
    }

    pub(crate) fn unreadable_pages(&self) -> Arc<Vec<(crate::store::FileId, String)>> {
        let mut rows = Arc::clone(&self.derived.unreadable_pages.read().unwrap());
        Arc::make_mut(&mut rows).extend(
            self.derived
                .discovery_errors
                .read()
                .unwrap()
                .iter()
                .map(|(id, error)| (id.clone(), error.to_string())),
        );
        Arc::make_mut(&mut rows).sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
        Arc::make_mut(&mut rows).dedup_by(|a, b| a.0 == b.0);
        rows
    }

    /// Pages skipped by the latest whole-graph build because their parse
    /// panicked. Paths are graph-relative and safe to surface.
    #[cfg(test)]
    pub fn page_index_failures(&self) -> Vec<String> {
        self.derived.page_index_failures.read().unwrap().clone()
    }

    /// Whether the cached page at `path` has a VCS anchor line in the bytes
    /// the store last observed for it: `Some(false)` means no read of the
    /// file can find one, `Some(true)` that one may exist (the caller
    /// scans), `None` that the page is not cached (not loaded yet, or never
    /// cached: shadow journals, sync copies, unreadable or oversized files),
    /// so only reading the file can tell. Cost O(1).
    pub(crate) fn vcs_anchor_state(&self, path: &Path) -> Option<bool> {
        let _cache = self.derived.cache.read().unwrap();
        if !self.derived.disk_revs.read().unwrap().contains_key(path) {
            return None;
        }
        Some(self.derived.vcs_anchored.read().unwrap().contains(path))
    }

    /// `content_rev` of the bytes the cached page at `path` was parsed from;
    /// None when it is not cached. Cost O(1).
    pub(crate) fn cached_rev(&self, path: &Path) -> Option<String> {
        let _cache = self.derived.cache.read().unwrap();
        self.derived.disk_revs.read().unwrap().get(path).cloned()
    }

    /// The claimant index's answer for `key` at cache generation `gen`, if
    /// it has one; `None` means only a walk can answer.
    pub(super) fn cached_claimants(
        &self,
        key: &(PageKind, String),
        gen: u64,
    ) -> Option<Vec<PageEntry>> {
        let cache = self.derived.find_entry_cache.read().unwrap();
        let (g, index) = cache.as_ref()?;
        (*g == gen && (index.has_kind(key.0) || index.entries.contains_key(key)))
            .then(|| index.entries.get(key).cloned().unwrap_or_default())
    }

    #[cfg(test)]
    pub(super) fn claimant_memo_built(&self) -> bool {
        self.derived.find_entry_cache.read().unwrap().is_some()
    }

    #[cfg(test)]
    pub(super) fn cache_index_built(&self) -> bool {
        self.derived.cache_index.read().unwrap().is_some()
    }

    // ---- listings (R2) ----------------------------------------------------

    /// `entries` (a listing of `kind`, or of every kind) with (G) applied:
    /// rows a held or unknown identity names are dropped, and every held key
    /// whose owner indexed bytes is listed once, named from those bytes.
    pub(super) fn certify(&self, mut entries: Vec<PageEntry>, kind: Option<PageKind>) -> Certified {
        if self.held.is_empty() {
            return Certified(entries);
        }
        let mut memo = Memo::default();
        entries.retain(|entry| self.disk_sourced_in(&mut memo, &entry.path));
        for (key, bytes) in self.held.published() {
            if let Some(entry) = self.owned_entry(&key, &bytes) {
                if kind.is_none_or(|kind| kind == entry.kind) {
                    entries.push(entry);
                }
            }
        }
        Certified(entries)
    }

    /// Every page and journal in the graph, one per duplicate journal day.
    pub(crate) fn list_pages_shared(&self) -> Arc<Vec<PageEntry>> {
        let gen = self.cache_gen.load(Ordering::Acquire);
        if let Some((g, entries)) = self.derived.page_list_cache.read().unwrap().as_ref() {
            if *g == gen {
                return Arc::clone(entries);
            }
        }
        let entries = self.certify(list_graph_pages(self), None);
        // A duplicate-day journal (canonical + leftover title-named file)
        // shows once in quick-switch and All Pages: both resolve to one page.
        let entries = Arc::new(dedup_journal_days(
            entries.0,
            &self.current_journal_format(),
            self.current_config().file_name_format,
        ));
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&entries)));
        entries
    }

    pub(crate) fn find_claimants(&self, name: &str, kind: PageKind) -> Vec<PageEntry> {
        let key = (kind, tine_core::refs::page_key(name));
        loop {
            let gen = self.cache_gen.load(Ordering::Acquire);
            if let Some(found) = self.cached_claimants(&key, gen) {
                return found;
            }
            let listed = self.certify(list_graph_pages_kind(self, Some(kind)), Some(kind));
            let mut built = FindEntryIndex::new();
            built.entries = page_claimants(self, &listed.0);
            built.mark_kind_loaded(kind);
            let found = {
                let mut guard = self.derived.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            index
                                .entries
                                .retain(|(loaded_kind, _), _| *loaded_kind != kind);
                            index.entries.extend(built.entries);
                            index.mark_kind_loaded(kind);
                        }
                        index.entries.get(&key).cloned().unwrap_or_default()
                    }
                    _ => {
                        let found = built.entries.get(&key).cloned().unwrap_or_default();
                        *guard = Some((gen, built));
                        found
                    }
                }
            };
            if self.cache_gen.load(Ordering::Acquire) == gen {
                return found;
            }
        }
    }

    /// A direct path read can observe a new or retitled file before the
    /// watcher. Drop the claimant memo if that live file is missing from its
    /// bucket (dropping a memo is always safe); a proposed destination that
    /// does not exist must not become a claimant.
    pub(super) fn observe_name_entry(&self, entry: &PageEntry) {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let mut cache = self.derived.find_entry_cache.write().unwrap();
        if let Some((g, index)) = cache.as_ref() {
            let key = (entry.kind, tine_core::refs::page_key(&entry.name));
            let known = index.entries.get(&key).is_some_and(|entries| {
                entries.iter().any(|candidate| candidate.path == entry.path)
            });
            if *g == gen
                && !known
                && fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_file())
            {
                *cache = None;
            }
        }
    }

    /// Cold journal inventory and its complete claimant index share one walk.
    /// Only called at open, before the watcher and load worker start.
    pub(crate) fn scan_journal_names(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let entries = self.certify(
            list_graph_pages_kind(self, Some(PageKind::Journal)),
            Some(PageKind::Journal),
        );
        let mut index = FindEntryIndex::new();
        index.entries = page_claimants(self, &entries.0);
        index.mark_kind_loaded(PageKind::Journal);
        *self.derived.find_entry_cache.write().unwrap() = Some((gen, index));
        entries.0
    }

    /// The page list and the effective-name claimants, from one certified
    /// inventory (R2): the snapshot's roots and the memos hold the same rows.
    pub(crate) fn snapshot_name_index(
        &self,
    ) -> (
        Arc<Vec<PageEntry>>,
        HashMap<(PageKind, String), Vec<PageEntry>>,
    ) {
        let gen = self.cache_gen.load(Ordering::Acquire);
        let format = self.current_journal_format();
        // The build that produced this generation already listed and named
        // every file from its own reads (GH #623: one read per file); a
        // later generation walks again.
        let launch = self
            .derived
            .launch_listing
            .write()
            .unwrap()
            .take()
            .filter(|(listed_gen, _)| *listed_gen == gen);
        let entries = match launch {
            Some((_, entries)) => entries,
            None => self.certify(list_graph_pages(self), None),
        };
        let claimants = page_claimants(self, &entries.0);
        *self.derived.find_entry_cache.write().unwrap() = Some((
            gen,
            FindEntryIndex {
                entries: claimants.clone(),
                // Reuse known claims, but a first miss must discover files
                // that arrived after this published snapshot.
                pages_loaded: false,
                journals_loaded: false,
            },
        ));
        let list = Arc::new(dedup_journal_days(
            entries.0,
            &format,
            self.current_config().file_name_format,
        ));
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::clone(&list)));
        (list, claimants)
    }

    /// What a snapshot names the changed `path` (R2): from disk when no host
    /// holds it, from its owner's bytes when one does, absent otherwise.
    pub(crate) fn named_entry(&self, path: &Path) -> Named {
        match self.authority(path) {
            Authority::Disk => match self.entry_for_path(path) {
                Some(entry) => Named::Entry(entry),
                None => Named::NotAPage,
            },
            Authority::Owner(key, bytes) => match self.owned_entry(&key, &bytes) {
                Some(entry) => Named::Entry(entry),
                None => Named::Absent,
            },
            Authority::Withheld => Named::Absent,
        }
    }

    /// The entry a snapshot removes for `path`, which it named `old` (R2): a
    /// tombstone named by the path alone, with no file read and no cache.
    pub(crate) fn tombstone(&self, path: &Path, (kind, name): &(PageKind, String)) -> PageEntry {
        let format = self.current_journal_format();
        let journals = self.journals_path();
        let listed = listed_entry(
            self,
            (&format, &journals, self.current_config().file_name_format),
            path.to_path_buf(),
        );
        PageEntry {
            name: name.clone(),
            kind: *kind,
            date_key: listed.and_then(|entry| entry.date_key),
            rel_path: Some(self.rel_path(path).into()),
            path: path.to_path_buf(),
        }
    }

    /// Paths a transition settled since the last capture (E105).
    pub(crate) fn take_retired(&self) -> BTreeSet<PathBuf> {
        std::mem::take(&mut self.held.retired.lock().unwrap())
    }

    /// Whether a transition settled paths no capture has taken yet.
    pub(crate) fn has_retired(&self) -> bool {
        !self.held.retired.lock().unwrap().is_empty()
    }

    // ---- bookkeeping --------------------------------------------------------

    pub(crate) fn replace_unreadable_walk_errors(
        &self,
        previous: &HashMap<PathBuf, String>,
        current: &HashMap<PathBuf, String>,
    ) {
        let affected: HashSet<_> = previous
            .keys()
            .chain(current.keys())
            .map(|path| crate::store::FileId::from(self.rel_path(path)))
            .collect();
        // A held page's errors are its owner's to report, never a walk's:
        // decided under the cache lock (cache → held → collection).
        let _cache = self.derived.cache.read().unwrap();
        let mut memo = Memo::default();
        let current: Vec<_> = current
            .iter()
            .filter(|(path, _)| self.disk_sourced_in(&mut memo, path))
            .collect();
        let mut guard = self.derived.unreadable_pages.write().unwrap();
        let rows = Arc::make_mut(&mut *guard);
        rows.retain(|(id, _)| !affected.contains(id));
        rows.extend(current.into_iter().map(|(path, reason)| {
            (
                crate::store::FileId::from(self.rel_path(path)),
                reason.clone(),
            )
        }));
        rows.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    }

    pub(crate) fn observe_page_mtime(&self, path: &Path, mtime: Option<std::time::SystemTime>) {
        let key = self.rel_path(path);
        let mut observed = self.derived.observed_mtimes.write().unwrap();
        if observed.contains_key(&key) {
            match mtime {
                Some(value) => {
                    Arc::make_mut(&mut observed).insert(key, value);
                }
                None => {
                    Arc::make_mut(&mut observed).remove(&key);
                }
            }
        }
    }

    /// Record that `path`'s parse panicked (its page is no longer indexed).
    /// A held page's index is its owner's, never a disk parse's.
    pub(super) fn record_index_failure(&self, path: &Path) {
        let _cache = self.derived.cache.read().unwrap();
        if self.disk_sourced(path) {
            self.derived
                .page_index_failures
                .write()
                .unwrap()
                .push(self.rel_path(path));
        }
    }

    /// A direct read's name discovery for `path`: a success clears its
    /// recorded failure; a failure is recorded only for a page no host
    /// holds (a held page's name is its owner's), decided under the cache
    /// lock so no transition passes between the decision and the record.
    pub(super) fn record_name_discovery(&self, path: &Path, failure: Option<io::Error>) {
        let id = crate::FileId::from(self.rel_path(path));
        let _cache = self.derived.cache.read().unwrap();
        let record = failure.filter(|_| self.disk_sourced(path));
        #[cfg(test)]
        crate::store::pause_at_hook(&self.discovery_record_pause);
        let mut known = self.derived.discovery_errors.write().unwrap();
        known.retain(|(failed, _)| *failed != id);
        if let Some(error) = record {
            known.push((id, error.into()));
        }
    }

    /// A full listing's discovery failures for `kind` (or every kind)
    /// replace that kind's previous ones; held pages record none.
    pub(super) fn replace_discovery_errors(
        &self,
        kind: Option<PageKind>,
        failures: Vec<(crate::FileId, crate::IoError)>,
    ) {
        let journals = self.journals_path();
        let _cache = self.derived.cache.read().unwrap();
        let mut memo = Memo::default();
        let failures: Vec<_> = failures
            .into_iter()
            .filter(|(id, _)| self.disk_sourced_in(&mut memo, &self.root.join(id.as_str())))
            .collect();
        let mut known = self.derived.discovery_errors.write().unwrap();
        known.retain(|(id, _)| match kind {
            None => false,
            Some(PageKind::Journal) => !self.root.join(id.as_str()).starts_with(&journals),
            Some(PageKind::Page) => self.root.join(id.as_str()).starts_with(&journals),
        });
        known.extend(failures);
    }

    // ---- whole-cache installs -----------------------------------------------

    /// Install a whole-graph build atomically, with (G): rows a held or
    /// unknown identity names are left out, and every held key whose owner
    /// indexed bytes gets the row those bytes give. Declines when a cache
    /// mutation, a publication or a hold raced the build, or (without
    /// `replace`) when a cache exists. `discovery`, a full listing's name
    /// failures, replaces the known ones when given.
    pub(super) fn install_built(
        &self,
        built: PageCacheBuild,
        (expected_gen, held_epoch): (u64, u64),
        replace: bool,
        observed: Option<&HashMap<PathBuf, crate::watch::Stamp>>,
        discovery: Option<Vec<(crate::FileId, crate::IoError)>>,
    ) -> bool {
        let PageCacheBuild {
            pages: built,
            mut failures,
            mut unreadable,
        } = built;
        let mut built = built;
        let mut discovery = discovery;
        // Identities are decided at the build's epoch (checked again under
        // the lock), so a disk row a hold now owns is left out.
        let mut owned: Vec<Owned> = Vec::new();
        if !self.held.is_empty() {
            let mut memo = Memo::default();
            built.retain(|(entry, _, _)| self.disk_sourced_in(&mut memo, &entry.path));
            let mut disk = |rel: &str| self.disk_sourced_in(&mut memo, &self.root.join(rel));
            failures.retain(|rel| disk(rel));
            unreadable.retain(|(rel, _)| disk(rel));
            if let Some(discovery) = discovery.as_mut() {
                discovery.retain(|(id, _)| disk(id.as_str()));
            }
            owned = self
                .held
                .published()
                .into_iter()
                .map(|(key, bytes)| Owned {
                    row: self.owned_row(&key, &bytes, None),
                    key,
                    bytes,
                })
                .collect();
        }
        let mut rows: Vec<Row> = built
            .into_iter()
            .map(|(entry, doc, obs)| (entry, Arc::new(doc), obs))
            .collect();
        // The launch pass observed every file's stamp before reading it; an
        // on-demand build did not, and stats each page here, as it does an
        // owner's row (E109).
        let stat = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).ok();
        let mut mtimes: SharedMap<String, std::time::SystemTime> = rows
            .iter()
            .filter_map(|(entry, _, _)| {
                let mtime = match observed {
                    Some(observed) => observed.get(&entry.path).and_then(|stamp| stamp.modified()),
                    None => stat(&entry.path),
                };
                Some((entry.rel_path_str().to_owned(), mtime?))
            })
            .collect();
        for owned in &mut owned {
            if let Some((entry, _, _)) = &owned.row {
                if let Some(mtime) = stat(&entry.path) {
                    mtimes.insert(entry.rel_path_str().to_owned(), mtime);
                }
            }
        }
        let mut guard = self.derived.cache.write().unwrap();
        if (guard.is_some() && !replace)
            || self.cache_gen.load(Ordering::Acquire) != expected_gen
            || self.held.epoch() != held_epoch
        {
            return false;
        }
        // R1: each owner's row was derived from the bytes it indexes now
        // (every publication also moves `cache_gen`; this is the
        // certificate, checked in the installing critical section).
        for owned in owned {
            let current = matches!(
                self.held.indexed_of(&owned.key),
                Some(Some(Some(bytes))) if Arc::ptr_eq(&bytes, &owned.bytes)
            );
            if !current {
                return false;
            }
            rows.extend(owned.row);
        }
        if let Some(discovery) = discovery {
            *self.derived.discovery_errors.write().unwrap() = discovery;
        }
        unreadable.extend(
            self.derived
                .discovery_errors
                .read()
                .unwrap()
                .iter()
                .map(|(id, error)| (id.as_str().to_owned(), error.to_string())),
        );
        // One row per path: a discovery and a parse failure of the same file
        // are one unreadable file (the stable sort keeps the parse reason).
        unreadable.sort_by(|a, b| a.0.cmp(&b.0));
        unreadable.dedup_by(|a, b| a.0 == b.0);
        let revs: HashMap<PathBuf, String> = rows
            .iter()
            .map(|(entry, _, obs)| (entry.path.clone(), obs.rev.clone()))
            .collect();
        let anchored: HashSet<PathBuf> = rows
            .iter()
            .filter(|(_, _, obs)| obs.anchored)
            .map(|(entry, _, _)| entry.path.clone())
            .collect();
        let pages = Pages::from(
            rows.into_iter()
                .map(|(entry, doc, _)| (entry, doc))
                .collect::<Vec<_>>(),
        );
        let index = build_page_cache_index(&pages);
        // Cache, then revisions (cache → disk_revs), so no reader observes a
        // fresh revision paired with a stale cache.
        *guard = Some(Arc::new(pages));
        *self.derived.observed_mtimes.write().unwrap() = Arc::new(mtimes);
        *self.derived.page_index_failures.write().unwrap() = failures;
        *self.derived.unreadable_pages.write().unwrap() = Arc::new(
            unreadable
                .into_iter()
                .map(|(path, reason)| (crate::store::FileId::from(path), reason))
                .collect(),
        );
        *self.derived.cache_index.write().unwrap() = Some(index);
        *self.derived.disk_revs.write().unwrap() = revs;
        *self.derived.vcs_anchored.write().unwrap() = anchored;
        if replace {
            // A replaced cache is new content under the old generation: the
            // generation-keyed page list, name index and block index must
            // rebuild against it. Bumped after the content, as everywhere.
            self.cache_gen.fetch_add(1, Ordering::Release);
        }
        drop(guard);
        true
    }

    /// The launch build's listing, installed at generation `gen` (R2): the
    /// page list memo and the first publication's name inventory.
    pub(super) fn install_launch_listing(&self, gen: u64, named: Vec<PageEntry>) {
        let named = self.certify(named, None);
        let deduped = dedup_journal_days(
            named.0.clone(),
            &self.current_journal_format(),
            self.current_config().file_name_format,
        );
        *self.derived.page_list_cache.write().unwrap() = Some((gen, Arc::new(deduped)));
        *self.derived.launch_listing.write().unwrap() = Some((gen, named));
    }

    /// Install a loaded checkpoint's page cache: false when a cache exists
    /// or any page is held. A checkpoint's rows, indexes and claimants carry
    /// no owner provenance, so a held page makes the launch a full build
    /// (R2): the whole checkpoint is declined. The caller holds the store
    /// writer, under which holds change.
    pub(super) fn install_checkpoint_rows(
        &self,
        pages: &Arc<Pages>,
        observed_mtimes: &Arc<SharedMap<String, std::time::SystemTime>>,
        failures: Vec<String>,
        (disk_revs, vcs_anchored): (Vec<(PathBuf, String)>, Vec<PathBuf>),
        (cache_generation, list): (u64, Arc<Vec<PageEntry>>),
    ) -> bool {
        let index = build_page_cache_index(pages);
        let mut guard = self.derived.cache.write().unwrap();
        if guard.is_some() || !self.held.is_empty() {
            return false;
        }
        *guard = Some(Arc::clone(pages));
        *self.derived.observed_mtimes.write().unwrap() = Arc::clone(observed_mtimes);
        *self.derived.page_index_failures.write().unwrap() = failures;
        *self.derived.unreadable_pages.write().unwrap() = Arc::new(Vec::new());
        *self.derived.cache_index.write().unwrap() = Some(index);
        *self.derived.disk_revs.write().unwrap() = disk_revs.into_iter().collect();
        *self.derived.vcs_anchored.write().unwrap() = vcs_anchored.into_iter().collect();
        // The loaded generation keeps its number, so the evaluator, the
        // reference index and the generation-keyed listing agree with it.
        self.cache_gen.store(cache_generation, Ordering::Release);
        *self.derived.page_list_cache.write().unwrap() = Some((cache_generation, list));
        drop(guard);
        true
    }

    /// The cache rows a checkpoint captures with `pages`, the published
    /// generation's: failures, sorted revisions and anchors.
    pub(super) fn checkpoint_rows(
        &self,
        pages: &Arc<Pages>,
        cache_generation: u64,
    ) -> Result<CheckpointRows, checkpoint_state::NotCaptured> {
        use checkpoint_state::NotCaptured;
        let cache = self.derived.cache.read().unwrap();
        if !cache
            .as_ref()
            .is_some_and(|built| Arc::ptr_eq(built, pages))
            || cache_generation != self.cache_generation()
        {
            return Err(NotCaptured::Unpublished);
        }
        if !self.derived.unreadable_pages.read().unwrap().is_empty()
            || !self.derived.discovery_errors.read().unwrap().is_empty()
        {
            return Err(NotCaptured::Unreadable);
        }
        let mut disk_revs: Vec<(PathBuf, String)> = self
            .derived
            .disk_revs
            .read()
            .unwrap()
            .iter()
            .map(|(path, rev)| (path.clone(), rev.clone()))
            .collect();
        disk_revs.sort();
        let mut anchored: Vec<PathBuf> = self
            .derived
            .vcs_anchored
            .read()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        anchored.sort();
        let failures = self.derived.page_index_failures.read().unwrap().clone();
        Ok((failures, disk_revs, anchored))
    }

    /// Discard the cache; it rebuilds on the next whole-graph question. Use
    /// when an external change may have touched many files.
    pub(crate) fn invalidate_cache(&self) {
        let mut guard = self.derived.cache.write().unwrap();
        *guard = None;
        *self.derived.observed_mtimes.write().unwrap() = Arc::new(SharedMap::new());
        self.derived.page_index_failures.write().unwrap().clear();
        *self.derived.unreadable_pages.write().unwrap() = Arc::new(Vec::new());
        self.derived.discovery_errors.write().unwrap().clear();
        *self.derived.cache_index.write().unwrap() = None;
        self.derived.disk_revs.write().unwrap().clear();
        self.derived.vcs_anchored.write().unwrap().clear();
        // Bump AFTER discarding (under the cache lock): a reader that loads
        // the new generation then reads the cache sees None and rebuilds.
        // The generation-keyed block index then rebuilds against fresh
        // content too.
        self.cache_gen.fetch_add(1, Ordering::Release);
        drop(guard);
    }

    /// The forced rebuild re-reads the listing: the generation-keyed memos
    /// go (dropping a memo is always safe).
    pub(super) fn forget_name_memos(&self) {
        *self.derived.page_list_cache.write().unwrap() = None;
        *self.derived.find_entry_cache.write().unwrap() = None;
    }

    /// A reconcile found no cache: bump the generation holding the lock the
    /// builders install under (a read guard excludes their install), so a
    /// build that read the earlier bytes declines. False when a cache was
    /// installed meanwhile.
    pub(super) fn cold_reconcile_bump(&self) -> bool {
        let cache = self.derived.cache.read().unwrap();
        if cache.is_some() {
            return false;
        }
        *self.derived.page_list_cache.write().unwrap() = None;
        *self.derived.find_entry_cache.write().unwrap() = None;
        *self.derived.cache_index.write().unwrap() = None;
        self.cache_gen.fetch_add(1, Ordering::Release);
        true
    }

    /// The generation-keyed memos at `before` stay valid at `after`: the
    /// publication between them changed no listed name.
    pub(super) fn retag_name_memos(&self, before: u64, after: u64) {
        if let Some((gen, _)) = self.derived.page_list_cache.write().unwrap().as_mut() {
            if *gen == before {
                *gen = after;
            }
        }
        if let Some((gen, _)) = self.derived.find_entry_cache.write().unwrap().as_mut() {
            if *gen == before {
                *gen = after;
            }
        }
    }

    pub(crate) fn transaction_bump_generation(&self) {
        let before = self.cache_gen.fetch_add(1, Ordering::Release);
        self.retag_name_memos(before, before + 1);
    }

    // ---- one-page installs --------------------------------------------------

    /// Put `row` in `pages` (copy-on-write) with its revision, anchor and
    /// file time, all under the held cache lock.
    fn put_row(&self, pages: &mut Pages, (entry, doc, disk): Row) {
        let path = entry.path.clone();
        let rel = entry.rel_path_str().to_owned();
        match self.cached_page_index_for_path(pages, &entry.path) {
            Some(i) => {
                let slot = pages.get_mut(i).unwrap();
                if slot.0.kind != entry.kind
                    || tine_core::refs::page_key(&slot.0.name)
                        != tine_core::refs::page_key(&entry.name)
                {
                    if let Some(index) = self.derived.cache_index.write().unwrap().as_mut() {
                        index.remove(&slot.0, i);
                        index.insert(&entry, i);
                    }
                }
                *slot = (entry, doc);
            }
            None => {
                let slot = pages.push((entry, doc));
                if let Some(index) = self.derived.cache_index.write().unwrap().as_mut() {
                    index.insert(&pages[slot].0, slot);
                }
            }
        }
        // The revision, anchor and time move with the document under the
        // cache lock (cache → disk_revs → vcs_anchored), so a reader never
        // pairs a fresh revision with a stale document and two same-page
        // writers cannot leave them diverged. Set only for a cached page
        // ("an entry exists IFF cached").
        {
            let mut anchored = self.derived.vcs_anchored.write().unwrap();
            if disk.anchored {
                anchored.insert(path.clone());
            } else {
                anchored.remove(&path);
            }
        }
        self.derived
            .disk_revs
            .write()
            .unwrap()
            .insert(path.clone(), disk.rev);
        if let Ok(mtime) = fs::metadata(&path).and_then(|meta| meta.modified()) {
            Arc::make_mut(&mut self.derived.observed_mtimes.write().unwrap())
                .insert(rel.clone(), mtime);
        }
        Arc::make_mut(&mut self.derived.unreadable_pages.write().unwrap())
            .retain(|(id, _)| id.as_str() != rel);
    }

    /// Remove `path`'s row from `pages` with its revision, anchor and file
    /// time; with `errors`, also its error bookkeeping. True when a row went.
    fn retire_row(&self, pages: Option<&mut Pages>, path: &Path, errors: bool) -> bool {
        let mut removed = false;
        if let Some(pages) = pages {
            if let Some(i) = self.cached_page_index_for_path(pages, path) {
                if let Some(index) = self.derived.cache_index.write().unwrap().as_mut() {
                    index.remove(&pages[i].0, i);
                }
                pages.remove(i);
                removed = true;
            }
        }
        // Under the cache lock (cache → disk_revs order), so the two never
        // diverge.
        self.derived.disk_revs.write().unwrap().remove(path);
        self.derived.vcs_anchored.write().unwrap().remove(path);
        let rel = self.rel_path(path);
        Arc::make_mut(&mut self.derived.observed_mtimes.write().unwrap()).remove(&rel);
        if errors {
            Arc::make_mut(&mut self.derived.unreadable_pages.write().unwrap())
                .retain(|(id, _)| id.as_str() != rel);
            self.derived
                .discovery_errors
                .write()
                .unwrap()
                .retain(|(id, _)| id.as_str() != rel);
            self.derived
                .page_index_failures
                .write()
                .unwrap()
                .retain(|failed| *failed != rel);
        }
        removed
    }

    /// Update one page in the cache from bytes the caller observed (no full
    /// rebuild); the page is then indexed from them. A no-op on an unbuilt
    /// cache. `disk` is `content_rev` of the exact bytes `doc` was parsed
    /// from. A row the caller derived is installed only for a page no host
    /// holds ((G)): a held page keeps its owner's row, and an unindexed or
    /// unknown one has none.
    pub(super) fn cache_upsert(&self, entry: PageEntry, mut doc: Document, disk: DiskObs) {
        // Fill runtime ids for any block that lacks one (e.g. PDF-highlight
        // writes) from this physical owner. Blocks saved from the frontend
        // already carry live ids, which the in-memory save path keeps.
        assign_doc_runtime_ids(&mut doc.roots, entry.rel_path_str());
        let path = entry.path.clone();
        let mut guard = self.derived.cache.write().unwrap();
        #[cfg(test)]
        crate::store::pause_at_hook(&self.cache_publish_pause);
        if let Some(pages) = guard.as_mut().map(Arc::make_mut) {
            match self.authority(&path) {
                Authority::Disk => self.put_row(pages, (entry, Arc::new(doc), disk)),
                Authority::Owner(..) => {}
                Authority::Withheld => {
                    self.retire_row(Some(pages), &path, true);
                }
            }
        }
        // Bump AFTER publishing the new content (and revision), still under
        // the cache write lock: a reader that observes the new generation
        // sees the new document, so a derived result computed at generation G
        // reflects every edit whose generation is <= G. The bump is
        // unconditional, even on a cold cache, so a racing lock-free build
        // detects the mutation and retries.
        self.cache_gen.fetch_add(1, Ordering::Release);
        drop(guard);
    }

    /// Drop one physical page from the cache after its file disappears,
    /// keeping same-name siblings. A disk disappearance never removes a held
    /// page: its row is its owner's (R1).
    pub(super) fn cache_remove_path(&self, entry: &PageEntry) {
        let mut guard = self.derived.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            if !matches!(self.authority(&entry.path), Authority::Owner(..)) {
                self.retire_row(Some(Arc::make_mut(pages)), &entry.path, false);
            }
        }
        // Bump AFTER the removal is published (under the cache lock); see
        // `cache_upsert`.
        self.cache_gen.fetch_add(1, Ordering::Release);
        drop(guard);
    }

    /// The reconcile found these bytes' document unchanged, but they may
    /// still carry an anchor line the cached bytes did not (parsing
    /// normalizes some lines). The flag may only err toward true, so raise
    /// it, for a row whose bytes are the file's.
    pub(super) fn raise_vcs_anchor(&self, path: &Path, content: &str) {
        if !tine_core::concord_queue::has_vcs_anchor(content.as_bytes()) {
            return;
        }
        let _cache = self.derived.cache.write().unwrap();
        // Only for a cached row ("an entry exists IFF cached").
        if self.derived.disk_revs.read().unwrap().contains_key(path) && self.disk_sourced(path) {
            self.derived
                .vcs_anchored
                .write()
                .unwrap()
                .insert(path.to_path_buf());
        }
    }

    // ---- owners ---------------------------------------------------------------

    /// The running host's spelling table becomes the held keys' (one table,
    /// owned by the host, B1). The caller holds the writer.
    pub(crate) fn attach_spellings(&self, spellings: Arc<Spellings>) {
        let _cache = self.derived.cache.write().unwrap();
        *self.held.spellings.write().unwrap() = spellings;
        self.held.bump();
    }

    /// The one ownership transition (module doc, REVIEW-3a4): in one cache
    /// critical section `change` moves the held map (false: it changed
    /// nothing), then every path whose authority that can move is settled:
    /// `spelled`, the keys' spellings before and after, and every installed
    /// row or error whose leaf collides with one of theirs. The caller holds
    /// the writer and publishes the settled paths
    /// ([`crate::Store::publish_retired`]).
    fn transition(&self, spelled: &[PathBuf], change: impl FnOnce() -> bool) {
        let mut guard = self.derived.cache.write().unwrap();
        if !change() {
            return;
        }
        self.held.bump();
        let affected = self.affected(guard.as_deref(), spelled);
        let mut pages = guard.as_mut().map(Arc::make_mut);
        let mut memo = Memo::default();
        let mut settled = false;
        for path in affected {
            settled |= self.settle(pages.as_deref_mut(), &mut memo, &path);
        }
        // The generation-keyed listings named settled pages as before.
        if settled {
            self.cache_gen.fetch_add(1, Ordering::Release);
        }
        drop(guard);
    }

    /// `spelled` and every cached row or recorded error whose leaf folds as
    /// one of theirs: the identity candidates a transition can move.
    fn affected(&self, pages: Option<&Pages>, spelled: &[PathBuf]) -> BTreeSet<PathBuf> {
        let folds: HashSet<String> = spelled
            .iter()
            .filter_map(|path| path.file_name())
            .map(fold_leaf)
            .collect();
        let mut paths: BTreeSet<PathBuf> = spelled.iter().cloned().collect();
        if let Some(pages) = pages {
            let mut index = self.derived.cache_index.write().unwrap();
            let index = index.get_or_insert_with(|| build_page_cache_index(pages));
            for fold in &folds {
                paths.extend(index.folded(fold).iter().cloned());
            }
        }
        let mut errors: Vec<String> = self.derived.page_index_failures.read().unwrap().clone();
        errors.extend(
            (self.derived.unreadable_pages.read().unwrap().iter())
                .map(|(id, _)| id.as_str().to_owned()),
        );
        errors.extend(
            (self.derived.discovery_errors.read().unwrap().iter())
                .map(|(id, _)| id.as_str().to_owned()),
        );
        paths.extend(
            errors
                .into_iter()
                .map(|rel| self.root.join(rel))
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|leaf| folds.contains(&fold_leaf(leaf)))
                }),
        );
        paths
    }

    /// Bring `path`'s installed state to its authority now ((G)): a disk
    /// row stays; an owner's row comes from its indexed bytes at the key's
    /// spelling (another spelling of it has none); a withheld path has no
    /// row and no errors. True when the path was not disk-sourced: the next
    /// capture renames it from what it names now.
    fn settle(&self, pages: Option<&mut Pages>, memo: &mut Memo, path: &Path) -> bool {
        match self.authority_in(memo, path) {
            Authority::Disk => return false,
            Authority::Owner(key, bytes) => {
                let at = self.root.join(self.held.spellings().spelling(&key));
                if at != path {
                    self.retire_row(pages, path, true);
                } else if let Some(pages) = pages {
                    let rev = std::str::from_utf8(&bytes).ok().map(content_rev);
                    if self.derived.disk_revs.read().unwrap().get(path) != rev.as_ref() {
                        match self.owned_row(&key, &bytes, None) {
                            Some(row) => self.put_row(pages, row),
                            None => {
                                self.retire_row(Some(pages), path, true);
                            }
                        }
                    }
                }
            }
            Authority::Withheld => {
                self.retire_row(pages, path, true);
            }
        }
        self.held.retired.lock().unwrap().insert(path.to_path_buf());
        true
    }

    /// Hand `key`'s index to its owner. A new hold settles the page and its
    /// colliding entries (R1, REVIEW-3a4 #3): until its owner publishes,
    /// nothing about them is installed. Holding it again keeps what that
    /// owner already indexed. The caller holds the writer.
    pub(crate) fn hold(&self, key: String) {
        let path = self.root.join(self.held.spellings().spelling(&key));
        let row = self.with_cached(&path, |row| row.cloned()).flatten();
        let disk = self.cached_rev(&path).map(|rev| DiskObs {
            rev,
            anchored: self.derived.vcs_anchored.read().unwrap().contains(&path),
        });
        self.transition(&[path], || {
            let mut keys = self.held.keys.write().unwrap();
            if keys.contains_key(&key) {
                return false;
            }
            if let (Some((entry, doc)), Some(disk)) = (row, disk) {
                let claim = (entry, doc, disk);
                self.held.claims.lock().unwrap().insert(key.clone(), claim);
            }
            keys.insert(key, None).is_none()
        });
    }

    /// Hold `key` at its own spelling with no host running (test stores).
    #[cfg(any(test, feature = "test-faults"))]
    pub(crate) fn hold_unhosted(&self, key: &str) {
        self.held.spellings().spell(key, key);
        self.hold(key.into());
    }

    /// Return `key` to the watcher (a transition): the paths its watcher
    /// reconcile rereads, its page file and the entries withheld as
    /// colliding with it, when it was held. Its rows stay the owner's until
    /// then. The caller holds the writer.
    pub(crate) fn release(&self, key: &str) -> Vec<PathBuf> {
        let path = self.root.join(self.held.spellings().spelling(key));
        let mut reread = Vec::new();
        self.transition(std::slice::from_ref(&path), || {
            if self.held.keys.write().unwrap().remove(key).is_none() {
                return false;
            }
            self.held.claims.lock().unwrap().remove(key);
            let withheld = self.held.withheld.lock().unwrap().remove(key);
            reread.push(path.clone());
            reread.extend(withheld.into_iter().flatten());
            true
        });
        reread
    }

    /// Return every held key to the watcher and detach the host's spelling
    /// table (a transition): the paths the watcher rereads. The caller
    /// holds the writer.
    pub(crate) fn release_all(&self) -> Vec<PathBuf> {
        let mut reread = Vec::new();
        self.transition(&[], || {
            let keys: Vec<String> = (self.held.keys.write().unwrap().drain())
                .map(|(key, _)| key)
                .collect();
            self.held.claims.lock().unwrap().clear();
            let spellings = std::mem::take(&mut *self.held.spellings.write().unwrap());
            reread.extend(
                keys.iter()
                    .map(|key| self.root.join(spellings.spelling(key))),
            );
            reread.extend(
                self.held
                    .withheld
                    .lock()
                    .unwrap()
                    .drain()
                    .flat_map(|(_, paths)| paths),
            );
            true
        });
        reread
    }

    /// The alias spelling move gave held `key` a new spelling (Q4), a
    /// transition: the rows at `from` go and the owner's bytes are
    /// installed at the new spelling. The caller holds the writer.
    pub(crate) fn respelled(&self, key: &str, from: &Path) {
        let to = self.root.join(self.held.spellings().spelling(key));
        self.transition(&[from.to_path_buf(), to], || {
            self.held.indexed_of(key).is_some()
        });
    }

    /// The owner's publication of held `key` (R1): its bytes (None: no
    /// file) become the page's index, and its row is derived from them, in
    /// one critical section. `doc` is the Document the bytes were
    /// serialized from, when the owner has it (R8, E104). Returns the entry
    /// named from the bytes, None when the page has no row. `Err` when no
    /// host holds `key`: the caller publishes as for disk.
    #[allow(clippy::result_unit_err)]
    pub(crate) fn publish_owned(
        &self,
        key: &str,
        bytes: Option<Arc<[u8]>>,
        doc: Option<&Document>,
    ) -> Result<Option<PageEntry>, ()> {
        let path = self.root.join(self.held.spellings().spelling(key));
        let rev = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(content_rev);
        // Bytes the installed row was already derived from change nothing
        // but the index (an owner's observation of its own save).
        let mut row = None;
        let mut claim = self.held.claims.lock().unwrap().remove(key);
        if self.cache_built() && self.cached_rev(&path) != rev {
            row = bytes
                .as_deref()
                .and_then(|bytes| self.claimed_row(key, bytes, doc, claim.take()));
        }
        let mut guard = self.derived.cache.write().unwrap();
        {
            let mut keys = self.held.keys.write().unwrap();
            let Some(indexed) = keys.get_mut(key) else {
                return Err(());
            };
            *indexed = Some(bytes.clone());
        }
        let unchanged = self.derived.disk_revs.read().unwrap().get(&path).cloned() == rev;
        if !unchanged || guard.is_none() {
            let before = self.cache_gen.load(Ordering::Acquire);
            let mut renamed = true;
            if let Some(pages) = guard.as_mut().map(Arc::make_mut) {
                // Derived before the lock unless the installed row changed
                // since (the writer excludes that; this keeps it exact).
                if row.is_none() {
                    row = bytes
                        .as_deref()
                        .and_then(|bytes| self.claimed_row(key, bytes, doc, claim.take()));
                }
                match row.take() {
                    Some(row) => {
                        let slot = self.cached_page_index_for_path(pages, &path);
                        renamed = !slot
                            .and_then(|slot| pages.get(slot))
                            .is_some_and(|(old, _)| {
                                old.kind == row.0.kind && old.name == row.0.name
                            });
                        self.put_row(pages, row);
                    }
                    // The owner's absence: settled like a transition.
                    None => {
                        self.settle(Some(pages), &mut Memo::default(), &path);
                    }
                }
            }
            self.cache_gen.fetch_add(1, Ordering::Release);
            // The memos list the page's earlier name: still valid only when
            // this publication named it as before.
            if !renamed {
                self.retag_name_memos(before, before + 1);
            }
        }
        drop(guard);
        self.recent_writes.lock().unwrap().remove(&path);
        Ok(bytes.and_then(|bytes| self.owned_entry(key, &bytes)))
    }

    /// Install `pages` as a test graph's cache and listing.
    #[cfg(test)]
    pub(crate) fn install_test_pages(&self, pages: Pages, entries: Vec<PageEntry>) {
        let index = build_page_cache_index(&pages);
        *self.derived.cache.write().unwrap() = Some(Arc::new(pages));
        *self.derived.cache_index.write().unwrap() = Some(index);
        *self.derived.page_list_cache.write().unwrap() = Some((0, Arc::new(entries)));
    }
}

/// What [`Graph::checkpoint_rows`] returns: failures, sorted revisions and
/// sorted anchored paths.
pub(super) type CheckpointRows = (Vec<String>, Vec<(PathBuf, String)>, Vec<PathBuf>);

#[cfg(test)]
#[path = "derived_tests.rs"]
mod tests;
