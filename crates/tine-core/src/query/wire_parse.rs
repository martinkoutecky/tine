//! The `{query, view}` pair every `query_parse` returns, and the ONE function
//! that produces it (SPEC §7.1, §4.1).
//!
//! It lives in tine-core rather than in the Tauri command layer because the
//! query publisher (`publish/app_export.rs`) bakes the exact answer the
//! frontend's `parseQuery` would receive; two implementations would drift
//! apart in precisely the merge order §4.1 fixes.

use super::ir::{
    Attr, CmpOp, Filter, Leaf, Query, ScopedDisplaySettings, Source, Value, ViewSettings,
};
use super::registry::Registry;
use super::QueryInput;

/// The wire dialect a `query_parse` caller names.
///
/// `og`, `tql` and `advanced` are explicit FORM inputs. `macro_query` and
/// `macro_tql` take the COMPLETE raw macro argument, without the outer
/// delimiters, and are the only inputs that split a trailing options map — one
/// splitter, in Rust, so the frontend's own splitters can be deleted in P0-ts
/// (X4, W2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryTextDialect {
    Og,
    Tql,
    /// `{{query #+BEGIN_QUERY …}}` — datalog, parsed as the advanced form.
    Advanced,
    /// The complete argument of a `{{query …}}` macro: OG or advanced, decided
    /// here by the one Rust discriminator rather than by a frontend regex.
    MacroQuery,
    /// The complete argument of a `{{tine-query …}}` macro: TQL.
    MacroTql,
}

impl QueryTextDialect {
    /// The core input a wire dialect parses as. `Advanced` is OG's
    /// `{{query #+BEGIN_QUERY …}}` form, which the OG parser already detects
    /// and reports (M5); it is not a third parser.
    pub fn input(self) -> QueryInput {
        match self {
            QueryTextDialect::Og => QueryInput::Og,
            QueryTextDialect::Advanced => QueryInput::Advanced,
            QueryTextDialect::Tql => QueryInput::Tql,
            QueryTextDialect::MacroQuery => QueryInput::MacroQuery,
            QueryTextDialect::MacroTql => QueryInput::MacroTql,
        }
    }
}

/// The `{query, view}` pair every parse returns (SPEC §7.1).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ParsedQuery {
    pub query: Query,
    pub view: ViewSettings,
    /// OG's read-only table request: options, query-table, or a trailing table.
    /// Explicit Tine presentation properties override this in the renderer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_table: bool,
    #[serde(default, flatten)]
    pub scoped: ScopedDisplaySettings,
    /// The D-18 on-query cue for a Logseq form that adds no condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub og_hint: Option<OgQueryHint>,
}

/// **The on-query cue for Logseq's "no condition" forms (D-18, GH #422).**
///
/// A bare `(task)` / `(todo)` / `(priority)` adds no condition in OG Logseq,
/// and a query with no condition left shows nothing ([`Query::evaluable_filter`]).
/// Tine evaluates them the same way and says so on the query, with one-click
/// rewrites to explicit forms. Every rewrite is a complete query the caller
/// saves through the ordinary print-and-write path; the marker lists come from
/// the one marker source ([`crate::doc::MARKERS`] / [`crate::doc::DONE_MARKERS`]).
/// Present only for a valid OG-dialect query that has one of these shapes.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OgQueryHint {
    /// The bare heads the query contains, `task` before `priority`.
    pub bare: Vec<BareHead>,
    /// The query keeps no condition (OG's nil query), so it shows nothing.
    pub no_conditions: bool,
    /// The explicit rewrites on offer, one per button.
    pub rewrites: Vec<OgHintRewrite>,
}

/// A bare head that adds no condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BareHead {
    Task,
    Priority,
}

/// Which explicit form a rewrite writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OgRewrite {
    /// Every marker except DONE / CANCELED / CANCELLED (carry-over's open rule).
    OpenTasks,
    /// Every marker.
    AnyTask,
    /// Priorities A, B and C.
    Priorities,
}

/// One rewrite: the whole query with every bare clause of its head replaced.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OgHintRewrite {
    pub rewrite: OgRewrite,
    pub query: Query,
}

/// The priority levels OG's `(priority …)` reads (`[#A]`, `[#B]`, `[#C]`).
const PRIORITY_LEVELS: [&str; 3] = ["A", "B", "C"];

/// Whether `filter` contains a dropped bare clause of `attr` outside `Off`.
fn has_bare(filter: &Filter, attr: Attr) -> bool {
    match filter {
        Filter::Leaf {
            leaf: Leaf::Attr { attr: found, .. },
        } => *found == attr && filter.is_dropped_clause(),
        Filter::Leaf {
            leaf: Leaf::Rel { pred, .. },
        } => has_bare(pred, attr),
        Filter::And { items } | Filter::Or { items } => items.iter().any(|f| has_bare(f, attr)),
        Filter::Not { inner } => has_bare(inner, attr),
        _ => false,
    }
}

/// Whether `filter` holds any condition besides OG's dropped clauses: a leaf,
/// a preserved unknown form, or a disabled subtree.
fn has_condition(filter: &Filter) -> bool {
    match filter {
        Filter::Leaf { .. } => !filter.is_dropped_clause(),
        Filter::Raw { .. } | Filter::Off { .. } => true,
        Filter::And { items } | Filter::Or { items } => items.iter().any(has_condition),
        Filter::Not { inner } => has_condition(inner),
        Filter::True | Filter::False => false,
    }
}

/// `filter` with every dropped clause of `attr` replaced by `values`.
fn with_explicit(filter: &Filter, attr: Attr, values: &[&str]) -> Filter {
    match filter {
        Filter::Leaf {
            leaf: Leaf::Attr { attr: found, .. },
        } if *found == attr && filter.is_dropped_clause() => Filter::attr(
            attr,
            CmpOp::In,
            Value::List {
                items: values.iter().map(|v| Value::text(*v)).collect(),
            },
        ),
        Filter::Leaf {
            leaf: Leaf::Rel { rel, quant, pred },
        } => Filter::rel(*rel, *quant, with_explicit(pred, attr, values)),
        Filter::And { items } => Filter::and(
            items
                .iter()
                .map(|f| with_explicit(f, attr, values))
                .collect(),
        ),
        Filter::Or { items } => Filter::or(
            items
                .iter()
                .map(|f| with_explicit(f, attr, values))
                .collect(),
        ),
        Filter::Not { inner } => Filter::not(with_explicit(inner, attr, values)),
        other => other.clone(),
    }
}

/// The D-18 cue for `query`, or `None`. O(query size × rewrites).
pub fn og_query_hint(query: &Query) -> Option<OgQueryHint> {
    if !matches!(query.source, Source::Og { .. }) || query.is_invalid() {
        return None;
    }
    let mut bare = Vec::new();
    let mut rewrites = Vec::new();
    let rewrite = |attr: Attr, values: &[&str]| Query {
        filter: with_explicit(&query.filter, attr, values),
        ..query.clone()
    };
    if has_bare(&query.filter, Attr::Task) {
        bare.push(BareHead::Task);
        let open: Vec<&str> = crate::doc::MARKERS
            .iter()
            .copied()
            .filter(|marker| !crate::doc::DONE_MARKERS.contains(marker))
            .collect();
        rewrites.push(OgHintRewrite {
            rewrite: OgRewrite::OpenTasks,
            query: rewrite(Attr::Task, &open),
        });
        rewrites.push(OgHintRewrite {
            rewrite: OgRewrite::AnyTask,
            query: rewrite(Attr::Task, crate::doc::MARKERS),
        });
    }
    if has_bare(&query.filter, Attr::Priority) {
        bare.push(BareHead::Priority);
        rewrites.push(OgHintRewrite {
            rewrite: OgRewrite::Priorities,
            query: rewrite(Attr::Priority, &PRIORITY_LEVELS),
        });
    }
    let no_conditions = !has_condition(&query.filter);
    (no_conditions || !bare.is_empty()).then_some(OgQueryHint {
        bare,
        no_conditions,
        rewrites,
    })
}

/// The whole of `query_parse` that is not slot plumbing: parse, then merge the
/// host block's `tine.*` properties over the lifted directives (§4.1), and
/// read OG table presentation from options, `query-table` and trailing `table`.
/// Pure, O(source length + supplied properties); never changes source bytes.
pub fn parse_query_pair(
    text: &str,
    dialect: QueryTextDialect,
    block_properties: &[(String, String)],
    registry: &Registry,
) -> ParsedQuery {
    let (query, parsed_view) = super::parse_query_input(
        text,
        dialect.input(),
        crate::date::JournalDate::today(),
        registry,
    );
    // OG components/query.cljs query: table? is options OR the host property
    // OR ends-with? on the trimmed query string. Keep this answer in Rust.
    let legacy_table = crate::query_edn::options(query.source.og_options())
        .is_some_and(|options| options.table)
        || block_properties.iter().any(|(key, value)| {
            crate::doc::property_key_norm(key) == "query-table"
                && !matches!(value.trim(), "false" | "nil")
        })
        || matches!(&query.source, super::ir::Source::Og { original, .. }
            if original.trim_end().ends_with("table"));
    let scoped = super::view::read_scoped_display_settings(block_properties);
    let og_hint = og_query_hint(&query);
    ParsedQuery {
        og_hint,
        query,
        legacy_table,
        view: super::view::merge_block_property_view(&parsed_view, block_properties),
        scoped,
    }
}

/// The view the app runs a parsed query under: `queryParsedDisplaySettings`
/// (`src/editor/queryDisplayDraft.ts`) transcribed. The result kind's scoped
/// presentation wins over the singular one (default list); a present scoped
/// draft replaces the singular display wholesale — it does not inherit from
/// the query text — while an absent one inherits the merged singular view.
pub fn anchored_view(parsed: &ParsedQuery, anchor: super::ir::Anchor) -> ViewSettings {
    use super::ir::{Anchor, ViewKind};
    let page = matches!(anchor, Anchor::Page);
    let scoped_presentation = if page {
        parsed.scoped.page_presentation
    } else {
        parsed.scoped.block_presentation
    };
    let presentation = scoped_presentation
        .or(parsed.view.view)
        .unwrap_or(ViewKind::List);
    let draft = if page {
        parsed.scoped.page_display.as_ref()
    } else {
        parsed.scoped.block_display.as_ref()
    };
    match draft {
        Some(draft) => ViewSettings {
            view: Some(presentation),
            sort: draft.sort.clone().unwrap_or_default(),
            group_by: draft.group_by.clone(),
            columns: draft.columns.clone().unwrap_or_default(),
            aggregates: draft.aggregates.clone().unwrap_or_default(),
            sample: draft.sample,
        },
        None => ViewSettings {
            view: Some(presentation),
            ..parsed.view.clone()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ir::Anchor;

    /// The producer half of the published-export view key: the engine writes
    /// its `ViewSettings` DENSELY (every list field present, empty or not),
    /// while the frontend resolves the same scoped draft sparsely
    /// (`queryParsedDisplaySettings` omits what the draft did not state).
    /// `publishedBackend.ts` `viewKey` folds the two together; this pins the
    /// shape it folds, so a serde change here fails before a reader gets the
    /// unsampled twin's rows (`publishedBackend.test.ts`, "matches a view the
    /// engine wrote densely…", is the consumer half).
    #[test]
    fn anchored_view_of_a_scoped_draft_serializes_densely() {
        // `tine.block-display:: 1` is the marker that makes the block-scoped
        // draft PRESENT; the sample rides inside it.
        let properties = vec![
            ("tine.block-display".to_string(), "1".to_string()),
            ("tine.block-sample".to_string(), "2".to_string()),
        ];
        let parsed = parse_query_pair(
            "(task TODO)",
            QueryTextDialect::MacroQuery,
            &properties,
            Registry::none(),
        );
        assert_eq!(
            parsed
                .scoped
                .block_display
                .as_ref()
                .and_then(|draft| draft.sample),
            Some(2),
            "the scoped draft carries the sample"
        );
        let block = serde_json::to_value(anchored_view(&parsed, Anchor::Block)).unwrap();
        assert_eq!(
            block,
            serde_json::json!({
                "view": "list",
                "sort": [],
                "columns": [],
                "aggregates": [],
                "sample": 2
            })
        );
        let page = serde_json::to_value(anchored_view(&parsed, Anchor::Page)).unwrap();
        assert_eq!(
            page,
            serde_json::json!({ "view": "list", "sort": [], "columns": [], "aggregates": [] }),
            "the page half inherits the singular view, which the draft did not touch"
        );
    }

    /// GH #619 item 9: the macro's "pages and blocks" choice rides in
    /// `tine.result-kinds`, a host property the engine does not read. Present or
    /// absent, the parse is identical (OG ignores it the same way).
    #[test]
    fn the_result_kinds_host_property_does_not_change_the_reading() {
        let with = vec![(
            "tine.result-kinds".to_string(),
            "pages-and-blocks".to_string(),
        )];
        let a = parse_query_pair(
            "(task TODO)",
            QueryTextDialect::MacroQuery,
            &with,
            Registry::none(),
        );
        let b = parse_query_pair(
            "(task TODO)",
            QueryTextDialect::MacroQuery,
            &[],
            Registry::none(),
        );
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            serde_json::to_value(&b).unwrap()
        );
    }

    /// GH #422, D-18: the on-query cue. A bare head is named, a query with no
    /// condition left says so, and every rewrite is a whole query that prints
    /// to the explicit OG form and reads back to itself (the save path prints
    /// it; nothing here writes).
    #[test]
    fn the_og_hint_names_bare_heads_and_offers_round_tripping_rewrites() {
        use crate::query::print::{og_expressible, query_print, PrintDialect};
        let hint = |text: &str| {
            parse_query_pair(text, QueryTextDialect::MacroQuery, &[], Registry::none()).og_hint
        };
        let printed = |query: &Query| {
            assert!(og_expressible(query, &ViewSettings::default()));
            let text = query_print(query, &ViewSettings::default(), PrintDialect::Og, false)
                .expect("an explicit form prints");
            let again =
                parse_query_pair(&text, QueryTextDialect::MacroQuery, &[], Registry::none());
            assert_eq!(again.query.normalized(), query.normalized(), "{text}");
            assert!(
                again.og_hint.is_none(),
                "{text}: an explicit form needs no cue"
            );
            text
        };
        let task = hint("(and (task) [[project]])").expect("bare task");
        assert_eq!(task.bare, vec![BareHead::Task]);
        assert!(!task.no_conditions);
        let kinds: Vec<_> = task.rewrites.iter().map(|r| r.rewrite).collect();
        assert_eq!(kinds, vec![OgRewrite::OpenTasks, OgRewrite::AnyTask]);
        assert_eq!(
            printed(&task.rewrites[0].query),
            "(and (task TODO DOING NOW LATER WAITING WAIT STARTED IN-PROGRESS) [[project]])"
        );
        assert_eq!(
            printed(&task.rewrites[1].query),
            "(and (task TODO DOING DONE NOW LATER WAITING WAIT CANCELED CANCELLED STARTED IN-PROGRESS) [[project]])"
        );

        let priority = hint("(priority)").expect("bare priority");
        assert_eq!(priority.bare, vec![BareHead::Priority]);
        assert!(priority.no_conditions, "alone it is OG's nil query");
        assert_eq!(priority.rewrites.len(), 1);
        assert_eq!(printed(&priority.rewrites[0].query), "(priority A B C)");

        let both = hint("(or (todo) (not (priority)))").expect("both heads");
        assert_eq!(both.bare, vec![BareHead::Task, BareHead::Priority]);
        assert!(both.no_conditions);

        for blank in ["", "  ", "(sort-by created-at)", "(and)"] {
            let empty = hint(blank).expect("no conditions");
            assert!(empty.no_conditions, "{blank:?}");
            assert!(
                empty.bare.is_empty() && empty.rewrites.is_empty(),
                "{blank:?}"
            );
        }
        for quiet in [
            "(task TODO)",
            "(priority A)",
            "[[project]]",
            "(and (task NOW) (frobnicate x))",
        ] {
            assert_eq!(hint(quiet), None, "{quiet}");
        }
        let tine = parse_query_pair(
            "@block and task in ()",
            QueryTextDialect::MacroTql,
            &[],
            Registry::none(),
        );
        assert_eq!(tine.og_hint, None, "Tine's dialect reads it literally");
    }
}
