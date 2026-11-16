//! Named templates: the one answer to "which blocks does this block define as
//! a template, and what do they insert". A template is a block whose
//! `template::` property (key folded by `property_key_norm`) is non-empty.
//! A page's property header (the document pre-block) is such a block too, as
//! OG's `get-all-templates` / `get-template-by-name` match any entity whose
//! `:block/properties` has `:template`, the pre-block included
//! (GH discussion #729); [`templates_in_document`] is the walk that sees it.
//! Graph-wide discovery (`tine-store`'s template listing) and the file-scoped
//! daily-template lookup (`tine-graph-features`) both call this, so the two
//! cannot disagree about a block (BqbeQ).

use crate::doc::{DocBlock, Document};
use crate::model::BlockDto;

/// The template `b` defines, if any: its name as written (the projected
/// `template::` value) and the blocks it inserts. Roots are included unless
/// `template-including-parent:: false`; inserted copies drop `id::` and, at an
/// included root, the `template*` properties. O(copied subtree).
pub fn template_of(b: &DocBlock) -> Option<(String, Vec<BlockDto>)> {
    let name = b.property("template")?;
    if name.is_empty() {
        return None;
    }
    let include_parent = b.property("template-including-parent").as_deref() != Some("false");
    let blocks = if include_parent {
        vec![template_copy(b, true)]
    } else {
        b.children.iter().map(|c| template_copy(c, false)).collect()
    };
    Some((name, blocks))
}

/// Every template of one page: its property header first (see the module
/// docs), then `doc.roots` as [`templates_in`]. The header is the one projection
/// [`crate::page_properties::document_page_property_block`] gives every other
/// pre-block answerer, so only parser-owned property lines count. It has no
/// children, so with `template-including-parent:: false` it inserts nothing,
/// as in OG (`(drop 1 sorted-blocks)` of a one-block tree). The header is
/// parsed only when its text can hold the key at all. O(page).
pub fn templates_in_document(doc: &Document, each: &mut impl FnMut(String, Vec<BlockDto>)) {
    if let Some(pre) = doc.pre_block.as_deref() {
        if may_define_template(pre.as_bytes()) {
            if let Some((name, blocks)) = crate::page_properties::document_page_property_block(doc)
                .as_ref()
                .and_then(template_of)
            {
                each(name, blocks);
            }
        }
    }
    templates_in(&doc.roots, each);
}

/// Whether these bytes can hold a block that defines a template: a property
/// key folds to `template` only by ASCII lowercasing, so its bytes contain
/// `template` in some ASCII case. O(bytes), no allocation.
pub fn may_define_template(bytes: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"template";
    // `b | 0x20 == b't'` holds exactly for `t` and `T`: a cheap first byte.
    bytes.iter().enumerate().any(|(at, &b)| {
        b | 0x20 == b't'
            && bytes
                .get(at..at + NEEDLE.len())
                .is_some_and(|window| window.eq_ignore_ascii_case(NEEDLE))
    })
}

/// Every template under `roots`, in depth-first preorder (a template's own
/// subtree is still visited). O(blocks plus copied subtrees).
pub fn templates_in(roots: &[DocBlock], each: &mut impl FnMut(String, Vec<BlockDto>)) {
    for b in roots {
        if let Some((name, blocks)) = template_of(b) {
            each(name, blocks);
        }
        templates_in(&b.children, each);
    }
}

/// Convert a template block subtree to a DTO, dropping `id::` (so inserted
/// copies get fresh ids) and, at the root, the `template*` properties.
pub fn template_copy(b: &DocBlock, strip_template: bool) -> BlockDto {
    let raw = b
        .projection()
        .regions
        .apply(
            b.raw(),
            b.is_org(),
            crate::block_regions::Edit::StripCopy {
                template: strip_template,
            },
        )
        .expect("parsed template");
    BlockDto {
        id: String::new(),
        raw,
        collapsed: b.collapsed(),
        children: b.children.iter().map(|c| template_copy(c, false)).collect(),
        breadcrumb: Vec::new(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_copy_drops_markdown_and_org_ids_regardless_of_case() {
        let block = DocBlock::new("body\nID:: md-id\n:PROPERTIES:\n:Id: org-id\n:END:\nkeep:: yes");
        let copied = template_copy(&block, false);
        assert_eq!(copied.raw, "body\n:PROPERTIES:\n:END:\nkeep:: yes");
    }

    #[test]
    fn template_copy_preserves_collapsed_state() {
        for (raw, expected) in [
            ("section\ncollapsed:: true", true),
            ("section\ncollapsed:: false", false),
            ("section", false),
            ("section\n:PROPERTIES:\n:collapsed: true\n:END:", true),
            ("```\ncollapsed:: true\n```", false),
        ] {
            let mut block = DocBlock::new(raw);
            block.set_org(raw.contains(":PROPERTIES:"));
            block
                .children
                .push(DocBlock::new("nested\ncollapsed:: true"));
            let copied = template_copy(&block, false);
            assert_eq!(copied.collapsed, expected, "{raw}");
            assert_eq!(copied.raw, raw);
            assert!(copied.children[0].collapsed);
            assert!(copied.id.is_empty());
        }
    }

    #[test]
    fn template_copy_keeps_literal_metadata_and_drops_org_metadata() {
        let raw = "Task\ntemplate:: Example\nid:: real\n```\nid:: literal\ntemplate:: literal\n```";
        let b = DocBlock::new(raw);
        assert_eq!(
            template_copy(&b, true).raw,
            "Task\n```\nid:: literal\ntemplate:: literal\n```"
        );
        let mut b = DocBlock::new("Task\n:PROPERTIES:\n:template: Example\n:id: real\n:END:");
        b.set_org(true);
        assert_eq!(template_copy(&b, true).raw, "Task");
    }

    #[test]
    fn template_names_keys_and_parent_inclusion() {
        let mut parent = DocBlock::new("Daily\nTemplate:: Plan\ntemplate-including-parent:: false");
        parent.children.push(DocBlock::new("child\nid:: x"));
        let (name, blocks) = template_of(&parent).unwrap();
        assert_eq!(name, "Plan");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].raw, "child");
        let included = DocBlock::new("Root\ntemplate:: Plan B");
        let (name, blocks) = template_of(&included).unwrap();
        assert_eq!(name, "Plan B");
        assert_eq!(blocks[0].raw, "Root");
        assert!(template_of(&DocBlock::new("x\ntemplate::")).is_none());
        assert!(template_of(&DocBlock::new("x\nstatus:: template")).is_none());
        let mut nested = DocBlock::new("outer\ntemplate:: A");
        nested.children.push(DocBlock::new("inner\ntemplate:: B"));
        let mut seen = Vec::new();
        templates_in(std::slice::from_ref(&nested), &mut |name, _| {
            seen.push(name)
        });
        assert_eq!(seen, ["A", "B"]);
    }

    fn names_and_raws(text: &str) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        templates_in_document(&crate::doc::parse(text), &mut |name, blocks| {
            out.push((name, blocks.into_iter().map(|b| b.raw).collect()));
        });
        out
    }

    #[test]
    fn page_property_header_template_precedes_block_templates() {
        // GH discussion #729: OG lists a template:: in the page's property
        // header. The report's header, verbatim.
        let found = names_and_raws(
            "tags::  \nlink:: \ncreate-date:: <% today %>\ntemplate:: Usual note\n\n- row\n  template:: In a block\n",
        );
        assert_eq!(
            found,
            [
                (
                    "Usual note".to_string(),
                    vec!["tags:: \nlink:: \ncreate-date:: <% today %>".to_string()]
                ),
                ("In a block".to_string(), vec!["row".to_string()]),
            ]
        );
        // Only parser-owned property lines of the header count: not an empty
        // value, another key's value, prose, or a line inside a code fence.
        for text in [
            "template::\n\n- row\n",
            "status:: template\n",
            "intro with template:: Prose\n\n- row\n",
            "```\ntemplate:: Fenced\n```\n\n- row\n",
        ] {
            assert!(names_and_raws(text).is_empty(), "{text:?}");
        }
        // `template-including-parent:: false` leaves nothing to insert (OG
        // drops the only block of the one-block tree).
        assert_eq!(
            names_and_raws("template:: P\ntemplate-including-parent:: false\n"),
            [("P".to_string(), Vec::<String>::new())]
        );
    }

    #[test]
    fn org_header_templates_insert_the_header_without_the_template_key() {
        // `#+KEY:` directives and the head drawer are the Org page properties.
        assert_eq!(
            names_and_raws("#+TITLE: T\n#+TEMPLATE: Org one\n\n* Row\n"),
            [(
                "Org one".to_string(),
                vec![":PROPERTIES:\n:title: T\n:END:".to_string()]
            )]
        );
        assert_eq!(
            names_and_raws(":PROPERTIES:\n:template: Drawer\n:END:\n\n* Row\n"),
            [("Drawer".to_string(), vec![String::new()])]
        );
    }
}
