//! One-page parse boundary. Parsed page trees are immutable cache entries, so
//! their block vectors release growth capacity before publication.

use super::*;

/// Isolate lsdoc's deliberate parser panics to one page rather than the cache.
pub(super) fn parse_page_entry_isolated(e: PageEntry) -> PageParseResult {
    let content = read_parse_input(&e.path).map_err(|error| {
        PageParseFailure::Unreadable(e.rel_path_str().to_owned(), error.to_string())
    })?;
    isolate_page_parse(e, |entry| Some(parse_page_content(entry, &content)))
}

pub(super) fn parse_page_content(e: &PageEntry, content: &str) -> (Document, String) {
    let rev = content_rev(content);
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
    parse: impl FnOnce(&PageEntry) -> Option<(Document, String)>,
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
            eprintln!("Tine search index skipped a page after parse/projection panic");
            Err(PageParseFailure::Panic(
                e.rel_path_str().to_owned(),
                format!("page parse/projection panicked: {detail}"),
            ))
        }
    }
}
