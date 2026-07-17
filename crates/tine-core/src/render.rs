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

#[path = "../../lsdoc-block-parse.rs"]
mod lsdoc_block_parse;

/// Parse one block body into lsdoc's block AST the way OG feeds mldoc: re-prepend
/// the block pattern (`-` Markdown / `*` Org) to the de-indented `raw`, then parse.
/// Tine's shared bridge applies one deliberate correction first: a line-leading
/// Markdown inline-code span containing `::` is code, not block metadata.
/// The first returned [`Block`] is the bullet/heading carrying the block header
/// (marker / priority / heading size); any continuation constructs follow as
/// sibling blocks.
pub fn parse_block(raw: &str, is_org: bool) -> Vec<Block> {
    lsdoc_block_parse::parse_block(raw, is_org)
}

/// OG-compatible inline references for one block body — the index path. Fed the
/// re-bulleted form (like OG) so a `TODO [#A]` marker/priority isn't mis-read as
/// a `#A` tag. Returns page names (original case) + UUID-gated block ids, each
/// sorted+deduped (`lsdoc::refs`). Tine layers tags::/alias::/namespace + rename
/// on top of this; `refs.rs` keeps that app-layer.
pub fn block_refs(raw: &str, is_org: bool) -> lsdoc::ast::Refs {
    lsdoc_block_parse::parse_projection(raw, is_org).refs
}

/// One block body → the FULL lsdoc projection (`{ blocks, refs }`) in a SINGLE parse.
/// `lsdoc::parse` and `lsdoc::refs` each call `parse_format` internally, so calling
/// both (as `projection()` did) parses the same block twice; this parses once and the
/// caller takes `.blocks` (facets / visible / body) and `.refs` (backlinks).
pub fn parse_projection(raw: &str, is_org: bool) -> lsdoc::ast::Projection {
    lsdoc_block_parse::parse_projection(raw, is_org)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsdoc::ast::{Block, Inline, Url};

    #[test]
    fn marker_and_priority_break_out_onto_the_bullet() {
        let blocks = parse_block("TODO [#A] do the thing", false);
        match &blocks[0] {
            Block::Bullet {
                marker,
                priority,
                inline,
                ..
            } => {
                assert_eq!(marker.as_deref(), Some("TODO"));
                assert_eq!(priority.as_deref(), Some("A"));
                // the marker/priority are NOT left in the rendered inline text
                assert!(
                    matches!(inline.first(), Some(Inline::Plain { text, .. }) if text == "do the thing")
                );
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
        let Block::Bullet { inline, .. } = &blocks[0] else {
            panic!("expected bullet")
        };
        // page ref present
        assert!(inline
            .iter()
            .any(|s| matches!(s, Inline::Link { url: Url::PageRef { v }, .. } if v == "Page")));
        // image flagged + metadata carried
        assert!(inline.iter().any(|s| matches!(s, Inline::Link { image, metadata, .. } if *image && metadata == "{:width 50%}")));
    }

    #[test]
    fn block_opener_splits_into_a_sibling() {
        // v0.1.1: `- ---` splits into [empty bullet, hr] rather than literal "---".
        let blocks = parse_block("---", false);
        assert!(
            blocks.iter().any(|b| matches!(b, Block::Hr { .. })),
            "expected an Hr sibling: {blocks:?}"
        );
    }

    #[test]
    fn org_uses_the_star_pattern() {
        let blocks = parse_block("DONE finished", true);
        match &blocks[0] {
            Block::Bullet { marker, .. } => assert_eq!(marker.as_deref(), Some("DONE")),
            other => panic!("expected bullet, got {other:?}"),
        }
    }

    #[test]
    fn leading_inline_code_property_lookalike_stays_code() {
        let projection = parse_projection("`tine.view:: [[Inside]] grid` and [[Outside]]", false);
        assert!(
            projection
                .blocks
                .iter()
                .all(|block| !matches!(block, Block::Properties { .. })),
            "an inline-code span must not become block metadata: {:?}",
            projection.blocks
        );
        let Block::Bullet { inline, .. } = &projection.blocks[0] else {
            panic!("expected a bullet: {:?}", projection.blocks)
        };
        assert!(inline
            .iter()
            .any(|node| matches!(node, Inline::Code { text, .. } if text == "tine.view:: [[Inside]] grid")));
        assert_eq!(projection.refs.page, vec!["Outside"]);
    }

    #[test]
    fn leading_code_guard_is_narrow_and_supports_backtick_runs() {
        let ordinary = parse_projection("tine.view:: grid", false);
        assert!(ordinary
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Properties { .. })));

        let double = parse_projection("``a:: b`` tail", false);
        let Block::Bullet { inline, .. } = &double.blocks[0] else {
            panic!("expected a bullet: {:?}", double.blocks)
        };
        assert!(inline
            .iter()
            .any(|node| matches!(node, Inline::Code { text, .. } if text == "a:: b")));
        assert!(double
            .blocks
            .iter()
            .all(|block| !matches!(block, Block::Properties { .. })));
    }

    #[test]
    fn line_leading_code_guard_covers_continuations_and_multiline_spans() {
        let later = parse_projection("body\n`tine.view:: [[Inside]] grid` and [[Outside]]", false);
        assert!(later
            .blocks
            .iter()
            .all(|block| !matches!(block, Block::Properties { .. })));
        assert_eq!(later.refs.page, vec!["Outside"]);
        assert!(later.blocks.iter().any(|block| match block {
            Block::Paragraph { inline, .. } => inline.iter().any(
                |node| matches!(node, Inline::Code { text, .. } if text == "tine.view:: [[Inside]] grid"),
            ),
            _ => false,
        }));

        let after_property = parse_projection("real:: x\n`tine.view:: grid`", false);
        assert!(after_property.blocks.iter().any(
            |block| matches!(block, Block::Properties { props, .. } if props.iter().any(|prop| prop.0 == "real" && prop.1 == "x")),
        ));
        assert!(after_property.blocks.iter().any(|block| match block {
            Block::Paragraph { inline, .. } => inline.iter().any(
                |node| matches!(node, Inline::Code { text, .. } if text == "tine.view:: grid"),
            ),
            _ => false,
        }));

        let multiline = parse_projection("`a:: b\ncontinued` tail", false);
        let Block::Bullet { inline, .. } = &multiline.blocks[0] else {
            panic!("expected a bullet: {:?}", multiline.blocks)
        };
        assert!(inline
            .iter()
            .any(|node| matches!(node, Inline::Code { text, .. } if text == "a:: b\ncontinued"),));
        assert!(multiline
            .blocks
            .iter()
            .all(|block| !matches!(block, Block::Properties { .. })));
    }

    #[test]
    fn line_leading_code_guard_is_collision_proof_and_keeps_utf8_spans() {
        let raw = "`a:: ;; é:: 中` tail";
        let projection = parse_projection(raw, false);
        let Block::Bullet { inline, .. } = &projection.blocks[0] else {
            panic!("expected a bullet: {:?}", projection.blocks)
        };
        assert!(inline.iter().any(|node| matches!(
            node,
            Inline::Code { text, span: Some(lsdoc::ast::Span(2, 19)) }
                if text == "a:: ;; é:: 中"
        )));
    }

    #[test]
    fn multiline_guard_handles_other_tick_runs_and_does_not_leak_fence_state() {
        let double = parse_projection("``a:: b\ncontinued`` tail", false);
        let Block::Bullet { inline, .. } = &double.blocks[0] else {
            panic!("expected a bullet: {:?}", double.blocks)
        };
        assert!(inline
            .iter()
            .any(|node| matches!(node, Inline::Code { text, .. } if text == "a:: b\ncontinued"),));

        let nested_ticks = parse_projection("`a:: b\n``inside``\ncontinued` tail", false);
        let Block::Bullet { inline, .. } = &nested_ticks.blocks[0] else {
            panic!("expected a bullet: {:?}", nested_ticks.blocks)
        };
        assert!(inline.iter().any(|node| matches!(
            node,
            Inline::Code { text, .. } if text == "a:: b\n``inside``\ncontinued"
        )));

        let fence_like = parse_projection("`a:: b\n```inside\ncontinued` tail\n`x:: y`", false);
        assert!(fence_like.blocks.iter().all(
            |block| !matches!(block, Block::Properties { props, .. } if props.iter().any(|prop| prop.0.contains('`'))),
        ));
        assert!(fence_like.blocks.iter().any(|block| match block {
            Block::Paragraph { inline, .. } => inline
                .iter()
                .any(|node| matches!(node, Inline::Code { text, .. } if text == "x:: y")),
            _ => false,
        }));
    }
}
