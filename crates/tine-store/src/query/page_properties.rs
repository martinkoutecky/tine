use super::{property_key_norm, strip_ref};
use tine_core::doc::{DocBlock, Document};

/// Page-level properties and `tags::` values parsed from a page's pre-block.
pub(super) fn page_property_lines(text: &str, is_org: bool) -> Vec<(String, String)> {
    if !is_org {
        return text
            .lines()
            .filter_map(tine_core::doc::parse_property_line)
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
        if let Some(property) = tine_core::doc::parse_property_line(line) {
            props.push(property);
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

pub(super) fn page_document_is_org(doc: &Document) -> bool {
    doc.roots.first().map(DocBlock::is_org).unwrap_or_else(|| {
        doc.pre_block
            .as_deref()
            .is_some_and(|pre| pre.lines().any(|line| line.trim_start().starts_with("#+")))
    })
}

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
