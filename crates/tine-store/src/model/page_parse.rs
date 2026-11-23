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
    pub(super) fn parse_page_entry_isolated(&self, mut e: PageEntry) -> PageParseResult {
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
        isolate_page_parse(e, |entry| Some(parse_page_content(entry, &content)))
    }
}

pub(super) fn parse_page_content(e: &PageEntry, content: &str) -> (Document, DiskObs) {
    let rev = DiskObs::of(content);
    let mut doc = parse_doc(&e.path, content);
    #[cfg(test)]
    if content.contains(TEST_PAGE_PARSE_PANIC_SENTINEL)
        || content.contains(TEST_EXTERNAL_SYNC_PARSE_PANIC_SENTINEL)
    {
        panic!("deterministic test sentinel for a page projection panic");
    }
    assign_doc_runtime_ids(&mut doc.roots, e.rel_path_str());
    // A 408-byte DocBlock makes unused doubling capacity costly at 10k pages.
    shrink_blocks(&mut doc.roots);
    prime_projections(&doc);
    (doc, rev)
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

pub(super) fn isolate_page_parse(
    e: PageEntry,
    parse: impl FnOnce(&PageEntry) -> Option<(Document, DiskObs)>,
) -> PageParseResult {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parse(&e))) {
        Ok(Some((doc, rev))) => Ok(Some((e, doc, rev))),
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

/// Give `old`, the fresh parse of the bytes a save replaces, the runtime ids
/// that `new` (the document saved over it) uses for those blocks, so layout
/// retention tells blocks apart by identity, not text (#960). A page read
/// gives each block either its position-derived id (a fresh parse of the
/// file at `owner`, graph-relative) or the cached row's id, which a save keeps
/// from the saved document; after a structural save the two differ. `new`
/// descends from one of them: the one holding every id of `new` that either
/// holds. When both qualify, only blocks they agree on keep an id; when
/// neither does, or `owner` is unknown, no block has one, and retention
/// matches by text alone. O(blocks).
pub(super) fn identify_old_blocks(
    old: &mut Document,
    owner: Option<&str>,
    cached: Option<&Document>,
    new: &Document,
) {
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
    let Some(owner) = owner.filter(|owner| !owner.is_empty()) else {
        return;
    };
    assign_doc_runtime_ids(&mut old.roots, owner);
    let Some(cached) = cached.filter(|cached| **cached == *old) else {
        return;
    };
    let (mut kept, mut fresh, mut saving) = (Vec::new(), Vec::new(), Vec::new());
    ids(&cached.roots, &mut kept);
    ids(&old.roots, &mut fresh);
    if kept == fresh {
        return;
    }
    ids(&new.roots, &mut saving);
    let (kept_ids, fresh_ids): (HashSet<_>, HashSet<_>) = (
        kept.iter().copied().collect(),
        fresh.iter().copied().collect(),
    );
    let holds = |space: &HashSet<&BlockId>, other: &HashSet<&BlockId>| {
        saving
            .iter()
            .all(|id| space.contains(id) || !other.contains(id))
    };
    let chosen: Vec<BlockId> = match (holds(&kept_ids, &fresh_ids), holds(&fresh_ids, &kept_ids)) {
        (true, false) => kept.into_iter().cloned().collect(),
        (false, true) => return,
        (true, true) => kept
            .iter()
            .zip(&fresh)
            .map(|(k, f)| {
                if k == f {
                    (*k).clone()
                } else {
                    BlockId::default()
                }
            })
            .collect(),
        (false, false) => vec![BlockId::default(); kept.len()],
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

#[cfg(test)]
mod identify_old_blocks_tests {
    use super::*;

    fn with_ids(source: &str, ids: &[&BlockId]) -> Document {
        let mut doc = parse_doc(Path::new("pages/p.md"), source);
        for (block, id) in doc.roots.iter_mut().zip(ids) {
            block.uuid = (*id).clone();
        }
        doc
    }

    /// The saving document's lineage picks the id space: the cached row's
    /// (an editor that saved keeps its ids) or the position-derived one (a
    /// fresh read); where both are possible, only agreeing blocks keep an id.
    #[test]
    fn the_saving_documents_lineage_picks_the_old_ids() {
        let source = "- p\n- p\n- r\n";
        let mut fresh = parse_doc(Path::new("pages/p.md"), source);
        assign_doc_runtime_ids(&mut fresh.roots, "pages/p.md");
        let s: Vec<BlockId> = fresh.roots.iter().map(|b| b.uuid.clone()).collect();
        let t = BlockId::from("kept-only");
        let identify = |owner: Option<&str>, cached: &[&BlockId], saving: &[&BlockId]| {
            let mut old = parse_doc(Path::new("pages/p.md"), source);
            let cached = with_ids(source, cached);
            identify_old_blocks(&mut old, owner, Some(&cached), &with_ids(source, saving));
            old.roots.iter().map(|b| b.uuid.clone()).collect::<Vec<_>>()
        };
        let none = BlockId::default();
        // After a save moved blocks, the cache keeps the saved ids.
        let kept = [&s[1], &s[0], &t];
        assert_eq!(
            identify(Some("pages/p.md"), &kept, &kept),
            [s[1].clone(), s[0].clone(), t.clone()]
        );
        assert_eq!(
            identify(Some("pages/p.md"), &kept, &[&s[0], &s[1], &s[2]]),
            s
        );
        // A permutation leaves both spaces possible: only `r` agrees.
        let swapped = [&s[1], &s[0], &s[2]];
        assert_eq!(
            identify(Some("pages/p.md"), &swapped, &[&s[0], &s[1], &s[2]]),
            [none.clone(), none.clone(), s[2].clone()]
        );
        assert_eq!(
            identify(None, &kept, &kept),
            [none.clone(), none.clone(), none]
        );
    }
}
