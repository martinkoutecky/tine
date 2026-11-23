//! One-page parse boundary. Parsed page trees are immutable cache entries, so
//! their block vectors release growth capacity before publication.

use super::*;

impl Graph {
    /// Publication eligibility shared by prepared saves and external parses.
    pub(super) fn cacheable_page_entry(&self, path: &Path) -> Option<PageEntry> {
        self.cacheable_page_entry_in(path, None)
    }

    /// [`Self::cacheable_page_entry`] naming the page from `text`, the bytes
    /// already in hand (GH #623).
    pub(super) fn cacheable_page_entry_in(
        &self,
        path: &Path,
        text: Option<&str>,
    ) -> Option<PageEntry> {
        let entry = self.entry_for_path_in(path, text)?;
        if path_is_sync_conflict(path) {
            return None;
        }
        Some(entry)
    }
}

impl Graph {
    /// Isolate lsdoc's deliberate parser panics to one page rather than the
    /// cache. The page is named and parsed from its file: a build leaves a
    /// held page out, and its row is its owner's (`install_built`, (G)).
    /// With `prior`, the rows a whole-cache invalidation retired (T10): bytes
    /// of the prior row's revision reuse its document whole, unparsed (a
    /// page's parse reads no configuration; its entry is named again).
    pub(super) fn parse_page_entry_isolated(
        &self,
        mut e: PageEntry,
        prior: Option<&RebuildPrior>,
    ) -> PageParseResult {
        let content = match self.read_parse_input(&e.path) {
            Ok(content) => content,
            Err(error) => {
                return PageParseFailure::unreadable(
                    e.rel_path_str().to_owned(),
                    error,
                    "Tine unreadable file producer=on-demand-build",
                )
            }
        };
        let _ = self.name_from(&mut e, &content);
        // The path's seed (a read or save before the cache, S6), else the
        // row the invalidation retired: bytes of its revision reuse it whole.
        let seed = (self.held.claims.lock().unwrap().get(&e.path))
            .map(|(_, doc, disk)| (Arc::clone(doc), disk.rev.clone()));
        let disk = DiskObs::of(&content);
        if let Some((doc, _)) = seed.as_ref().filter(|(_, rev)| *rev == disk.rev) {
            return Ok(Some((e, Arc::clone(doc), disk)));
        }
        if let Some(row) = prior.and_then(|prior| prior.row_at(&e, &disk.rev)) {
            return Ok(Some((e, row, disk)));
        }
        // Changed bytes keep the prior's keys when they hold its outline
        // (Observed); a path with no prior takes positional keys (Fresh).
        let retired = match &seed {
            Some((doc, _)) => Some(doc),
            None => prior.and_then(|prior| prior.doc(e.rel_path_str())),
        };
        isolate_page_parse(e, |entry| {
            let (doc, disk) = parse_page_unkeyed(entry, &content);
            let input = match retired {
                Some(_) => page_ids::Input::Observed(doc),
                None => page_ids::Input::Fresh(doc),
            };
            let doc = self.page_ids(&entry.path, entry.rel_path_str(), input, retired);
            Some((doc, disk))
        })
    }
}

pub(super) fn parse_page_content(e: &PageEntry, content: &str) -> (Document, DiskObs) {
    let (mut doc, rev) = parse_page_unkeyed(e, content);
    key_page(&mut doc, e);
    (doc, rev)
}

/// The parse of a page's `content`, its blocks still without runtime ids:
/// the caller decides which ids they keep, then [`key_page`]s it.
pub(super) fn parse_page_unkeyed(e: &PageEntry, content: &str) -> (Document, DiskObs) {
    let rev = DiskObs::of(content);
    let mut doc = parse_doc(&e.path, content);
    #[cfg(test)]
    if content.contains(TEST_PAGE_PARSE_PANIC_SENTINEL)
        || content.contains(TEST_EXTERNAL_SYNC_PARSE_PANIC_SENTINEL)
    {
        panic!("deterministic test sentinel for a page projection panic");
    }
    // A 408-byte DocBlock makes unused doubling capacity costly at 10k pages.
    shrink_blocks(&mut doc.roots);
    (doc, rev)
}

/// Give each block of `doc`, the page file `e`'s, that has no runtime id
/// one ([`assign_doc_runtime_ids`], which leaves a fully keyed page alone),
/// and validate its projections.
pub(super) fn key_page(doc: &mut Document, e: &PageEntry) {
    assign_doc_runtime_ids(&mut doc.roots, e.rel_path_str());
    prime_projections(doc);
}

/// Validate every projection a page's readers need: block bodies and the
/// pre-blocks' page properties and prose. Publication, DTO reads and save-base
/// validation share this answer inside their page-level panic boundaries.
/// Block projections are cached on the document; pre-block projections are
/// temporary. Cost O(page bytes + blocks), with no filesystem access.
pub(super) fn prime_projections(doc: &Document) {
    fn walk(blocks: &[DocBlock]) {
        for block in blocks {
            block.projection();
            walk(&block.children);
        }
    }
    for block in tine_core::page_properties::document_pre_blocks(doc) {
        block.projection();
    }
    walk(&doc.roots);
}

fn shrink_blocks(blocks: &mut Vec<DocBlock>) {
    blocks.shrink_to_fit();
    for block in blocks.iter_mut() {
        shrink_blocks(&mut block.children);
    }
}

pub(super) fn isolate_page_parse<D: Into<Arc<Document>>>(
    e: PageEntry,
    parse: impl FnOnce(&PageEntry) -> Option<(D, DiskObs)>,
) -> PageParseResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(&e))) {
        Ok(Some((doc, rev))) => Ok(Some((e, doc.into(), rev))),
        Ok(None) => Ok(None),
        Err(payload) => {
            let detail = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic payload");
            tine_core::diag_line::diagnostic_line(
                "Tine search index skipped a page after parse/projection panic producer=page-parse",
            );
            Err(PageParseFailure::Panic(
                e,
                format!("page parse/projection panicked: {detail}"),
            ))
        }
    }
}

/// A committed DTO save publishes parsed content with the live identities of
/// blocks that survived serialization: each parsed block takes its saved
/// counterpart's id, or none where the saved block has none. Header promotion
/// has already changed the saved tree, so corresponding nodes have the same
/// structural position. When a parser changes the tree shape, leave that
/// subtree's parsed ids alone. The publication then gives every block without
/// an id one ([`assign_doc_runtime_ids`]).
pub(super) fn carry_saved_runtime_ids(parsed: &mut [DocBlock], saved: &[DocBlock]) {
    if parsed.len() != saved.len() {
        return;
    }
    for (parsed, saved) in parsed.iter_mut().zip(saved) {
        parsed.uuid.clone_from(&saved.uuid);
        carry_saved_runtime_ids(&mut parsed.children, &saved.children);
    }
}

impl Graph {
    /// Give `old`, the fresh parse of `source` (the bytes a save of the page
    /// file at `path` replaces), the keys its base provenance gives
    /// (`RUNTIME-ID-MODEL.md`, provenance table), so layout retention tells
    /// blocks apart by identity, not text (#960):
    /// - Live: the path's row, or its claim or seed, holds `old`'s outline
    ///   (as the owner's Observed rule keeps keys): its keys are carried.
    /// - None at all (no row, claim or seed: a first read before any row):
    ///   the positional keys that read was served, as the seed rule gives.
    /// - Provisional (a row of another outline): no keys; retention matches
    ///   by text alone.
    ///
    /// O(blocks); no hash of `source` (PC-4: a save hashes its bytes once).
    pub(super) fn key_save_base(&self, old: &mut Document, path: &Path) {
        let base = match self.cached_observation(path) {
            Some((_, doc, _)) => Some(doc),
            None => (self.held.claims.lock().unwrap().get(path)).map(|(_, doc, _)| Arc::clone(doc)),
        };
        match base {
            Some(doc) if *doc == *old => carry_saved_runtime_ids(&mut old.roots, &doc.roots),
            Some(_) => {}
            None => {
                if let Some(rel) = path.strip_prefix(&self.root).ok().and_then(Path::to_str) {
                    assign_doc_runtime_ids(&mut old.roots, rel);
                }
            }
        }
    }
}

/// Build a page DTO from a cached document. `read_only` is left false here (the
/// on-disk bytes aren't known at this point); `load_page` sets it from the file
/// it reads.
pub(crate) fn page_dto(entry: &PageEntry, doc: &Document) -> PageDto {
    PageDto {
        name: entry.name.clone(),
        kind: entry.kind,
        title: entry.name.clone(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: Format::from_path(&entry.path),
        read_only: false,

        guide: false,
    }
}

/// The complete visible S(P) a whole-cache invalidation retired (T10, D6):
/// every row with its revision. The next build reuses a row whose bytes are
/// unchanged and keeps the keys of one whose outline its bytes still hold;
/// dropped when that build installs. Peak: these roots (shared Arcs) plus the
/// rows rebuilt for changed paths.
pub(crate) struct RebuildPrior {
    pub(super) pages: Arc<Pages>,
    pub(super) revs: HashMap<PathBuf, String>,
}

impl RebuildPrior {
    /// The prior document of the page file `rel` (graph-relative).
    pub(super) fn doc(&self, rel: &str) -> Option<&Arc<Document>> {
        let slot = *self.pages.positions.get(rel)?;
        self.pages.get(slot).map(|(_, doc)| doc)
    }

    /// The prior document of the page file `e` when `rev` is the revision
    /// of the bytes it was parsed from.
    fn row_at(&self, e: &PageEntry, rev: &str) -> Option<Arc<Document>> {
        let at = self.revs.get(&e.path)?;
        (at == rev)
            .then(|| self.doc(e.rel_path_str()).cloned())
            .flatten()
    }
}
