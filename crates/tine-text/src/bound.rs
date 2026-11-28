//! The page outline parse under the free-text bound (I-22): lsdoc recurses
//! while parsing a quote staircase, so a page whose staircase is too deep is
//! refused before lsdoc sees it. The one production caller of
//! `lsdoc::parse_outline` (`tine-core/tests/lsdoc_parse_boundary.rs` fails on
//! any other outside `tests/`); tine-outline's block boundary uses this
//! crate's quote count, so both bounds agree.

/// Source-side ceiling on a line's quote staircase for free-text doors; the
/// same 128-level ceiling the page admission applies (`PARSE_INPUT_MAX_DEPTH`).
pub const SOURCE_QUOTE_DEPTH_MAX: usize = 128;

/// `>` markers that open a quote staircase on one line: after leading spaces
/// and any list markers, each `>` optionally followed by spaces. lsdoc parses
/// every level by recursing *during the parse* (`markdown_blockquote_sequence`
/// re-parses the stripped body), so a tree check after the parse is too late:
/// ~1,300 levels overflow a 2 MiB debug stack before any tree exists (I-22).
/// Counts markers, an upper bound on levels (lsdoc may strip two per level).
pub fn line_quote_depth(line: &str) -> usize {
    let spaces = |bytes: &[u8], mut at: usize| {
        while matches!(bytes.get(at), Some(b' ' | b'\t' | b'\x0c' | b'\x1a')) {
            at += 1;
        }
        at
    };
    let bytes = line.as_bytes();
    let mut at = spaces(bytes, 0);
    loop {
        let digits = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let marker_end = match bytes.get(at + digits) {
            Some(b'.' | b')') if digits > 0 => at + digits + 1,
            Some(b'-' | b'+' | b'*') if digits == 0 => at + 1,
            _ => break,
        };
        if !matches!(bytes.get(marker_end), Some(b' ' | b'\t')) {
            break;
        }
        at = spaces(bytes, marker_end);
    }
    let mut depth = 0;
    while bytes.get(at) == Some(&b'>') {
        depth += 1;
        at = spaces(bytes, at + 1);
    }
    depth
}

/// Whether no line of `text` opens a quote staircase deeper than `limit`.
pub fn quote_depth_within(text: &str, limit: usize) -> bool {
    text.split(['\n', '\r'])
        .all(|line| line_quote_depth(line) <= limit)
}

/// A whole page's lsdoc outline headers ([`lsdoc::parse_outline`]) under the
/// free-text bound: `None` when the page's quote staircase is deeper than
/// [`SOURCE_QUOTE_DEPTH_MAX`] (refused before lsdoc recurses) or lsdoc does not
/// take ownership of the text. lsdoc drops its own parse tree stack-safely.
pub fn parse_outline_bounded(text: &str, org: bool) -> Option<Vec<lsdoc::OutlineHeader>> {
    if !quote_depth_within(text, SOURCE_QUOTE_DEPTH_MAX) {
        return None;
    }
    lsdoc::parse_outline(text, if org { "org" } else { "md" })
        .ok()
        .map(|outline| outline.headers)
}
