//! SPEC §7.1's two computations that are neither parse, print nor run: the
//! §4.1 precedence merge of block properties into the view, and §Q14/N19's
//! explanation of an empty result.
//!
//! Both live here rather than in the Tauri command layer because they are
//! query-language behaviour with unit tests, not IPC plumbing (D-4: one
//! producer). The commands call them and do nothing else.

use crate::query::ir::{
    AggFn, DisplayDraft, Field, FriendlyPageMatchScope, ScopedDisplaySettings, SortDir, ViewKind,
    ViewSettings,
};

/// The property namespace §7.6 persists the view under.
const VIEW_PROPERTY_PREFIX: &str = "tine.";

/// A `tine.<name>` property EXACTLY as it stands, first normalized occurrence
/// wins. `Some("")` is a present-but-empty value and is deliberately distinct
/// from `None`: an empty `tine.columns::`/`tine.group-field::` is an explicit
/// statement, not a gap.
fn raw_property<'a>(block_properties: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let wanted = format!("{VIEW_PROPERTY_PREFIX}{name}");
    block_properties
        .iter()
        .find(|(key, _)| crate::doc::property_key_norm(key) == wanted)
        .map(|(_, value)| value.as_str())
}

/// The same read, trimmed, with a blank value treated as absent — the reading
/// the fields that have no "explicit clear" spelling want.
fn property_value<'a>(block_properties: &'a [(String, String)], name: &str) -> Option<&'a str> {
    raw_property(block_properties, name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// **Which columns a query block SHOWS — the one resolver** (P5A).
///
/// Visible columns and a typed sheet schema are two different questions that
/// used to share one property. `tine.columns::` owns the ordered list of
/// query-visible field names; `tine.fields::` keeps the typed schema
/// (`name=type`), and a value containing `=` is therefore never a column list.
///
/// Every consumer calls THIS function rather than re-deriving the precedence
/// (D-4/D-12: one producer of one answer). In og the only consumer so far is
/// `merge_block_property_view`; master's static publisher and TypeScript half
/// (`src/editor/queryViewProperties.ts`) are later lanes, and the precedence is
/// pinned by `query/fixtures/query-columns/resolution.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryColumns {
    /// A property named these columns, in this order. Token spelling is
    /// retained; a renderer may deduplicate identical field ids without
    /// rewriting the source.
    Named(Vec<Field>),
    /// `tine.columns::` is PRESENT and says nothing readable — empty, or a list
    /// one token invalidated. That is an explicit statement, not a gap: there
    /// is no legacy fallback and no DSL fallback behind it, so clearing the
    /// property cannot resurrect an older list.
    Cleared,
    /// No property answers the question at all. Whatever the query text itself
    /// carried stands.
    Unset,
}

/// The column-list grammar, applied to a WHOLE property value (P5A):
/// trim, split on `;`, trim each token, discard empty segments. One token
/// containing `=`, NUL, CR or LF invalidates the ENTIRE list rather than just
/// itself — a half-read column list is worse evidence than none, and `=`
/// anywhere means the value is a schema or a mixed value, never columns.
///
/// `None` is "this value is not a column list"; `Some(vec![])` is "this value
/// is a column list with nothing in it".
///
/// Deliberately no per-name length cap: these are property bytes an outside
/// editor may have authored, and refusing a long but well-formed name would
/// drop a column the author can see in their own file. Session/UI caps are a
/// different boundary.
fn column_list(value: &str) -> Option<Vec<Field>> {
    let mut out = Vec::new();
    for token in value.trim().split(';') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if token.contains(|c| matches!(c, '=' | '\0' | '\r' | '\n')) {
            return None;
        }
        out.push(Field::new(token));
    }
    Some(out)
}

/// SPEC §7.6 + P5A precedence for the visible columns of a query block, read
/// from its normalized block properties (first occurrence of a key wins, as
/// every other reader here does).
///
///  1. `tine.columns` PRESENT → its own answer, and nothing behind it.
///  2. `tine.columns` ABSENT → `tine.fields` is read as a LEGACY column list,
///     but only when every nonempty token passes the same grammar and at least
///     one exists. This is compatibility for notes authored before the split,
///     not private-state migration (D-1 is not engaged).
///  3. Neither → `Unset`.
///
/// Nothing here writes: opening, parsing or rebuilding a graph never rewrites a
/// note to move a legacy list (I-4).
pub fn resolve_query_columns(block_properties: &[(String, String)]) -> QueryColumns {
    let raw = |name: &str| raw_property(block_properties, name);
    if let Some(value) = raw("columns") {
        return match column_list(value) {
            Some(columns) if !columns.is_empty() => QueryColumns::Named(columns),
            _ => QueryColumns::Cleared,
        };
    }
    match raw("fields").and_then(column_list) {
        Some(columns) if !columns.is_empty() => QueryColumns::Named(columns),
        _ => QueryColumns::Unset,
    }
}

/// **Which field a query block GROUPS BY — the one resolver** (P5B).
///
/// The legacy `tine.group-by::` token is ambiguous by construction: the board
/// read a bare `state` as the TASK MARKER (`SheetBoard.tsx`'s `isFieldId`
/// fallback) while the list grouper read it as an ordinary property named
/// `state`. One bare string cannot mean both, and neither meaning may be taken
/// away from the notes that already rely on it.
///
/// So grouping gets a query-owned key, `tine.group-field::`, whose value is a
/// canonical **sheet `FieldId`** — the spelling `src/sheet/fields.ts` already
/// uses: `state`/`priority`/`scheduled`/`deadline`/`tags`/`page` for the
/// builtins, `prop:<exact property key>` for an ordinary property,
/// `formula:<name>` for a formula. `prop:state` is therefore the ordinary
/// property named `state`, bare `state` is the task marker, and
/// `prop:prop:state` is a literal property named `prop:state`. No new predicate
/// syntax, and nothing about sort or aggregate field names changes.
///
/// This is authored METADATA, not a database or authority migration: reading a
/// legacy value never rewrites the note (I-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryGrouping {
    /// Group by this canonical `FieldId`.
    Field(Field),
    /// `tine.group-field::` is PRESENT and says nothing readable — empty, or a
    /// token outside the canonical grammar. That is an explicit "no grouping":
    /// it blocks the legacy key and the DSL directive behind it, so a Board
    /// default cannot silently reinstate itself on the next view switch.
    Cleared,
    /// Nothing says anything about grouping. A Board may apply its own default
    /// here (ADR 0030) — and only here.
    Unset,
}

#[path = "group_field.rs"]
mod group_field;
use group_field::{group_token_serializable, GROUP_BUILTINS};

/// Canonical grouping field, using the shared borrowed grammar (O(value bytes)).
pub fn canonical_group_field(value: &str) -> Option<Field> {
    group_field::canonical_group_token(value).map(Field::new)
}

/// The **legacy** token's meaning, captured at the view the note is CURRENTLY
/// persisted with — never at the view the user is switching to. That is the
/// whole point: an existing list that groups by a property named `state` keeps
/// grouping by that property through a view change, and an existing board that
/// groups by the task marker keeps the task marker.
///
///  * board/table (a sheet face): the existing sheet spellings stand — a
///    builtin, `prop:…`, `formula:…`, and `formula.<name>` (the app's alternate
///    spelling, `SheetBoard.tsx`) are sheet fields. **Every other bare name now
///    means an ordinary property**, which is the deliberate fix: `status` used
///    to fall through `isFieldId` and silently become the task marker.
///  * list/search: `page` is the source page; every other token is an EXACT
///    property key, `state` and a literal `prop:` prefix included. That
///    preserves what `queryAggregate.ts::groupRows` has always done.
///
/// Total by construction: a nonempty serializable token always names something.
fn legacy_group_field(value: &str, sheet_face: bool) -> Option<Field> {
    let token = value.trim();
    if token.is_empty() || !group_token_serializable(token) {
        return None;
    }
    if !sheet_face {
        return Some(if token == "page" {
            Field::new("page")
        } else {
            Field::new(format!("prop:{token}"))
        });
    }
    if GROUP_BUILTINS.contains(&token) {
        return Some(Field::new(token));
    }
    if token
        .strip_prefix("prop:")
        .is_some_and(|rest| !rest.is_empty())
        || token
            .strip_prefix("formula:")
            .is_some_and(|rest| !rest.is_empty())
    {
        return Some(Field::new(token));
    }
    if let Some(rest) = token.strip_prefix("formula.") {
        if !rest.is_empty() {
            return Some(Field::new(format!("formula:{rest}")));
        }
    }
    Some(Field::new(format!("prop:{token}")))
}

/// The view a legacy grouping token must be READ under: the block's own
/// `tine.view::` when it is readable, otherwise whatever the query text asked
/// for, otherwise the default list.
fn effective_view_kind(block_properties: &[(String, String)], parsed: &ViewSettings) -> ViewKind {
    property_value(block_properties, "view")
        .and_then(parse_view_kind)
        .or(parsed.view)
        .unwrap_or(ViewKind::List)
}

/// SPEC §7.6 + P5B precedence for the grouping field of a query block.
///
///  1. `tine.group-field` **present** → its own answer, and nothing behind it
///     (an unreadable or empty value is an explicit `Cleared`).
///  2. otherwise a nonempty legacy `tine.group-by`, read at the CURRENT view.
///  3. otherwise the `(group-by …)` directive the parser lifted, read the same
///     way.
///  4. otherwise `Unset`.
///
/// In og, called by `merge_block_property_view`; master's query-backed
/// publisher and TypeScript adapter (`resolveQueryGrouping`) are later lanes.
/// The precedence is pinned by `query/fixtures/query-grouping/resolution.json`.
/// Components never interpret the property themselves.
pub fn resolve_query_grouping(
    block_properties: &[(String, String)],
    parsed: &ViewSettings,
) -> QueryGrouping {
    if let Some(value) = raw_property(block_properties, "group-field") {
        return match canonical_group_field(value) {
            Some(field) => QueryGrouping::Field(field),
            None => QueryGrouping::Cleared,
        };
    }
    let sheet_face = matches!(
        effective_view_kind(block_properties, parsed),
        ViewKind::Table | ViewKind::Board
    );
    if let Some(field) =
        raw_property(block_properties, "group-by").and_then(|v| legacy_group_field(v, sheet_face))
    {
        return QueryGrouping::Field(field);
    }
    match parsed
        .group_by
        .as_ref()
        .and_then(|field| legacy_group_field(field.as_str(), sheet_face))
    {
        Some(field) => QueryGrouping::Field(field),
        None => QueryGrouping::Unset,
    }
}

/// SPEC §4.1 precedence (N17, M14): **for each view field**, a `tine.*` block
/// property wins; the DSL directive the parser lifted is read only when the
/// property is absent. The merge happens in exactly one place, this function,
/// so a caller cannot get the order wrong.
///
/// For `view`, `sort`, `col-aggregates` and `sample`, a property whose value
/// does not parse (or parses to nothing) is not a reason to drop the field: the
/// DSL's value stands. `col-aggregates` is read partially: unknown segments are
/// dropped and the recognised ones replace the DSL's list.
///
/// Grouping and columns are different: they go through
/// [`resolve_query_grouping`] and [`resolve_query_columns`], where a PRESENT
/// but empty or unreadable `tine.group-field` / `tine.columns` is an explicit
/// clear that overrides the DSL. An explicit no-grouping is returned as
/// `group_by = Some(Field(""))` (distinct from `None`, "nothing said
/// anything", the only state a Board default may fill); an explicit no-columns
/// empties `columns`. A legacy `tine.group-by` or DSL `(group-by …)` value is
/// returned rewritten to its canonical `FieldId`, interpreted at the current
/// view. Nothing here rewrites the query.
pub fn merge_block_property_view(
    parsed: &ViewSettings,
    block_properties: &[(String, String)],
) -> ViewSettings {
    let property = |name: &str| property_value(block_properties, name);

    let mut merged = parsed.clone();
    if let Some(view) = property("view").and_then(parse_view_kind) {
        merged.view = Some(view);
    }
    if let Some(sort) = property("sort").map(parse_sort) {
        if !sort.is_empty() {
            merged.sort = sort;
        }
    }
    // **Grouping goes through the one resolver** (P5B). The wire shape is
    // unchanged — `group_by` is still `Option<Field>` — but the value is now the
    // canonical `FieldId` the resolver produced, and an explicit "no grouping"
    // is spelled as the empty field. `None` therefore means "nothing anywhere
    // said anything", which is the only state a Board default may fill.
    merged.group_by = match resolve_query_grouping(block_properties, parsed) {
        QueryGrouping::Field(field) => Some(field),
        QueryGrouping::Cleared => Some(Field::new("")),
        QueryGrouping::Unset => None,
    };
    match resolve_query_columns(block_properties) {
        QueryColumns::Named(columns) => merged.columns = columns,
        // An explicit "no columns" clears whatever the text asked for; `Unset`
        // leaves the author's own directive standing.
        QueryColumns::Cleared => merged.columns.clear(),
        QueryColumns::Unset => {}
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
            parse_col_aggregate_segment(segment)
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

/// The two durable mixed-result namespaces. Keeping the suffix table here gives
/// page and block state one reader and one grammar rather than parallel parsers.
#[derive(Clone, Copy)]
enum ScopedDisplayNamespace {
    Page,
    Block,
}

impl ScopedDisplayNamespace {
    fn prefix(self) -> &'static str {
        match self {
            ScopedDisplayNamespace::Page => "page",
            ScopedDisplayNamespace::Block => "block",
        }
    }

    fn key(self, member: &str) -> String {
        format!("{}-{member}", self.prefix())
    }

    fn full_key(self, member: &str) -> String {
        format!("{VIEW_PROPERTY_PREFIX}{}-{member}", self.prefix())
    }
}

/// A list field which the scoped property writer can round-trip without
/// changing its identity. Whitespace inside a field name is valid; separators,
/// line terminators and NUL are not.
fn scoped_list_field(field: &Field) -> bool {
    let value = field.as_str();
    !value.is_empty() && !value.contains(|c| matches!(c, '=' | ';' | '\0' | '\r' | '\n'))
}

fn parse_scoped_sort(value: &str) -> Option<Vec<(Field, SortDir)>> {
    let parsed = parse_sort(value);
    parsed
        .iter()
        .all(|(field, _)| scoped_list_field(field))
        .then_some(parsed)
}

fn parse_col_aggregate_segment(segment: &str) -> Option<(Field, AggFn)> {
    match segment.split_once('=') {
        Some((field, function)) => Some((Field::new(field.trim()), parse_agg_fn(function.trim())?)),
        None => segment
            .eq_ignore_ascii_case("count")
            .then(|| (Field::new(""), AggFn::Count)),
    }
}

/// The scoped form is atomic: one unknown aggregate makes the authored member
/// unreadable. The singular merge above keeps its historical partial-recognition
/// behaviour; this stricter boundary applies only to newly namespaced state.
fn parse_scoped_col_aggregates(value: &str) -> Option<Vec<(Field, AggFn)>> {
    let mut out = Vec::new();
    for segment in value.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let aggregate = parse_col_aggregate_segment(segment)?;
        if aggregate.0.as_str().is_empty() {
            if aggregate.1 != AggFn::Count {
                return None;
            }
        } else if !scoped_list_field(&aggregate.0) {
            return None;
        }
        out.push(aggregate);
    }
    Some(out)
}

fn read_scoped_display_namespace(
    block_properties: &[(String, String)],
    namespace: ScopedDisplayNamespace,
    unreadable: &mut Vec<String>,
) -> (Option<ViewKind>, Option<DisplayDraft>) {
    let view_key = namespace.key("view");
    let presentation = match raw_property(block_properties, &view_key) {
        Some(value) => match parse_view_kind(value) {
            Some(view) => Some(view),
            None => {
                unreadable.push(namespace.full_key("view"));
                None
            }
        },
        None => None,
    };

    let marker_key = namespace.key("display");
    let marker = raw_property(block_properties, &marker_key);
    if marker.is_none() {
        return (presentation, None);
    }
    if marker != Some("1") {
        unreadable.push(namespace.full_key("display"));
        return (presentation, None);
    }

    let mut draft = DisplayDraft::default();
    let mut valid = true;

    let sort_key = namespace.key("sort");
    if let Some(value) = raw_property(block_properties, &sort_key) {
        match parse_scoped_sort(value) {
            Some(sort) => draft.sort = Some(sort),
            None => {
                unreadable.push(namespace.full_key("sort"));
                valid = false;
            }
        }
    }

    let grouping_key = namespace.key("group-field");
    if let Some(value) = raw_property(block_properties, &grouping_key) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            draft.group_by = Some(Field::new(""));
        } else if let Some(group_by) = canonical_group_field(value) {
            draft.group_by = Some(group_by);
        } else {
            unreadable.push(namespace.full_key("group-field"));
            valid = false;
        }
    }

    let columns_key = namespace.key("columns");
    if let Some(value) = raw_property(block_properties, &columns_key) {
        match column_list(value) {
            Some(columns) => draft.columns = Some(columns),
            None => {
                unreadable.push(namespace.full_key("columns"));
                valid = false;
            }
        }
    }

    let aggregates_key = namespace.key("col-aggregates");
    if let Some(value) = raw_property(block_properties, &aggregates_key) {
        match parse_scoped_col_aggregates(value) {
            Some(aggregates) => draft.aggregates = Some(aggregates),
            None => {
                unreadable.push(namespace.full_key("col-aggregates"));
                valid = false;
            }
        }
    }

    let sample_key = namespace.key("sample");
    if let Some(value) = raw_property(block_properties, &sample_key) {
        match value.trim().parse::<u32>() {
            Ok(sample) => draft.sample = Some(sample),
            Err(_) => {
                unreadable.push(namespace.full_key("sample"));
                valid = false;
            }
        }
    }

    (presentation, valid.then_some(draft))
}

/// Read the page/block display state persisted beside a saved query.
///
/// The existing singular `tine.*` merge remains owned by
/// [`merge_block_property_view`]. This helper reads only scoped additions, so
/// old notes retain byte-for-byte semantics. The Tauri `ParsedQuery` flattens
/// this typed state alongside its unchanged singular `{query, view}` answer.
pub fn read_scoped_display_settings(
    block_properties: &[(String, String)],
) -> ScopedDisplaySettings {
    let mut unreadable_settings = Vec::new();
    let (page_presentation, page_display) = read_scoped_display_namespace(
        block_properties,
        ScopedDisplayNamespace::Page,
        &mut unreadable_settings,
    );
    let (block_presentation, block_display) = read_scoped_display_namespace(
        block_properties,
        ScopedDisplayNamespace::Block,
        &mut unreadable_settings,
    );

    let page_match_scope = match raw_property(block_properties, "page-match-scope") {
        Some(value) => match value.trim() {
            "names" => Some(FriendlyPageMatchScope::Names),
            "content" => Some(FriendlyPageMatchScope::Content),
            "both" => Some(FriendlyPageMatchScope::Both),
            _ => {
                unreadable_settings.push(format!("{VIEW_PROPERTY_PREFIX}page-match-scope"));
                None
            }
        },
        None => None,
    };

    ScopedDisplaySettings {
        page_presentation,
        page_display,
        block_presentation,
        block_display,
        page_match_scope,
        unreadable_settings,
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

/// Why a query returned nothing, as a plan (N19): for a root `And` after
/// normalization, one entry per top-level conjunct with the rows it matches
/// alone and the rows the rest match without it; for any other root, one
/// entry for the whole query. It decomposes the RESOLVED tree (§4.4); when the
/// binding failed there is nothing honest to count and the plan is empty.
///
/// The decomposition is separated from the evaluator so
/// that one explanation's probes can be answered from ONE coherent snapshot
/// rather than one page walk each (RET1).
///
/// Nothing here reads a graph: it takes the resolved tree apart, prints each
/// conjunct and states which probe queries have to be counted. The evaluator is
/// the caller's (in og, lane Q2's; nothing counts probes yet), and
/// [`ExplainPlan::answer`] reassembles the rows from its counts, so the
/// decomposition, the printing and the `And`/non-`And` rule exist once.
pub struct ExplainPlan {
    /// Every probe query, in the order the evaluator must count them. Empty
    /// when the binding failed: there is nothing honest to count then.
    pub probes: Vec<crate::query::ir::Query>,
    /// One entry per answer row: the printed conjunct, the index of its `alone`
    /// probe, and of its `without` probe when the root is an `And`.
    rows: Vec<(String, usize, Option<usize>)>,
}

/// The plan and the counts disagree: the evaluator answered a different plan
/// than it was given (RET2).
///
/// It carries no payload on purpose. Every caller maps it onto its own
/// backend's bounded availability vocabulary — `InvalidSnapshot` on both — and
/// a length is not something a user can act on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExplainCountMismatch;

impl ExplainPlan {
    /// The rows, given EXACTLY one count per [`ExplainPlan::probes`] entry, in
    /// order.
    ///
    /// **RET2 made the length check central and fallible.** A `counts` slice
    /// that does not match the plan can only mean the evaluator answered a
    /// different plan than it was given; reading a missing count as `0` printed
    /// "0 rows match this conjunct" — a confident, wrong explanation of why a
    /// query was empty, indistinguishable from a real zero. An empty plan with
    /// an empty vector is the one valid empty case (a refused binding has
    /// nothing honest to count), and it is accepted by the same rule rather
    /// than by an exception.
    pub fn answer(
        &self,
        resolved: &crate::query::ResolvedQuery,
        counts: &[usize],
    ) -> Result<crate::query::ir::ExplainEmptyResult, ExplainCountMismatch> {
        if counts.len() != self.probes.len() {
            return Err(ExplainCountMismatch);
        }
        let at = |index: usize| counts.get(index).copied().unwrap_or(0);
        Ok(crate::query::ir::ExplainEmptyResult {
            rows: self
                .rows
                .iter()
                .map(|(conjunct, alone, without)| EmptyExplanation {
                    conjunct: conjunct.clone(),
                    alone: at(*alone),
                    without: without.map(at),
                })
                .collect(),
            diagnostics: resolved.query().diagnostics.clone(),
            report: resolved.report().clone(),
        })
    }
}

/// Build the [`ExplainPlan`] for a resolved query. A non-executable query
/// ([`ResolvedQuery::is_executable`](crate::query::ResolvedQuery::is_executable)
/// false) gets an empty plan. Otherwise the evaluable (`Off`-free), normalized
/// filter is decomposed: a root `And` of two or more conjuncts yields two
/// probes per conjunct (it alone, then the `And` of all the others), and any
/// other root yields one probe for the whole filter. Each probe keeps the
/// query's anchor and source, has its diagnostics cleared, and is printed as
/// TQL for the row label. Pure; O(conjuncts²) filter clones.
pub fn explain_empty_plan(resolved: &crate::query::ResolvedQuery) -> ExplainPlan {
    use crate::query::ir::Filter;

    let query = resolved.query();
    let mut plan = ExplainPlan {
        probes: Vec::new(),
        rows: Vec::new(),
    };
    if !resolved.is_executable() {
        return plan;
    }

    // The probe carries the query's anchor and its immutable source (so the
    // printer still reads as query text) with the conjunct as its filter and no
    // diagnostics — an explanation counts rows, it does not re-refuse.
    let probe = |filter: Filter| -> crate::query::ir::Query {
        let mut probe = query.clone();
        probe.filter = filter;
        probe.diagnostics.clear();
        probe
    };
    let printed = |filter: &Filter| -> String {
        let mut probe = query.clone();
        probe.filter = filter.clone();
        crate::query::print::print_tql(&probe)
    };

    let mut evaluable = query.clone();
    evaluable.filter = query.evaluable_filter();
    match evaluable.normalized().filter {
        Filter::And { items } if items.len() > 1 => {
            for (index, item) in items.iter().enumerate() {
                let others = items
                    .iter()
                    .enumerate()
                    .filter(|(other, _)| *other != index)
                    .map(|(_, filter)| filter.clone())
                    .collect::<Vec<_>>();
                let conjunct = printed(item);
                let alone = plan.probes.len();
                plan.probes.push(probe(item.clone()));
                let without = plan.probes.len();
                plan.probes.push(probe(Filter::and(others)));
                plan.rows.push((conjunct, alone, Some(without)));
            }
        }
        whole => {
            let conjunct = printed(&whole);
            let alone = plan.probes.len();
            plan.probes.push(probe(whole));
            plan.rows.push((conjunct, alone, None));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ir::ViewKind;
    use serde::Deserialize;

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
        // P5B: the merge now returns the CANONICAL grouping FieldId. On a board
        // face a bare `status` is an ordinary property — it used to fall through
        // `isFieldId` in the renderer and silently become the task marker.
        assert_eq!(merged.group_by, Some(Field::new("prop:status")));
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
    fn tine_columns_owns_the_visible_columns_and_tine_fields_keeps_the_typed_schema() {
        // The clobber this split exists to end: a block can carry BOTH a typed
        // sheet schema and a column selection, and neither erases the other.
        let merged = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[
                ("tine.columns", "page; cost"),
                ("tine.fields", "cost=number;severity=text"),
            ]),
        );
        assert_eq!(merged.columns, vec![Field::new("page"), Field::new("cost")]);
    }

    #[test]
    fn a_typed_or_mixed_tine_fields_is_schema_and_never_columns() {
        for value in ["cost=number;severity=text", "page;cost=number"] {
            let merged = merge_block_property_view(
                &ViewSettings::default(),
                &properties(&[("tine.fields", value)]),
            );
            assert!(
                merged.columns.is_empty(),
                "`=` anywhere means schema or mixed, never columns: {value:?}"
            );
        }
    }

    #[test]
    fn a_present_but_empty_or_invalid_columns_list_clears_and_never_falls_back() {
        for value in ["", "   ", "a;cost=number", "a;b\rc"] {
            let merged = merge_block_property_view(
                &ViewSettings::default(),
                &properties(&[("tine.columns", value), ("tine.fields", "a;b")]),
            );
            assert!(
                merged.columns.is_empty(),
                "a PRESENT tine.columns is an explicit statement; clearing cannot \
                 resurrect the legacy list: {value:?}"
            );
            assert_eq!(
                resolve_query_columns(&properties(&[("tine.columns", value)])),
                QueryColumns::Cleared
            );
        }
    }

    #[test]
    fn a_legacy_bare_tine_fields_list_still_names_columns_when_the_new_key_is_absent() {
        assert_eq!(
            resolve_query_columns(&properties(&[("tine.fields", "page; status; cost")])),
            QueryColumns::Named(vec![
                Field::new("page"),
                Field::new("status"),
                Field::new("cost")
            ]),
        );
        // Nothing here writes: reading a legacy list never rewrites the note
        // (I-4). This is authored-note compatibility, not a D-1 private-format
        // migration.
        assert_eq!(resolve_query_columns(&properties(&[])), QueryColumns::Unset);
    }

    #[test]
    fn duplicate_column_names_are_retained_verbatim_for_the_renderer_to_decide() {
        assert_eq!(
            resolve_query_columns(&properties(&[("tine.columns", "cost;cost;Cost")])),
            QueryColumns::Named(vec![
                Field::new("cost"),
                Field::new("cost"),
                Field::new("Cost")
            ]),
        );
    }

    #[test]
    fn the_merge_returns_the_canonical_grouping_field_id() {
        // Both meanings of the same bare legacy token, at the two view families.
        let board = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[("tine.view", "board"), ("tine.group-by", "state")]),
        );
        assert_eq!(board.group_by, Some(Field::new("state")), "task marker");
        let list = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[("tine.group-by", "state")]),
        );
        assert_eq!(
            list.group_by,
            Some(Field::new("prop:state")),
            "the list grouper has always read this as an ordinary property"
        );
    }

    #[test]
    fn the_new_group_key_wins_and_an_empty_one_is_an_explicit_clear() {
        let explicit = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[
                ("tine.view", "board"),
                ("tine.group-field", "prop:state"),
                ("tine.group-by", "state"),
            ]),
        );
        assert_eq!(explicit.group_by, Some(Field::new("prop:state")));
        // The empty field is the wire spelling of "explicitly no grouping". It
        // is NOT `None`: `None` is the only state a Board default may fill.
        let cleared = merge_block_property_view(
            &ViewSettings {
                group_by: Some(Field::new("status")),
                ..ViewSettings::default()
            },
            &properties(&[("tine.group-field", ""), ("tine.group-by", "state")]),
        );
        assert_eq!(cleared.group_by, Some(Field::new("")));
        let unset = merge_block_property_view(
            &ViewSettings::default(),
            &properties(&[("tine.view", "board")]),
        );
        assert_eq!(unset.group_by, None);
    }

    #[test]
    fn a_property_outside_the_tine_namespace_is_not_a_view_setting() {
        let merged =
            merge_block_property_view(&ViewSettings::default(), &properties(&[("sample", "9")]));
        assert_eq!(merged.sample, None);
    }

    #[derive(Debug, Deserialize)]
    struct ScopedDisplayFixture {
        name: String,
        properties: Vec<(String, String)>,
        expected: serde_json::Value,
    }

    #[test]
    fn scoped_display_properties_follow_the_golden_contract() {
        let fixtures: Vec<ScopedDisplayFixture> = serde_json::from_str(include_str!(
            "fixtures/query-ir/scoped_display_settings.json"
        ))
        .expect("scoped display fixtures parse");
        for fixture in fixtures {
            let actual = read_scoped_display_settings(&fixture.properties);
            assert_eq!(
                serde_json::to_value(&actual).expect("scoped display state serializes"),
                fixture.expected,
                "{} wire",
                fixture.name
            );
            let expected: ScopedDisplaySettings = serde_json::from_value(fixture.expected)
                .expect("fixture expected state deserializes");
            assert_eq!(actual, expected, "{} typed state", fixture.name);
        }
    }

    #[test]
    fn every_page_match_scope_value_is_typed_without_an_execution_default() {
        for (raw, expected) in [
            ("names", FriendlyPageMatchScope::Names),
            ("content", FriendlyPageMatchScope::Content),
            ("both", FriendlyPageMatchScope::Both),
        ] {
            let state =
                read_scoped_display_settings(&properties(&[("tine.page-match-scope", raw)]));
            assert_eq!(state.page_match_scope, Some(expected));
            assert!(state.unreadable_settings.is_empty());
        }
        assert_eq!(
            read_scoped_display_settings(&[]).page_match_scope,
            None,
            "absence stays distinct; the execution layer owns the names fallback"
        );
    }

    #[test]
    fn the_singular_merge_boundary_does_not_consume_scoped_properties() {
        let parsed = ViewSettings {
            view: Some(ViewKind::List),
            sort: vec![(Field::new("page"), SortDir::Asc)],
            columns: vec![Field::new("legacy")],
            sample: Some(3),
            ..ViewSettings::default()
        };
        let block_properties = properties(&[
            ("tine.page-view", "table"),
            ("tine.page-display", "1"),
            ("tine.page-sort", "name desc"),
            ("tine.page-columns", "name"),
            ("tine.page-sample", "9"),
        ]);

        assert_eq!(
            merge_block_property_view(&parsed, &block_properties),
            parsed,
            "before this reader is wired, the existing singular merge has no scoped answer"
        );
        let scoped = read_scoped_display_settings(&block_properties);
        assert_eq!(scoped.page_presentation, Some(ViewKind::Table));
        assert_eq!(
            scoped.page_display,
            Some(DisplayDraft {
                sort: Some(vec![(Field::new("name"), SortDir::Desc)]),
                columns: Some(vec![Field::new("name")]),
                sample: Some(9),
                ..DisplayDraft::default()
            })
        );
    }
}
/// The view statistics actually execute under, without authoring a saved
/// default. Three rewrites, in order: a Board with no grouping groups by
/// `state` (ADR 0030); an explicit empty grouping (`Some(Field(""))`) becomes
/// `None`; and a view that groups but requests no aggregate gains one
/// whole-result `("", Count)` aggregate. Q3 consumes this same effective-view
/// seam after scoped resolution. Pure.
pub fn effective_statistics_view(view: &super::ir::ViewSettings) -> super::ir::ViewSettings {
    use super::ir::{AggFn, Field, ViewKind};
    let mut effective = view.clone();
    if effective.group_by.is_none() && effective.view == Some(ViewKind::Board) {
        effective.group_by = Some(Field::new("state"));
    }
    if effective
        .group_by
        .as_ref()
        .is_some_and(|field| field.as_str().is_empty())
    {
        effective.group_by = None;
    }
    if effective.aggregates.is_empty() && effective.group_by.is_some() {
        effective.aggregates.push((Field::new(""), AggFn::Count));
    }
    effective
}

/// Advanced result transforms do not request query-wide statistics. Keep their
/// ordering/sample settings while explicitly clearing even implicit grouping.
pub fn statistics_execution_view(
    query: &super::ir::Query,
    view: &super::ir::ViewSettings,
) -> super::ir::ViewSettings {
    let mut effective = view.clone();
    if matches!(query.source, super::ir::Source::Advanced { .. }) {
        effective.aggregates.clear();
        effective.group_by = Some(super::ir::Field::new(""));
    }
    effective
}
