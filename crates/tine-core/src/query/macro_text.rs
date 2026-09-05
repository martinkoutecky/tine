//! The document form of a query (SPEC §4.3.1, §7.9).
//!
//! A query does not live in a text box; it lives inside a `{{query …}}` or
//! `{{tine-query …}}` macro in a Markdown or Org document. Four questions
//! follow from that, and this module is the ONE place each is answered (I-12):
//!
//! 1. **Where does the trailing options map begin?** [`split_trailing_map`] —
//!    the W3 transcription of `src/editor/edn.ts:89`, widened to take the input
//!    language family so a TQL `'{'` literal and an EDN `"{"` string are both
//!    protected by the rules of their own grammar. After this wave nothing
//!    outside `query_parse` splits a query argument.
//! 2. **Where does a macro's raw extent run?** [`query_macro_extent`] — the
//!    transcription of `edn.ts::queryMacroExtent` (`:43-86`). **Raw bytes, not
//!    reconstructed AST arguments, are the query transport:** upstream
//!    `macro_args` splits on commas and the macro parser stops before the first
//!    `}`, so `{{tine-query @block and content = 'a,b'}}` comes back from the
//!    document parser as the two fragments `@block and content = 'a` and `b'`.
//!    Rejoining them is lossy by construction, so this reader never joins.
//! 3. **Can this argument survive as macro bytes?** [`macro_safe`] — the exact
//!    §4.3.1 lexical rule: no CR/LF, no `#{`, and no `}` except the final
//!    closing brace of the trailing options map.
//! 4. **Does the real parser actually read it back?** [`recognizable_macro`] —
//!    because (3) is necessary and not sufficient. The already pinned `lsdoc`
//!    inline parser is run in BOTH Markdown and Org modes and must produce the
//!    intended `Macro` node at the wrapper start; then (2) must recover the
//!    complete form and options from the same bytes. This is a save-time
//!    serializer check, not a second query reader and not a parser change.
//!
//! Every one of these is a LEXICAL question about bytes in a document. None of
//! them parses the query language; the grammar lives in `og.rs` and `tql.rs`.

use crate::query::ir::{Diagnostic, DiagnosticKind, Span};

/// The macro names a query can be spelled with.
///
/// **Deliberately private to this wave.** SPEC §7.9 (Y1) makes the public
/// `QUERY_MACRO_NAMES` in `query/ir.rs`, together with its doc-code consistency
/// test against the TypeScript twin, a P0-ts edit. P0-ts can promote this
/// constant without renaming a single reader.
const QUERY_MACRO_NAMES: [&str; 2] = ["tine-query", "query"];

/// Which grammar's literals protect a delimiter while scanning FORM text.
///
/// This is a property of the text being scanned, not of the query: an OG or
/// advanced form is EDN-shaped (`"…"` strings, `;` comments), a TQL form is
/// SQL-shaped (`'…'` strings with `''` doubling). Inside an options map the
/// EDN rules always apply, whichever family the form was — the map is EDN
/// either way (§4.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormFamily {
    /// `{{query …}}`: the OG DSL and the advanced `{:query …}` map.
    Edn,
    /// `{{tine-query …}}`: TQL.
    Tql,
}

impl FormFamily {
    /// The family the macro NAME implies. `query` carries OG or advanced text,
    /// `tine-query` carries TQL (§7.1).
    pub fn for_macro_name(name: &str) -> FormFamily {
        if name.eq_ignore_ascii_case("tine-query") {
            FormFamily::Tql
        } else {
            FormFamily::Edn
        }
    }
}

// ---------------------------------------------------------------------------
// The one lexical scan (§4.3.1 W3)
// ---------------------------------------------------------------------------

/// One brace the scan found outside every literal, comment and page ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Brace {
    /// Byte offset of the brace.
    at: usize,
    /// `true` for `{`, `false` for `}`.
    open: bool,
    /// Nesting depth AFTER this brace, counting from `form_depth`.
    depth: i32,
}

/// **The one scan.** Walk `text` once and report every `{` / `}` that is not
/// inside a protected region, with the depth it produces.
///
/// `form_depth` is the depth at which the form text sits: 0 when scanning a
/// macro ARGUMENT (the splitter), 2 when scanning from inside `{{` (the extent
/// reader). While the depth is at `form_depth` the `family` decides which
/// literals protect a brace; deeper than that we are inside an options map and
/// EDN rules apply — strings and semicolon comments protect delimiters.
///
/// An unterminated literal consumes to end of input rather than resynchronising:
/// that is what makes an unbalanced `}` inside a literal invisible to the split,
/// which is the fixture §4.3.1 names.
fn scan_braces(text: &str, family: FormFamily, form_depth: i32) -> Vec<Brace> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = form_depth;
    let mut i = 0usize;
    while i < bytes.len() {
        // Inside a map the text is EDN whatever the form was; an EDN symbol's
        // apostrophe (`'foo`, `#'x`) is never a SQL string, and a semicolon in
        // TQL form text is never a comment.
        let edn = depth > form_depth || family == FormFamily::Edn;
        match bytes[i] {
            b'"' if edn => {
                i = edn_string_end(text, i);
                continue;
            }
            b'\'' if !edn => {
                i = tql_string_end(text, i);
                continue;
            }
            b';' if edn => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'[' if text[i..].starts_with("[[") => {
                i = page_ref_end(text, i);
                continue;
            }
            b'{' => {
                depth += 1;
                out.push(Brace {
                    at: i,
                    open: true,
                    depth,
                });
            }
            b'}' => {
                depth -= 1;
                out.push(Brace {
                    at: i,
                    open: false,
                    depth,
                });
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Index just past an EDN double-quoted string opening at `at`; end of input if
/// unterminated. Only `\` escapes the next byte (`edn.ts::strClose`).
fn edn_string_end(text: &str, at: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = at + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return j + 1,
            _ => j += 1,
        }
    }
    text.len()
}

/// Index just past a TQL single-quoted string opening at `at`; end of input if
/// unterminated. SQL doubles the quote (`''`) rather than backslash-escaping it.
fn tql_string_end(text: &str, at: usize) -> usize {
    let bytes = text.as_bytes();
    let mut j = at + 1;
    while j < bytes.len() {
        if bytes[j] == b'\'' {
            if bytes.get(j + 1) == Some(&b'\'') {
                j += 2;
                continue;
            }
            return j + 1;
        }
        j += 1;
    }
    text.len()
}

/// Index just past a `[[page ref]]` opening at `at`; end of input if
/// unterminated. Page refs do not nest, so the first `]]` closes it
/// (`edn.ts::pageRefEnd`) — which is what makes `[[a}}b]]` opaque to the scan.
fn page_ref_end(text: &str, at: usize) -> usize {
    match text[at + 2..].find("]]") {
        Some(offset) => at + 2 + offset + 2,
        None => text.len(),
    }
}

// ---------------------------------------------------------------------------
// C1 / W3 — the one `split_trailing_map`
// ---------------------------------------------------------------------------

/// Split a macro argument into its form and the trailing balanced `{…}` options
/// map, INCLUDING the braces and verbatim (EDN comments and all).
///
/// The transcription of `src/editor/edn.ts:89`, widened per §4.3.1: the family
/// chooses the literals that protect the opening brace, and the map is only
/// split off **when a nonempty form precedes it**, so a whole advanced
/// `{:query … :inputs …}` map is the FORM and never mistaken for options
/// (§4.4, X4).
///
/// Both parts come back trimmed, exactly as the TypeScript helper trims them:
/// the map's own extent is unaffected (it starts at `{` and ends at `}`), and
/// re-emitting `form + " " + options` is then idempotent.
pub fn split_trailing_map(argument: &str, family: FormFamily) -> (String, String) {
    let trimmed = argument.trim_end();
    if !trimmed.ends_with('}') {
        return (argument.trim().to_string(), String::new());
    }
    let last = trimmed.len() - 1;
    // ONE scan: remember where the current outermost map opened, and take it
    // only if it closes on the argument's very last byte.
    let mut opened_at = None;
    for brace in scan_braces(trimmed, family, 0) {
        if brace.open {
            if brace.depth == 1 {
                opened_at = Some(brace.at);
            }
            continue;
        }
        if brace.depth != 0 || brace.at != last {
            continue;
        }
        let Some(start) = opened_at else { break };
        let form = trimmed[..start].trim();
        // §4.3.1: split only a map that FOLLOWS a nonempty form. A form that is
        // itself one map — the advanced `{:query …}` shape — stays whole.
        if form.is_empty() {
            break;
        }
        return (form.to_string(), trimmed[start..].trim().to_string());
    }
    (argument.trim().to_string(), String::new())
}

// ---------------------------------------------------------------------------
// C7 — the Rust raw-extent reader (§4.3.1, §7.9)
// ---------------------------------------------------------------------------

/// One query macro as it sits in the ORIGINAL raw source.
///
/// `argument` is the exact byte slice between the macro name and the closing
/// braces — never a rejoin of the document parser's comma-split arguments, and
/// never missing the options map's closing brace the way the AST's argument is
/// (§4.3.1, measured on installed mldoc 1.5.7 and on the pinned `lsdoc`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroExtent {
    /// Byte offset of the opening `{{`.
    pub start: usize,
    /// Byte offset just past the closing `}}`.
    pub end: usize,
    pub name: String,
    pub argument: String,
}

/// The first query macro in `raw`, or `None`.
///
/// Brace-, string- and page-ref-aware (`edn.ts::queryMacroExtent`): a `}}`
/// inside a string, a nested `{…}` options map, or a `[[page]]` ref does not end
/// it early — which is exactly what a lazy `/\{\{query.*?\}\}/` gets wrong.
pub fn query_macro_extent(raw: &str) -> Option<MacroExtent> {
    query_macro_extent_from(raw, 0)
}

/// Every query macro in `raw`, in source order. A block may hold several
/// (X2), and a rewrite must target the right one by extent.
pub fn query_macro_extents(raw: &str) -> Vec<MacroExtent> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while from < raw.len() {
        let Some(found) = query_macro_extent_from(raw, from) else {
            break;
        };
        from = found.end;
        out.push(found);
    }
    out
}

fn query_macro_extent_from(raw: &str, from: usize) -> Option<MacroExtent> {
    let mut search = from;
    while let Some(offset) = raw[search..].find("{{") {
        let start = search + offset;
        match macro_at(raw, start) {
            Some(extent) => return Some(extent),
            None => search = start + 2,
        }
    }
    None
}

/// Read one macro whose `{{` is at `start`, if its name is a query macro name.
///
/// **Widened from the TypeScript, recorded:** `edn.ts` matches `/\{\{query\b/i`,
/// which knows only one name and would also accept `{{query-foo}}` (`-` is a
/// word boundary in JavaScript). Here the name is read as a token and compared
/// against [`QUERY_MACRO_NAMES`] whole, longest first, so `{{tine-query …}}` is
/// recognised and `{{query-foo …}}` is not.
fn macro_at(raw: &str, start: usize) -> Option<MacroExtent> {
    let after_braces = start + 2;
    let rest = raw.get(after_braces..)?;
    let name = QUERY_MACRO_NAMES.iter().find(|candidate| {
        rest.len() >= candidate.len()
            && rest[..candidate.len()].eq_ignore_ascii_case(candidate)
            && matches!(
                rest.as_bytes().get(candidate.len()),
                None | Some(b' ') | Some(b'\t') | Some(b'}')
            )
    })?;
    let argument_start = after_braces + name.len();
    let family = FormFamily::for_macro_name(name);
    // Depth 2 is what the two opening braces already contributed, so form text
    // sits at depth 2 and a `{` of the options map takes it to 3.
    let braces = scan_braces(raw.get(argument_start..)?, family, 2);
    let close = braces
        .iter()
        .find(|brace| !brace.open && brace.depth == 0)?;
    let end = argument_start + close.at + 1;
    // Everything between the name and the LAST closing brace is the argument;
    // one leading space is the macro's separator, not part of it.
    let argument = &raw[argument_start..end - 2];
    Some(MacroExtent {
        start,
        end,
        name: name.to_string(),
        argument: argument.strip_prefix(' ').unwrap_or(argument).to_string(),
    })
}

// ---------------------------------------------------------------------------
// C5 — `macro_safe` (§4.3.1)
// ---------------------------------------------------------------------------

/// The exact §4.3.1 lexical rule, shared by all three macro dialects.
///
/// The complete argument is refused when it contains CR or LF, `#{`, or a `}`
/// anywhere except the final closing brace of its trailing options map — and
/// that brace must be the argument's last byte, so it immediately precedes the
/// outer `}}` with no inserted space. This keeps existing flat options maps
/// working while avoiding the document parser's early-close failure.
///
/// **An unmatched form `{` is not by itself a refusal** (measured: a lone `{`
/// inside a TQL literal parses as a macro in both Markdown and Org; a lone `}`
/// does not). This corrects the old three-hazard rule.
///
/// A refusal is a located diagnostic and nothing is written: the authored bytes
/// stay byte-identical (I-4). Nothing is ever silently stripped and no escape
/// syntax is invented.
pub fn macro_safe(argument: &str, family: FormFamily) -> Result<(), Diagnostic> {
    let refuse = |at: usize, message: &str| {
        Err(Diagnostic::new(DiagnosticKind::Syntax, message)
            .with_span(Some(Span::from_byte_range(argument, at, at + 1))))
    };
    if let Some(at) = argument.find(['\r', '\n']) {
        return refuse(
            at,
            "a query macro is one line: remove the line break before saving",
        );
    }
    if let Some(at) = argument.find("#{") {
        return refuse(at, "`#{` cannot appear inside a query macro");
    }
    let (_, options) = split_trailing_map(argument, family);
    // With options, the ONE legal `}` is the argument's last byte. Without
    // them, no `}` is legal at all.
    let allowed = (!options.is_empty())
        .then(|| argument.len().checked_sub(1))
        .flatten()
        .filter(|at| argument.as_bytes()[*at] == b'}');
    for (at, _) in argument.match_indices('}') {
        if Some(at) != allowed {
            return refuse(
                at,
                "`}` cannot appear inside a query macro except as the options map's final brace",
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// C6 — recognition, proved by the real parser (§4.3.1, R1)
// ---------------------------------------------------------------------------

/// Prove that the document parser reads this argument back as the intended
/// macro — the sufficiency check [`macro_safe`] cannot be.
///
/// Wrap the complete argument with its selected macro name, run the already
/// pinned `lsdoc` inline parser in **both** Markdown and Org modes, and require
/// the intended `Macro` node at the wrapper start; then require
/// [`query_macro_extent`] to recover the entire form and options from the same
/// bytes, with only the known trailing options brace outside the AST span.
///
/// This exists because the lexical rule is not enough: mldoc's macro grammar has
/// a leading-page-reference argument alternative and splits arguments on commas,
/// so `@block and content = 'a,[[b]]tail'` is NOT a macro even though every
/// lexical check passes, while `@block and content = 'a,b'` is (and its comma
/// must come back exactly, with no inserted space).
///
/// Reuse of the pinned parser is deliberate (D-14): a handwritten recognition
/// grammar would be a second answer to "is this a macro?".
pub fn recognizable_macro(name: &str, argument: &str) -> Result<(), Diagnostic> {
    let wrapped = format!("{{{{{name} {argument}}}}}");
    let refuse = |message: String| {
        Err(Diagnostic::new(DiagnosticKind::Syntax, message)
            .with_span(Some(Span::from_byte_range(argument, 0, argument.len()))))
    };
    let family = FormFamily::for_macro_name(name);
    let (_, options) = split_trailing_map(argument, family);
    // The options map's closing brace is consumed by the outer `}}`, so the AST
    // span stops one byte short of the wrapper. That is the ONLY byte allowed
    // outside it (measured on lsdoc and on mldoc 1.5.7).
    let expected_end = wrapped.len() - usize::from(!options.is_empty());

    for format in ["md", "org"] {
        let nodes = lsdoc::inline(&wrapped, format);
        let recognized = matches!(
            nodes.first(),
            Some(lsdoc::ast::Inline::Macro {
                name: parsed,
                span: Some(lsdoc::ast::Span(0, end)),
                ..
            }) if parsed.eq_ignore_ascii_case(name) && *end == expected_end
        );
        if !recognized {
            let reader = if format == "org" { "Org" } else { "Markdown" };
            return refuse(format!(
                "the {reader} reader does not read this back as one `{{{{{name}}}}}` macro"
            ));
        }
    }

    // The AST argument is lossy by construction; the raw reader is the transport,
    // so it must recover the complete form and options from the same bytes.
    match query_macro_extent(&wrapped) {
        Some(extent)
            if extent.start == 0
                && extent.end == wrapped.len()
                && extent.name.eq_ignore_ascii_case(name)
                && extent.argument == argument => {}
        _ => {
            return refuse(
                "the raw query reader cannot recover this macro's argument unchanged".to_string(),
            )
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- C1: the one splitter ---------------------------------------------

    #[test]
    fn split_takes_the_trailing_map_off_an_og_form() {
        assert_eq!(
            split_trailing_map("(task TODO) {:title \"T\"}", FormFamily::Edn),
            ("(task TODO)".to_string(), "{:title \"T\"}".to_string())
        );
    }

    #[test]
    fn split_takes_the_trailing_map_off_a_tql_form() {
        assert_eq!(
            split_trailing_map("@block and [[a]] {:title \"T\"}", FormFamily::Tql),
            ("@block and [[a]]".to_string(), "{:title \"T\"}".to_string())
        );
    }

    /// X4: a whole advanced `{:query …}` map is the FORM. Splitting it would
    /// leave an empty query and an "options map" that is the entire datalog.
    #[test]
    fn split_never_mistakes_a_whole_advanced_map_for_options() {
        let advanced = "{:query [:find (pull ?b [*]) :where [?b :block/marker \"TODO\"]]}";
        assert_eq!(
            split_trailing_map(advanced, FormFamily::Edn),
            (advanced.to_string(), String::new())
        );
    }

    #[test]
    fn split_takes_a_map_that_follows_a_whole_advanced_map() {
        let form = "{:query [:find ?b :where [?b :block/marker \"TODO\"]]}";
        let arg = format!("{form} {{:title \"T\"}}");
        assert_eq!(
            split_trailing_map(&arg, FormFamily::Edn),
            (form.to_string(), "{:title \"T\"}".to_string())
        );
    }

    /// §4.3.1's named fixtures: `{`, `}` and `;` inside a TQL literal.
    #[test]
    fn split_protects_braces_and_semicolons_inside_a_tql_literal() {
        for form in [
            "content = '{'",
            "content = '}'",
            "content = '; not a comment'",
        ] {
            let arg = format!("{form} {{:title \"T\"}}");
            assert_eq!(
                split_trailing_map(&arg, FormFamily::Tql),
                (form.to_string(), "{:title \"T\"}".to_string()),
                "TQL literal in {form} must not be scanned as EDN"
            );
        }
    }

    /// The same characters inside an OG double-quoted string.
    #[test]
    fn split_protects_braces_and_semicolons_inside_an_og_string() {
        for form in [
            "(property k \"{\")",
            "(property k \"}\")",
            "(property k \"; not a comment\")",
        ] {
            let arg = format!("{form} {{:title \"T\"}}");
            assert_eq!(
                split_trailing_map(&arg, FormFamily::Edn),
                (form.to_string(), "{:title \"T\"}".to_string()),
                "OG string in {form} must protect its delimiters"
            );
        }
    }

    #[test]
    fn split_ignores_an_unbalanced_brace_inside_a_literal_before_a_real_map() {
        assert_eq!(
            split_trailing_map("content = '}' {:title \"T\"}", FormFamily::Tql),
            ("content = '}'".to_string(), "{:title \"T\"}".to_string())
        );
        assert_eq!(
            split_trailing_map("(property k \"}\") {:title \"T\"}", FormFamily::Edn),
            (
                "(property k \"}\")".to_string(),
                "{:title \"T\"}".to_string()
            )
        );
    }

    #[test]
    fn a_form_ending_in_a_literal_that_ends_with_a_brace_has_no_map() {
        assert_eq!(
            split_trailing_map("content = 'a}'", FormFamily::Tql),
            ("content = 'a}'".to_string(), String::new())
        );
        assert_eq!(
            split_trailing_map("(property k \"a}\")", FormFamily::Edn),
            ("(property k \"a}\")".to_string(), String::new())
        );
    }

    /// An EDN symbol's apostrophe is not a SQL string: the `'` in `#'foo` must
    /// not swallow the rest of an OG/advanced form.
    #[test]
    fn an_edn_apostrophe_is_not_a_sql_string() {
        assert_eq!(
            split_trailing_map("(property k #'sym) {:title \"T\"}", FormFamily::Edn),
            (
                "(property k #'sym)".to_string(),
                "{:title \"T\"}".to_string()
            )
        );
    }

    /// A semicolon in TQL form text is data, not a comment, so it may not hide
    /// the options map from the split.
    #[test]
    fn a_semicolon_in_tql_form_text_is_not_a_comment() {
        assert_eq!(
            split_trailing_map("content = 'a;b' {:title \"T\"}", FormFamily::Tql),
            ("content = 'a;b'".to_string(), "{:title \"T\"}".to_string())
        );
    }

    /// Inside the map, semicolon comments and strings protect delimiters
    /// whatever the form family was — the map is EDN either way.
    #[test]
    fn the_options_map_is_edn_even_under_a_tql_form() {
        let arg = "@block {:title \"a}b\" :note \"x\"}";
        assert_eq!(
            split_trailing_map(arg, FormFamily::Tql),
            (
                "@block".to_string(),
                "{:title \"a}b\" :note \"x\"}".to_string()
            )
        );
    }

    #[test]
    fn a_page_ref_holding_braces_is_opaque_to_the_split() {
        assert_eq!(
            split_trailing_map("[[a{b}c]] {:title \"T\"}", FormFamily::Tql),
            ("[[a{b}c]]".to_string(), "{:title \"T\"}".to_string())
        );
    }

    // --- C7: the raw extent reader ----------------------------------------

    #[test]
    fn extent_covers_a_nested_options_map_not_the_first_double_brace() {
        let raw = "before {{query (task TODO) {:title \"T\"}}} after";
        let found = query_macro_extent(raw).expect("a macro");
        assert_eq!(
            &raw[found.start..found.end],
            "{{query (task TODO) {:title \"T\"}}}"
        );
        assert_eq!(found.name, "query");
        assert_eq!(found.argument, "(task TODO) {:title \"T\"}");
    }

    /// The reason this reader exists: the document parser's AST argument for
    /// this text is the two fragments `@block and content = 'a` / `b'`, and
    /// rejoining them inserts a separator the author never wrote.
    #[test]
    fn extent_recovers_a_comma_exactly_where_the_ast_argument_splits_it() {
        let raw = "{{tine-query @block and content = 'a,b'}}";
        let found = query_macro_extent(raw).expect("a macro");
        assert_eq!(found.argument, "@block and content = 'a,b'");
        assert_eq!(found.end, raw.len());
    }

    #[test]
    fn extent_is_not_ended_early_by_braces_inside_a_literal_or_a_page_ref() {
        for raw in [
            "{{tine-query content = '}}'}}",
            "{{query (property k \"}}\")}}",
            "{{tine-query [[a}}b]]}}",
        ] {
            let found = query_macro_extent(raw).unwrap_or_else(|| panic!("a macro in {raw}"));
            assert_eq!(found.end, raw.len(), "{raw} must consume its full extent");
        }
    }

    #[test]
    fn extent_reads_both_macro_names_and_no_lookalike() {
        assert_eq!(
            query_macro_extent("{{tine-query @block}}").map(|e| e.name),
            Some("tine-query".to_string())
        );
        assert_eq!(
            query_macro_extent("{{query (task TODO)}}").map(|e| e.name),
            Some("query".to_string())
        );
        assert_eq!(query_macro_extent("{{embed ((abc))}}"), None);
        assert_eq!(query_macro_extent("{{query-table x}}"), None);
    }

    #[test]
    fn extents_find_every_macro_in_source_order_without_overlapping() {
        let raw = "a {{query (task TODO)}} b {{tine-query @block and [[x]]}} c";
        let found = query_macro_extents(raw);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].argument, "(task TODO)");
        assert_eq!(found[1].argument, "@block and [[x]]");
        assert!(
            found[0].end <= found[1].start,
            "extents consumed exactly once"
        );
    }

    #[test]
    fn an_unterminated_macro_is_not_an_extent() {
        assert_eq!(query_macro_extent("{{tine-query @block and [[x]]"), None);
    }

    // --- C5: macro safety --------------------------------------------------

    #[test]
    fn macro_safe_accepts_a_flat_options_map_at_the_very_end() {
        assert!(macro_safe("@block and [[a]] {:title \"T\"}", FormFamily::Tql).is_ok());
        assert!(macro_safe("(task TODO) {:title \"T\"}", FormFamily::Edn).is_ok());
    }

    /// The correction R1 makes to the old three-hazard rule: a lone `{` is not
    /// a refusal, a lone `}` is.
    #[test]
    fn a_lone_open_brace_is_safe_and_a_lone_close_brace_is_not() {
        assert!(macro_safe("content = '{'", FormFamily::Tql).is_ok());
        assert!(macro_safe("content = '}'", FormFamily::Tql).is_err());
    }

    #[test]
    fn macro_safe_refuses_line_breaks_and_set_literals() {
        assert!(macro_safe("@block\nand [[a]]", FormFamily::Tql).is_err());
        assert!(macro_safe("@block\r\nand [[a]]", FormFamily::Tql).is_err());
        assert!(macro_safe("content = '#{a}'", FormFamily::Tql).is_err());
    }

    #[test]
    fn macro_safe_refuses_a_space_between_the_options_brace_and_the_wrapper() {
        assert!(macro_safe("@block {:title \"T\"} ", FormFamily::Tql).is_err());
    }

    #[test]
    fn macro_safe_refuses_a_nested_map_inside_the_options() {
        assert!(macro_safe("@block {:title {:a 1}}", FormFamily::Tql).is_err());
    }

    #[test]
    fn a_macro_safety_refusal_is_located() {
        let diagnostic = macro_safe("content = '}'", FormFamily::Tql).expect_err("refused");
        assert_eq!(diagnostic.kind, DiagnosticKind::Syntax);
        assert!(diagnostic.span.is_some(), "a refusal points at the byte");
    }

    // --- C6: recognition ---------------------------------------------------

    #[test]
    fn recognition_accepts_the_canonical_macro_forms() {
        for (name, argument) in [
            ("tine-query", "@block and off([[a]]) and off([[b]])"),
            ("tine-query", "@block and not off([[a]])"),
            ("tine-query", "@block and [[a]] {:title \"T\"}"),
            ("query", "(task TODO) {:title \"T\"}"),
        ] {
            assert!(
                recognizable_macro(name, argument).is_ok(),
                "{name} / {argument} must be recognized"
            );
        }
    }

    /// §4.3.1's two pinned examples. Both pass every lexical check; only the
    /// real parser separates them.
    #[test]
    fn recognition_rejects_a_comma_that_exposes_a_reference_and_keeps_the_plain_one() {
        assert!(macro_safe("@block and content = 'a,[[b]]tail'", FormFamily::Tql).is_ok());
        assert!(
            recognizable_macro("tine-query", "@block and content = 'a,[[b]]tail'").is_err(),
            "a later-argument page reference is not a macro"
        );
        assert!(recognizable_macro("tine-query", "@block and content = 'a,b'").is_ok());
    }

    /// X1's consequence: a leading page reference takes the document parser's
    /// other argument alternative, which is why canonical macro output always
    /// starts with an explicit anchor.
    #[test]
    fn recognition_rejects_a_leading_page_reference_and_accepts_the_anchored_form() {
        assert!(recognizable_macro("tine-query", "[[b]] and task = 'TODO'").is_err());
        assert!(recognizable_macro("tine-query", "@block and [[b]] and task = 'TODO'").is_ok());
    }

    #[test]
    fn recognition_rejects_what_the_document_parser_turns_into_plain_text() {
        assert!(recognizable_macro("tine-query", "content = 'x}'").is_err());
        assert!(recognizable_macro("tine-query", "@block\nand [[a]]").is_err());
    }
}
