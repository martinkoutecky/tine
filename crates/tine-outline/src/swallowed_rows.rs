//! Metadata rows that mldoc folds into a final quote (GH #721, #895): the
//! kernel still owns them (regions) and the render AST drops them.

use crate::block_regions::{line_ranges, raw_range, trimmed_range, BlockRegions, Property, Range};
use lsdoc::ast::{Block, Inline};

/// Built-in metadata rows that mldoc folds into a final quote's lazy
/// continuation (GH #721). OG writes `collapsed:: true` after the last line of a
/// block, so for `Outside\n> Something\ncollapsed:: true` the parser answers
/// "quote text" and no property node. OG nevertheless hides, replaces and
/// removes such a line by text (`property/remove-built-in-properties`,
/// `remove-property`), so the kernel must own it too, or every collapse toggle
/// appends another row and the rows render as text. Only [`SWALLOWED_KEYS`]
/// qualify, only as the trailing
/// unquoted rows of the block's last quote (a `>` line stops the scan, so quoted
/// content is never touched), and only when nothing but accepted property nodes
/// follows the quote. O(bytes of those trailing rows).
pub(crate) fn add_regions(raw: &str, blocks: &[Block], out: &mut BlockRegions) {
    let mut found = rows(raw, blocks);
    if found.is_empty() {
        return;
    }
    let region = Range(found[0].line.0, found.last().unwrap().line.1);
    for p in &mut found {
        p.region = out.property_regions.len();
    }
    out.property_regions.push(region);
    out.properties.extend(found);
    out.properties.sort_by_key(|p| p.line.0);
}

/// Hidden metadata claimed after a final quote, including Tine's per-block author.
/// Not `id`
/// (GH #721 review B1): mldoc ends a quote at any `id:: ` row
/// case-insensitively, lsdoc only at lowercase, so a swallowed `ID::` row is
/// one lsdoc's Properties facet (the backend uuid index) does not see;
/// claiming it would give the block a frontend-only identity, and Copy block
/// ref would hand out an unresolvable `((uuid))`. A lowercase `id::` row is a
/// Properties node already and needs no claim.
const SWALLOWED_KEYS: [&str; 3] = [
    "collapsed",
    "logseq.order-list-type",
    crate::tine_keys::AUTHOR,
];

/// Tine's author chrome reads the same recovered row as its editor. OG has no
/// special meaning for this key; fold/list facets retain their existing policy.
pub fn extend_author_facet(properties: &mut Vec<(String, String)>, regions: &BlockRegions) {
    if properties
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(crate::tine_keys::AUTHOR))
    {
        return;
    }
    properties.extend(
        regions
            .properties
            .iter()
            .filter(|p| p.primary && p.key.eq_ignore_ascii_case(crate::tine_keys::AUTHOR))
            .map(|p| (p.key.clone(), p.value.clone())),
    );
}

/// The swallowed rows of [`add_regions`], in source order
/// (`region` unset).
fn rows(raw: &str, blocks: &[Block]) -> Vec<Property> {
    let Some(quote) = blocks
        .iter()
        .rposition(|b| !matches!(b, Block::Properties { .. }))
        .and_then(|i| match &blocks[i] {
            Block::Quote { span, .. } => Some(span),
            _ => None,
        })
    else {
        return Vec::new();
    };
    let Some(range) = raw_range(raw, quote, false) else {
        return Vec::new();
    };
    let mut found: Vec<Property> = Vec::new();
    let lines = line_ranges(raw, range);
    for line in lines.iter().rev() {
        let t = trimmed_range(raw, *line);
        let s = t.slice(raw);
        if s.is_empty() && found.is_empty() {
            continue;
        }
        if s.starts_with('>') {
            break;
        }
        let row = s.split_once("::").filter(|(key, value)| {
            (value.is_empty() || value.starts_with(' '))
                && SWALLOWED_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key))
        });
        let Some((key, value)) = row else {
            // The first ordinary row above the swallowed ones ends the run.
            break;
        };
        let ks = t.0;
        let vs = ks + key.len() + 2;
        found.push(Property {
            key: key.to_string(),
            value: value.trim().to_string(),
            line: *line,
            key_range: Range(ks, ks + key.len()),
            value_range: trimmed_range(raw, Range(vs, t.1)),
            region: 0,
            primary: true,
            directive: false,
            applicable: true,
        });
    }
    // A quote needs at least one quoted row above the rows it swallowed.
    let content = lines
        .iter()
        .filter(|l| !trimmed_range(raw, **l).slice(raw).is_empty())
        .count();
    if found.len() >= content {
        return Vec::new();
    }
    found.reverse();
    found
}

/// How a Markdown block relates to quotes (GH #721 follow-up F1/F3). Writers use
/// it to place metadata BEFORE a quote, where the parser accepts it: a row
/// appended after a final quote is its lazy continuation, and a row put after
/// the first line of a titleless block lands inside that quote (the window's
/// `joinProps` #684 placement and OG's `insert-property`, which prepends to a
/// titleless quote, agree).
pub(crate) struct QuoteTail {
    /// Start of the line the final quote opens on, when the block's last
    /// non-property node is a quote.
    pub start: Option<usize>,
    /// Start of the line the block's FIRST non-property node opens on, when that
    /// node is itself a quote: the block has no title, its first line is quoted.
    pub titleless: Option<usize>,
    /// Line ranges of the swallowed built-in rows ([`SWALLOWED_KEYS`]).
    pub swallowed: Vec<Range>,
}

/// The quote facts of one raw Markdown block, or `None` when it has no
/// titleless first quote and does not end in a quote. One parse, only for a
/// block containing `>`, and only on the add/relocate paths of a property edit.
pub(crate) fn quote_tail(raw: &str) -> Option<QuoteTail> {
    if !raw.contains('>') {
        return None;
    }
    let blocks = crate::render::parse_block(raw, false);
    let line_start = |at: usize| raw[..at].rfind('\n').map_or(0, |p| p + 1);
    let quote_line = |b: &Block| match b {
        Block::Quote { span, .. } => raw_range(raw, span, false).map(|r| line_start(r.0)),
        _ => None,
    };
    // The header bullet of a titleless block carries no title text, marker or priority.
    let untitled = |b: &&Block| match b {
        Block::Properties { .. } => true,
        Block::Bullet {
            inline,
            marker: None,
            priority: None,
            ..
        } => inline.is_empty(),
        _ => false,
    };
    let titleless = blocks.iter().find(|b| !untitled(b)).and_then(quote_line);
    let start = blocks
        .iter()
        .rfind(|b| !matches!(b, Block::Properties { .. }))
        .and_then(quote_line);
    if start.is_none() && titleless.is_none() {
        return None;
    }
    Some(QuoteTail {
        start,
        titleless,
        swallowed: rows(raw, &blocks).iter().map(|p| p.line).collect(),
    })
}

/// The lines of the swallowed rows whose key is `key`, empty unless `key` is one
/// of [`SWALLOWED_KEYS`] (so an ordinary property edit never parses).
pub(crate) fn swallowed_lines(raw: &str, key: &str) -> Vec<Range> {
    if !SWALLOWED_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        return Vec::new();
    }
    let Some(tail) = quote_tail(raw) else {
        return Vec::new();
    };
    tail.swallowed
        .into_iter()
        .filter(|line| {
            let t = trimmed_range(raw, *line).slice(raw);
            t.split_once("::")
                .is_some_and(|(k, _)| k.eq_ignore_ascii_case(key))
        })
        .collect()
}

/// Remove the swallowed rows' text from the render AST so they are not painted
/// inside the quote (GH #721, #895). Source spans identify the start of the
/// owned trailing run, including author values parsed as markup or hard breaks.
/// An unexpected AST shape stays untouched rather than losing quoted content.
pub fn strip_swallowed_rows(raw: &str, blocks: &mut [Block]) {
    let found = rows(raw, blocks);
    let Some(first) = found.first() else {
        return;
    };
    let start = trimmed_range(raw, first.line).0;
    let Some(Block::Quote { children, .. }) = blocks
        .iter_mut()
        .rev()
        .find(|b| !matches!(b, Block::Properties { .. }))
    else {
        return;
    };
    let Some(Block::Paragraph { inline, .. }) = children.last_mut() else {
        return;
    };
    let at = inline.iter().position(|i| match i {
        Inline::Plain { span, .. } => raw_range(raw, span, false).is_some_and(|r| r.0 == start),
        _ => false,
    });
    if let Some(at) = at.filter(|at| {
        *at > 0
            && matches!(
                inline[*at - 1],
                Inline::Break { .. } | Inline::HardBreak { .. }
            )
    }) {
        inline.truncate(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_author_text_is_stripped_with_markup_and_hard_breaks() {
        for nl in ["\n", "\r\n"] {
            for author in ["Ada", "Ada Test  \t", "**Ada**", "[[Ada]]", "艾达"] {
                let raw = format!("> quoted words{nl}tine.author:: {author}");
                let mut blocks = crate::render::parse_block(&raw, false);
                strip_swallowed_rows(&raw, &mut blocks);
                let json = lsdoc::blocks_to_json(&blocks).unwrap();
                assert!(!json.contains("tine.author"), "{raw:?}: {json}");
                assert!(json.contains("quoted words"), "{json}");
            }
        }
    }

    /// GH #721 review B1: an uppercase `ID::` row lsdoc folds into a quote is
    /// never claimed (lsdoc's facets do not see it as an id).
    #[test]
    fn a_swallowed_id_row_is_not_claimed() {
        for raw in ["> a\nID:: x", "> a\nId:: x"] {
            let blocks = crate::render::parse_block(raw, false);
            assert!(rows(raw, &blocks).is_empty(), "{raw:?}");
        }
        // Since lsdoc v0.5.9 the quote ends at `ID::` (as mldoc), so `ID:: x` is a
        // Properties node the backend sees; only the swallowed `collapsed::` row
        // above it is claimed.
        let raw = "> a\ncollapsed:: true\nID:: x";
        let blocks = crate::render::parse_block(raw, false);
        assert!(
            matches!(blocks.last(), Some(Block::Properties { .. })),
            "{blocks:?}"
        );
        let found = rows(raw, &blocks);
        assert_eq!(
            found.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
            ["collapsed"],
            "{raw:?}"
        );
        let raw = "> a\ncollapsed:: true";
        let blocks = crate::render::parse_block(raw, false);
        assert_eq!(rows(raw, &blocks).len(), 1);
    }
}
