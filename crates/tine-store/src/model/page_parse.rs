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
        if entry.kind == PageKind::Journal {
            if let Some(date) = entry
                .date_key
                .map(tine_core::date::JournalDate::from_ordinal)
            {
                if self.is_shadow_journal(path, date) {
                    return None;
                }
            }
        }
        Some(entry)
    }
}

impl Graph {
    /// Isolate lsdoc's deliberate parser panics to one page rather than the
    /// cache. The page is named and parsed from its file: a build leaves a
    /// held page out, and its row is its owner's (`install_built`, (G)).
    pub(super) fn parse_page_entry_isolated(&self, mut e: PageEntry) -> PageParseResult {
        let content = read_parse_input(&e.path).map_err(|error| {
            PageParseFailure::Unreadable(e.rel_path_str().to_owned(), error.to_string())
        })?;
        if let Ok(name) = self.name_from(&e, &content) {
            e.name = name;
        }
        isolate_page_parse(e, |entry| Some(parse_page_content(entry, &content)))
    }
}

pub(super) fn parse_page_content(e: &PageEntry, content: &str) -> (Document, DiskObs) {
    let rev = DiskObs::of(content);
    let mut doc = parse_doc(&e.path, content);
    #[cfg(test)]
    if content.contains(TEST_PAGE_PARSE_PANIC_SENTINEL) {
        panic!("deterministic test sentinel for a page projection panic");
    }
    assign_doc_runtime_ids(&mut doc.roots, e.rel_path_str());
    // A 408-byte DocBlock makes unused doubling capacity costly at 10k pages.
    shrink_blocks(&mut doc.roots);
    (doc, rev)
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
                "Tine search index skipped a page after parse/projection panic",
            );
            Err(PageParseFailure::Panic(
                e.rel_path_str().to_owned(),
                format!("page parse/projection panicked: {detail}"),
            ))
        }
    }
}

/// A committed DTO save publishes parsed content with the live identities of
/// blocks that survived serialization. Header promotion has already changed
/// the saved tree, so corresponding nodes have the same structural position.
/// When a parser changes the tree shape, leave that subtree's parsed ids alone.
pub(super) fn carry_saved_runtime_ids(parsed: &mut [DocBlock], saved: &[DocBlock]) {
    if parsed.len() != saved.len() {
        return;
    }
    for (parsed, saved) in parsed.iter_mut().zip(saved) {
        if !saved.uuid.is_empty() {
            parsed.uuid.clone_from(&saved.uuid);
        }
        carry_saved_runtime_ids(&mut parsed.children, &saved.children);
    }
}

/// Build a page DTO from a cached document. `read_only` is left false here (the
/// on-disk bytes aren't known at this point); `load_page` sets it from the file
/// it reads.
pub(super) fn page_dto(entry: &PageEntry, doc: &Document) -> PageDto {
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
