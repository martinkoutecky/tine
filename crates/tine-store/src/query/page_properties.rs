use super::{property_key_norm, strip_ref};
use tine_core::doc::{DocBlock, Document};

/// Page properties owned by lsdoc, including Org directives and head drawers.
/// O(input bytes + AST nodes), without I/O; literal/prose entries are excluded.
pub(crate) fn page_property_lines(text: &str, is_org: bool) -> Vec<(String, String)> {
    tine_core::block_regions::parse_document(text, is_org)
        .page_properties()
        .map(|p| {
            (
                if is_org {
                    p.key.to_ascii_lowercase()
                } else {
                    p.key.clone()
                },
                p.value.clone(),
            )
        })
        .collect()
}

/// The first root carries the format; an empty document's parser-owned Org
/// metadata distinguishes an Org drawer/directive preamble from Markdown.
pub(crate) fn page_document_is_org(doc: &Document) -> bool {
    doc.roots.first().map(DocBlock::is_org).unwrap_or_else(|| {
        doc.pre_block.as_deref().is_some_and(|pre| {
            tine_core::block_regions::parse_document(pre, true)
                .page_properties()
                .next()
                .is_some()
        })
    })
}

/// Extract property pairs and comma-separated tags only from document preblock.
/// A properties-only first root is excluded, unlike document_aliases. Malformed
/// lines are skipped. O(preblock text), without external I/O.
pub(super) fn page_facets(doc: &Document) -> (Vec<(String, String)>, Vec<String>) {
    let mut props = Vec::new();
    let mut tags = Vec::new();
    if let Some(pre) = doc.pre_block.as_deref() {
        for (k, v) in page_property_lines(pre, page_document_is_org(doc)) {
            if property_key_norm(&k) == "tags" {
                tags = v
                    .split(tine_core::refs::is_linkable_property_separator)
                    .map(|t| strip_ref(t.trim()))
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            props.push((k, v));
        }
    }
    (props, tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_preamble_does_not_treat_markdown_alias_as_page_property() {
        assert_eq!(
            page_property_lines("alias:: Ghost\n#+ALIAS: Novel", true),
            vec![("alias".into(), "Novel".into())],
            "I-12: Org page properties come from Org syntax; alias:: Ghost is Markdown syntax"
        );
    }

    #[test]
    fn full_width_comma_separates_page_tags_like_every_other_answerer() {
        let doc = Document {
            pre_block: Some("tags:: Alpha，Beta, Gamma".into()),
            roots: Vec::new(),
        };
        assert_eq!(
            page_facets(&doc).1,
            vec!["Alpha".to_string(), "Beta".into(), "Gamma".into()],
            "OG sep-by-comma splits on `,` and `，`; page tags must agree with aliases and refs"
        );
    }

    #[test]
    fn drawer_only_org_page_has_facets() {
        let doc = Document {
            pre_block: Some(":PROPERTIES:\n:alias: Vacant\n:END:".into()),
            roots: Vec::new(),
        };
        assert_eq!(
            page_facets(&doc).0,
            vec![("alias".into(), "Vacant".into())],
            "I-12: a drawer-only Org page still contributes its alias facet"
        );
    }

    #[test]
    fn org_page_aliases_follow_org_syntax_through_graph_resolution() {
        let dir =
            std::env::temp_dir().join(format!("tine-org-page-properties-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pages")).unwrap();
        std::fs::write(
            dir.join("pages/Book.org"),
            "alias:: Ghost\n#+ALIAS: Novel\n\n* chapter\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("pages/Empty.org"),
            ":PROPERTIES:\n:alias: Vacant\n:END:\n",
        )
        .unwrap();
        let (store, _, _) =
            crate::store::Store::open(&dir, crate::store::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap();
        assert!(
            !matches!(graph.resolve("Ghost", false), crate::Resolved::Alias { .. }),
            "I-12: plain alias:: is not an Org page alias"
        );
        assert!(matches!(
            graph.resolve("Novel", false),
            crate::Resolved::Alias { .. }
        ));
        assert!(
            matches!(
                graph.resolve("Vacant", false),
                crate::Resolved::Alias { .. }
            ),
            "I-12: a drawer-only .org file still contributes a page alias"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
