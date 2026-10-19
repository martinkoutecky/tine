//! Named templates: the one answer to "which blocks does this block define as
//! a template, and what do they insert". A template is a block whose
//! `template::` property (key folded by `property_key_norm`) is non-empty.
//! Graph-wide discovery (`tine-store`'s template listing) and the file-scoped
//! daily-template lookup (`tine-graph-features`) both call this, so the two
//! cannot disagree about a block (BqbeQ).

use crate::doc::DocBlock;
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
}
