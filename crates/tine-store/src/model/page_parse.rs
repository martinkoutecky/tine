//! One-page parse boundary. Parsed page trees are immutable cache entries, so
//! their block vectors release growth capacity before publication.

use super::*;
use tine_core::BlockId;

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
    /// With `prior`, the rows a whole-cache invalidation retired (T10), keyed
    /// as [`Self::keyed`] keys (a page's parse reads no configuration; its
    /// entry is named again).
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
        isolate_page_parse(e, |e| {
            Some(self.keyed(e, &content, PriorFrom::Build(prior)))
        })
    }

    /// `content`, the bytes of the page file `e`, keyed by the owner against
    /// the path's S(P) ([`Self::prior_at`] in `from`): an S(P) of exactly
    /// these bytes is served whole, unparsed; other bytes are parsed once
    /// and keep the prior's keys when they hold its outline, else take
    /// positional keys clear of the registry (Observed). Installs nothing.
    /// The one keying of capture, rebuild, launch, Rescan and owner bytes.
    pub(super) fn keyed(
        &self,
        e: &PageEntry,
        content: &str,
        from: PriorFrom<'_>,
    ) -> (Arc<Document>, DiskObs) {
        let disk = DiskObs::of(content);
        let doc = match self.prior_at(&e.path, Some(&disk.rev), from) {
            Some((doc, true)) => doc,
            prior => {
                let input = page_ids::Input::Observed(parse_page_unkeyed(e, content));
                let prior = prior.map(|(doc, _)| doc);
                self.page_ids(&e.path, e.rel_path_str(), input, prior.as_ref())
            }
        };
        (doc, disk)
    }
}

/// A page's positional parse, outside the owner (contained retries).
pub(super) fn parse_page_content(e: &PageEntry, content: &str) -> (Document, DiskObs) {
    let mut doc = parse_page_unkeyed(e, content);
    assign_doc_runtime_ids(&mut doc.roots, e.rel_path_str());
    prime_projections(&doc);
    (doc, DiskObs::of(content))
}

/// The parse of a page's `content`, its blocks still without runtime ids:
/// the owner ([`Graph::page_ids`]) decides which ids they take.
pub(super) fn parse_page_unkeyed(e: &PageEntry, content: &str) -> Document {
    let mut doc = parse_doc(&e.path, content);
    #[cfg(test)]
    if content.contains(TEST_PAGE_PARSE_PANIC_SENTINEL)
        || content.contains(TEST_EXTERNAL_SYNC_PARSE_PANIC_SENTINEL)
    {
        panic!("deterministic test sentinel for a page projection panic");
    }
    // A 408-byte DocBlock makes unused doubling capacity costly at 10k pages.
    shrink_blocks(&mut doc.roots);
    doc
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

pub(super) fn shrink_blocks(blocks: &mut Vec<DocBlock>) {
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
    /// - None at all (no row, claim, seed or retired row: a first read
    ///   before any row): the owner's positional keys, as that read was served.
    /// - Otherwise the positional keys, and when the path's S(P)
    ///   ([`Self::prior_at`]) holds `old`'s outline, develop's lineage rule
    ///   ([`identify_old_blocks`]) between its keys and those, by `new`, the
    ///   saving document: the bytes written are develop's (DS-4).
    ///
    /// O(blocks); no hash of `source` (PC-4: a save hashes its bytes once).
    pub(super) fn key_save_base(&self, old: &mut Document, path: &Path, new: &Document) {
        let rel = self.rel_path(path);
        let Some((doc, _)) = self.prior_at(path, None, PriorFrom::Now) else {
            let input = page_ids::Input::Observed(std::mem::take(old));
            *old = Arc::unwrap_or_clone(self.page_ids(path, &rel, input, None));
            return;
        };
        assign_doc_runtime_ids(&mut old.roots, &rel);
        if *doc == *old {
            identify_old_blocks(old, &doc, new);
        }
    }
}

/// Develop's lineage rule (#960), for `old` holding its positional keys and
/// `kept`, the path's S(P) of the same outline: `new` descends from the id
/// space holding every id of `new` that either holds. When both qualify,
/// only blocks they agree on keep an id; when neither does, none does, and
/// retention matches by text alone. O(blocks).
fn identify_old_blocks(old: &mut Document, kept: &Document, new: &Document) {
    fn ids<'a>(blocks: &'a [DocBlock], out: &mut Vec<&'a BlockId>) {
        for block in blocks {
            out.push(&block.uuid);
            ids(&block.children, out);
        }
    }
    fn set(blocks: &mut [DocBlock], ids: &mut impl Iterator<Item = BlockId>) {
        for block in blocks {
            block.uuid = ids.next().unwrap_or_default();
            set(&mut block.children, ids);
        }
    }
    let (mut kept_ids, mut fresh, mut saving) = (Vec::new(), Vec::new(), Vec::new());
    ids(&kept.roots, &mut kept_ids);
    ids(&old.roots, &mut fresh);
    if kept_ids == fresh {
        return;
    }
    ids(&new.roots, &mut saving);
    let (in_kept, in_fresh): (HashSet<_>, HashSet<_>) = (
        kept_ids.iter().copied().collect(),
        fresh.iter().copied().collect(),
    );
    let holds = |space: &HashSet<&BlockId>, other: &HashSet<&BlockId>| {
        (saving.iter()).all(|id| space.contains(id) || !other.contains(id))
    };
    let chosen: Vec<BlockId> = match (holds(&in_kept, &in_fresh), holds(&in_fresh, &in_kept)) {
        (true, false) => kept_ids.into_iter().cloned().collect(),
        (false, true) => return,
        (true, true) => (kept_ids.iter().zip(&fresh))
            .map(|(k, f)| {
                if k == f {
                    (*k).clone()
                } else {
                    BlockId::default()
                }
            })
            .collect(),
        (false, false) => vec![BlockId::default(); kept_ids.len()],
    };
    set(&mut old.roots, &mut chosen.into_iter());
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
    /// The retired document of the page file at `path` (`rel`, canonical
    /// slash-relative), with the revision of the bytes it was parsed from.
    pub(super) fn row(&self, path: &Path, rel: &str) -> Option<(&Arc<Document>, Option<String>)> {
        let slot = *self.pages.positions.get(rel)?;
        let (_, doc) = self.pages.get(slot)?;
        Some((doc, self.revs.get(path).cloned()))
    }
}
