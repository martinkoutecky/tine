//! The ONE text → IR entry for every query input shape (SPEC §4, §7.1), and
//! the execution-time binding boundary [`resolve_for_execution`] (§4.4).
//!
//! Transcribed from master `crates/tine-core/src/query.rs` (5dfc84503). Pure:
//! no graph, no SQL. Execution (results, statistics, explain-empty runs)
//! consumes a [`ResolvedQuery`] and lives outside this module.

use super::advanced_patterns::advanced_pred;
use super::ir::{self, Anchor, Filter, Query, Source, ViewSettings};
use super::{
    is_advanced, macro_text, og, query_nesting_within_limit, query_source_within_limit, registry,
    tql,
};
use crate::date::JournalDate;

/// Which surface syntax a query's text is written in (SPEC §4).
///
/// The macro name chooses it when the block is saved (Q3): `{{query …}}` is the
/// OG DSL, `{{tine-query …}}` is TQL. Both are the same IR afterwards — the
/// dialect is a property of the TEXT, never of the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryDialect {
    Og,
    Tql,
}

/// The ONE text → IR entry, for either dialect: the I-22 source limits, then
/// the dialect's parser (§4.1, §4.2). A source that is refused, or is datalog
/// rather than a simple query, comes back as a query carrying its diagnostic —
/// never as a silently empty one.
pub fn parse_query_text(
    query_src: &str,
    dialect: QueryDialect,
    today: JournalDate,
) -> (Query, ViewSettings) {
    parse_query_text_with_registry(query_src, dialect, today, registry::Registry::none())
}

/// [`parse_query_text`] against a registry snapshot. The parse is identical;
/// only an `UnknownIdent` diagnostic differs, gaining the nearest property keys
/// the graph actually has as `prop('…')` suggestions (§4.2.2). The OG dialect
/// has no identifier vocabulary to be wrong about, so it ignores the registry.
pub fn parse_query_text_with_registry(
    query_src: &str,
    dialect: QueryDialect,
    today: JournalDate,
    registry: &registry::Registry,
) -> (Query, ViewSettings) {
    match dialect {
        QueryDialect::Og => parse_query_source(query_src, today),
        QueryDialect::Tql => {
            use ir::{Diagnostic, DiagnosticKind};
            if !query_source_within_limit(query_src) {
                let mut query = Query::new(
                    Anchor::Block,
                    Filter::False,
                    Source::Tql {
                        original: query_src.to_string(),
                        og_options: String::new(),
                    },
                );
                query.diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Size,
                    "the query source is too large",
                ));
                return (query, ViewSettings::default());
            }
            tql::parse_tql(query_src, registry)
        }
    }
}

/// **The macro-input dispatch (§7.1, C3).** Which INPUT a caller has, which is
/// not the same question as which grammar the text is written in.
///
/// `Og`, `Tql` and `Advanced` are explicit FORM inputs: the caller already knows
/// the grammar (the TQL pane, the `#+BEGIN_QUERY` container extractor). The two
/// `Macro*` inputs take the COMPLETE raw macro argument, without the outer
/// `{{`/`}}`, and are the only place a query argument is ever split — after this
/// wave nothing outside `query_parse` splits one (§4.3, Y2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryInput {
    /// An OG DSL form.
    Og,
    /// A TQL form: filter and anchor only, never options (§4.3.1).
    Tql,
    /// A datalog form, including a whole `{:query … :inputs …}` map (§4.4).
    Advanced,
    /// The complete argument of a `{{query …}}` macro: OG or advanced.
    MacroQuery,
    /// The complete argument of a `{{tine-query …}}` macro: TQL.
    MacroTql,
}

/// Whether a `{{query …}}` form is datalog rather than the OG DSL.
///
/// **The ONE discriminator** (§7.1): the existing `Macro.tsx` / `ExportModal.tsx`
/// regexes are deleted in P0-ts and every caller asks this instead, so the two
/// cannot disagree about which source variant a block holds. A `:find` or
/// `:where` token inside an OG string or a page ref is text, not datalog — which
/// is exactly the case the TypeScript regexes got wrong — so the scan protects
/// both. There is **no speculative parse-and-fallback**: the token decides.
pub(crate) fn advanced_form(form: &str) -> bool {
    let bytes = form.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                // An OG double-quoted string, backslash-escaped.
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'[' if form[i..].starts_with("[[") => {
                i = match form[i + 2..].find("]]") {
                    Some(offset) => i + 2 + offset + 2,
                    None => form.len(),
                };
            }
            b':' => {
                let rest = &form[i..];
                if rest.starts_with(":find") || rest.starts_with(":where") {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// The ONE text → IR entry for every input shape (§7.1, C3).
///
/// The macro inputs split their argument once, here, with the one
/// [`macro_text::split_trailing_map`]; the source variant records the grammar,
/// `source.original` holds the exact form slice and `source.og_options` the
/// opaque map or the empty string. The §4.1 precedence merge of the host
/// block's `tine.*` properties happens above this, in the command.
///
/// Limits (I-22) are not uniform. `Og` and the OG branch of `MacroQuery` apply
/// the size and nesting limits to the form; `Tql`/`MacroTql` apply the size
/// limit to the form (nesting is bounded by the TQL parser's recursion limit
/// and reported as `Syntax`). `Advanced` and the advanced branch of
/// `MacroQuery` apply **no** limit here: the whole text is stored in
/// `Source::Advanced.original` and the limits are enforced only at
/// [`resolve_for_execution`], where an oversize form resolves as unsupported.
/// The complete macro argument, including its options map, must fit the shared
/// byte ceiling before the splitter scans it. The advanced form is still
/// inspected at execution time; an oversized macro is refused here.
pub fn parse_query_input(
    text: &str,
    input: QueryInput,
    today: JournalDate,
    registry: &registry::Registry,
) -> (Query, ViewSettings) {
    match input {
        QueryInput::Og => parse_query_source(text, today),
        QueryInput::Advanced => advanced_source_query(text, String::new()),
        QueryInput::Tql => parse_query_text_with_registry(text, QueryDialect::Tql, today, registry),
        QueryInput::MacroTql => {
            if !query_source_within_limit(text) {
                return refuse_tql_source(text, String::new());
            }
            let (form, og_options) =
                macro_text::split_trailing_map(text, macro_text::FormFamily::Tql);
            if !query_source_within_limit(&form) {
                return refuse_tql_source(&form, og_options);
            }
            tql::parse_tql_with_options(&form, og_options, registry)
        }
        QueryInput::MacroQuery => {
            if !query_source_within_limit(text) {
                return parse_query_source(text, today);
            }
            let (form, og_options) =
                macro_text::split_trailing_map(text, macro_text::FormFamily::Edn);
            // A whole advanced map is the FORM, never options: the splitter
            // already refused to split a map with nothing before it (§4.4).
            if advanced_form(&form) {
                return advanced_source_query(&form, og_options);
            }
            let (mut query, view) = parse_query_source(&form, today);
            if let Source::Og {
                og_options: slot, ..
            } = &mut query.source
            {
                *slot = og_options;
            }
            (query, view)
        }
    }
}

/// A datalog form, retained as [`Source::Advanced`] with its complete authored
/// text (§4.4, C2).
///
/// The SIMPLE engine still refuses to run it — that is unchanged, and §4.4's
/// shared `resolve_for_execution` boundary is Wave D. What changes here is that
/// the source survives as the advanced variant, so a title-only edit can print
/// it back through the source-preserving path instead of being told the OG
/// printer cannot express it. `original` is the whole form, `:query`/`:inputs`
/// map and all; only a map that FOLLOWS it is options.
fn advanced_source_query(form: &str, og_options: String) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let mut query = Query::new(
        Anchor::Block,
        Filter::False,
        Source::Advanced {
            original: form.to_string(),
            og_options,
        },
    );
    query.diagnostics.push(Diagnostic::new(
        DiagnosticKind::Syntax,
        "this is an advanced (datalog) query, not the simple DSL",
    ));
    (query, ViewSettings::default())
}

fn refuse_tql_source(form: &str, og_options: String) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let mut query = Query::new(
        Anchor::Block,
        Filter::False,
        Source::Tql {
            original: form.to_string(),
            og_options,
        },
    );
    query.diagnostics.push(Diagnostic::new(
        DiagnosticKind::Size,
        "the query source is too large",
    ));
    (query, ViewSettings::default())
}

/// The OG `{{query}}` half of [`parse_query_text`].
pub(crate) fn parse_query_source(query_src: &str, today: JournalDate) -> (Query, ViewSettings) {
    use ir::{Diagnostic, DiagnosticKind};
    let refuse = |kind, message: &str| {
        let mut query = Query::new(
            Anchor::Block,
            Filter::False,
            Source::Og {
                original: query_src.to_string(),
                og_options: String::new(),
            },
        );
        query.diagnostics.push(Diagnostic::new(kind, message));
        (query, ViewSettings::default())
    };
    if !query_source_within_limit(query_src) {
        return refuse(DiagnosticKind::Size, "the query source is too large");
    }
    if !query_nesting_within_limit(query_src) {
        return refuse(DiagnosticKind::Depth, "the query nests too deeply");
    }
    if is_advanced(query_src) {
        return refuse(
            DiagnosticKind::Syntax,
            "this is an advanced (datalog) query, not the simple DSL",
        );
    }
    og::parse_og(query_src, today)
    // NOTE: the advanced refusal above is the SIMPLE-query engine's answer and
    // is unchanged. `query_parse`'s advanced inspection (§4.4) is Wave D's
    // `resolve_for_execution` boundary; `advanced_form` above is only the
    // §7.1 discriminator, and this wave routes both to the OG parser exactly as
    // Wave B did, so no behaviour depends on it yet.
}

// ---------------------------------------------------------------------------
// SPEC §4.4 (R5): execution-time binding
// ---------------------------------------------------------------------------

/// The provisional diagnostic `query_parse(advanced)` attaches to an advanced
/// form it has only INSPECTED (§4.4).
///
/// It is not a syntax verdict — the form may be perfectly well formed — it says
/// "the simple engine cannot answer this as it stands". §4.4 calls this a
/// *provisional inspection diagnostic* and requires the bound lowering's own
/// diagnostics to REPLACE it at execution time, which
/// [`resolve_for_execution`] does by matching this exact message. Every other
/// parse diagnostic (an I-22 size or depth refusal) is STATIC and survives.
pub(crate) const ADVANCED_UNRESOLVED_MESSAGE: &str =
    "this is an advanced (datalog) query, not the simple DSL";

/// The message a resolution that could not bind the query reports (§4.4):
/// at least one clause (or `:result-transform`, or `:inputs` binding) could not
/// be lowered, or the source exceeded the size/nesting limits. The refusal is
/// WHOLE: a partially recognized form does not run (see
/// [`resolve_for_execution`]). The diagnostic carries this text followed by the
/// parenthesized list of what was not lowered (see [`unsupported_message`]).
pub(crate) const ADVANCED_UNSUPPORTED_MESSAGE: &str =
    "this advanced query has clauses Tine cannot run, so it returns no results";

/// [`ADVANCED_UNSUPPORTED_MESSAGE`] naming the ignored clauses (distinct, in
/// first-seen order, at most eight).
fn unsupported_message(ignored: &[String]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for item in ignored {
        if !names.contains(&item.as_str()) {
            names.push(item);
        }
    }
    if names.is_empty() {
        return ADVANCED_UNSUPPORTED_MESSAGE.to_string();
    }
    let more = names.len().saturating_sub(8);
    names.truncate(8);
    let tail = if more > 0 {
        format!(", and {more} more")
    } else {
        String::new()
    };
    format!(
        "{ADVANCED_UNSUPPORTED_MESSAGE} (not lowered: {}{tail})",
        names.join(", ")
    )
}

/// A query BOUND to one execution (SPEC §4.4, R5).
///
/// **The type is the guarantee.** Every evaluator, every explain-empty
/// decomposition and every result cache below takes a `ResolvedQuery`, and the
/// only way to obtain one is [`resolve_for_execution`], which consumes an
/// unresolved [`Query`]. A resolved query therefore cannot be resolved again —
/// not by convention, but because there is no function that accepts one and
/// returns another.
///
/// It carries its own `today`, the ONE execution-day snapshot: taken once here
/// rather than by each evaluator, so a rollover cannot land between the
/// page-anchored and block-anchored halves of a single answer, nor between a
/// result and the explanation of why it was empty.
#[derive(Debug, Clone)]
pub struct ResolvedQuery {
    query: Query,
    report: ir::QueryReport,
    today: JournalDate,
}

impl ResolvedQuery {
    /// The bound IR — an advanced form's lowered filter, or the OG/TQL IR
    /// unchanged.
    pub fn query(&self) -> &Query {
        &self.query
    }

    /// The support report this binding produced (M5). OG and TQL report an
    /// empty `ignored` and `supported = true`.
    pub fn report(&self) -> &ir::QueryReport {
        &self.report
    }

    /// The one execution-day snapshot every leaf in this execution reads.
    pub fn today(&self) -> JournalDate {
        self.today
    }

    /// Whether this execution may be evaluated: `false` when the advanced
    /// lowering refused (`report().supported == false`) **or** when the bound
    /// query carries any enabled diagnostic ([`Query::is_invalid`]): an OG/TQL
    /// parse error, a size/depth refusal, or datalog that reached the OG parser
    /// as `Source::Og`. A non-executable query has diagnostics and a report,
    /// and no counts or explain plan.
    pub fn is_executable(&self) -> bool {
        self.report.supported && !self.query.is_invalid()
    }
}

/// **The ONE execution-time binding boundary** (SPEC §4.4, R5).
///
/// It runs BEFORE the invalidity check, before normalization and cache lookup,
/// before SQL/walk dispatch, and before explain-empty decomposition — so that
/// every one of those sees the same bound tree, and none of them can be handed
/// an advanced placeholder to interpret on its own.
///
/// For [`Source::Advanced`] it calls the ONE existing lowerer,
/// [`advanced_pred`], with the AUTHORED source (`Source::Advanced.original`,
/// `:query`/`:inputs` and all), the caller's current page, and this execution's
/// day, and carries its `ran`/`ignored` report through verbatim. The
/// provisional inspection diagnostic is removed; static (size/depth)
/// diagnostics survive.
///
/// **Execution is all-or-nothing** (Martin, 2026-10-03; OG runs the whole
/// DataScript query, so a subset answer is a different query's answer). If
/// EVERY clause lowers the conjunction runs with `supported = true` and an
/// empty `report.ignored`. If any clause cannot be lowered (an unsupported
/// pattern or function, a `?current-page` pattern with no current page, an
/// `:inputs` binding Tine does not resolve such as `:current-block`, a
/// `:result-transform`), or the source exceeds the size/nesting limits, the
/// WHOLE query is refused: the filter is [`Filter::False`], `supported =
/// false`, `report.ignored` names every clause that did not lower, and the
/// [`ADVANCED_UNSUPPORTED_MESSAGE`] `Syntax` diagnostic (naming them) is added.
/// No partial result is ever produced.
///
/// For every other source the IR is already the query; only the execution-day
/// snapshot is added, which is what makes an OG `(between -7d today)` and a TQL
/// `day > -7d` read the same clock as an advanced `?today`.
pub fn resolve_for_execution(
    query: &Query,
    context: &ir::ExecutionContext,
    today: JournalDate,
) -> ResolvedQuery {
    use ir::{Diagnostic, DiagnosticKind};

    let Source::Advanced { original, .. } = &query.source else {
        return ResolvedQuery {
            query: query.clone(),
            report: ir::QueryReport {
                ran: Vec::new(),
                ignored: Vec::new(),
                supported: true,
            },
            today,
        };
    };

    // Static diagnostics survive the binding; the provisional inspection one
    // does not (§4.4).
    let static_diagnostics: Vec<Diagnostic> = query
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            !(diagnostic.kind == DiagnosticKind::Syntax
                && diagnostic.message == ADVANCED_UNRESOLVED_MESSAGE)
        })
        .cloned()
        .collect();

    let (lowered, ran, ignored) = advanced_pred(original, context.current_page.as_deref(), today);
    let mut bound = Query {
        anchor: query.anchor,
        filter: lowered
            .as_ref()
            .map_or(Filter::False, |query| query.filter.clone()),
        diagnostics: static_diagnostics,
        // The immutable source stays available for printing (§4.4). It is never
        // re-read as a filter after this point.
        source: query.source.clone(),
    };
    let supported = lowered.is_some();
    if !supported {
        bound.diagnostics.push(Diagnostic::new(
            DiagnosticKind::Syntax,
            unsupported_message(&ignored),
        ));
    }
    ResolvedQuery {
        query: bound,
        report: ir::QueryReport {
            ran,
            ignored,
            supported,
        },
        today,
    }
}
