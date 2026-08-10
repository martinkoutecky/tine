//! Prototype spine of a fast trusted-local commit.
//!
//! An ordinary edit on a trusted local device does not need consensus, a
//! receipt, or a projection round trip before the user may keep typing. It
//! needs its base to still be current, one durable record of what it is about
//! to do, and the audited graph-text replacement that does it. This module is
//! that spine and nothing else:
//!
//! 1. stale/base validation against the committer's own trusted-local state;
//! 2. one canonical [`tine_storage::LocalJournalFrame`] append plus its single
//!    durability barrier;
//! 3. the existing audited guarded Markdown/Org replacement
//!    ([`Graph::save_page`]);
//! 4. direct return of the already-computed post-edit state.
//!
//! Nothing else is synchronous. SQLite, archive publication, receipt
//! construction, remote validation, and page reload are all absent, and
//! [`ForbiddenCommitWork`] makes that a measured fact rather than a claim.
//!
//! The journal payload is an *existing* encoding. A semantic effect is carried
//! in the canonical [`SemanticEffect`] encoding and a CRDT update is carried in
//! the engine's own exported update bytes. Its discriminator is explicitly
//! prototype-only: these incomplete frames are not the managed-local durable
//! record and cannot be replayed by that bridge.
//!
//! This is a narrowly scoped internal API. It is deliberately not wired into
//! the shipping save route yet: the runtime integration is a later lane's work,
//! and this lane's contract is to prove the spine can meet the ordinary-edit
//! latency budget first.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use tine_storage::{
    LocalJournalAppend, LocalJournalError, LocalJournalFrame, LocalJournalRecovery,
    LocalJournalSegment, LocalJournalStats,
};

use crate::oplog::semantic::{SemanticEffect, SemanticError};
use crate::{Graph, PageDto};

/// Directory, relative to a committer's journal root, that holds per-device
/// segments. Versioned so a future frame layout can coexist during migration.
pub const FAST_COMMIT_JOURNAL_DIR: &str = "fast-commit-journal-v1";

/// Structural work an ordinary fast commit must never perform.
///
/// Each field is incremented at the *real* boundary — the SQLite tail drain, an
/// archive object read, a projection receipt load, a graph-wide catalog decode,
/// and an application page load — so asserting that a commit leaves them at
/// zero is a statement about reachable code, not about this module's own
/// bookkeeping.
///
/// The counters are thread-scoped, matching the rest of the oplog's
/// instrumentation: each of those boundaries is reached on the thread that
/// requested the work, and a fast commit is entirely synchronous on its
/// caller's thread.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForbiddenCommitWork {
    /// Accepted events drained from the tail overlay into SQLite.
    pub sqlite_drains: usize,
    /// Accepted archive objects read out of the object store.
    pub archive_object_reads: usize,
    /// Completed projection receipts loaded from the projection store.
    pub projection_receipt_loads: usize,
    /// Whole page-catalog CRDT documents decoded out of scratch.
    pub graph_wide_catalog_decodes: usize,
    /// Application page DTOs rebuilt from graph text.
    pub application_page_loads: usize,
}

impl ForbiddenCommitWork {
    /// Work performed between two observations.
    pub const fn since(self, earlier: Self) -> Self {
        Self {
            sqlite_drains: self.sqlite_drains - earlier.sqlite_drains,
            archive_object_reads: self.archive_object_reads - earlier.archive_object_reads,
            projection_receipt_loads: self.projection_receipt_loads
                - earlier.projection_receipt_loads,
            graph_wide_catalog_decodes: self.graph_wide_catalog_decodes
                - earlier.graph_wide_catalog_decodes,
            application_page_loads: self.application_page_loads - earlier.application_page_loads,
        }
    }

    pub const fn is_none(self) -> bool {
        self.sqlite_drains == 0
            && self.archive_object_reads == 0
            && self.projection_receipt_loads == 0
            && self.graph_wide_catalog_decodes == 0
            && self.application_page_loads == 0
    }
}

thread_local! {
    static FORBIDDEN_COMMIT_WORK: Cell<ForbiddenCommitWork> = const {
        Cell::new(ForbiddenCommitWork {
            sqlite_drains: 0,
            archive_object_reads: 0,
            projection_receipt_loads: 0,
            graph_wide_catalog_decodes: 0,
            application_page_loads: 0,
        })
    };
}

/// This thread's running count of structural work a fast commit forbids.
pub fn forbidden_commit_work() -> ForbiddenCommitWork {
    FORBIDDEN_COMMIT_WORK.with(Cell::get)
}

fn note_forbidden(select: impl FnOnce(&mut ForbiddenCommitWork)) {
    FORBIDDEN_COMMIT_WORK.with(|counters| {
        let mut current = counters.get();
        select(&mut current);
        counters.set(current);
    });
}

pub(crate) fn note_sqlite_drain() {
    note_forbidden(|counters| counters.sqlite_drains = counters.sqlite_drains.saturating_add(1));
}

pub(crate) fn note_archive_object_read() {
    note_forbidden(|counters| {
        counters.archive_object_reads = counters.archive_object_reads.saturating_add(1);
    });
}

pub(crate) fn note_projection_receipt_load() {
    note_forbidden(|counters| {
        counters.projection_receipt_loads = counters.projection_receipt_loads.saturating_add(1);
    });
}

pub(crate) fn note_graph_wide_catalog_decode() {
    note_forbidden(|counters| {
        counters.graph_wide_catalog_decodes = counters.graph_wide_catalog_decodes.saturating_add(1);
    });
}

pub(crate) fn note_application_page_load() {
    note_forbidden(|counters| {
        counters.application_page_loads = counters.application_page_loads.saturating_add(1);
    });
}

/// Whole-graph work an ordinary edit performs but has no latency budget for.
///
/// The fast commit's own spine performs none of it. The audited guarded
/// replacement consults the retained graph-text identity generation, so these
/// counters permanently prove that an ordinary warm save did not fall back to
/// the complete inventory used after watcher uncertainty. Counting both scans
/// and entries turns that contract into an exact structural fact at every graph
/// size.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GraphWideCommitWork {
    /// Whole-graph text inventories derived.
    pub text_inventory_scans: usize,
    /// Graph text entries those inventories visited.
    pub text_inventory_entries: usize,
    /// Complete effective-identity indexes rebuilt from the parsed page cache.
    pub effective_identity_rebuilds: usize,
    /// Parsed page-cache entries visited by complete effective-identity rebuilds.
    pub effective_identity_entries: usize,
    /// Complete real-page-name maps materialized from the reference index.
    pub real_page_name_materializations: usize,
    /// Real page names copied into complete materialized maps.
    pub real_page_names_materialized: usize,
}

impl GraphWideCommitWork {
    /// Work performed between two observations.
    pub const fn since(self, earlier: Self) -> Self {
        Self {
            text_inventory_scans: self.text_inventory_scans - earlier.text_inventory_scans,
            text_inventory_entries: self.text_inventory_entries - earlier.text_inventory_entries,
            effective_identity_rebuilds: self.effective_identity_rebuilds
                - earlier.effective_identity_rebuilds,
            effective_identity_entries: self.effective_identity_entries
                - earlier.effective_identity_entries,
            real_page_name_materializations: self.real_page_name_materializations
                - earlier.real_page_name_materializations,
            real_page_names_materialized: self.real_page_names_materialized
                - earlier.real_page_names_materialized,
        }
    }
}

thread_local! {
    static GRAPH_WIDE_COMMIT_WORK: Cell<GraphWideCommitWork> = const {
        Cell::new(GraphWideCommitWork {
            text_inventory_scans: 0,
            text_inventory_entries: 0,
            effective_identity_rebuilds: 0,
            effective_identity_entries: 0,
            real_page_name_materializations: 0,
            real_page_names_materialized: 0,
        })
    };
}

/// This thread's running count of whole-graph work performed on the edit path.
pub fn graph_wide_commit_work() -> GraphWideCommitWork {
    GRAPH_WIDE_COMMIT_WORK.with(Cell::get)
}

pub(crate) fn note_graph_text_inventory(entries: usize) {
    GRAPH_WIDE_COMMIT_WORK.with(|counters| {
        let mut current = counters.get();
        current.text_inventory_scans = current.text_inventory_scans.saturating_add(1);
        current.text_inventory_entries = current.text_inventory_entries.saturating_add(entries);
        counters.set(current);
    });
}

pub(crate) fn note_effective_identity_rebuild(entries: usize) {
    GRAPH_WIDE_COMMIT_WORK.with(|counters| {
        let mut current = counters.get();
        current.effective_identity_rebuilds = current.effective_identity_rebuilds.saturating_add(1);
        current.effective_identity_entries =
            current.effective_identity_entries.saturating_add(entries);
        counters.set(current);
    });
}

pub(crate) fn note_real_page_name_materialization(entries: usize) {
    GRAPH_WIDE_COMMIT_WORK.with(|counters| {
        let mut current = counters.get();
        current.real_page_name_materializations =
            current.real_page_name_materializations.saturating_add(1);
        current.real_page_names_materialized =
            current.real_page_names_materialized.saturating_add(entries);
        counters.set(current);
    });
}

/// The already-derived effect of one edit, in an existing encoding.
#[derive(Clone, Copy, Debug)]
pub enum FastCommitIntent<'a> {
    /// The canonical semantic-effect encoding.
    SemanticEffect(&'a SemanticEffect),
    /// Engine-exported CRDT update bytes.
    CrdtUpdate(&'a [u8]),
}

/// Discriminator for this unpublished latency prototype's incomplete payloads.
/// These are deliberately not managed-local records and cannot be replayed by
/// the production record bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FastCommitPrototypePayloadKind {
    SemanticEffectV1,
    CrdtUpdateOpaque,
}

impl FastCommitIntent<'_> {
    /// The frame's payload kind and bytes. A CRDT update is already encoded, so
    /// it is journalled without a copy.
    fn journal_payload(
        &self,
    ) -> Result<(FastCommitPrototypePayloadKind, Cow<'_, [u8]>), FastCommitError> {
        match self {
            Self::SemanticEffect(effect) => Ok((
                FastCommitPrototypePayloadKind::SemanticEffectV1,
                Cow::Owned(effect.encode()?),
            )),
            Self::CrdtUpdate(bytes) => Ok((
                FastCommitPrototypePayloadKind::CrdtUpdateOpaque,
                Cow::Borrowed(bytes),
            )),
        }
    }
}

/// A journalled intent recovered from a frame, decoded back to its typed form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveredCommitIntent {
    SemanticEffect(SemanticEffect),
    CrdtUpdate(Vec<u8>),
}

/// Decode one recovered frame back into the typed payload that was journalled.
pub fn recover_commit_intent(
    frame: &LocalJournalFrame<FastCommitPrototypePayloadKind>,
) -> Result<RecoveredCommitIntent, FastCommitError> {
    match frame.payload_kind() {
        FastCommitPrototypePayloadKind::SemanticEffectV1 => Ok(
            RecoveredCommitIntent::SemanticEffect(SemanticEffect::decode(frame.payload())?),
        ),
        FastCommitPrototypePayloadKind::CrdtUpdateOpaque => {
            Ok(RecoveredCommitIntent::CrdtUpdate(frame.payload().to_vec()))
        }
    }
}

/// Where one fast commit spent its time, by contract step.
///
/// This is permanent, always-on accounting rather than a temporary probe. Four
/// clock reads cost tens of nanoseconds against a millisecond-scale durable
/// operation, and a latency-contracted path that cannot say which of its own
/// steps regressed is not one anybody can defend later.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FastCommitTimings {
    /// Stale/base validation.
    pub validation: Duration,
    /// Payload encode, frame encode, append, and the durability barrier.
    pub journal: Duration,
    /// The existing audited guarded Markdown/Org replacement.
    pub replacement: Duration,
    /// Retaining the new base and attaching it to the returned page.
    pub publish: Duration,
}

impl FastCommitTimings {
    pub const fn total(self) -> Duration {
        self.validation
            .saturating_add(self.journal)
            .saturating_add(self.replacement)
            .saturating_add(self.publish)
    }
}

/// The post-edit state a fast commit returns to its caller.
///
/// The page is the caller's own already-computed post-edit page with its new
/// revision attached. Nothing is re-read to produce it.
#[derive(Clone, Debug)]
pub struct FastCommitOutcome {
    pub page: PageDto,
    pub journal: LocalJournalAppend,
    pub timings: FastCommitTimings,
}

/// A failure at the fast trusted-local commit boundary.
#[derive(Debug)]
pub enum FastCommitError {
    /// A fast commit replaces a specific known file; a page with no path would
    /// have to be resolved by name, which is not a trusted-local fast path.
    UnpinnedPage,
    /// The committer has no trusted-local base for this page, so it cannot
    /// decide staleness without reading the graph.
    UntrackedPage(String),
    StaleBase {
        path: String,
        expected: String,
        offered: String,
    },
    Journal(LocalJournalError),
    Semantic(SemanticError),
    Io(io::Error),
}

impl fmt::Display for FastCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnpinnedPage => formatter.write_str("a fast commit requires a path-pinned page"),
            Self::UntrackedPage(path) => {
                write!(formatter, "no trusted-local base is retained for {path}")
            }
            Self::StaleBase {
                path,
                expected,
                offered,
            } => write!(
                formatter,
                "stale base for {path}: expected {expected}, offered {offered}"
            ),
            Self::Journal(error) => error.fmt(formatter),
            Self::Semantic(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FastCommitError {}

impl From<LocalJournalError> for FastCommitError {
    fn from(error: LocalJournalError) -> Self {
        Self::Journal(error)
    }
}

impl From<SemanticError> for FastCommitError {
    fn from(error: SemanticError) -> Self {
        Self::Semantic(error)
    }
}

impl From<io::Error> for FastCommitError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A device-local fast committer over one graph and one journal segment.
pub struct FastLocalCommitter {
    graph: Arc<Graph>,
    segment: LocalJournalSegment<FastCommitPrototypePayloadKind>,
    /// Graph-relative page path to the revision this device last committed.
    /// This is the trusted-local base; the audited replacement still proves the
    /// same revision against the file itself.
    base_revisions: HashMap<String, String>,
}

impl FastLocalCommitter {
    /// Open the committer's journal segment under `journal_root`, adopting any
    /// frames a previous process left behind.
    pub fn open(
        graph: Arc<Graph>,
        journal_root: &Path,
        device_id: Uuid,
    ) -> Result<(Self, LocalJournalRecovery<FastCommitPrototypePayloadKind>), FastCommitError> {
        let directory = journal_root.join(FAST_COMMIT_JOURNAL_DIR);
        std::fs::create_dir_all(&directory)?;
        let directory = Dir::open_ambient_dir(&directory, ambient_authority())?;
        let name = format!("{}.journal", device_id.simple());
        let (segment, recovery) = LocalJournalSegment::open(&directory, &name, device_id)?;
        Ok((
            Self {
                graph,
                segment,
                base_revisions: HashMap::new(),
            },
            recovery,
        ))
    }

    pub fn graph(&self) -> &Arc<Graph> {
        &self.graph
    }

    pub const fn journal_stats(&self) -> LocalJournalStats {
        self.segment.stats()
    }

    pub const fn next_sequence(&self) -> u64 {
        self.segment.next_sequence()
    }

    /// Stream every committed journal frame in append order.
    pub fn replay_journal(
        &self,
        visit: impl FnMut(LocalJournalFrame<FastCommitPrototypePayloadKind>),
    ) -> Result<u64, FastCommitError> {
        Ok(self.segment.replay(visit)?)
    }

    /// Adopt a freshly loaded page as this device's trusted-local base.
    ///
    /// The page must carry the path and revision it was loaded with; that pair
    /// is exactly what the audited replacement will re-prove against the file.
    pub fn adopt_loaded_page(&mut self, page: &PageDto) -> Result<(), FastCommitError> {
        if page.path.is_empty() {
            return Err(FastCommitError::UnpinnedPage);
        }
        let revision = page
            .rev
            .clone()
            .ok_or_else(|| FastCommitError::UntrackedPage(page.path.clone()))?;
        self.base_revisions.insert(page.path.clone(), revision);
        Ok(())
    }

    /// This device's trusted-local base revision for `path`, if any.
    pub fn base_revision(&self, path: &str) -> Option<&str> {
        self.base_revisions.get(path).map(String::as_str)
    }

    /// Commit one ordinary edit.
    ///
    /// `page` is the caller's already-computed post-edit page; it is returned
    /// with its new revision rather than reloaded. `base_rev` is the revision
    /// the edit was derived from. `intent` is the edit's already-derived effect
    /// in an existing encoding, which is what the journal frame carries.
    pub fn commit(
        &mut self,
        page: PageDto,
        base_rev: &str,
        intent: FastCommitIntent<'_>,
    ) -> Result<FastCommitOutcome, FastCommitError> {
        // 1. Stale/base validation, before anything durable happens.
        let started = Instant::now();
        if page.path.is_empty() {
            return Err(FastCommitError::UnpinnedPage);
        }
        match self.base_revisions.get(&page.path) {
            Some(retained) if retained == base_rev => {}
            Some(retained) => {
                return Err(FastCommitError::StaleBase {
                    path: page.path.clone(),
                    expected: retained.clone(),
                    offered: base_rev.to_owned(),
                })
            }
            None => return Err(FastCommitError::UntrackedPage(page.path.clone())),
        }
        let validated = Instant::now();

        // 2. One canonical journal append plus its single durability barrier.
        let (kind, payload) = intent.journal_payload()?;
        let journal = self.segment.append(kind, payload.as_ref())?;
        let journalled = Instant::now();

        // 3. The existing audited guarded Markdown/Org replacement.
        let revision = self.graph.save_page(&page, Some(base_rev))?;
        let replaced = Instant::now();

        // 4. Direct return of the already-computed post-edit state.
        self.base_revisions
            .insert(page.path.clone(), revision.clone());
        let mut page = page;
        page.rev = Some(revision);
        Ok(FastCommitOutcome {
            page,
            journal,
            timings: FastCommitTimings {
                validation: validated.duration_since(started),
                journal: journalled.duration_since(validated),
                replacement: replaced.duration_since(journalled),
                publish: replaced.elapsed(),
            },
        })
    }
}

#[cfg(test)]
mod fixtures {
    //! Shared synthetic-graph builders for the fast-commit proofs and the
    //! release benchmark. Both surfaces use exactly these builders so a
    //! correctness proof and a latency receipt describe the same graph.

    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use tine_storage::ContentDigest;
    use uuid::Uuid;

    use super::{FastCommitError, FastCommitPrototypePayloadKind, FastLocalCommitter};
    use crate::oplog::semantic::{BlockDelta, BlockOwner, BlockState, SemanticEffect};
    use crate::oplog::{BlockId, DocumentId, PageId};
    use crate::{Graph, PageDto, PageKind};

    pub(super) const DEFAULT_BLOCKS_PER_PAGE: usize = 10;
    pub(super) const PAGE_STEM: &str = "Fast-Commit";

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(super) enum FixtureFormat {
        Markdown,
        Org,
    }

    impl FixtureFormat {
        pub(super) const fn extension(self) -> &'static str {
            match self {
                Self::Markdown => "md",
                Self::Org => "org",
            }
        }

        pub(super) const fn label(self) -> &'static str {
            match self {
                Self::Markdown => "markdown",
                Self::Org => "org",
            }
        }

        /// One page's on-disk source. Both formats carry the same semantic
        /// shape — a task marker, a page reference, and a tag — so a
        /// differential between them is about the format, not the content.
        pub(super) fn page_source(self, page: usize, blocks_per_page: usize) -> String {
            let neighbour = page.saturating_sub(1);
            let mut source = String::new();
            for block in 0..blocks_per_page {
                let marker = if block % 2 == 0 { "TODO " } else { "" };
                let text = format!(
                    "{marker}block {page}-{block} references [[{PAGE_STEM}-{neighbour}]] and #fast-tag"
                );
                match self {
                    Self::Markdown => source.push_str(&format!("- {text}\n")),
                    Self::Org => source.push_str(&format!("* {text}\n")),
                }
            }
            source
        }
    }

    /// One synthetic graph on disk plus its opened Direct Files backend.
    pub(super) struct GraphFixture {
        pub(super) root: PathBuf,
        pub(super) graph_root: PathBuf,
        pub(super) journal_root: PathBuf,
        pub(super) graph: Arc<Graph>,
    }

    impl GraphFixture {
        /// Build a `pages`-page graph of `blocks_per_page` blocks each under
        /// `base`, then open it. `base` selects the filesystem under test.
        pub(super) fn build(
            base: &Path,
            label: &str,
            pages: usize,
            blocks_per_page: usize,
            format: FixtureFormat,
        ) -> Self {
            assert!(pages > 0 && blocks_per_page > 0);
            let root = base.join(format!(
                "tine-fast-commit-{label}-{}",
                Uuid::new_v4().simple()
            ));
            let graph_root = root.join("graph");
            let pages_dir = graph_root.join("pages");
            fs::create_dir_all(&pages_dir).unwrap();
            fs::create_dir_all(graph_root.join("journals")).unwrap();
            for page in 0..pages {
                fs::write(
                    pages_dir.join(format!("{PAGE_STEM}-{page}.{}", format.extension())),
                    format.page_source(page, blocks_per_page),
                )
                .unwrap();
            }
            let journal_root = root.join("private");
            fs::create_dir_all(&journal_root).unwrap();
            let graph =
                Arc::new(Graph::open_checked(&graph_root).expect("the synthetic graph opens"));
            Self {
                root,
                graph_root,
                journal_root,
                graph,
            }
        }

        pub(super) fn page_name(&self, page: usize) -> String {
            format!("{PAGE_STEM}-{page}")
        }

        pub(super) fn load(&self, page: usize) -> PageDto {
            self.graph
                .load_named(&self.page_name(page), PageKind::Page)
                .expect("a synthetic page loads")
                .expect("the synthetic page exists")
        }

        /// Reopen the same graph bytes through a brand new backend.
        pub(super) fn reopen(&self) -> Arc<Graph> {
            Arc::new(Graph::open_checked(&self.graph_root).expect("the synthetic graph reopens"))
        }

        pub(super) fn committer(&self, device: Uuid) -> FastLocalCommitter {
            let (committer, recovery) =
                FastLocalCommitter::open(Arc::clone(&self.graph), &self.journal_root, device)
                    .expect("the fast committer opens");
            assert_eq!(recovery.discarded_tail_bytes, 0);
            committer
        }

        pub(super) fn committer_over(
            &self,
            graph: Arc<Graph>,
            device: Uuid,
        ) -> Result<
            (
                FastLocalCommitter,
                tine_storage::LocalJournalRecovery<FastCommitPrototypePayloadKind>,
            ),
            FastCommitError,
        > {
            FastLocalCommitter::open(graph, &self.journal_root, device)
        }

        /// Content digest of every graph text file, keyed by graph-relative path.
        pub(super) fn text_digests(&self) -> BTreeMap<String, ContentDigest> {
            let mut digests = BTreeMap::new();
            collect_text_digests(&self.graph_root, &self.graph_root, &mut digests);
            digests
        }

        pub(super) fn journal_segment_path(&self, device: Uuid) -> PathBuf {
            self.journal_root
                .join(super::FAST_COMMIT_JOURNAL_DIR)
                .join(format!("{}.journal", device.simple()))
        }
    }

    impl Drop for GraphFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn collect_text_digests(
        root: &Path,
        directory: &Path,
        into: &mut BTreeMap<String, ContentDigest>,
    ) {
        let mut entries: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect_text_digests(root, &path, into);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                into.insert(relative, ContentDigest::of(&fs::read(&path).unwrap()));
            }
        }
    }

    /// The post-edit page for one ordinary content edit, plus the semantic
    /// effect that describes it in the existing canonical encoding.
    pub(super) fn content_edit(
        page: &PageDto,
        page_index: usize,
        block_index: usize,
        generation: usize,
    ) -> (PageDto, SemanticEffect) {
        let mut edited = page.clone();
        let block = edited
            .blocks
            .get_mut(block_index)
            .expect("the synthetic page has this block");
        let before = block.raw.clone();
        let after = format!("{before} revision {generation}");
        block.raw = after.clone();

        let page_id = PageId::from_uuid(Uuid::from_u128(0x_fa57_0000 + page_index as u128));
        let home_document_id =
            DocumentId::from_uuid(Uuid::from_u128(0x_d0c0_0000 + page_index as u128));
        let block_id = BlockId::from_uuid(Uuid::from_u128(
            0x_b10c_0000 + (page_index * 1024 + block_index) as u128,
        ));
        let state = |content: &str| BlockState {
            block_id,
            home_document_id,
            owner: BlockOwner::Page(page_id),
            logseq_uuid: None,
            logseq_identity_origin: None,
            content: content.to_owned(),
        };
        let effect = SemanticEffect::new(
            Vec::new(),
            vec![BlockDelta {
                block_id,
                home_document_id,
                before: Some(state(&before)),
                after: Some(state(&after)),
            }],
            Vec::new(),
        )
        .expect("a one-block content edit is a valid semantic effect");
        (edited, effect)
    }

    /// The semantics a proof compares: what the page says, not how it is stored.
    pub(super) fn page_semantics(
        page: &PageDto,
    ) -> (String, PageKind, Option<String>, Vec<String>) {
        fn flatten(blocks: &[crate::BlockDto], into: &mut Vec<String>) {
            for block in blocks {
                into.push(block.raw.clone());
                flatten(&block.children, into);
            }
        }
        let mut blocks = Vec::new();
        flatten(&page.blocks, &mut blocks);
        (page.name.clone(), page.kind, page.pre_block.clone(), blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{
        content_edit, page_semantics, FixtureFormat, GraphFixture, DEFAULT_BLOCKS_PER_PAGE,
    };
    use super::*;

    use std::fs;

    use crate::model::content_rev;
    use crate::PageKind;

    fn every_format() -> [FixtureFormat; 2] {
        [FixtureFormat::Markdown, FixtureFormat::Org]
    }

    fn overlay_base() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    fn journal_frames(
        committer: &FastLocalCommitter,
    ) -> Vec<LocalJournalFrame<FastCommitPrototypePayloadKind>> {
        let mut frames = Vec::new();
        committer
            .replay_journal(|frame| frames.push(frame))
            .unwrap();
        frames
    }

    #[test]
    fn a_fast_commit_journals_one_frame_and_replaces_exactly_one_file() {
        for format in every_format() {
            let fixture = GraphFixture::build(
                &overlay_base(),
                "one-file",
                6,
                DEFAULT_BLOCKS_PER_PAGE,
                format,
            );
            let device = Uuid::from_u128(0x_fa57_c0de);
            let mut committer = fixture.committer(device);
            let loaded = fixture.load(2);
            assert!(
                !loaded.read_only,
                "the {} fixture must be editable, so the audited replacement can run",
                format.label()
            );
            committer.adopt_loaded_page(&loaded).unwrap();

            let before_digests = fixture.text_digests();
            let base_rev = loaded.rev.clone().unwrap();
            let (edited, effect) = content_edit(&loaded, 2, 3, 1);
            let before_syncs = committer.journal_stats().data_durability_syncs;
            let outcome = committer
                .commit(
                    edited.clone(),
                    &base_rev,
                    FastCommitIntent::SemanticEffect(&effect),
                )
                .unwrap();

            // One durable journal record, one durability barrier.
            assert_eq!(outcome.journal.sequence, 0);
            assert_eq!(outcome.journal.data_durability_syncs, 1);
            assert_eq!(
                committer.journal_stats().data_durability_syncs - before_syncs,
                1,
                "an ordinary commit performs exactly one durability barrier"
            );

            // Exactly one graph file changed.
            let after_digests = fixture.text_digests();
            let changed: Vec<_> = after_digests
                .iter()
                .filter(|(path, digest)| before_digests.get(*path) != Some(*digest))
                .map(|(path, _)| path.clone())
                .collect();
            assert_eq!(
                changed,
                vec![format!("pages/Fast-Commit-2.{}", format.extension())],
                "exactly the edited page's file may change"
            );
            assert_eq!(
                before_digests.keys().collect::<Vec<_>>(),
                after_digests.keys().collect::<Vec<_>>(),
                "no graph file may be created or removed"
            );

            // The returned state is the caller's post-edit page with its new
            // revision, and that revision is the file's actual content.
            assert_eq!(page_semantics(&outcome.page), page_semantics(&edited));
            let on_disk = fs::read_to_string(
                fixture
                    .graph_root
                    .join(format!("pages/Fast-Commit-2.{}", format.extension())),
            )
            .unwrap();
            assert_eq!(outcome.page.rev, Some(content_rev(&on_disk)));
            assert_ne!(outcome.page.rev, loaded.rev);

            // The frame decodes to exactly the typed payload that was committed.
            let frames = journal_frames(&committer);
            assert_eq!(frames.len(), 1);
            assert_eq!(frames[0].sequence(), 0);
            assert_eq!(frames[0].device_id(), device);
            assert_eq!(
                frames[0].payload_kind(),
                FastCommitPrototypePayloadKind::SemanticEffectV1
            );
            assert_eq!(
                recover_commit_intent(&frames[0]).unwrap(),
                RecoveredCommitIntent::SemanticEffect(effect)
            );
        }
    }

    #[test]
    fn a_fast_commit_performs_no_sqlite_archive_receipt_catalog_or_page_reload_work() {
        for format in every_format() {
            let fixture = GraphFixture::build(
                &overlay_base(),
                "structural",
                8,
                DEFAULT_BLOCKS_PER_PAGE,
                format,
            );
            let mut committer = fixture.committer(Uuid::from_u128(0x_5747_0001));
            let mut current = fixture.load(1);
            committer.adopt_loaded_page(&current).unwrap();

            for generation in 1..=5 {
                let base_rev = current.rev.clone().unwrap();
                let (edited, effect) = content_edit(
                    &current,
                    1,
                    generation % DEFAULT_BLOCKS_PER_PAGE,
                    generation,
                );
                let before = forbidden_commit_work();
                let outcome = committer
                    .commit(edited, &base_rev, FastCommitIntent::SemanticEffect(&effect))
                    .unwrap();
                let performed = forbidden_commit_work().since(before);
                assert!(
                    performed.is_none(),
                    "a fast commit must perform no SQLite drain, archive read, projection \
                     receipt load, graph-wide catalog decode, or application page reload, \
                     but generation {generation} on {} performed {performed:?}",
                    format.label()
                );
                current = outcome.page;
            }

            // The counters are live, not dead code: the same thread's ordinary
            // application page load is counted.
            let before = forbidden_commit_work();
            let reloaded = fixture.load(1);
            let performed = forbidden_commit_work().since(before);
            assert_eq!(
                performed.application_page_loads, 1,
                "the structural counters must observe a real application page load"
            );
            assert_eq!(
                page_semantics(&reloaded),
                page_semantics(&current),
                "the projected {} page must reparse to the intended semantic page",
                format.label()
            );
        }
    }

    #[test]
    fn warm_guarded_saves_do_identical_work_at_four_and_four_hundred_pages() {
        let mut observations = Vec::new();
        for pages in [4_usize, 400] {
            let fixture = GraphFixture::build(
                &overlay_base(),
                "spine-scaling",
                pages,
                DEFAULT_BLOCKS_PER_PAGE,
                FixtureFormat::Markdown,
            );
            let mut committer = fixture.committer(Uuid::from_u128(0x_5747_0006));
            // The same page index at both sizes, so the journalled payload is
            // byte-identical and any difference is a scaling effect.
            let mut current = fixture.load(1);
            committer.adopt_loaded_page(&current).unwrap();

            let mut warm_work = Vec::new();
            for generation in 1..=3 {
                let base_rev = current.rev.clone().unwrap();
                let (edited, effect) = content_edit(&current, 1, 0, generation);
                let before = forbidden_commit_work();
                let graph_wide_before = graph_wide_commit_work();
                current = committer
                    .commit(edited, &base_rev, FastCommitIntent::SemanticEffect(&effect))
                    .unwrap()
                    .page;
                if generation > 1 {
                    warm_work.push((
                        forbidden_commit_work().since(before),
                        graph_wide_commit_work().since(graph_wide_before),
                    ));
                }
            }
            observations.push((
                pages,
                committer.journal_stats(),
                warm_work,
                fixture.graph.guarded_graph_text_identity_stats(),
            ));
        }

        let [(_, small_journal, small_work, small_index), (_, large_journal, large_work, large_index)] =
            observations.as_slice()
        else {
            unreachable!("two observations")
        };

        // The spine: identical at both graph sizes.
        assert_eq!(small_journal.frames_appended, 3);
        assert_eq!(
            small_journal, large_journal,
            "the journal spine must do identical work at every graph size"
        );
        assert_eq!(
            small_index.0, 0,
            "managed fast commits must not build the whole-graph cache"
        );
        assert_eq!(
            large_index.0, 0,
            "graph size cannot introduce a whole-graph cache build"
        );
        assert_eq!(
            small_index.1, 0,
            "managed saves must not maintain a whole-graph cache delta index"
        );
        assert_eq!(
            large_index.1, 0,
            "graph size cannot introduce whole-graph cache delta work"
        );
        assert!(small_work.iter().all(|(forbidden, graph_wide)| {
            forbidden.is_none() && *graph_wide == GraphWideCommitWork::default()
        }));
        assert_eq!(
            small_work, large_work,
            "every repeated warm replacement must do identical graph-size-invariant work"
        );
    }

    #[test]
    fn the_projected_page_reparses_and_a_fresh_reopen_sees_the_last_committed_edit() {
        for format in every_format() {
            let fixture = GraphFixture::build(
                &overlay_base(),
                "reopen",
                5,
                DEFAULT_BLOCKS_PER_PAGE,
                format,
            );
            let mut committer = fixture.committer(Uuid::from_u128(0x_5747_0002));
            let mut current = fixture.load(4);
            committer.adopt_loaded_page(&current).unwrap();
            for generation in 1..=3 {
                let base_rev = current.rev.clone().unwrap();
                let (edited, effect) = content_edit(&current, 4, 0, generation);
                current = committer
                    .commit(edited, &base_rev, FastCommitIntent::SemanticEffect(&effect))
                    .unwrap()
                    .page;
            }

            let reopened = fixture.reopen();
            let seen = reopened
                .load_named(&fixture.page_name(4), PageKind::Page)
                .unwrap()
                .expect("the committed page exists after a fresh reopen");
            assert_eq!(
                page_semantics(&seen),
                page_semantics(&current),
                "a fresh {} reopen must see the last committed edit",
                format.label()
            );
            assert_eq!(seen.rev, current.rev);
            assert!(seen.blocks[0].raw.ends_with("revision 3"));
        }
    }

    #[test]
    fn a_stale_or_untracked_base_is_refused_before_anything_durable_happens() {
        let fixture = GraphFixture::build(
            &overlay_base(),
            "stale",
            4,
            DEFAULT_BLOCKS_PER_PAGE,
            FixtureFormat::Markdown,
        );
        let mut committer = fixture.committer(Uuid::from_u128(0x_5747_0003));
        let loaded = fixture.load(0);
        let before_digests = fixture.text_digests();

        // Untracked: the committer holds no trusted-local base for this page.
        let (edited, effect) = content_edit(&loaded, 0, 0, 1);
        let base_rev = loaded.rev.clone().unwrap();
        assert!(matches!(
            committer.commit(
                edited.clone(),
                &base_rev,
                FastCommitIntent::SemanticEffect(&effect)
            ),
            Err(FastCommitError::UntrackedPage(_))
        ));

        committer.adopt_loaded_page(&loaded).unwrap();
        // Stale: the offered base is not the base this device last committed.
        assert!(matches!(
            committer.commit(
                edited,
                "0".repeat(64).as_str(),
                FastCommitIntent::SemanticEffect(&effect)
            ),
            Err(FastCommitError::StaleBase { .. })
        ));

        assert_eq!(committer.next_sequence(), 0, "no frame may be journalled");
        assert_eq!(committer.journal_stats().frames_appended, 0);
        assert_eq!(committer.journal_stats().data_durability_syncs, 0);
        assert_eq!(
            fixture.text_digests(),
            before_digests,
            "a refused commit may not change any graph file"
        );
    }

    #[test]
    fn a_crdt_update_intent_round_trips_through_the_journal() {
        let fixture = GraphFixture::build(
            &overlay_base(),
            "crdt-update",
            4,
            DEFAULT_BLOCKS_PER_PAGE,
            FixtureFormat::Markdown,
        );
        let device = Uuid::from_u128(0x_5747_0004);
        let mut committer = fixture.committer(device);
        let loaded = fixture.load(1);
        committer.adopt_loaded_page(&loaded).unwrap();

        // An engine-exported update is opaque bytes to the journal; it must come
        // back byte-identical and typed as a CRDT update. This is the engine's
        // own update encoding, not a second one invented here.
        let document = loro::LoroDoc::new();
        document
            .get_text("content")
            .insert(0, "fast commit crdt update")
            .unwrap();
        document.commit();
        let update_bytes = document.export(loro::ExportMode::all_updates()).unwrap();
        assert!(!update_bytes.is_empty());

        let base_rev = loaded.rev.clone().unwrap();
        let (edited, _) = content_edit(&loaded, 1, 2, 1);
        committer
            .commit(
                edited,
                &base_rev,
                FastCommitIntent::CrdtUpdate(&update_bytes),
            )
            .unwrap();

        let frames = journal_frames(&committer);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].payload_kind(),
            FastCommitPrototypePayloadKind::CrdtUpdateOpaque
        );
        assert_eq!(
            recover_commit_intent(&frames[0]).unwrap(),
            RecoveredCommitIntent::CrdtUpdate(update_bytes)
        );
    }

    #[test]
    fn a_torn_final_append_is_recovered_without_losing_earlier_commits() {
        let fixture = GraphFixture::build(
            &overlay_base(),
            "torn",
            4,
            DEFAULT_BLOCKS_PER_PAGE,
            FixtureFormat::Org,
        );
        let device = Uuid::from_u128(0x_5747_0005);
        let (segment_bytes, kept_bytes, final_len, committed_page) = {
            let mut committer = fixture.committer(device);
            let mut current = fixture.load(3);
            committer.adopt_loaded_page(&current).unwrap();
            for generation in 1..=3 {
                let base_rev = current.rev.clone().unwrap();
                let (edited, effect) = content_edit(&current, 3, 0, generation);
                current = committer
                    .commit(edited, &base_rev, FastCommitIntent::SemanticEffect(&effect))
                    .unwrap()
                    .page;
            }
            let kept_bytes = committer.journal_stats().bytes_appended;
            let base_rev = current.rev.clone().unwrap();
            let (edited, effect) = content_edit(&current, 3, 0, 4);
            let outcome = committer
                .commit(edited, &base_rev, FastCommitIntent::SemanticEffect(&effect))
                .unwrap();
            (
                fs::read(fixture.journal_segment_path(device)).unwrap(),
                kept_bytes,
                outcome.journal.frame_bytes,
                outcome.page,
            )
        };
        assert_eq!(segment_bytes.len() as u64, kept_bytes + final_len);

        // Tear the final append at every byte boundary: the three earlier
        // commits and their typed payloads must all survive, and the graph text
        // the completed commits published is untouched by journal recovery.
        for torn in 0..final_len as usize {
            fs::write(
                fixture.journal_segment_path(device),
                &segment_bytes[..kept_bytes as usize + torn],
            )
            .unwrap();
            let reopened = fixture.reopen();
            let (committer, recovery) = fixture
                .committer_over(Arc::clone(&reopened), device)
                .expect("a torn tail must not stop the journal from opening");
            assert_eq!(
                recovery.frames_recovered, 3,
                "tearing the fourth append at {torn} bytes must keep the first three"
            );
            assert_eq!(recovery.discarded_tail_bytes, torn as u64);
            let frames = journal_frames(&committer);
            assert_eq!(frames.len(), 3);
            for (index, frame) in frames.iter().enumerate() {
                assert_eq!(frame.sequence(), index as u64);
                assert!(matches!(
                    recover_commit_intent(frame).unwrap(),
                    RecoveredCommitIntent::SemanticEffect(_)
                ));
            }
            drop(committer);
            let seen = reopened
                .load_named(&fixture.page_name(3), PageKind::Page)
                .unwrap()
                .expect("the page survives journal recovery");
            assert_eq!(page_semantics(&seen), page_semantics(&committed_page));
        }
    }
}

#[cfg(test)]
mod benchmark {
    //! Permanent release receipt for the fast trusted-local commit.
    //!
    //! The hard receipt is a real local ext4/NVMe filesystem. A volatile
    //! overlay (`/tmp` on many Linux hosts) is measured as a diagnostic only:
    //! its `fsync` is nearly free, so it prices CPU and memory work rather than
    //! durability. Both surfaces are reported and labelled.
    //!
    //! ```text
    //! RUST_MIN_STACK=134217728 cargo test --release -p tine-core --lib \
    //!   fast_local_commit_latency_manual_release_benchmark -- --ignored --nocapture
    //! ```
    //!
    //! Environment overrides: `TINE_FAST_COMMIT_BENCH_PAGES` (comma-separated
    //! graph sizes), `_BLOCKS_PER_PAGE`, `_EDITS`, `_WARMUPS`, `_FORMATS`
    //! (`markdown`, `org`), `_EXT4_ROOT`, `_OVERLAY_ROOT`, `_SURFACES`
    //! (`ext4`, `overlay`).

    use super::fixtures::{
        content_edit, page_semantics, FixtureFormat, GraphFixture, DEFAULT_BLOCKS_PER_PAGE,
    };
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::PageKind;

    /// Contract from the performance dossier, in nanoseconds.
    const HARD_P50: Duration = Duration::from_millis(10);
    const TARGET_P50: Duration = Duration::from_millis(5);
    const HARD_P95: Duration = Duration::from_millis(20);
    /// The 10,000-page kill gate: p50 at the largest size versus the smallest.
    const SCALE_GATE: f64 = 2.0;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Surface {
        Ext4,
        Overlay,
    }

    impl Surface {
        const fn label(self) -> &'static str {
            match self {
                Self::Ext4 => "ext4",
                Self::Overlay => "overlay",
            }
        }

        /// Whether this surface's numbers are contractual or diagnostic.
        const fn is_receipt(self) -> bool {
            matches!(self, Self::Ext4)
        }
    }

    struct Configuration {
        surface: Surface,
        base: PathBuf,
        format: FixtureFormat,
        pages: usize,
        blocks_per_page: usize,
        warmups: usize,
        edits: usize,
    }

    struct Receipt {
        surface: Surface,
        format: FixtureFormat,
        pages: usize,
        samples: Vec<Duration>,
        /// Per-step samples, so a missed gate names the step that missed it.
        validation: Vec<Duration>,
        journal_phase: Vec<Duration>,
        replacement: Vec<Duration>,
        journal: LocalJournalStats,
        forbidden: ForbiddenCommitWork,
        graph_wide: GraphWideCommitWork,
        journal_bytes_per_commit: f64,
    }

    fn percentile_of(samples: &[Duration], fraction: f64) -> Duration {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    impl Receipt {
        fn percentile(&self, fraction: f64) -> Duration {
            let mut sorted = self.samples.clone();
            sorted.sort_unstable();
            let index = ((sorted.len() as f64 - 1.0) * fraction).round() as usize;
            sorted[index.min(sorted.len() - 1)]
        }

        fn p50(&self) -> Duration {
            self.percentile(0.50)
        }

        fn p95(&self) -> Duration {
            self.percentile(0.95)
        }

        fn min(&self) -> Duration {
            *self.samples.iter().min().expect("at least one sample")
        }

        fn max(&self) -> Duration {
            *self.samples.iter().max().expect("at least one sample")
        }

        fn mean(&self) -> Duration {
            self.samples.iter().sum::<Duration>() / self.samples.len() as u32
        }
    }

    fn milliseconds(duration: Duration) -> f64 {
        duration.as_secs_f64() * 1_000.0
    }

    fn environment_usize(name: &str, fallback: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(fallback)
    }

    fn graph_sizes() -> Vec<usize> {
        match std::env::var("TINE_FAST_COMMIT_BENCH_PAGES") {
            Ok(value) => {
                let sizes: Vec<usize> = value
                    .split(',')
                    .filter_map(|entry| entry.trim().parse::<usize>().ok())
                    .filter(|pages| *pages > 0)
                    .collect();
                assert!(!sizes.is_empty(), "TINE_FAST_COMMIT_BENCH_PAGES is empty");
                sizes
            }
            Err(_) => vec![100, 1_000, 10_000],
        }
    }

    fn formats() -> Vec<FixtureFormat> {
        match std::env::var("TINE_FAST_COMMIT_BENCH_FORMATS") {
            Ok(value) => value
                .split(',')
                .filter_map(|entry| match entry.trim() {
                    "markdown" | "md" => Some(FixtureFormat::Markdown),
                    "org" => Some(FixtureFormat::Org),
                    _ => None,
                })
                .collect(),
            Err(_) => vec![FixtureFormat::Markdown, FixtureFormat::Org],
        }
    }

    /// The workspace target directory, which lives on the repository's own
    /// filesystem. That is the real local disk on a development machine, which
    /// is what the contract's hard receipt requires.
    fn default_ext4_root() -> PathBuf {
        std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("..")
                    .join("target")
            })
            .join("fast-commit-bench")
    }

    fn surfaces() -> Vec<(Surface, PathBuf)> {
        let ext4 = std::env::var("TINE_FAST_COMMIT_BENCH_EXT4_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_ext4_root());
        let overlay = std::env::var("TINE_FAST_COMMIT_BENCH_OVERLAY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let selected = std::env::var("TINE_FAST_COMMIT_BENCH_SURFACES")
            .unwrap_or_else(|_| "ext4,overlay".to_owned());
        let mut chosen = Vec::new();
        for entry in selected.split(',') {
            match entry.trim() {
                "ext4" => chosen.push((Surface::Ext4, ext4.clone())),
                "overlay" => chosen.push((Surface::Overlay, overlay.clone())),
                _ => {}
            }
        }
        assert!(!chosen.is_empty(), "no benchmark surface was selected");
        chosen
    }

    /// Measure one configuration. Every sample is a complete fast commit: the
    /// stale check, the journal append and its durability barrier, the audited
    /// replacement, and the returned post-edit state. Nothing else is timed and
    /// nothing else is elided — each sample also proves it did the right thing.
    fn measure(configuration: &Configuration) -> Receipt {
        std::fs::create_dir_all(&configuration.base).unwrap();
        let fixture = GraphFixture::build(
            &configuration.base,
            &format!(
                "bench-{}-{}-{}",
                configuration.surface.label(),
                configuration.format.label(),
                configuration.pages
            ),
            configuration.pages,
            configuration.blocks_per_page,
            configuration.format,
        );
        // The edited page sits in the middle of the graph, so nothing about the
        // measurement depends on the target being the first or last entry.
        let page_index = configuration.pages / 2;
        let mut committer = fixture.committer(Uuid::from_u128(0x_be6c_4000));
        let mut current = fixture.load(page_index);
        assert!(
            !current.read_only,
            "the {} fixture must be editable",
            configuration.format.label()
        );
        committer.adopt_loaded_page(&current).unwrap();

        let before_digests = fixture.text_digests();
        let mut samples = Vec::with_capacity(configuration.edits);
        let mut validation = Vec::with_capacity(configuration.edits);
        let mut journal_phase = Vec::with_capacity(configuration.edits);
        let mut replacement = Vec::with_capacity(configuration.edits);
        let mut forbidden = ForbiddenCommitWork::default();
        let mut graph_wide = GraphWideCommitWork::default();
        let total = configuration.warmups + configuration.edits;
        for generation in 1..=total {
            let base_rev = current.rev.clone().unwrap();
            let (edited, effect) = content_edit(
                &current,
                page_index,
                generation % configuration.blocks_per_page,
                generation,
            );
            let expected = page_semantics(&edited);
            let intent = FastCommitIntent::SemanticEffect(&effect);

            let observed_before = forbidden_commit_work();
            let graph_wide_before = graph_wide_commit_work();
            let started = Instant::now();
            let outcome = committer.commit(edited, &base_rev, intent).unwrap();
            let elapsed = started.elapsed();
            let performed = forbidden_commit_work().since(observed_before);
            let performed_graph_wide = graph_wide_commit_work().since(graph_wide_before);

            assert_eq!(
                outcome.journal.data_durability_syncs, 1,
                "an ordinary fast commit performs exactly one durability barrier"
            );
            assert_eq!(
                page_semantics(&outcome.page),
                expected,
                "the returned state must be the caller's post-edit page"
            );
            assert!(outcome.page.rev.is_some());
            if generation > configuration.warmups {
                samples.push(elapsed);
                validation.push(outcome.timings.validation);
                journal_phase.push(outcome.timings.journal);
                replacement.push(outcome.timings.replacement);
                forbidden = ForbiddenCommitWork {
                    sqlite_drains: forbidden.sqlite_drains + performed.sqlite_drains,
                    archive_object_reads: forbidden.archive_object_reads
                        + performed.archive_object_reads,
                    projection_receipt_loads: forbidden.projection_receipt_loads
                        + performed.projection_receipt_loads,
                    graph_wide_catalog_decodes: forbidden.graph_wide_catalog_decodes
                        + performed.graph_wide_catalog_decodes,
                    application_page_loads: forbidden.application_page_loads
                        + performed.application_page_loads,
                };
                graph_wide = GraphWideCommitWork {
                    text_inventory_scans: graph_wide.text_inventory_scans
                        + performed_graph_wide.text_inventory_scans,
                    text_inventory_entries: graph_wide.text_inventory_entries
                        + performed_graph_wide.text_inventory_entries,
                    effective_identity_rebuilds: graph_wide.effective_identity_rebuilds
                        + performed_graph_wide.effective_identity_rebuilds,
                    effective_identity_entries: graph_wide.effective_identity_entries
                        + performed_graph_wide.effective_identity_entries,
                    real_page_name_materializations: graph_wide.real_page_name_materializations
                        + performed_graph_wide.real_page_name_materializations,
                    real_page_names_materialized: graph_wide.real_page_names_materialized
                        + performed_graph_wide.real_page_names_materialized,
                };
            }
            current = outcome.page;
        }

        // Exactly one file changed across the whole measured run, and the
        // reparsed page is the page the last commit returned.
        let after_digests = fixture.text_digests();
        let changed: Vec<_> = after_digests
            .iter()
            .filter(|(path, digest)| before_digests.get(*path) != Some(*digest))
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            changed,
            vec![format!(
                "pages/Fast-Commit-{page_index}.{}",
                configuration.format.extension()
            )],
            "only the edited page's file may change"
        );
        assert_eq!(
            before_digests.len(),
            after_digests.len(),
            "no graph file may be created or removed"
        );
        let reopened = fixture.reopen();
        let seen = reopened
            .load_named(&fixture.page_name(page_index), PageKind::Page)
            .unwrap()
            .expect("the committed page exists after a fresh reopen");
        assert_eq!(
            page_semantics(&seen),
            page_semantics(&current),
            "a fresh reopen must see the last committed edit"
        );

        let journal = committer.journal_stats();
        assert_eq!(journal.frames_appended, total as u64);
        assert_eq!(journal.data_durability_syncs, total as u64);
        assert_eq!(journal.directory_durability_syncs, 1);
        assert_eq!(journal.recovery_truncations, 0);
        Receipt {
            surface: configuration.surface,
            format: configuration.format,
            pages: configuration.pages,
            journal_bytes_per_commit: journal.bytes_appended as f64 / total as f64,
            samples,
            validation,
            journal_phase,
            replacement,
            journal,
            forbidden,
            graph_wide,
        }
    }

    #[test]
    #[ignore = "manual release benchmark: fast trusted-local commit latency at 100/1,000/10,000 pages"]
    fn fast_local_commit_latency_manual_release_benchmark() {
        assert!(
            !cfg!(debug_assertions),
            "this receipt is release-only; run cargo test --release -p tine-core --lib \
             fast_local_commit_latency_manual_release_benchmark -- --ignored --nocapture"
        );
        let sizes = graph_sizes();
        let formats = formats();
        let blocks_per_page = environment_usize(
            "TINE_FAST_COMMIT_BENCH_BLOCKS_PER_PAGE",
            DEFAULT_BLOCKS_PER_PAGE,
        );
        let edits = environment_usize("TINE_FAST_COMMIT_BENCH_EDITS", 100);
        let warmups = environment_usize("TINE_FAST_COMMIT_BENCH_WARMUPS", 10);

        let mut receipts = Vec::new();
        for (surface, base) in surfaces() {
            for format in &formats {
                for pages in &sizes {
                    let receipt = measure(&Configuration {
                        surface,
                        base: base.clone(),
                        format: *format,
                        pages: *pages,
                        blocks_per_page,
                        warmups,
                        edits,
                    });
                    eprintln!(
                        "fast_commit surface={} format={} pages={} blocks_per_page={} warmups={} edits={} \
                         p50_ms={:.3} p95_ms={:.3} min_ms={:.3} max_ms={:.3} mean_ms={:.3} \
                         durability_syncs_per_commit={:.3} directory_syncs={} journal_bytes_per_commit={:.1} \
                         forbidden={:?}",
                        receipt.surface.label(),
                        receipt.format.label(),
                        receipt.pages,
                        blocks_per_page,
                        warmups,
                        receipt.samples.len(),
                        milliseconds(receipt.p50()),
                        milliseconds(receipt.p95()),
                        milliseconds(receipt.min()),
                        milliseconds(receipt.max()),
                        milliseconds(receipt.mean()),
                        receipt.journal.data_durability_syncs as f64
                            / receipt.journal.frames_appended as f64,
                        receipt.journal.directory_durability_syncs,
                        receipt.journal_bytes_per_commit,
                        receipt.forbidden,
                    );
                    // Per-step attribution, so a missed gate names the step that
                    // missed it instead of leaving the cause to be guessed.
                    eprintln!(
                        "fast_commit_steps surface={} format={} pages={} \
                         validation_p50_ms={:.4} journal_p50_ms={:.4} replacement_p50_ms={:.4} \
                         journal_p95_ms={:.4} replacement_p95_ms={:.4} \
                         graph_text_inventories_per_commit={:.3} \
                         graph_text_entries_visited_per_commit={:.1}",
                        receipt.surface.label(),
                        receipt.format.label(),
                        receipt.pages,
                        milliseconds(percentile_of(&receipt.validation, 0.50)),
                        milliseconds(percentile_of(&receipt.journal_phase, 0.50)),
                        milliseconds(percentile_of(&receipt.replacement, 0.50)),
                        milliseconds(percentile_of(&receipt.journal_phase, 0.95)),
                        milliseconds(percentile_of(&receipt.replacement, 0.95)),
                        receipt.graph_wide.text_inventory_scans as f64
                            / receipt.samples.len() as f64,
                        receipt.graph_wide.text_inventory_entries as f64
                            / receipt.samples.len() as f64,
                    );
                    // Raw samples, so the receipt can be re-derived rather than
                    // trusted.
                    eprintln!(
                        "fast_commit_samples_ms surface={} format={} pages={} {}",
                        receipt.surface.label(),
                        receipt.format.label(),
                        receipt.pages,
                        receipt
                            .samples
                            .iter()
                            .map(|sample| format!("{:.3}", milliseconds(*sample)))
                            .collect::<Vec<_>>()
                            .join(","),
                    );
                    receipts.push(receipt);
                }
            }
        }

        // Gate evaluation. Every gate is reported for every configuration
        // before any of them is enforced, so a failure still produces a
        // complete receipt.
        let smallest = *sizes.iter().min().expect("at least one graph size");
        let largest = *sizes.iter().max().expect("at least one graph size");
        let mut failures = Vec::new();
        for receipt in &receipts {
            let key = format!(
                "{}/{}/{} pages",
                receipt.surface.label(),
                receipt.format.label(),
                receipt.pages
            );
            let hard = receipt.p50() <= HARD_P50;
            let target = receipt.p50() <= TARGET_P50;
            let tail = receipt.p95() <= HARD_P95;
            eprintln!(
                "fast_commit_gate {key}: p50={:.3}ms hard<=10ms:{} target<=5ms:{} p95={:.3}ms <=20ms:{} \
                 forbidden_work_zero:{}",
                milliseconds(receipt.p50()),
                if hard { "PASS" } else { "FAIL" },
                if target { "PASS" } else { "MISS" },
                milliseconds(receipt.p95()),
                if tail { "PASS" } else { "FAIL" },
                receipt.forbidden.is_none(),
            );
            assert!(
                receipt.forbidden.is_none(),
                "{key} performed forbidden structural work: {:?}",
                receipt.forbidden
            );
            if receipt.surface.is_receipt() {
                if !hard {
                    failures.push(format!(
                        "{key}: p50 {:.3}ms > 10ms",
                        milliseconds(receipt.p50())
                    ));
                }
                if !tail {
                    failures.push(format!(
                        "{key}: p95 {:.3}ms > 20ms",
                        milliseconds(receipt.p95())
                    ));
                }
            }
        }

        for format in &formats {
            for (surface, _) in surfaces() {
                let at = |pages: usize| {
                    receipts.iter().find(|receipt| {
                        receipt.surface == surface
                            && receipt.format == *format
                            && receipt.pages == pages
                    })
                };
                let (Some(small), Some(large)) = (at(smallest), at(largest)) else {
                    continue;
                };
                if smallest == largest {
                    continue;
                }
                let ratio = large.p50().as_secs_f64() / small.p50().as_secs_f64();
                let key = format!("{}/{}", surface.label(), format.label());
                eprintln!(
                    "fast_commit_scale_gate {key}: p50({largest})/p50({smallest}) = {ratio:.3} \
                     (<= {SCALE_GATE:.1}) {}",
                    if ratio <= SCALE_GATE { "PASS" } else { "FAIL" }
                );
                if surface.is_receipt() && ratio > SCALE_GATE {
                    failures.push(format!(
                        "{key}: p50 at {largest} pages is {ratio:.3}x p50 at {smallest} pages"
                    ));
                }
            }
        }

        assert!(
            failures.is_empty(),
            "the ext4 receipt did not meet the ordinary-edit latency contract:\n  {}",
            failures.join("\n  ")
        );
    }
}
