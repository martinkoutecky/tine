//! Row order and statistics inputs for query execution (split from `exec.rs`
//! along its seam: these read one page entry or one block's properties and
//! never the graph). SPEC §3.5 base order and sort decorations; the shared sort
//! meaning for IR and Friendly rows (`query_plan`) lives here too (I-12).

use tine_core::date::JournalDate;
use tine_core::doc::{property_key_norm, DocBlock};
use tine_core::model::{PageEntry, PageKind};
use tine_core::query::sort::{lexical_property_sort_text, SortDecor};
use tine_core::query::statistics::StatisticsFold;

/// The recency axis (Unix seconds): a journal by the day it represents, any
/// other page by the mtime captured with the page table; oldest when unknown.
pub(super) fn recency(
    entry: &PageEntry,
    mtimes: &crate::model::persistent::Map<String, std::time::SystemTime>,
) -> i64 {
    if let Some(day) = entry.date_key {
        return JournalDate::from_ordinal(day).to_days() * 86_400;
    }
    mtimes
        .get(entry.rel_path_str())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(i64::MIN, |elapsed| elapsed.as_secs() as i64)
}

pub(super) fn is_recency_field(field: &str) -> bool {
    matches!(
        field.to_ascii_lowercase().as_str(),
        "modified" | "updated" | "updated-at" | "date"
    )
}

fn kind_rank(kind: PageKind) -> u8 {
    match kind {
        PageKind::Journal => 0,
        PageKind::Page => 1,
    }
}

/// SPEC §3.5's base order for block groups (M13), amended for OG parity (#8a):
/// OG renders grouped results with `(sort-by (comp :block/journal-day first) >)`
/// (`components/block.cljs:3497,3523,3552`), so journal days run NEWEST first and
/// pages that are not journals follow. The reference groups (`query::collect_bounded`)
/// already order this way; one rule for every group list (I-12). Ties fall back
/// to page name, then kind rank, then physical path.
pub(super) fn base_order(a: &PageEntry, b: &PageEntry) -> std::cmp::Ordering {
    b.date_key
        .unwrap_or(i64::MIN)
        .cmp(&a.date_key.unwrap_or(i64::MIN))
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
        .then_with(|| a.rel_path_str().as_bytes().cmp(b.rel_path_str().as_bytes()))
}

fn first_property(properties: &[(String, String)], field: &str) -> Option<String> {
    let field = property_key_norm(field);
    properties
        .iter()
        .find(|(key, _)| property_key_norm(key) == field)
        .map(|(_, value)| value.clone())
}

/// One row's aggregate inputs: `count` reads no value; every other function
/// reads the row's first value of the named property.
pub(super) fn statistics_values(
    fold: &StatisticsFold,
    properties: &[(String, String)],
) -> Vec<Option<String>> {
    fold.view()
        .aggregates
        .iter()
        .map(|(field, op)| match op {
            tine_core::query::ir::AggFn::Count => None,
            _ => first_property(properties, field.as_str()),
        })
        .collect()
}

pub(super) fn statistics_keys(
    fold: &StatisticsFold,
    special: impl FnOnce(&str) -> Option<Vec<Option<String>>>,
    properties: &[(String, String)],
) -> Vec<Option<String>> {
    let mut keys = match fold.view().group_by.as_ref().map(|field| field.as_str()) {
        None => Vec::new(),
        Some(field) if field.starts_with("formula:") => Vec::new(),
        Some(field) => special(field).unwrap_or_else(|| {
            vec![first_property(
                properties,
                field.strip_prefix("prop:").unwrap_or(field),
            )]
        }),
    };
    if keys.is_empty() {
        keys.push(None);
    }
    keys
}

/// Shared block sort meaning for IR and Friendly rows. Missing properties use
/// visible first-line text; planning fields use their stable missing sentinel.
/// Cost O(properties on one block); no failure for a parsed block.
pub(crate) fn block_sort_decor(
    field: &str,
    entry: &PageEntry,
    block: &DocBlock,
    page_recency: impl FnOnce() -> i64,
) -> SortDecor {
    let projection = block.projection();
    match field.to_ascii_lowercase().as_str() {
        _ if is_recency_field(field) => SortDecor::Num(page_recency()),
        "priority" => SortDecor::Text(
            block
                .priority()
                .map_or_else(|| "Z".to_string(), str::to_ascii_uppercase),
        ),
        "page" => SortDecor::Text(entry.name.to_lowercase()),
        "deadline" => SortDecor::Text(
            projection
                .deadline()
                .map_or_else(|| "~".to_string(), str::to_owned),
        ),
        "scheduled" => SortDecor::Text(
            projection
                .scheduled()
                .map_or_else(|| "~".to_string(), str::to_owned),
        ),
        _ => SortDecor::Text(lexical_property_sort_text(
            projection
                .properties()
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            field,
            || {
                block
                    .visible_text()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string()
            },
        )),
    }
}

/// Shared page sort meaning for IR and Friendly rows. Missing properties use
/// the page name. Cost O(properties on one page); no failure for a page entry.
pub(crate) fn page_sort_decor(
    field: &str,
    entry: &PageEntry,
    properties: &[(String, String)],
    page_recency: i64,
) -> SortDecor {
    match field.to_ascii_lowercase().as_str() {
        "name" | "page" => SortDecor::Text(entry.name.to_lowercase()),
        "kind" => SortDecor::Text(
            match entry.kind {
                PageKind::Journal => "journal",
                PageKind::Page => "page",
            }
            .into(),
        ),
        "day" | "journal-day" | "journal_day" => SortDecor::Num(entry.date_key.unwrap_or(i64::MIN)),
        _ if is_recency_field(field) => SortDecor::Num(page_recency),
        _ => SortDecor::Text(lexical_property_sort_text(
            properties
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            field,
            || entry.name.clone(),
        )),
    }
}
