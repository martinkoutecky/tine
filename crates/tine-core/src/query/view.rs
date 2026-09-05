//! SPEC §7.1's two computations that are neither parse, print nor run: the
//! §4.1 precedence merge of block properties into the view, and §Q14/N19's
//! explanation of an empty result.
//!
//! Both live here rather than in the Tauri command layer because they are
//! query-language behaviour with unit tests, not IPC plumbing (D-4: one
//! producer). The commands call them and do nothing else.

use crate::query::ir::{AggFn, Field, Query, SortDir, ViewKind, ViewSettings};

/// The property namespace §7.6 persists the view under.
const VIEW_PROPERTY_PREFIX: &str = "tine.";

/// SPEC §4.1 precedence (N17, M14): **for each view field**, a `tine.*` block
/// property wins; the DSL directive the parser lifted is read only when the
/// property is absent. The merge happens in exactly one place, this function,
/// so a caller cannot get the order wrong.
///
/// A property whose value does not parse is not a reason to drop the field: the
/// DSL's value stands, because a half-read property is worse evidence than the
/// text the author wrote. Nothing here rewrites the query.
pub fn merge_block_property_view(
    parsed: &ViewSettings,
    block_properties: &[(String, String)],
) -> ViewSettings {
    let property = |name: &str| -> Option<&str> {
        let wanted = format!("{VIEW_PROPERTY_PREFIX}{name}");
        block_properties
            .iter()
            .find(|(key, _)| crate::doc::property_key_norm(key) == wanted)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
    };

    let mut merged = parsed.clone();
    if let Some(view) = property("view").and_then(parse_view_kind) {
        merged.view = Some(view);
    }
    if let Some(sort) = property("sort").map(parse_sort) {
        if !sort.is_empty() {
            merged.sort = sort;
        }
    }
    if let Some(group_by) = property("group-by") {
        merged.group_by = Some(Field::new(group_by));
    }
    if let Some(fields) = property("fields").map(parse_fields) {
        if !fields.is_empty() {
            merged.columns = fields;
        }
    }
    if let Some(aggregates) = property("col-aggregates").map(parse_col_aggregates) {
        if !aggregates.is_empty() {
            merged.aggregates = aggregates;
        }
    }
    if let Some(sample) = property("sample").and_then(|value| value.parse::<u32>().ok()) {
        merged.sample = Some(sample);
    }
    merged
}

fn parse_view_kind(value: &str) -> Option<ViewKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "search" => Some(ViewKind::Search),
        "list" => Some(ViewKind::List),
        "table" => Some(ViewKind::Table),
        "board" => Some(ViewKind::Board),
        _ => None,
    }
}

/// `tine.sort:: <field> <asc|desc>[; …]` (§7.6). A segment with no direction
/// sorts ascending, which is what the Display popover writes.
fn parse_sort(value: &str) -> Vec<(Field, SortDir)> {
    value
        .split(';')
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() {
                return None;
            }
            let (name, direction) = match segment.rsplit_once(char::is_whitespace) {
                Some((name, "desc")) => (name.trim(), SortDir::Desc),
                Some((name, "asc")) => (name.trim(), SortDir::Asc),
                _ => (segment, SortDir::Asc),
            };
            (!name.is_empty()).then(|| (Field::new(name), direction))
        })
        .collect()
}

fn parse_fields(value: &str) -> Vec<Field> {
    value
        .split(';')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(Field::new)
        .collect()
}

/// `tine.col-aggregates:: <field>=<fn>[; …]` (§7.6). A bare `count` segment
/// with no `=` is the whole-result count, `(Field(""), Count)` (X3).
fn parse_col_aggregates(value: &str) -> Vec<(Field, AggFn)> {
    value
        .split(';')
        .filter_map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() {
                return None;
            }
            match segment.split_once('=') {
                Some((field, function)) => {
                    Some((Field::new(field.trim()), parse_agg_fn(function.trim())?))
                }
                None => {
                    (segment.eq_ignore_ascii_case("count")).then(|| (Field::new(""), AggFn::Count))
                }
            }
        })
        .collect()
}

fn parse_agg_fn(value: &str) -> Option<AggFn> {
    match value.to_ascii_lowercase().as_str() {
        "count" => Some(AggFn::Count),
        "sum" => Some(AggFn::Sum),
        "avg" => Some(AggFn::Avg),
        _ => None,
    }
}

/// One line of `query_explain_empty` (SPEC §7.1, Q14/N19).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EmptyExplanation {
    /// The conjunct, printed in TQL so the answer reads as query text.
    pub conjunct: String,
    /// Anchor rows matching this conjunct **alone**.
    pub alone: usize,
    /// Anchor rows matching every OTHER conjunct — absent when the root is not
    /// an `And`, because then there is no "other".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub without: Option<usize>,
}

/// Why a query returned nothing: for a root `And` after normalization, one
/// entry per top-level conjunct with the rows it matches alone and the rows the
/// rest match without it; for any other root, one entry for the whole query
/// (N19). Every count is the ANCHOR row count of the same evaluator the query
/// itself ran through — nothing here is a second engine.
pub(crate) fn explain_empty(
    source: &dyn crate::query::QueryPageSource,
    query: &Query,
    view: &ViewSettings,
    bounds: crate::query::ir::Bounds,
) -> Vec<EmptyExplanation> {
    use crate::query::ir::Filter;

    let count = |filter: Filter| -> usize {
        let mut probe = query.clone();
        probe.filter = filter;
        probe.diagnostics.clear();
        crate::query::run_query_result_over(source, &probe, view, bounds).total
    };
    let printed = |filter: &Filter| -> String {
        let mut probe = query.clone();
        probe.filter = filter.clone();
        crate::query::print::print_tql(&probe)
    };

    let mut evaluable = query.clone();
    evaluable.filter = query.evaluable_filter();
    match evaluable.normalized().filter {
        Filter::And { items } if items.len() > 1 => items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let others = items
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(_, filter)| filter.clone())
                    .collect::<Vec<_>>();
                EmptyExplanation {
                    conjunct: printed(item),
                    alone: count(item.clone()),
                    without: Some(count(Filter::and(others))),
                }
            })
            .collect(),
        whole => vec![EmptyExplanation {
            conjunct: printed(&whole),
            alone: count(whole),
            without: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ir::ViewKind;

    fn properties(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn a_block_property_wins_over_the_directive_the_parser_lifted() {
        let parsed = ViewSettings {
            sort: vec![(Field::new("page"), SortDir::Asc)],
            sample: Some(3),
            ..ViewSettings::default()
        };
        let merged = merge_block_property_view(
            &parsed,
            &properties(&[("tine.sort", "created desc"), ("tine.sample", "10")]),
        );
        assert_eq!(merged.sort, vec![(Field::new("created"), SortDir::Desc)]);
        assert_eq!(merged.sample, Some(10));
    }

    #[test]
    fn a_directive_is_read_only_where_no_property_covers_it() {
        let parsed = ViewSettings {
            sort: vec![(Field::new("page"), SortDir::Asc)],
            sample: Some(3),
            ..ViewSettings::default()
        };
        let merged = merge_block_property_view(&parsed, &properties(&[("tine.sample", "10")]));
        assert_eq!(
            merged.sort,
            vec![(Field::new("page"), SortDir::Asc)],
            "the property covered `sample`, not `sort`"
        );
        assert_eq!(merged.sample, Some(10));
    }

    #[test]
    fn an_unreadable_property_leaves_the_authors_directive_standing() {
        let parsed = ViewSettings {
            sample: Some(3),
            view: Some(ViewKind::Table),
            ..ViewSettings::default()
        };
        let merged = merge_block_property_view(
            &parsed,
            &properties(&[
                ("tine.sample", "lots"),
                ("tine.view", "kanban"),
                ("tine.sort", "  "),
            ]),
        );
        assert_eq!(merged.sample, Some(3));
        assert_eq!(merged.view, Some(ViewKind::Table));
        assert!(merged.sort.is_empty());
    }

    #[test]
    fn the_view_properties_parse_the_forms_section_7_6_persists() {
        let merged = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[
                ("tine.view", "board"),
                ("tine.group-by", "status"),
                ("tine.fields", "page; status; cost"),
                ("tine.col-aggregates", "count;cost=sum"),
                ("tine.sort", "status; created desc"),
                ("tine.sample", "25"),
            ]),
        );
        assert_eq!(merged.view, Some(ViewKind::Board));
        assert_eq!(merged.group_by, Some(Field::new("status")));
        assert_eq!(
            merged.columns,
            vec![Field::new("page"), Field::new("status"), Field::new("cost")]
        );
        // X3: the bare `count` entry is the whole-result count.
        assert_eq!(
            merged.aggregates,
            vec![
                (Field::new(""), AggFn::Count),
                (Field::new("cost"), AggFn::Sum)
            ]
        );
        assert_eq!(
            merged.sort,
            vec![
                (Field::new("status"), SortDir::Asc),
                (Field::new("created"), SortDir::Desc)
            ]
        );
        assert_eq!(merged.sample, Some(25));
    }

    #[test]
    fn a_property_outside_the_tine_namespace_is_not_a_view_setting() {
        let merged =
            merge_block_property_view(&ViewSettings::default(), &properties(&[("sample", "9")]));
        assert_eq!(merged.sample, None);
    }
}
