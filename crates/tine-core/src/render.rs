//! Bridge to [`lsdoc`]: parse a Tine outline block's `raw` body into lsdoc's
//! render AST, reproducing OG's recipe.
//!
//! Tine owns the outline layer — a page is split into blocks each carrying `raw`
//! with the leading `- `/`* ` stripped and continuations de-indented (Logseq's
//! `:block/content`). To render, OG re-prepends the block pattern and re-parses
//! with mldoc (`frontend/format/block.cljs` `parse-title-and-body`), which breaks
//! out `marker`/`priority`/heading-`size` onto the first (bullet/heading) node.
//! We do exactly that so the AST carries the block-header semantics natively —
//! verified 1:1 against `mldoc@1.5.7` for this re-bulleted form (lsdoc v0.1.1).

use lsdoc::ast::Block;

/// Parse one block body into lsdoc's block AST the way OG feeds mldoc: re-prepend
/// the block pattern (`-` Markdown / `*` Org) to the de-indented `raw`, then parse.
/// The first returned [`Block`] is the bullet/heading carrying the block header
/// (marker / priority / heading size); any continuation constructs follow as
/// sibling blocks.
pub fn parse_block(raw: &str, is_org: bool) -> Vec<Block> {
    let (pattern, fmt) = if is_org { ("*", "org") } else { ("-", "md") };
    // `trim_start` mirrors OG's `(string/triml content)` — only the leading run is
    // touched; interior continuation lines keep their (already de-indented) text.
    let input = format!("{pattern} {}", raw.trim_start());
    lsdoc::parse(&input, fmt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsdoc::ast::{Block, Inline, Url};

    #[test]
    fn marker_and_priority_break_out_onto_the_bullet() {
        let blocks = parse_block("TODO [#A] do the thing", false);
        match &blocks[0] {
            Block::Bullet { marker, priority, inline, .. } => {
                assert_eq!(marker.as_deref(), Some("TODO"));
                assert_eq!(priority.as_deref(), Some("A"));
                // the marker/priority are NOT left in the rendered inline text
                assert!(matches!(inline.first(), Some(Inline::Plain { text }) if text == "do the thing"));
            }
            other => panic!("expected bullet, got {other:?}"),
        }
    }

    #[test]
    fn heading_block_keeps_its_level_in_size() {
        // The v0.1.1 fix: a `##`-style heading block carries `size` on the bullet.
        let blocks = parse_block("## A heading", false);
        match &blocks[0] {
            Block::Bullet { size, .. } => assert_eq!(*size, Some(2)),
            other => panic!("expected bullet, got {other:?}"),
        }
    }

    #[test]
    fn inline_refs_and_image_survive() {
        let blocks = parse_block("see [[Page]] and ![alt](a.png){:width 50%}", false);
        let Block::Bullet { inline, .. } = &blocks[0] else { panic!("expected bullet") };
        // page ref present
        assert!(inline.iter().any(|s| matches!(s, Inline::Link { url: Url::PageRef { v }, .. } if v == "Page")));
        // image flagged + metadata carried
        assert!(inline.iter().any(|s| matches!(s, Inline::Link { image, metadata, .. } if *image && metadata == "{:width 50%}")));
    }

    #[test]
    fn block_opener_splits_into_a_sibling() {
        // v0.1.1: `- ---` splits into [empty bullet, hr] rather than literal "---".
        let blocks = parse_block("---", false);
        assert!(blocks.iter().any(|b| matches!(b, Block::Hr { .. })), "expected an Hr sibling: {blocks:?}");
    }

    #[test]
    fn org_uses_the_star_pattern() {
        let blocks = parse_block("DONE finished", true);
        match &blocks[0] {
            Block::Bullet { marker, .. } => assert_eq!(marker.as_deref(), Some("DONE")),
            other => panic!("expected bullet, got {other:?}"),
        }
    }
}
