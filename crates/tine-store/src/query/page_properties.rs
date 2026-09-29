use super::{property_key_norm, strip_ref};
use tine_core::doc::{DocBlock, Document};

/// Parse Markdown or Org page-property syntax; skip malformed lines.
/// O(input text length), without external I/O.
pub(super) fn page_property_lines(text: &str, is_org: bool) -> Vec<(String, String)> {
    if !is_org {
        return text
            .lines()
            .filter_map(tine_core::doc::parse_property_line)
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
    }
    let mut props = Vec::new();
    let mut in_drawer = false;
    for line in text.lines() {
        let line = line.trim();
        if line.eq_ignore_ascii_case(":PROPERTIES:") {
            in_drawer = true;
            continue;
        }
        if line.eq_ignore_ascii_case(":END:") {
            in_drawer = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("#+") {
            if let Some((key, value)) = rest.split_once(':') {
                if !key.is_empty()
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    props.push((key.to_ascii_lowercase(), value.trim().to_owned()));
                }
            }
        } else if in_drawer {
            if let Some(rest) = line.strip_prefix(':') {
                if let Some((key, value)) = rest.split_once(':') {
                    if !key.is_empty()
                        && key
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    {
                        props.push((key.to_ascii_lowercase(), value.trim().to_owned()));
                    }
                }
            }
        }
    }
    props
}

/// Use the first root's format, or preblock Org markers if there is no root.
/// O(first root metadata or preblock lines).
pub(super) fn page_document_is_org(doc: &Document) -> bool {
    doc.roots.first().map(DocBlock::is_org).unwrap_or_else(|| {
        doc.pre_block.as_deref().is_some_and(|pre| {
            pre.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("#+") || line.eq_ignore_ascii_case(":PROPERTIES:")
            })
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
                    .split(',')
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
