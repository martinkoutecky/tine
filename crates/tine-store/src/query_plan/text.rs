//! The same search text supplies membership, ranking, snippets and evidence.
use super::*;
use std::borrow::Cow;

pub(super) fn pre_blocks_with_config(
    entry: &PageEntry,
    pre: &str,
    config: &tine_core::query::atom::ParseConfig,
) -> Vec<DocBlock> {
    use tine_core::page_properties::{pre_block_projections, PreBlockKind};
    let org = tine_core::model::Format::from_path(&entry.path) == tine_core::model::Format::Org;
    pre_block_projections(pre, org)
        .into_iter()
        .map(|(kind, mut block)| {
            block.set_property_ref_config(config);
            let prefix = match kind {
                PreBlockKind::Properties => "page-property",
                PreBlockKind::Preamble => "page-preamble",
            };
            block.uuid =
                format!("{prefix}:{:?}:{}", entry.kind, refs::page_key(&entry.name)).into();
            block
        })
        .collect()
}

pub(super) struct SearchText<'a> {
    pub original: Cow<'a, str>,
    pub folded: Cow<'a, str>,
}

#[inline]
pub(super) fn search_text<'a>(
    block: &'a DocBlock,
    plan: &QueryPlan,
    graph: &ReadSnapshot,
) -> SearchText<'a> {
    if !plan.include_user_properties {
        return SearchText {
            original: Cow::Borrowed(block.visible_text()),
            folded: Cow::Borrowed(block.visible_folded(plan.remove_accents)),
        };
    }
    let hidden = &graph.config().block_hidden_properties;
    if block.has_hidden_search_properties(hidden) {
        let original = block.search_text_excluding(hidden);
        let folded = plan.fold(&original);
        SearchText {
            original: Cow::Owned(original),
            folded: Cow::Owned(folded),
        }
    } else {
        let (original, folded) = block.search_text_pair(plan.remove_accents);
        SearchText {
            original: Cow::Borrowed(original),
            folded: Cow::Borrowed(folded),
        }
    }
}

/// Conservative prefilter: simple ASCII native property sources cannot invent
/// an alphabetic atom when projected. Everything else still takes the parser
/// path (YAML, escapes, Unicode, punctuation, regex, OR and negation included).
pub(super) fn pre_block_may_match(plan: &QueryPlan, expr: &QueryExpr, raw: &str) -> bool {
    fn required_atom(expr: &QueryExpr) -> Option<&str> {
        match expr {
            QueryExpr::Text(pred)
                if pred.field == TextField::VisibleContent
                    && pred.mode == TextMatchMode::Contains
                    && !pred.value.is_empty()
                    && pred.value.bytes().all(|b| b.is_ascii_alphabetic()) =>
            {
                Some(&pred.value)
            }
            QueryExpr::And(children) => children.iter().find_map(required_atom),
            _ => None,
        }
    }
    if raw.trim_start().starts_with("---")
        || !raw.bytes().all(|b| {
            b.is_ascii_alphabetic()
                || b.is_ascii_whitespace()
                || matches!(b, b':' | b'#' | b'+' | b'.' | b'-' | b'_')
        })
    {
        return true;
    }
    required_atom(expr).is_none_or(|atom| plan.fold(raw).contains(atom))
}
