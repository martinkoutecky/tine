//! Where a page header ends: its page properties and the prose after them.
//! The parser owns the answer (`block_regions::parse_document` marks the page
//! properties primary, OG's rule); these functions only read it.

use crate::block_regions::{parse_document, BlockRegions, Range};
use serde::Serialize;

/// One Markdown page-header property: the parser's accepted key and value, its
/// 0-based line index, and the byte range of its line without the newline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeaderEntry {
    pub key: String,
    pub value: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// A Markdown text's page properties (see [`page_header`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PageHeader {
    /// Byte end of the last header line, newline excluded; 0 with no entries.
    pub end: usize,
    pub entries: Vec<HeaderEntry>,
    /// Every line of the text is one of `entries` (at least one), none with a
    /// `#` key, which OG reads as no property (`valid-property-name?`): the
    /// text is a complete page header ([`page_header_only`]).
    pub whole: bool,
    /// The 0-based index of the first non-blank line that is no such entry:
    /// the line an incomplete header names.
    pub stray: Option<usize>,
}

/// The page header of a Markdown page preamble or first root block: its page
/// properties as the parser marks them (the first top-level property node;
/// OG `block.cljs extract-blocks`). A parser-refused input has no header.
/// O(bytes).
pub fn page_header(raw: &str) -> PageHeader {
    page_header_in(raw, &parse_document(raw, false))
}

/// The page header of `raw` when every line of the text is a page property
/// (at least one). Single answerer (I-12) for "is this first block a
/// page-properties block": a header line is a parser page property, so
/// `key::value` (no space), literal-fence content, a `#` key and a line after
/// a blank one (OG reads only the first property node) are not
/// (`block_regions::mapped_regions` owns the rule).
pub fn page_header_only(raw: &str) -> Option<PageHeader> {
    let header = page_header(raw);
    header.whole.then_some(header)
}

/// [`page_header`] over `raw`'s Markdown regions already parsed (one parse
/// for callers that also read other answers from it).
pub fn page_header_in(raw: &str, regions: &BlockRegions) -> PageHeader {
    let mut header = PageHeader::default();
    let (mut at, mut line) = (0, 0);
    for p in regions.page_properties() {
        line += raw[at..p.line.0].matches('\n').count();
        at = p.line.0;
        let end = p.line.1 - usize::from(raw.as_bytes()[p.line.1 - 1] == b'\n');
        header.entries.push(HeaderEntry {
            key: p.key.clone(),
            value: p.value.clone(),
            line,
            start: p.line.0,
            end,
        });
        header.end = end;
    }
    // OG reads a `#` key as no property (`valid-property-name?`).
    let mut good = header
        .entries
        .iter()
        .filter(|e| !e.key.starts_with('#'))
        .map(|e| e.line)
        .peekable();
    header.stray = raw.split('\n').enumerate().position(|(i, text)| {
        while good.next_if(|&g| g < i).is_some() {}
        !text.trim().is_empty() && good.peek() != Some(&i)
    });
    header.whole = header.stray.is_none()
        && !header.entries.is_empty()
        && header.end == raw.len()
        && header.entries.iter().enumerate().all(|(i, e)| e.line == i);
    header
}

/// Where the prose of a page preamble is: the text after its last page
/// property (Org: after the drawer holding it), without the newlines at
/// either edge; the whole text when it has none. `None` when nothing is left.
/// In Markdown every other non-blank byte is a page-property line.
/// OG reads the text before a page's first block as its first block
/// (graph-parser `block.cljs with-pre-block-if-exists`); Tine shows this part
/// as that block's row (`Page.tsx`) and promotes it to a real block when
/// edited (GH #85), so every reference question reads it as a block. The one
/// answer for both sides (the window asks it through wasm). O(bytes).
pub fn preamble_content_range(raw: &str, is_org: bool) -> Option<Range> {
    preamble_content_range_in(raw, is_org, &parse_document(raw, is_org))
}

/// [`preamble_content_range`] over already parsed regions, with no second parse.
pub fn preamble_content_range_in(raw: &str, is_org: bool, regions: &BlockRegions) -> Option<Range> {
    if regions.quarantined {
        return None;
    }
    let mut start = regions
        .find_org_preamble_property_region(raw)
        .filter(|_| is_org)
        .map_or(0, |region| region.1);
    for property in regions.page_properties() {
        start = start.max(property.line.1);
        // An Org drawer's `:END:` is header; a Markdown node's line after its
        // last page property is prose (`block_regions::mapped_regions`).
        if let Some(region) = regions
            .property_regions
            .get(property.region)
            .filter(|_| is_org)
        {
            start = start.max(region.1);
        }
    }
    let start = start.min(raw.len());
    let rest = &raw[start..];
    let leading = rest.len() - rest.trim_start_matches('\n').len();
    let content = rest.trim_matches('\n');
    (!content.is_empty()).then(|| Range(start + leading, start + leading + content.len()))
}

#[cfg(test)]
mod tests {
    use super::{page_header, preamble_content_range};
    use serde::Deserialize;

    fn text(raw: &str) -> Option<&str> {
        preamble_content_range(raw, false).map(|r| &raw[r.0..r.1])
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

    /// One row of `og_page_header.json`: an input and OG's answer for it,
    /// taken from mldoc 1.5.9's AST (the lsdoc oracle harness) under OG's
    /// `extract-blocks` rule, never from Tine. Generator (private evidence):
    /// `tine-agents/evidence/cm-header-boundary/oracle/fixture.sh`.
    #[derive(Deserialize)]
    struct Row {
        format: String,
        input: String,
        /// OG's page property keys, lowercased, in file order.
        properties: Vec<String>,
        /// Where OG's page-property node ends (UTF-8 bytes); `null` without one.
        header_end: Option<usize>,
        /// A named Tine difference from OG on this row (asserted to persist).
        #[serde(default)]
        known_difference: Option<String>,
    }

    /// Tine's page properties are OG's, and no page property lies at or after
    /// the start of the prose (the boundary has one owner).
    #[test]
    fn page_properties_and_the_prose_boundary_are_ogs() {
        let rows: Vec<Row> =
            serde_json::from_str(include_str!("page_preamble/og_page_header.json")).unwrap();
        assert!(rows.len() >= 40, "{}", rows.len());
        let mut wrong = Vec::new();
        for row in rows {
            let org = row.format == "org";
            let regions = crate::block_regions::parse_document(&row.input, org);
            let keys: Vec<String> = regions
                .page_properties()
                .map(|p| p.key.to_lowercase())
                .collect();
            let prose = preamble_content_range(&row.input, org);
            if let Some(prose) = prose {
                assert!(
                    regions.page_properties().all(|p| p.line.1 <= prose.0),
                    "a page property inside the prose: {:?}",
                    row.input
                );
                // A node with no valid key (`#tag:: x`) leaves its text to the prose.
                let same = row.known_difference.is_none();
                if let Some(end) = row.header_end.filter(|_| same && !keys.is_empty()) {
                    assert!(prose.0 >= end, "{:?}", row.input);
                }
            }
            if !org {
                let header = page_header(&row.input);
                let entries: Vec<String> = header
                    .entries
                    .iter()
                    .map(|e| e.key.to_lowercase())
                    .collect();
                assert_eq!(entries, keys, "{:?}", row.input);
            }
            if (keys == row.properties) != row.known_difference.is_none() {
                wrong.push(format!(
                    "{:?}: Tine {keys:?}, OG {:?}, known {:?}",
                    row.input, row.properties, row.known_difference
                ));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Where `raw` breaks the Markdown header/prose partition: a non-blank
    /// byte in both a page-property line and the prose, or in neither (shown
    /// nowhere, indexed nowhere, skipped by rename).
    fn partition_error(raw: &str) -> Option<String> {
        let regions = crate::block_regions::parse_document(raw, false);
        if regions.quarantined {
            return None;
        }
        let prose = preamble_content_range(raw, false);
        let lines: Vec<_> = regions.page_properties().map(|p| p.line).collect();
        let (i, c) = raw.char_indices().find(|&(i, c)| {
            let property = lines.iter().any(|l| l.0 <= i && i < l.1);
            let in_prose = prose.is_some_and(|r| r.0 <= i && i < r.1);
            !c.is_whitespace() && c != '\u{feff}' && property == in_prose
        })?;
        Some(format!(
            "{raw:?}: byte {i} ({c:?}) lines {lines:?} prose {prose:?}"
        ))
    }

    /// Every non-blank byte of a Markdown pre-block is in exactly one of the
    /// page header and the prose row (REG-CM-HEADER-BOUNDARY, review F1):
    /// over the OG fixture and every sequence of up to three generated lines.
    #[test]
    fn every_markdown_preamble_byte_is_header_or_prose_once() {
        let rows: Vec<Row> =
            serde_json::from_str(include_str!("page_preamble/og_page_header.json")).unwrap();
        let mut inputs: Vec<String> = (rows.into_iter())
            .filter(|r| r.format == "md")
            .map(|r| r.input)
            .collect();
        let pieces = [
            "",
            " ",
            "alias:: y",
            " k:: v",
            "#tag:: x",
            "Intro [[X]]",
            "#+b: [[X]]",
            "```",
            ":PROPERTIES:",
            ":END:",
            "k::v",
            "title:: T",
            "\u{feff}a:: 1",
            "> q:: r",
        ];
        for a in pieces {
            for b in pieces {
                for c in pieces {
                    for eol in ["\n", "\r\n"] {
                        inputs.push([a, b, c].join(eol));
                        inputs.push([a, b, c, ""].join(eol));
                    }
                }
            }
        }
        let wrong: Vec<_> = inputs
            .iter()
            .filter_map(|raw| partition_error(raw))
            .collect();
        assert!(
            wrong.is_empty(),
            "{} of {}: {:#?}",
            wrong.len(),
            inputs.len(),
            &wrong[..wrong.len().min(20)]
        );
    }
}
