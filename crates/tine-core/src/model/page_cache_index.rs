//! The page-cache index: the cache and effective-identity indexes and their
//! builders, Direct creation proof and evidence, reference candidates, and
//! single-flight page builds.

use super::*;

pub(super) struct PageCacheIndex {
    pub(super) by_name: std::collections::HashMap<(PageKind, String), usize>,
    pub(super) by_path: std::collections::HashMap<PathBuf, usize>,
}

pub(super) struct EffectiveIdentityIndex {
    pub(super) generation: std::sync::atomic::AtomicU64,
    pub(super) owners: std::collections::HashMap<(PageKind, String), Vec<PageEntry>>,
    pub(super) physical_paths: std::collections::HashSet<PathBuf>,
    pub(super) failures: Vec<String>,
}

impl Clone for EffectiveIdentityIndex {
    fn clone(&self) -> Self {
        Self {
            generation: std::sync::atomic::AtomicU64::new(self.generation()),
            owners: self.owners.clone(),
            physical_paths: self.physical_paths.clone(),
            failures: self.failures.clone(),
        }
    }
}

impl EffectiveIdentityIndex {
    pub(super) fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn retag_generation(&self, previous: u64, next: u64) -> bool {
        self.generation
            .compare_exchange(
                previous,
                next,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }
}

/// Single-use evidence for an ordinary Direct Files creation. The census owns
/// only exact names and fingerprints; graph bytes are streamed through one
/// fixed buffer and are never retained here.
pub(super) struct DirectCreationProof {
    pub(super) target: GraphTextPath,
    pub(super) generation: u64,
}

pub(super) enum DirectCreationEvidence {
    Cold,
    Warm {
        generation: u64,
        identity_index: Arc<EffectiveIdentityIndex>,
    },
}

#[derive(Default)]
pub(super) struct PageCacheBuild {
    pub(super) pages: Vec<ParsedPage>,
    pub(super) failures: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageCacheInstallOutcome {
    Installed,
    AlreadyAvailable,
    GenerationDrift,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageBuildOutcome {
    Installed,
    AlreadyAvailable,
    GenerationDrift,
    Cancelled,
    Failed,
}

impl PageBuildOutcome {
    pub(super) fn installed(self) -> bool {
        matches!(self, Self::Installed | Self::AlreadyAvailable)
    }

    pub(super) fn creation_error(self) -> io::Error {
        let message = match self {
            Self::GenerationDrift => "Direct creation identity repair crossed a cache generation",
            Self::Cancelled => "Direct creation identity repair joined a cancelled cache build",
            Self::Failed => "Direct creation identity repair joined a failed cache build",
            Self::Installed | Self::AlreadyAvailable => {
                "Direct creation identity repair completed without coherent evidence"
            }
        };
        io::Error::new(io::ErrorKind::Interrupted, message)
    }
}

impl From<PageCacheInstallOutcome> for PageBuildOutcome {
    fn from(outcome: PageCacheInstallOutcome) -> Self {
        match outcome {
            PageCacheInstallOutcome::Installed => Self::Installed,
            PageCacheInstallOutcome::AlreadyAvailable => Self::AlreadyAvailable,
            PageCacheInstallOutcome::GenerationDrift => Self::GenerationDrift,
        }
    }
}

pub(super) struct PageBuildFlight {
    pub(super) expected_generation: u64,
    /// `cache_structural_gen` at the claim: with the parsed revisions, it
    /// tells a harmless generation move from real drift at installation.
    pub(super) expected_structural: u64,
    outcome: std::sync::Mutex<Option<PageBuildOutcome>>,
    completed: std::sync::Condvar,
}

impl PageBuildFlight {
    pub(super) fn new(expected_generation: u64, expected_structural: u64) -> Self {
        Self {
            expected_generation,
            expected_structural,
            outcome: std::sync::Mutex::new(None),
            completed: std::sync::Condvar::new(),
        }
    }

    pub(super) fn complete(&self, outcome: PageBuildOutcome) {
        *self.outcome.lock().unwrap() = Some(outcome);
        self.completed.notify_all();
    }

    pub(super) fn wait(&self) -> PageBuildOutcome {
        let mut outcome = self.outcome.lock().unwrap();
        while outcome.is_none() {
            outcome = self.completed.wait(outcome).unwrap();
        }
        outcome.expect("completed page build flight has an outcome")
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct PageBuildTestState {
    pub(super) owner_pause: std::sync::Mutex<Option<Arc<PageBuildTestPause>>>,
    /// GH #543: pause one warm validation after it has announced itself and
    /// before it reads page bytes, so a test can land a query in that window.
    pub(super) warm_validation_pause: std::sync::Mutex<Option<Arc<PageBuildTestPause>>>,
    /// Pause one warm validation after it has read every page and before it
    /// checks the generation, so a test can publish a page it already read.
    pub(super) warm_read_done_pause: std::sync::Mutex<Option<Arc<PageBuildTestPause>>>,
    /// Pause one page publication right after it releases the cache lock,
    /// with its new generation observable, before it returns.
    pub(super) upsert_published_pause: std::sync::Mutex<Option<Arc<PageBuildTestPause>>>,
    /// Pause one public query whose read failed, before it asks for repair.
    pub(super) failed_read_repair_pause: std::sync::Mutex<Option<Arc<PageBuildTestPause>>>,
    pub(super) joined: std::sync::Mutex<usize>,
    pub(super) joined_changed: std::sync::Condvar,
    pub(super) force_warm_failure: std::sync::atomic::AtomicBool,
    pub(super) drift_before_install: std::sync::atomic::AtomicBool,
    /// GH #543: open this unchanged page once inside an index-backed derived
    /// read, after its SQL answer and before its generation check.
    pub(super) derived_read_open_once: std::sync::Mutex<Option<PathBuf>>,
    pub(super) enumerations: std::sync::atomic::AtomicUsize,
    pub(super) parses: std::sync::atomic::AtomicUsize,
    /// Pages a warm repair parsed (one per changed page, never a graph pass).
    pub(super) repair_parses: std::sync::atomic::AtomicUsize,
    pub(super) installs: std::sync::atomic::AtomicUsize,
    pub(super) censuses: std::sync::atomic::AtomicUsize,
    /// R6: pages parsed on demand for reference/fuzzy hydration without a
    /// parsed cache.
    pub(super) on_demand_parses: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
pub(crate) struct PageBuildTestPause {
    pub(crate) reached: std::sync::Barrier,
    pub(crate) release: std::sync::Barrier,
}

#[cfg(test)]
impl PageBuildTestPause {
    pub(crate) fn new() -> Self {
        Self {
            reached: std::sync::Barrier::new(2),
            release: std::sync::Barrier::new(2),
        }
    }
}

type ParsedPage = (PageEntry, Document, String);
pub(super) type PageParseResult = Result<Option<ParsedPage>, String>;

impl PageCacheBuild {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            pages: Vec::with_capacity(capacity),
            failures: Vec::new(),
        }
    }

    pub(super) fn append(&mut self, mut other: Self) {
        self.pages.append(&mut other.pages);
        self.failures.append(&mut other.failures);
    }

    pub(super) fn collect(&mut self, parsed: PageParseResult) -> bool {
        match parsed {
            Ok(Some(page)) => {
                self.pages.push(page);
                true
            }
            Ok(None) => false,
            Err(path) => {
                self.failures.push(path);
                false
            }
        }
    }
}

pub(super) fn page_cache_key(kind: PageKind, name: &str) -> (PageKind, String) {
    (kind, crate::refs::page_key(name))
}

pub(super) fn document_block_ref_counts(
    doc: &Document,
) -> io::Result<std::collections::HashMap<String, usize>> {
    let mut counts = std::collections::HashMap::new();
    let mut frames: [Option<std::slice::Iter<'_, DocBlock>>; MAX_BLOCK_DEPTH] =
        std::array::from_fn(|_| None);
    let mut len = usize::from(!doc.roots.is_empty());
    if len != 0 {
        frames[0] = Some(doc.roots.iter());
    }
    while len != 0 {
        let mut frame = frames[len - 1]
            .take()
            .expect("active document reference frame");
        let Some(block) = frame.next() else {
            len -= 1;
            continue;
        };
        frames[len - 1] = Some(frame);
        // projection().block_refs is already de-duplicated per referrer block,
        // matching the badge's OG-compatible counting semantics.
        for id in &block.projection().block_refs {
            let count = counts.entry(id.clone()).or_insert(0_usize);
            *count = count.checked_add(1).ok_or_else(allocation_overflow)?;
        }
        if !block.children.is_empty() {
            if len == MAX_BLOCK_DEPTH {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "cached document nesting exceeds 128 levels",
                ));
            }
            frames[len] = Some(block.children.iter());
            len += 1;
        }
    }
    Ok(counts)
}

pub(super) fn build_page_cache_index(pages: &[(PageEntry, Arc<Document>)]) -> PageCacheIndex {
    let mut by_name = std::collections::HashMap::with_capacity(pages.len());
    let mut by_path = std::collections::HashMap::with_capacity(pages.len());
    for (i, (entry, _)) in pages.iter().enumerate() {
        // Preserve Vec `.find` semantics if duplicates ever slip in: first wins.
        by_name
            .entry(page_cache_key(entry.kind, &entry.name))
            .or_insert(i);
        by_path.insert(entry.path.clone(), i);
    }
    PageCacheIndex { by_name, by_path }
}

pub(super) fn build_effective_identity_index(
    generation: u64,
    pages: &[(PageEntry, Arc<Document>)],
    failures: Vec<String>,
) -> EffectiveIdentityIndex {
    let mut owners = std::collections::HashMap::with_capacity(pages.len());
    let mut physical_paths = std::collections::HashSet::with_capacity(pages.len());
    for (entry, _) in pages {
        physical_paths.insert(entry.path.clone());
        owners
            .entry(page_cache_key(entry.kind, &entry.name))
            .or_insert_with(Vec::new)
            .push(entry.clone());
    }
    EffectiveIdentityIndex {
        generation: std::sync::atomic::AtomicU64::new(generation),
        owners,
        physical_paths,
        failures,
    }
}

pub(super) fn is_date_stem_entry(entry: &PageEntry) -> bool {
    entry
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| crate::date::JournalDate::from_file_stem(s).is_some())
}
