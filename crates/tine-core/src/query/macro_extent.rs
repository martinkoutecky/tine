//! Native and wasm raw query macro extents, compiled from this one source.
//! O(raw bytes) time and space, no query IR, render context or I/O. Offsets are
//! UTF-8 bytes; the frontend converts them to UTF-16 at its boundary. Malformed
//! or unterminated candidates are omitted; arguments remain authored bytes.
use super::macro_names::QUERY_MACRO_NAMES;

/// Whether `name` is one of [`QUERY_MACRO_NAMES`], case-insensitively and as a
/// WHOLE name (§7.9).
///
/// The ONE recognizer for "is this macro a query", so a neighbour cannot answer
/// it with its own `name == "query"` and thereby publish `{{tine-query …}}` as
/// literal text (Y1). Callers hold a name the document parser already tokenized;
/// callers holding raw bytes want [`query_macro_extent`] instead.
pub fn is_query_macro_name(name: &str) -> bool {
    QUERY_MACRO_NAMES
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

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
pub(super) struct Brace {
    /// Byte offset of the brace.
    pub(super) at: usize,
    /// `true` for `{`, `false` for `}`.
    pub(super) open: bool,
    /// Nesting depth AFTER this brace, counting from `form_depth`.
    pub(super) depth: i32,
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
pub(super) fn scan_braces(text: &str, family: FormFamily, form_depth: i32) -> Vec<Brace> {
    scan_work(text.len());
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut depth = form_depth;
    let mut i = 0usize;
    while i < bytes.len() {
        // Inside a map the text is EDN whatever the form was; an EDN symbol's
        // apostrophe (`'foo`, `#'x`) is never a SQL string, and a semicolon in
        // TQL form text is never a comment.
        let edn = depth > form_depth || family == FormFamily::Edn;
        if let Some(end) = protected_end(text, i, edn) {
            i = end;
            continue;
        }
        match bytes[i] {
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

#[cfg(test)]
thread_local! {
    pub(super) static SCAN_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[inline(always)]
fn scan_work(_bytes: usize) {
    #[cfg(test)]
    SCAN_WORK.with(|w| w.set(w.get() + _bytes));
}

// Literal/comment/reference protection shared by splitting and extent reading.
fn protected_end(text: &str, at: usize, edn: bool) -> Option<usize> {
    match text.as_bytes()[at] {
        b'"' if edn => Some(edn_string_end(text, at)),
        b'\'' if !edn => Some(sql_quoted_end(text, at, b'\'')),
        b';' if edn => Some(text[at..].find('\n').map_or(text.len(), |end| at + end)),
        b'[' if text.as_bytes().get(at + 1) == Some(&b'[') => Some(page_ref_end(text, at)),
        _ => None,
    }
}

/// Index just past an EDN double-quoted string opening at `at`; end of input if
/// unterminated. Only `\` escapes the next byte (`edn.ts::strClose`).
fn edn_string_end(text: &str, at: usize) -> usize {
    crate::query_edn::string_end(text, at).unwrap_or(text.len())
}

/// Index just past an SQL quoted token (`'…'` literal or `"…"` identifier)
/// opening at `at`; end of input if unterminated. SQL doubles the quote rather
/// than backslash-escaping it.
pub(super) fn sql_quoted_end(text: &str, at: usize, quote: u8) -> usize {
    let bytes = text.as_bytes();
    let mut j = at + 1;
    while j < bytes.len() {
        if bytes[j] == quote {
            if bytes.get(j + 1) == Some(&quote) {
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
pub(super) fn page_ref_end(text: &str, at: usize) -> usize {
    match text[at + 2..].find("]]") {
        Some(offset) => at + 2 + offset + 2,
        None => text.len(),
    }
}

/// One query macro as it sits in the ORIGINAL raw source.
///
/// `argument` is the exact byte slice between the macro name and the closing
/// braces — never a rejoin of the document parser's comma-split arguments, and
/// never missing the options map's closing brace the way the AST's argument is
/// (§4.3.1, measured on installed mldoc 1.5.7 and on the pinned `lsdoc`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MacroExtent {
    /// Byte offset of the opening `{{`.
    pub start: usize,
    /// Byte offset just past the closing `}}`.
    pub end: usize,
    /// Canonical query macro name.
    pub name: String,
    /// Verbatim argument, with the wrapper's single leading space removed.
    pub argument: String,
}

/// The first query macro in `raw`, or `None`.
///
/// Brace-, string- and page-ref-aware (`edn.ts::queryMacroExtent`): a `}}`
/// inside a string, a nested `{…}` options map, or a `[[page]]` ref does not end
/// it early — which is exactly what a lazy `/\{\{query.*?\}\}/` gets wrong.
///
/// Cost: O(raw bytes), including unterminated candidates. Candidate ranges are
/// tracked during one forward lexical scan, and copied only when selected.
pub fn query_macro_extent(raw: &str) -> Option<MacroExtent> {
    macro_extents(raw, 1).into_iter().next()
}

/// Every query macro in source order, with no overlapping extents.
/// O(raw bytes) time and space; literals and options use the shared lexical rules.
pub fn query_macro_extents(raw: &str) -> Vec<MacroExtent> {
    macro_extents(raw, usize::MAX)
}

fn macro_name_at(raw: &str, start: usize) -> Option<&'static str> {
    let rest = raw.get(start + 2..)?;
    QUERY_MACRO_NAMES
        .iter()
        .copied()
        .filter(|candidate| {
            rest.get(..candidate.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(candidate))
                && matches!(
                    rest.as_bytes().get(candidate.len()),
                    None | Some(b' ' | b'\t' | b'}')
                )
        })
        .max_by_key(|candidate| candidate.len())
}

fn macro_extents(raw: &str, limit: usize) -> Vec<MacroExtent> {
    struct Candidate {
        start: usize,
        argument: usize,
        name: &'static str,
        base: usize,
        end: Option<usize>,
    }
    scan_work(raw.len());
    let bytes = raw.as_bytes();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let (mut i, mut depth) = (0usize, 0usize);
    while i < raw.len() {
        if active.is_empty() {
            let Some(offset) = raw[i..].find("{{") else {
                break;
            };
            i += offset;
            let Some(name) = macro_name_at(raw, i) else {
                i += 2;
                continue;
            };
            candidates.push(Candidate {
                start: i,
                argument: i + 2 + name.len(),
                name,
                base: 0,
                end: None,
            });
            active.push(candidates.len() - 1);
            depth = 2;
            i += 2 + name.len();
            continue;
        }
        let candidate = &candidates[*active.last().unwrap()];
        let edn = depth > candidate.base + 2
            || FormFamily::for_macro_name(candidate.name) == FormFamily::Edn;
        if let Some(end) = protected_end(raw, i, edn) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'{' => {
                if bytes.get(i + 1) == Some(&b'{') {
                    if let Some(name) = macro_name_at(raw, i) {
                        candidates.push(Candidate {
                            start: i,
                            argument: i + 2 + name.len(),
                            name,
                            base: depth,
                            end: None,
                        });
                        active.push(candidates.len() - 1);
                        depth += 2;
                        i += 2 + name.len();
                        continue;
                    }
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                let index = *active.last().unwrap();
                if depth == candidates[index].base {
                    // An extent ends only with adjacent closing braces.
                    if i > 0 && bytes[i - 1] == b'}' {
                        candidates[index].end = Some(i + 1);
                    }
                    active.pop();
                }
            }
            _ => {}
        }
        i += 1;
    }
    let mut out = Vec::new();
    let mut covered = 0usize;
    for candidate in candidates {
        let Some(end) = candidate.end else { continue };
        if candidate.start < covered {
            continue;
        }
        let argument = &raw[candidate.argument..end - 2];
        out.push(MacroExtent {
            start: candidate.start,
            end,
            name: candidate.name.to_string(),
            argument: argument.strip_prefix(' ').unwrap_or(argument).to_string(),
        });
        covered = end;
        if out.len() == limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod sql_quoted_end_tests {
    use super::sql_quoted_end;

    #[test]
    fn one_scanner_closes_both_sql_quote_kinds_and_doubles_through_escapes() {
        for quote in [b'\'', b'"'] {
            let q = quote as char;
            let closed = format!("{q}a{q}{q}b{q} tail");
            assert_eq!(
                sql_quoted_end(&closed, 0, quote),
                6,
                "doubled quote stays inside"
            );
            let plain = format!("x {q}ab{q}c");
            assert_eq!(sql_quoted_end(&plain, 2, quote), 6);
            let open = format!("{q}never closed");
            assert_eq!(
                sql_quoted_end(&open, 0, quote),
                open.len(),
                "unterminated runs to the end"
            );
            let trailing_double = format!("{q}a{q}{q}");
            assert_eq!(
                sql_quoted_end(&trailing_double, 0, quote),
                trailing_double.len()
            );
        }
        // The other quote kind is ordinary text inside a literal.
        assert_eq!(sql_quoted_end("'a\"b'", 0, b'\''), 5);
    }
}
