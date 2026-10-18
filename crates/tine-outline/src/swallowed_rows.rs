//! Built-in metadata rows that mldoc folds into a final quote (GH #721): the
//! kernel still owns them (regions) and the render AST drops them.

use crate::block_regions::{line_ranges, raw_range, trimmed_range, BlockRegions, Property, Range};
use lsdoc::ast::{Block, Inline};

/// Built-in metadata rows that mldoc folds into a final quote's lazy
/// continuation (GH #721). OG writes `collapsed:: true` after the last line of a
/// block, so for `Outside\n> Something\ncollapsed:: true` the parser answers
/// "quote text" and no property node. OG nevertheless hides, replaces and
/// removes such a line by text (`property/remove-built-in-properties`,
/// `remove-property`), so the kernel must own it too, or every collapse toggle
/// appends another row and the rows render as text. Only the hidden built-ins
/// (`id`, `collapsed`, `logseq.order-list-type`) qualify, only as the trailing
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
                && crate::editor_text::BUILTIN_HIDDEN
                    .iter()
                    .any(|k| k.eq_ignore_ascii_case(key))
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

/// Remove the swallowed rows' text from the render AST so they are not painted
/// inside the quote (GH #721). The final quote paragraph ends
/// `[…, Plain(row), Break]` once per row; any other shape is left untouched, so
/// the worst case is the row staying visible, never quoted text lost.
pub fn strip_swallowed_rows(raw: &str, blocks: &mut [Block]) {
    let n = rows(raw, blocks).len();
    if n == 0 {
        return;
    }
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
    let is_row = |i: &Inline| match i {
        Inline::Plain { text, .. } => text.split_once("::").is_some_and(|(key, _)| {
            crate::editor_text::BUILTIN_HIDDEN
                .iter()
                .any(|k| k.eq_ignore_ascii_case(key.trim()))
        }),
        _ => false,
    };
    let tail = 2 * n;
    let at = inline.len().saturating_sub(tail);
    let shaped = inline.len() >= tail + 2
        && inline[at..]
            .chunks(2)
            .all(|pair| is_row(&pair[0]) && matches!(pair.get(1), Some(Inline::Break { .. })));
    if shaped {
        inline.truncate(at);
    }
}
