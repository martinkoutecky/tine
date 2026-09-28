//! TQL SPEC §4.2.1 pre-pass: rewrites the three non-SQL surface forms
//! (`@block`/`@page` anchors, `[[x]]`/`#x` page names, `-- ` disabled runs)
//! into SQLite expression text before [`sqlparser`] sees it.

use super::*;

// ---------------------------------------------------------------------------
// 4.2.1 Pre-pass
// ---------------------------------------------------------------------------

pub(super) struct PrePass {
    pub(super) sql: String,
    pub(super) anchor: Anchor,
    /// The anchor token was the whole query: every row of the anchor.
    pub(super) empty: bool,
    /// Where `sql` starts inside the ORIGINAL text — `Some` only when the
    /// pre-pass rewrote NOTHING, so a byte offset into `sql` is also a byte
    /// offset into text the author actually typed.
    ///
    /// §4.3.2 makes spans presentation metadata, and §7.4's retained
    /// wrong-anchor leaf carries one. A span into desugared or run-lifted text
    /// would point at characters the user never wrote, so when the pre-pass
    /// rewrote anything this is `None` and the retained leaf is simply
    /// unspanned. Nothing downstream depends on the span: the frontend finds
    /// the retained leaves by walking the tree.
    pub(super) offset: Option<usize>,
}

/// Byte ranges of every `'…'` string literal, quotes included, with SQL's `''`
/// doubling honoured.
///
/// **M18's invariant, not its letter.** The spec describes one lexical scan
/// whose literal map every later step consults; the steps below re-derive the
/// map after each rewrite instead, because a rewrite moves the offsets. What
/// M18 buys — nothing inside a literal is ever recognised or rewritten,
/// including a line beginning `-- ` inside a multi-line literal — is exactly
/// what re-deriving preserves.
pub(super) fn literal_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\'' {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < bytes.len() {
            if bytes[i] == b'\'' {
                if bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                i += 1;
                break;
            }
            i += 1;
        }
        spans.push((start, i));
    }
    spans
}

pub(super) fn inside_literal(spans: &[(usize, usize)], at: usize) -> bool {
    spans.iter().any(|(start, end)| at >= *start && at < *end)
}

pub(super) fn pre_pass(text: &str, diagnostics: &mut Vec<Diagnostic>) -> PrePass {
    let (anchor, rest, offset) = take_anchor(text);
    report_stray_anchor(text, &rest, offset, diagnostics);
    report_unquoted_relative_dates(text, &rest, offset, diagnostics);
    if rest.trim().is_empty() {
        return PrePass {
            sql: "true".to_string(),
            anchor,
            empty: true,
            offset: None,
        };
    }
    // **Disabled runs are isolated FIRST (§4.2.1, §4.3.2 R4).** A run's payload
    // may be malformed — an unmatched quote or bracket — and scanning it as
    // part of the surrounding text lets it swallow the next ACTIVE row, which
    // is exactly the defect `-- task = '` followed by a valid row exposes.
    // Each run's payload is desugared in isolation with this same scanner.
    let lifted = lift_disabled_runs(&rest, diagnostics);
    let sql = desugar(&lifted, diagnostics);
    let unchanged = sql == rest;
    PrePass {
        sql,
        anchor,
        empty: false,
        offset: unchanged.then_some(offset),
    }
}

/// Step 1: a leading `@block` / `@page`, plus one following `and`.
pub(super) fn take_anchor(text: &str) -> (Anchor, String, usize) {
    let lead = text.len() - text.trim_start().len();
    let body = &text[lead..];
    let lower = body.to_ascii_lowercase();
    for (token, anchor) in [("@block", Anchor::Block), ("@page", Anchor::Page)] {
        if !lower.starts_with(token) {
            continue;
        }
        let after = &body[token.len()..];
        if after
            .chars()
            .next()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            continue;
        }
        let trimmed = after.trim_start();
        let skipped = after.len() - trimmed.len();
        let mut consumed = lead + token.len() + skipped;
        let rest = if trimmed.len() >= 3
            && trimmed[..3].eq_ignore_ascii_case("and")
            && !trimmed[3..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            consumed += 3;
            &trimmed[3..]
        } else {
            trimmed
        };
        return (anchor, rest.to_string(), consumed);
    }
    (Anchor::Block, text.to_string(), 0)
}

/// `@` anywhere but the front is the author reaching for an anchor in the wrong
/// place — never silently ignored.
pub(super) fn report_stray_anchor(
    original: &str,
    rest: &str,
    offset: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let spans = literal_spans(rest);
    if let Some(at) = rest
        .char_indices()
        .find(|(index, ch)| *ch == '@' && !inside_literal(&spans, *index))
        .map(|(index, _)| index)
    {
        diagnostics.push(
            Diagnostic::new(DiagnosticKind::Syntax, "the anchor goes first").with_span(Some(
                Span::from_byte_range(original, offset + at, offset + at + 1),
            )),
        );
    }
}

/// Where the next `[[x]]` / `#x` sits: a value position spells the page NAME as
/// a string literal (OG `parse-property-value`, M19), anywhere else it is a
/// `refs` leaf.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Prev {
    Start,
    Cmp,
    Boolean,
    Open,
    Comma,
    Other,
}

/// Step 2: sugar. `[[x]]` → `ref('x')`, `#x` → `ref('x')`, except in value
/// position where both become the string literal `'x'`.
pub(super) fn desugar(text: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut prev = Prev::Start;
    // One entry per open paren: whether it is the list of an `in`, and
    // whether the pre-pass inserted an outer paren around a quantifier call.
    let mut parens: Vec<(bool, bool)> = Vec::new();
    let mut pending_in = false;
    let mut pending_quantifier = false;
    let mut i = 0usize;
    while i < bytes.len() {
        let value_position = prev == Prev::Cmp
            || (matches!(prev, Prev::Open | Prev::Comma)
                && parens.last().is_some_and(|(in_list, _)| *in_list));
        match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                out.push(bytes[i] as char);
                i += 1;
            }
            b'\'' => {
                let end = literal_end(text, i);
                out.push_str(&text[i..end]);
                prev = Prev::Other;
                i = end;
            }
            b'"' => {
                let end = double_quoted_end(text, i);
                out.push_str(&text[i..end]);
                prev = Prev::Other;
                i = end;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                let end = text[i..].find('\n').map_or(bytes.len(), |at| i + at);
                out.push_str(&text[i..end]);
                i = end;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |at| i + 2 + at + 2);
                out.push_str(&text[i..end]);
                i = end;
            }
            b'[' if text[i..].starts_with("[[") => {
                match text[i + 2..].find("]]") {
                    Some(offset) => {
                        let inner = &text[i + 2..i + 2 + offset];
                        emit_page_name(&mut out, inner, value_position);
                        i += 2 + offset + 2;
                    }
                    None => {
                        diagnostics.push(Diagnostic::new(
                            DiagnosticKind::Syntax,
                            "a page reference opened with `[[` is never closed",
                        ));
                        out.push_str(&text[i..]);
                        i = bytes.len();
                    }
                }
                prev = Prev::Other;
            }
            b'#' => {
                let mut end = i + 1;
                while end < bytes.len() {
                    let ch = text[end..].chars().next().expect("boundary");
                    if ch.is_whitespace() || matches!(ch, '(' | ')' | ',') {
                        break;
                    }
                    end += ch.len_utf8();
                }
                if end == i + 1 {
                    out.push('#');
                } else {
                    emit_page_name(&mut out, &text[i + 1..end], value_position);
                }
                prev = Prev::Other;
                i = end;
            }
            b'(' => {
                parens.push((pending_in, pending_quantifier));
                pending_in = false;
                pending_quantifier = false;
                out.push('(');
                prev = Prev::Open;
                i += 1;
            }
            b')' => {
                let wrapped_quantifier = parens.pop().is_some_and(|(_, wrapped)| wrapped);
                out.push(')');
                if wrapped_quantifier {
                    out.push(')');
                }
                prev = Prev::Other;
                i += 1;
            }
            b',' => {
                out.push(',');
                prev = Prev::Comma;
                i += 1;
            }
            b'=' | b'<' | b'>' | b'!' => {
                let start = i;
                while i < bytes.len() && matches!(bytes[i], b'=' | b'<' | b'>' | b'!') {
                    i += 1;
                }
                out.push_str(&text[start..i]);
                prev = Prev::Cmp;
            }
            byte if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                let word = &text[start..i];
                let next = next_code_byte(text, i);
                pending_quantifier = prev == Prev::Boolean
                    && word.eq_ignore_ascii_case("any")
                    && next.is_some_and(|at| bytes.get(at) == Some(&b'('));
                // sqlparser treats `ANY` after a boolean operator as SQL's
                // reserved quantified-comparison operator. An extra expression
                // boundary makes the same token unambiguously the TQL function;
                // the matching close is emitted by the paren frame above.
                if pending_quantifier {
                    out.push('(');
                }
                out.push_str(word);
                pending_in = word.eq_ignore_ascii_case("in");
                // The word-spelled comparison operators put the next token in
                // value position exactly as `=` does.
                prev = if ["like", "match", "between"]
                    .iter()
                    .any(|op| word.eq_ignore_ascii_case(op))
                {
                    Prev::Cmp
                } else if word.eq_ignore_ascii_case("and") || word.eq_ignore_ascii_case("or") {
                    Prev::Boolean
                } else {
                    Prev::Other
                };
            }
            _ => {
                let ch = text[i..].chars().next().expect("boundary");
                out.push(ch);
                prev = Prev::Other;
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Next non-comment token byte. Quantifier calls remain calls when SQL comments
/// separate their name and argument list, and comment payload is never scanned.
pub(super) fn next_code_byte(text: &str, mut at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    loop {
        while bytes.get(at).is_some_and(|byte| byte.is_ascii_whitespace()) {
            at += 1;
        }
        if bytes.get(at) == Some(&b'-') && bytes.get(at + 1) == Some(&b'-') {
            at = text[at..]
                .find('\n')
                .map_or(bytes.len(), |end| at + end + 1);
            continue;
        }
        if bytes.get(at) == Some(&b'/') && bytes.get(at + 1) == Some(&b'*') {
            at = text[at + 2..]
                .find("*/")
                .map_or(bytes.len(), |end| at + 2 + end + 2);
            continue;
        }
        return (at < bytes.len()).then_some(at);
    }
}

/// End offset for a double-quoted SQL token, including doubled quotes. The
/// pre-pass must not recognize a quantifier name inside quoted text.
pub(super) fn double_quoted_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

pub(super) fn emit_page_name(out: &mut String, inner: &str, value_position: bool) {
    let quoted = format!("'{}'", inner.replace('\'', "''"));
    if value_position {
        out.push_str(&quoted);
    } else {
        out.push_str("ref(");
        out.push_str(&quoted);
        out.push(')');
    }
}

/// The end offset (exclusive) of the `'…'` literal starting at `start`.
pub(super) fn literal_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if bytes.get(i + 1) == Some(&b'\'') {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// `today` is a vocabulary identifier; every other relative date is quoted. An
/// unquoted `-7d` would parse as arithmetic on an unknown identifier, so it is
/// caught here where the suggestion can name the fix.
pub(super) fn report_unquoted_relative_dates(
    original: &str,
    text: &str,
    offset: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let spans = literal_spans(text);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if !matches!(bytes[i], b'-' | b'+') || inside_literal(&spans, i) {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == i + 1 || !bytes.get(end).is_some_and(|b| b"dwmy".contains(b)) {
            i += 1;
            continue;
        }
        let unit = end;
        end += 1;
        if bytes
            .get(end)
            .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
        {
            i += 1;
            continue;
        }
        let literal = &text[i..=unit];
        diagnostics.push(
            Diagnostic {
                suggestions: vec![format!("quote relative dates: '{literal}'")],
                ..Diagnostic::new(
                    DiagnosticKind::Syntax,
                    format!("`{literal}` is a relative date and must be quoted"),
                )
            }
            .with_span(Some(Span::from_byte_range(
                original,
                offset + i,
                offset + end,
            ))),
        );
        i = end;
    }
}

/// One `-- ` run and where it sits in the text.
pub(super) struct DisabledRun {
    first_line: usize,
    last_line: usize,
    indent: String,
    payload: String,
}

/// Step 3 (Q12): a maximal run of `-- ` lines becomes a positional
/// `<connector> off(<rest>)`. Positional replacement is what makes nesting free:
/// a run inside a parenthesized group becomes an `off()` operand of that group.
pub(super) fn lift_disabled_runs(text: &str, diagnostics: &mut Vec<Diagnostic>) -> String {
    let spans = literal_spans(text);
    let mut lines: Vec<String> = Vec::new();
    let mut kinds: Vec<Option<(String, String)>> = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\n') {
        let indent_len = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        let starts_in_literal = inside_literal(&spans, offset + indent_len);
        let payload = if starts_in_literal || trimmed == "--" || !trimmed.starts_with("-- ") {
            None
        } else {
            let rest = trimmed[3..].trim();
            (!rest.is_empty()).then(|| (line[..indent_len].to_string(), rest.to_string()))
        };
        kinds.push(payload);
        lines.push(line.to_string());
        offset += line.len() + 1;
    }

    let mut runs: Vec<DisabledRun> = Vec::new();
    let mut index = 0usize;
    while index < kinds.len() {
        let Some((indent, payload)) = kinds[index].clone() else {
            index += 1;
            continue;
        };
        let first_line = index;
        let mut joined = payload;
        index += 1;
        while let Some(Some((_, next))) = kinds.get(index) {
            joined.push(' ');
            joined.push_str(next);
            index += 1;
        }
        runs.push(DisabledRun {
            first_line,
            last_line: index - 1,
            indent,
            payload: joined,
        });
    }

    for run in &runs {
        let (connector, rest) = split_connector(&run.payload);
        // The isolated operand gets the same sugar treatment as active text,
        // from the same scanner (§4.2.1) — a disabled `[[a]]` is still a ref.
        // Its diagnostics are disabled: a broken row a user turned off must not
        // invalidate the query (§3.5).
        let mut inner_diagnostics = Vec::new();
        let sugared = desugar(rest, &mut inner_diagnostics);
        let parses = parse_expr_guarded(&sugared).is_ok();
        if parses {
            for diagnostic in inner_diagnostics {
                diagnostics.push(Diagnostic {
                    disabled: true,
                    ..diagnostic
                });
            }
        }
        let replacement = if parses {
            format!("{}{connector}off({sugared})", run.indent)
        } else {
            // **Captured before the surrounding parse, never substituted with
            // `off(false)` (§4.2.1, R4).** `off(false)` is a lie the author
            // never wrote and it destroys the payload; the capsule keeps the
            // exact bytes, so a save and reopen returns the same broken row and
            // the renderer can show what the author typed. The disabled
            // diagnostic that goes with it is derived at lowering, from the
            // `off(…)` this replacement puts around it.
            format!(
                "{}{connector}off(raw_hex('{}', '{}'))",
                run.indent,
                DiagnosticKind::Syntax.capsule_name(),
                encode_raw_hex(rest)
            )
        };
        lines[run.first_line] = replacement;
        for line in run.first_line + 1..=run.last_line {
            lines[line] = String::new();
        }
    }
    lines.join("\n")
}

pub(super) fn split_connector(payload: &str) -> (&'static str, &str) {
    for (word, connector) in [("and", "and "), ("or", "or ")] {
        if payload.len() > word.len()
            && payload[..word.len()].eq_ignore_ascii_case(word)
            && payload.as_bytes()[word.len()].is_ascii_whitespace()
        {
            return (connector, payload[word.len()..].trim_start());
        }
    }
    ("", payload)
}
