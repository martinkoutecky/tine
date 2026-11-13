//! Comment policy shared by publication planning and print. Projection visits
//! admitted blocks once, dropping a root's entire thread; page roots stay body
//! blocks. No I/O or live-document mutations. The predicate is the editor's
//! Rust discriminator, also used by the window through wasm.
use tine_core::{doc::DocBlock, model::BlockDto, CorpusPage};

#[cfg(test)]
thread_local! { static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(crate) fn is_comment(block: &DocBlock, has_parent: bool) -> bool {
    #[cfg(test)]
    VISITS.with(|visits| visits.set(visits.get() + 1));
    is_comment_properties(block.projection().properties(), has_parent)
}

pub(crate) fn is_comment_properties(properties: &[(String, String)], has_parent: bool) -> bool {
    has_parent
        && tine_core::editor_text::comment_quote(
            properties.iter().map(|(k, v)| (k.as_str(), v.as_str())),
        )
        .is_some()
}

/// Close the selecting query's displayed rows to the projected body too.
pub(crate) fn close_result(result: &mut tine_core::query::ir::QueryResult, pages: &[CorpusPage]) {
    use tine_core::query::ir::QueryRows;
    let QueryRows::Block { groups } = &mut result.rows else {
        return;
    };
    let mut ids = std::collections::HashSet::new();
    fn collect<'a>(blocks: &'a [DocBlock], ids: &mut std::collections::HashSet<&'a str>) {
        for block in blocks {
            ids.insert(block.uuid.as_str());
            collect(&block.children, ids);
        }
    }
    for page in pages {
        collect(&page.document.roots, &mut ids);
    }
    for group in groups.iter_mut() {
        group.blocks.retain(|block| ids.contains(block.id.as_str()));
    }
    groups.retain(|group| !group.blocks.is_empty());
    result.total = groups.iter().map(|group| group.blocks.len()).sum();
    result.matched_total = Some(result.total);
    result.statistics = None;
}

/// O(source blocks + query rows), O(query rows) temporary pointers, no I/O.
/// Omission changes sibling paths and therefore runtime ids in the private
/// build graph. Carry the reviewed rows onto those published identities.
pub(crate) fn remap_result_ids(
    result: &mut tine_core::query::ir::QueryResult,
    source: &[CorpusPage],
    published: &[CorpusPage],
) {
    use tine_core::query::ir::QueryRows;
    let QueryRows::Block { groups } = &mut result.rows else {
        return;
    };
    let wanted: std::collections::HashSet<_> = groups
        .iter()
        .flat_map(|group| group.blocks.iter().map(|block| block.id.as_str()))
        .collect();
    let mut ids = std::collections::HashMap::new();
    fn align<'a, 'b>(
        source: &'a [DocBlock],
        published: &'b [DocBlock],
        wanted: &std::collections::HashSet<&str>,
        ids: &mut std::collections::HashMap<&'a str, &'b str>,
    ) {
        for (source, published) in source.iter().zip(published) {
            if wanted.contains(source.uuid.as_str()) {
                ids.insert(source.uuid.as_str(), published.uuid.as_str());
            }
            align(&source.children, &published.children, wanted, ids);
        }
    }
    // with_build_graph returns its corpus in the source pages' order.
    for (source, published) in source.iter().zip(published) {
        align(
            &source.document.roots,
            &published.document.roots,
            &wanted,
            &mut ids,
        );
    }
    drop(wanted);
    for group in groups {
        for block in &mut group.blocks {
            if let Some(id) = ids.get(block.id.as_str()).filter(|id| **id != block.id) {
                block.id = (*id).to_owned();
            }
        }
    }
}

/// Absent means include; an explicit request overrides the definition property.
pub(crate) fn included(choice: Option<bool>, properties: &[(String, String)]) -> bool {
    choice.unwrap_or_else(|| {
        !properties.iter().any(|(k, v)| {
            tine_core::doc::property_key_norm(k) == "tine.comments" && v.trim() == "false"
        })
    })
}

pub(crate) fn project(pages: &mut [CorpusPage], include: bool) {
    if include {
        return;
    }
    fn body(blocks: &mut [DocBlock]) {
        for block in blocks {
            block.children.retain(|child| !is_comment(child, true));
            body(&mut block.children);
        }
    }
    fn has_comments(blocks: &[DocBlock]) -> bool {
        blocks.iter().any(|block| {
            block.children.iter().any(|child| is_comment(child, true))
                || has_comments(&block.children)
        })
    }
    for page in pages {
        // Keep body-only pages shared with the held source. The prefix probe
        // avoids cloning a whole parsed document when exclusion changes nothing.
        if has_comments(&page.document.roots) {
            body(&mut std::sync::Arc::make_mut(&mut page.document).roots);
        }
    }
}

/// Serializer-owned markup; all graph-supplied metadata is escaped as text.
pub(crate) fn open_annotation(properties: &[(String, String)], out: &mut String) {
    fn escape(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    out.push_str("<aside class=\"comment\"><header>Comment");
    for key in ["tine.author", "tine.date"] {
        if let Some((_, value)) = properties
            .iter()
            .find(|(k, v)| tine_core::doc::property_key_norm(k) == key && !v.trim().is_empty())
        {
            out.push_str(" · ");
            out.push_str(&escape(&value));
        }
    }
    out.push_str("</header>");
}

pub(crate) trait OutlineNode: Sized {
    fn properties(&self) -> &[(String, String)];
    fn raw_text(&self) -> &str;
    fn child_nodes(&self) -> &[Self];
}

impl OutlineNode for BlockDto {
    fn properties(&self) -> &[(String, String)] {
        &self.properties
    }
    fn raw_text(&self) -> &str {
        &self.raw
    }
    fn child_nodes(&self) -> &[Self] {
        &self.children
    }
}

impl OutlineNode for DocBlock {
    fn properties(&self) -> &[(String, String)] {
        self.projection().properties()
    }
    fn raw_text(&self) -> &str {
        self.raw()
    }
    fn child_nodes(&self) -> &[Self] {
        &self.children
    }
}

/// A subtree's raw text, one block per line in preorder (the over-deep fallback).
pub(crate) fn flat_text<N: OutlineNode>(root: &N) -> String {
    let mut text = String::new();
    let mut stack = vec![root];
    while let Some(block) = stack.pop() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(block.raw_text());
        stack.extend(block.child_nodes().iter().rev());
    }
    text
}

pub(crate) fn collect_wanted_doc_blocks<'a>(
    blocks: &'a [DocBlock],
    wanted: &std::collections::HashSet<&str>,
    found: &mut std::collections::HashMap<&'a str, (&'a DocBlock, bool)>,
) {
    let mut stack: Vec<_> = blocks.iter().rev().map(|b| (b, false, false)).collect();
    while let Some((block, thread, has_parent)) = stack.pop() {
        let comment = thread || crate::output_comments::is_comment(block, has_parent);
        if wanted.contains(block.uuid.as_str()) {
            found.insert(block.uuid.as_str(), (block, comment));
        }
        // OG can retain a matching descendant below a non-matching child of a
        // retained ancestor. Keep walking so both roots hydrate from source.
        stack.extend(block.children.iter().rev().map(|b| (b, comment, true)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tine_core::{
        doc,
        model::{PageId, PageKind},
    };

    #[test]
    fn exclusion_clones_only_changed_pages_and_preserves_the_held_source() {
        for count in [1, 60] {
            let raw = "- Body\n  - Child\n  - Note\n    tine.quote:: \n    - Reply\n      - Nested reply\n".repeat(count);
            let source = Arc::new(doc::parse(&raw));
            let mut pages = vec![CorpusPage {
                id: PageId::from("pages/Example.md".to_owned()),
                name: "Example".into(),
                kind: PageKind::Page,
                document: source.clone(),
            }];
            VISITS.with(|visits| visits.set(0));
            project(&mut pages, true);
            assert!(Arc::ptr_eq(&source, &pages[0].document));
            assert_eq!(VISITS.with(|visits| visits.get()), 0);
            let start = std::time::Instant::now();
            project(&mut pages, false);
            assert_eq!(VISITS.with(|visits| visits.get()), 2 + 2 * count);
            assert!(start.elapsed() < std::time::Duration::from_millis(250));
            assert_eq!(pages[0].document.roots.len(), count);
            assert!(pages[0]
                .document
                .roots
                .iter()
                .all(|b| b.children.len() == 1 && b.children[0].raw() == "Child"));
            assert_eq!(source.roots[0].children.len(), 2);
            let body = Arc::new(doc::parse(
                &"- Body\n  - Child\n  - Another child\n".repeat(count),
            ));
            pages[0].document = body.clone();
            VISITS.with(|visits| visits.set(0));
            let start = std::time::Instant::now();
            project(&mut pages, false);
            assert!(Arc::ptr_eq(&body, &pages[0].document));
            assert_eq!(VISITS.with(|visits| visits.get()), 2 * count);
            assert!(start.elapsed() < std::time::Duration::from_millis(250));
        }
    }

    #[test]
    fn root_quotes_and_code_are_body_and_only_the_approved_property_excludes() {
        let document =
            doc::parse("- Root\n  tine.quote:: x\n  - ```\n    tine.quote:: literal\n    ```\n");
        assert!(!is_comment(&document.roots[0], false));
        assert!(!is_comment(&document.roots[0].children[0], true));
        assert!(included(None, &[]));
        assert!(!included(None, &[("TINE.comments".into(), "false".into())]));
        assert!(included(
            Some(true),
            &[("tine.comments".into(), "false".into())]
        ));
    }
}
