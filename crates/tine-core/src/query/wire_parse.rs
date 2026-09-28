//! The `{query, view}` pair every `query_parse` returns, and the ONE function
//! that produces it (SPEC §7.1, §4.1).
//!
//! It lives in tine-core rather than in the Tauri command layer because the
//! query publisher (`publish/app_export.rs`) bakes the exact answer the
//! frontend's `parseQuery` would receive; two implementations would drift
//! apart in precisely the merge order §4.1 fixes.

use super::ir::{Query, ScopedDisplaySettings, ViewSettings};
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

    /// The wire dialect a core input is named by (the inverse of [`Self::input`]).
    pub fn from_input(input: QueryInput) -> Self {
        match input {
            QueryInput::Og => QueryTextDialect::Og,
            QueryInput::Advanced => QueryTextDialect::Advanced,
            QueryInput::Tql => QueryTextDialect::Tql,
            QueryInput::MacroQuery => QueryTextDialect::MacroQuery,
            QueryInput::MacroTql => QueryTextDialect::MacroTql,
        }
    }
}

/// The `{query, view}` pair every parse returns (SPEC §7.1).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ParsedQuery {
    pub query: Query,
    pub view: ViewSettings,
    #[serde(default, flatten)]
    pub scoped: ScopedDisplaySettings,
}

/// The whole of `query_parse` that is not slot plumbing: parse, then merge the
/// host block's `tine.*` properties over the lifted directives (§4.1).
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
    let scoped = super::view::read_scoped_display_settings(block_properties);
    ParsedQuery {
        query,
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
}
