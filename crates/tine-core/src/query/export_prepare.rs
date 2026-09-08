//! Source preparation for snapshot-owned export batches.

use super::{ir::ViewSettings, QueryExportSpec};

/// One macro bound under the execution day shared by its export batch.
#[derive(Clone)]
pub(crate) struct PreparedExportQuery {
    pub(crate) query: Option<crate::query::ir::Query>,
    pub(crate) view: ViewSettings,
}

/// Prepare only the macros the existing export selection cap processes.
/// Registry vocabulary affects diagnostic suggestions, which export does not
/// expose; property coercion still uses the executor's captured registry.
pub(crate) fn prepare_export_queries(
    specs: &[QueryExportSpec],
    max_queries: usize,
    today: crate::date::JournalDate,
) -> Vec<PreparedExportQuery> {
    specs
        .iter()
        .take(max_queries.max(1))
        .map(|spec| {
            let (query, view) = if spec.advanced {
                match crate::query::resolve_advanced_source_at(
                    &spec.query,
                    spec.current_page.as_deref(),
                    today,
                ) {
                    crate::query::ResolvedAdvanced::Executable { query, .. } => {
                        (Some(query), ViewSettings::default())
                    }
                    crate::query::ResolvedAdvanced::Refused(_) => (None, ViewSettings::default()),
                }
            } else {
                let (query, view) =
                    crate::query::parse_query_text(&spec.query, spec.simple_dialect(), today);
                (Some(query), view)
            };
            PreparedExportQuery {
                query: query
                    .filter(|query| !query.is_invalid())
                    .map(|query| crate::query::block_anchored_query(&query)),
                view,
            }
        })
        .collect()
}
