//! A Markdown page preamble: its page header and its prose (CM step 2, batch 8;
//! the header moved here from `block_regions`, which re-exports it).

use crate::block_regions::{parse_document, BlockRegions, HeaderEntry, PageHeader, Range};

/// The page header of a Markdown page preamble or first root block: the parser's
/// accepted properties (`parse_document`; literals and no-space `key::value`
/// excluded) that form the leading run of the text, joined only by empty lines.
/// Named Tine policies over that answer: the run starts at byte 0, every line
/// starts at column zero, and a key may not start with `#` (a `#tag::` line is
/// prose that merely parses). A parser-refused input has no header. O(bytes).
pub fn page_header(raw: &str) -> PageHeader {
    page_header_in(raw, &parse_document(raw, false))
}

/// [`page_header`] over `raw`'s Markdown regions already parsed (one parse
/// for callers that also read other answers from it).
pub fn page_header_in(raw: &str, regions: &BlockRegions) -> PageHeader {
    let mut header = PageHeader::default();
    if regions.quarantined {
        return header;
    }
    let (mut at, mut line_no) = (0, 0);
    for p in regions.properties.iter().filter(|p| p.primary) {
        let gap = &raw[at..p.line.0.max(at)];
        let eligible = p.line.0 >= at
            && gap.bytes().all(|b| b == b'\n' || b == b'\r')
            && p.key_range.0 == p.line.0
            && !p.key.starts_with('#')
            && (!header.entries.is_empty() || p.line.0 == 0);
        if !eligible {
            break;
        }
        line_no += gap.matches('\n').count();
        let end = p.line.1 - usize::from(raw.as_bytes()[p.line.1 - 1] == b'\n');
        header.entries.push(HeaderEntry {
            key: p.key.clone(),
            value: p.value.clone(),
            line: line_no,
            start: p.line.0,
            end,
        });
        header.end = end;
        at = p.line.1;
        line_no += 1;
    }
    header
}

/// The prose of a Markdown page preamble: what follows the
/// [`page_header`] (the whole text when it
/// has none), without the newlines at either edge. `None` when nothing is
/// left. OG reads the text before a page's first block as its first block
/// (graph-parser `block.cljs with-pre-block-if-exists`); Tine shows this part
/// as that block's row (`Page.tsx`) and promotes it to a real block when
/// edited (GH #85), so every reference question reads it as a block. The one
/// answer for both sides (the window asks it through wasm). O(bytes).
pub fn page_preamble_content(raw: &str) -> Option<Range> {
    page_preamble_content_in(raw, &parse_document(raw, false))
}

/// The shared prose boundary for Markdown and Org pre-blocks. Org prose
/// starts after the last parser-owned page property (including its drawer).
pub fn preamble_content_range(raw: &str, is_org: bool) -> Option<Range> {
    preamble_content_range_in(raw, is_org, &parse_document(raw, is_org))
}

/// [`preamble_content_range`] over already parsed regions, with no second parse.
pub fn preamble_content_range_in(raw: &str, is_org: bool, regions: &BlockRegions) -> Option<Range> {
    if !is_org {
        return page_preamble_content_in(raw, regions);
    }
    if regions.quarantined {
        return None;
    }
    let mut start = regions
        .find_org_preamble_property_region(raw)
        .map_or(0, |region| region.1);
    for property in regions.page_properties() {
        start = start.max(property.line.1);
        if let Some(drawer) = regions.property_regions.get(property.region) {
            start = start.max(drawer.1);
        }
    }
    let start = start.min(raw.len());
    let rest = &raw[start..];
    let leading = rest.len() - rest.trim_start_matches('\n').len();
    let content = rest.trim_matches('\n');
    (!content.is_empty()).then(|| Range(start + leading, start + leading + content.len()))
}

/// [`page_preamble_content`] over `raw`'s Markdown regions already parsed.
pub fn page_preamble_content_in(raw: &str, regions: &BlockRegions) -> Option<Range> {
    let header = page_header_in(raw, regions);
    let start = if header.entries.is_empty() {
        0
    } else {
        header.end
    };
    let rest = &raw[start..];
    let leading = rest.len() - rest.trim_start_matches('\n').len();
    let content = rest.trim_matches('\n');
    (!content.is_empty()).then(|| Range(start + leading, start + leading + content.len()))
}

#[cfg(test)]
mod tests {
    use super::page_preamble_content;

    fn text(raw: &str) -> Option<&str> {
        page_preamble_content(raw).map(|r| &raw[r.0..r.1])
    }

    #[test]
    fn the_prose_after_the_page_header_without_edge_newlines() {
        assert_eq!(text("[[Target]] in prose"), Some("[[Target]] in prose"));
        assert_eq!(
            text("tags:: x\n\n\nsee [[Target]]\n\n"),
            Some("see [[Target]]")
        );
        assert_eq!(text("tags:: x"), None);
        assert_eq!(text(""), None);
    }
}
