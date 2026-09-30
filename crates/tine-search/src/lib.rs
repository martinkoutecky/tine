//! Shared parser for the Ctrl-K quick-search query dialect (GH #44).
//!
//! One native/browser answerer: the frontend calls this crate through lsdoc-wasm.
//! Fold/map cost O(text bytes); parse cost O(query bytes plus bounded regex compilation).
//! Matching costs O(text bytes × query terms); mapped evidence costs O(text scalars ×
//! needle scalars), bounded by a caller-supplied result limit. No graph access or I/O.
//! Invalid or oversized regexes return an error and match nothing; callers surface the
//! error. Regexes use original text, while boolean callers supply the selected fold.
//! Callers need no Unicode tables or engine-specific syntax checks.
//!
//! Grammar (the mainstream full-text convention — see the cited scan in
//! `subagent-tasks/notes/search-syntax-industry-scan.md`):
//!   - whitespace between terms = order-independent **AND**
//!   - `OR` (uppercase keyword, its own token) = **OR** between groups
//!   - `-term` / `-"phrase"` = **exclude** (negation)
//!   - `"phrase"` = exact contiguous substring
//!   - `/regex/` (the WHOLE query, slash-delimited) = regex, matched
//!     case-sensitively against original-case text (so `[A-Z]` works)
//!   - everything else is a case-insensitive substring term
//!
//! A **single bare positive term** parses to `is_simple() == true`, which the
//! quick switcher uses to keep today's fuzzy page-name ranking; any second
//! term / operator / regex switches both pages and blocks to this grammar.

use std::ops::Range;
use std::sync::OnceLock;
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

fn is_search_whitespace(ch: char) -> bool {
    matches!(ch, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// Maximum regex program and lazy DFA cache bytes, for every search/TQL engine.
pub const REGEX_PROGRAM_MAX_BYTES: usize = 1 << 20;

/// Compile the shared Rust Unicode regex dialect, including inline flags. Invalid
/// syntax and programs above 1 MiB fail explicitly; matching stays linear in text.
pub fn compile_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    regex::RegexBuilder::new(pattern)
        .size_limit(REGEX_PROGRAM_MAX_BYTES)
        .dfa_size_limit(REGEX_PROGRAM_MAX_BYTES)
        .build()
}

fn is_nonspacing_mark(ch: char) -> bool {
    if ch.is_ascii() {
        return false;
    }
    static MN: OnceLock<regex::Regex> = OnceLock::new();
    MN.get_or_init(|| regex::Regex::new(r"\A\p{Mn}\z").unwrap())
        .is_match(ch.encode_utf8(&mut [0; 4]))
}

fn is_ignorable_mark(ch: char) -> bool {
    matches!(ch, '\u{034f}' | '\u{17b4}'..='\u{17b5}' | '\u{180b}'..='\u{180d}'
        | '\u{180f}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
}

fn is_cyrillic(ch: char) -> bool {
    matches!(ch, '\u{0400}'..='\u{052f}' | '\u{1c80}'..='\u{1c8f}'
        | '\u{2de0}'..='\u{2dff}' | '\u{a640}'..='\u{a69f}')
}

fn unstroke(ch: char) -> char {
    match ch {
        'ł' => 'l',
        'ø' => 'o',
        'đ' => 'd',
        'ħ' => 'h',
        'ŧ' => 't',
        other => other,
    }
}

fn fold_lowered(lowered: &str) -> String {
    let mut out = String::with_capacity(lowered.len());
    let mut base = None;
    for ch in lowered.nfkd().map(unstroke) {
        let mn = is_nonspacing_mark(ch);
        let class = canonical_combining_class(ch);
        let retained = !mn
            || (!is_ignorable_mark(ch)
                && (class == 0
                    || matches!(class, 8 | 9 | 84 | 91 | 103 | 118 | 129 | 130 | 132)
                    || base.is_some_and(|b| is_cyrillic(b) && !(b == 'е' && ch == '\u{0308}'))));
        if retained {
            out.push(ch);
        }
        if class == 0 && !mn {
            base = Some(ch);
        }
    }
    out.nfc().collect()
}

/// The one comparison form for non-regex search. It matches master's accent,
/// compatibility and letter-making-mark policy without changing stored text.
pub fn canonical_fold(value: &str) -> String {
    if value.is_ascii() {
        return value.to_ascii_lowercase();
    }
    fold_lowered(&value.to_lowercase())
}

/// OG's explicit `:feature/enable-search-remove-accents? false` behavior:
/// compatibility forms still fold, but accents remain significant.
pub fn literal_fold(value: &str) -> String {
    if value.is_ascii() {
        return value.to_ascii_lowercase();
    }
    value.to_lowercase().nfkc().collect()
}

/// Lowercase plus NFC page identity, without compatibility or accent folding.
pub fn identity_fold(value: &str) -> String {
    value.to_lowercase().nfc().collect()
}

/// Fold with original UTF-16 spans for search evidence. Each output scalar
/// points to its whole source grapheme, including discarded combining marks.
pub fn canonical_fold_with_map(value: &str) -> (String, Vec<Range<usize>>) {
    fold_with_map(value, true)
}

/// Mapped comparison form when the graph disables accent removal.
pub fn literal_fold_with_map(value: &str) -> (String, Vec<Range<usize>>) {
    fold_with_map(value, false)
}

fn fold_with_map(value: &str, remove_accents: bool) -> (String, Vec<Range<usize>>) {
    let lowered = value.to_lowercase();
    let mut sources = Vec::new();
    let mut at = 0;
    for ch in value.chars() {
        let start = at;
        at += ch.len_utf16();
        for _ in ch.to_lowercase() {
            sources.push(start..at);
        }
    }
    let mut output = String::new();
    let mut spans = Vec::new();
    let mut source_at = 0;
    for grapheme in lowered.graphemes(true) {
        let count = grapheme.chars().count();
        let contributors = &sources[source_at..source_at + count];
        source_at += count;
        let span =
            contributors.first().map_or(0, |s| s.start)..contributors.last().map_or(0, |s| s.end);
        let folded = if remove_accents {
            fold_lowered(grapheme)
        } else {
            grapheme.nfkc().collect()
        };
        for ch in folded.chars() {
            output.push(ch);
            spans.push(span.clone());
        }
    }
    // Compatibility decomposition can make adjacent raw graphemes compose:
    // `ㄱㅏ` becomes the Hangul L+V pair and then one syllable. Compose the
    // complete output and union the contributing original spans.
    let mut composed = String::new();
    let mut composed_spans = Vec::new();
    let mut at = 0;
    for grapheme in output.graphemes(true) {
        let count = grapheme.chars().count();
        let contributors = &spans[at..at + count];
        at += count;
        let span =
            contributors.first().map_or(0, |s| s.start)..contributors.last().map_or(0, |s| s.end);
        for ch in grapheme.nfc() {
            composed.push(ch);
            composed_spans.push(span.clone());
        }
    }
    let output = composed;
    let spans = composed_spans;
    debug_assert_eq!(
        output,
        if remove_accents {
            canonical_fold(value)
        } else {
            literal_fold(value)
        }
    );
    (output, spans)
}

/// One AND-term: a substring folded according to the matcher policy, plus whether it is
/// negated (`-term` → must NOT be present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Needle folded with the matcher's policy. Compare only with a body folded
    /// the same way: `canonical_fold` by default, `literal_fold` when accents
    /// remain significant. `projection().visible_lower` is canonical only.
    pub text: String,
    pub negated: bool,
    /// The term came from a `"quoted phrase"` — an explicit opt-in to the
    /// grammar, so even a single quoted word is not treated as `is_simple`.
    pub quoted: bool,
}

/// A conjunction of terms (all must be satisfied). Groups are OR-ed together.
pub type AndGroup = Vec<Term>;

/// Mirrored behavioral examples displayed by Ctrl+K. Tests execute every row so
/// visible help cannot describe syntax the Rust matcher does not implement.
pub const SEARCH_SYNTAX_EXAMPLES: &[(&str, &str, &str)] = &[
    ("foo bar", "bar then foo", "foo only"),
    ("foo OR bar", "bar only", "neither"),
    ("foo -draft", "foo ready", "foo draft"),
    (
        "\"exact phrase\"",
        "an exact phrase here",
        "exact other phrase",
    ),
    ("/[A-Z]{3}/", "ABC", "abc"),
];

/// A parsed query, ready to test blocks/page-names against.
#[derive(Debug, Clone)]
pub enum Matcher {
    /// Whole query was `/pattern/` and compiled.
    Regex(regex::Regex),
    /// Whole query was `/pattern/` but the pattern failed to compile; carries the
    /// error message for the frontend to surface. Matches nothing.
    InvalidRegex(String),
    /// OR of AND-groups. Every retained group has ≥1 positive term.
    Boolean(Vec<AndGroup>),
    /// No effective query (blank, or only exclusions). Matches nothing.
    Empty,
}

impl Matcher {
    /// Parse a raw query string into a matcher.
    pub fn parse(query: &str) -> Matcher {
        Self::parse_with_policy(query, true)
    }

    /// Parse using the graph's OG accent-removal setting.
    pub fn parse_with_policy(query: &str, remove_accents: bool) -> Matcher {
        let q = query.trim_matches(is_search_whitespace);
        if q.is_empty() {
            return Matcher::Empty;
        }
        // Whole-query regex: `/pattern/` with a non-empty pattern. (A lone `//`
        // would be an empty pattern that matches everything — not useful, so we
        // treat it as a literal boolean term instead.)
        if q.len() >= 3 && q.starts_with('/') && q.ends_with('/') {
            let pat = &q[1..q.len() - 1];
            return match compile_regex(pat) {
                Ok(re) => Matcher::Regex(re),
                Err(e) => Matcher::InvalidRegex(e.to_string()),
            };
        }
        let groups = parse_boolean(q, remove_accents);
        // A group with no positive term (e.g. the whole query is `-foo`) would
        // match nearly everything — drop it; if none survive, the query is Empty.
        let groups: Vec<AndGroup> = groups
            .into_iter()
            .filter(|g| g.iter().any(|t| !t.negated))
            .collect();
        if groups.is_empty() {
            Matcher::Empty
        } else {
            Matcher::Boolean(groups)
        }
    }

    /// Does the body match? `lower` must use the same policy as the matcher:
    /// `canonical_fold` for `parse`/policy true, `literal_fold` for policy false.
    /// A mismatched fold silently misses accent-bearing terms. `orig` is the
    /// original body for regex; Empty/InvalidRegex match nothing.
    pub fn matches(&self, lower: &str, orig: &str) -> bool {
        match self {
            Matcher::Regex(re) => re.is_match(orig),
            Matcher::InvalidRegex(_) | Matcher::Empty => false,
            Matcher::Boolean(groups) => groups.iter().any(|g| group_matches(g, lower)),
        }
    }

    /// The single positive term when this is a bare one-term query, else `None`.
    /// Drives the quick switcher's "keep fuzzy page ranking" fast path.
    pub fn simple_term(&self) -> Option<&str> {
        match self {
            Matcher::Boolean(groups) if groups.len() == 1 && groups[0].len() == 1 => {
                let t = &groups[0][0];
                (!t.negated && !t.quoted && !t.text.is_empty()).then_some(t.text.as_str())
            }
            _ => None,
        }
    }

    /// Rank a page name (already folded in `lower`, original in `orig`) for
    /// the non-simple path: prefix > substring, else `None` if it doesn't match.
    pub fn score_name(&self, lower: &str, orig: &str) -> Option<i32> {
        match self {
            Matcher::Regex(re) => re.is_match(orig).then_some(500),
            Matcher::InvalidRegex(_) | Matcher::Empty => None,
            Matcher::Boolean(groups) => {
                let g = groups.iter().find(|g| group_matches(g, lower))?;
                // Prefix match on any positive term ranks above a mid-name hit.
                let prefix = g
                    .iter()
                    .any(|t| !t.negated && !t.text.is_empty() && lower.starts_with(&t.text));
                Some(if prefix { 1000 } else { 500 })
            }
        }
    }
}

fn group_matches(group: &AndGroup, lower: &str) -> bool {
    group.iter().all(|t| {
        let present = !t.text.is_empty() && lower.contains(&t.text);
        present != t.negated
    })
}

/// Tokenize + group a boolean query. `OR` (bare, uppercase) starts a new group;
/// other tokens accumulate into the current group.
fn parse_boolean(q: &str, remove_accents: bool) -> Vec<AndGroup> {
    let tokens = tokenize(q);
    let mut groups: Vec<AndGroup> = Vec::new();
    let mut cur: AndGroup = Vec::new();
    for tok in tokens {
        if tok.is_or {
            groups.push(std::mem::take(&mut cur));
            continue;
        }
        if tok.text.is_empty() {
            continue;
        }
        cur.push(Term {
            text: if remove_accents {
                canonical_fold(&tok.text)
            } else {
                literal_fold(&tok.text)
            },
            negated: tok.negated,
            quoted: tok.quoted,
        });
    }
    groups.push(cur);
    groups.into_iter().filter(|g| !g.is_empty()).collect()
}

struct Token {
    text: String,
    negated: bool,
    quoted: bool,
    /// A bare `OR` separator (never both `is_or` and non-empty `text`).
    is_or: bool,
}

/// Split into tokens, honoring `"quoted phrases"` (which may contain spaces) and
/// a leading `-` for negation. A bare unquoted `OR` becomes an OR separator.
fn tokenize(q: &str) -> Vec<Token> {
    let chars: Vec<char> = q.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if is_search_whitespace(chars[i]) {
            i += 1;
            continue;
        }
        let mut negated = false;
        // A leading `-` negates, but only when something follows it (a lone `-`
        // is treated as a literal term).
        if chars[i] == '-' && i + 1 < chars.len() && !is_search_whitespace(chars[i + 1]) {
            negated = true;
            i += 1;
        }
        let (text, quoted) = if i < chars.len() && chars[i] == '"' {
            // Quoted phrase: read to the closing quote (or end of input).
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            if i < chars.len() {
                i += 1; // consume closing quote
            }
            (s, true)
        } else {
            // Bare token: read to the next whitespace.
            let start = i;
            while i < chars.len() && !is_search_whitespace(chars[i]) {
                i += 1;
            }
            (chars[start..i].iter().collect::<String>(), false)
        };
        if !quoted && !negated && text == "OR" {
            out.push(Token {
                text: String::new(),
                negated: false,
                quoted: false,
                is_or: true,
            });
        } else {
            out.push(Token {
                text,
                negated,
                quoted,
                is_or: false,
            });
        }
    }
    out
}

/// Original UTF-16 ranges of overlapping folded substring matches, deduplicated
/// and capped at `limit`. Empty needles produce no evidence.
pub fn substring_spans(
    text: &str,
    needle: &str,
    limit: usize,
    remove_accents: bool,
) -> Vec<Range<usize>> {
    let (folded, map) = fold_with_map(text, remove_accents);
    let hay: Vec<_> = folded.chars().collect();
    let needle: Vec<_> = if remove_accents {
        canonical_fold(needle)
    } else {
        literal_fold(needle)
    }
    .chars()
    .collect();
    if needle.is_empty() || needle.len() > hay.len() || limit == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for at in 0..=hay.len() - needle.len() {
        if hay[at..at + needle.len()] == needle {
            let span = map[at].start..map[at + needle.len() - 1].end;
            if !out.contains(&span) {
                out.push(span);
            }
            if out.len() == limit {
                break;
            }
        }
    }
    out
}

/// Regex evidence in original UTF-16 coordinates, including zero-width matches.
pub fn regex_spans(re: &regex::Regex, text: &str, limit: usize) -> Vec<Range<usize>> {
    re.find_iter(text)
        .take(limit)
        .map(|hit| {
            text[..hit.start()].encode_utf16().count()..text[..hit.end()].encode_utf16().count()
        })
        .collect()
}

impl Matcher {
    /// First positive occurrence (even in a nonmatching boolean group), as used
    /// by snippet highlighting. Regex zero-width hits retain their position.
    pub fn first_span(&self, text: &str, remove_accents: bool) -> Option<Range<usize>> {
        match self {
            Self::Regex(re) => regex_spans(re, text, 1).into_iter().next(),
            Self::Boolean(groups) => groups
                .iter()
                .flatten()
                .filter(|term| !term.negated && !term.text.is_empty())
                .filter_map(|term| {
                    substring_spans(text, &term.text, 1, remove_accents)
                        .into_iter()
                        .next()
                })
                .min_by_key(|span| span.start),
            _ => None,
        }
    }

    /// Bounded positive evidence for the first satisfied group. Regex zero-width
    /// hits are omitted for multi-range presentation; first_span preserves them.
    pub fn spans(&self, text: &str, limit: usize, remove_accents: bool) -> Vec<Range<usize>> {
        if limit == 0 {
            return Vec::new();
        }
        match self {
            Self::Regex(re) => re
                .find_iter(text)
                .filter(|hit| hit.end() > hit.start())
                .take(limit)
                .map(|hit| {
                    text[..hit.start()].encode_utf16().count()
                        ..text[..hit.end()].encode_utf16().count()
                })
                .collect(),
            Self::Boolean(groups) => {
                let lower = if remove_accents {
                    canonical_fold(text)
                } else {
                    literal_fold(text)
                };
                let Some(group) = groups.iter().find(|group| group_matches(group, &lower)) else {
                    return Vec::new();
                };
                let mut out = Vec::new();
                for term in group
                    .iter()
                    .filter(|term| !term.negated && !term.text.is_empty())
                {
                    out.extend(substring_spans(
                        text,
                        &term.text,
                        limit - out.len(),
                        remove_accents,
                    ));
                }
                out.sort_by_key(|span| (span.start, span.end));
                out
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(q: &str) -> Matcher {
        Matcher::parse(q)
    }
    // Convenience: match against text (folds the boolean side once).
    fn hit(q: &str, text: &str) -> bool {
        m(q).matches(&canonical_fold(text), text)
    }

    #[test]
    fn shared_whitespace_and_regex_contract() {
        assert_eq!(
            Matcher::parse("\u{feff}foo\u{feff}").simple_term(),
            Some("foo")
        );
        assert_eq!(
            Matcher::parse("\u{85}foo\u{85}").simple_term(),
            Some("\u{85}foo\u{85}")
        );
        assert!(hit("foo\u{2003}bar", "bar then foo"));
        assert!(hit(r"/\p{L}+/", "café"));
        assert!(hit(r"/(?i)abc/", "ABC"));
        for query in [r"/foo(?=bar)/", r"/(a)\1/"] {
            assert!(matches!(m(query), Matcher::InvalidRegex(_)), "{query}");
        }
    }

    #[test]
    fn shared_parser_contract_fixtures() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/search-query-contract.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let query = row["query"].as_str().unwrap();
            let body = row["match"].as_str().unwrap();
            let matcher = m(query);
            let kind = match matcher {
                Matcher::Boolean(_) => "boolean",
                Matcher::Regex(_) => "regex",
                Matcher::InvalidRegex(_) => "invalid",
                Matcher::Empty => "empty",
            };
            assert_eq!(kind, row["kind"], "{query}");
            assert_eq!(matcher.simple_term(), row["simple"].as_str(), "{query}");
            assert_eq!(hit(query, body), kind != "invalid", "{query}");
        }
    }

    #[test]
    fn single_term_is_simple() {
        let mt = m("hello");
        assert_eq!(mt.simple_term(), Some("hello"));
        assert!(hit("hello", "well HELLO there"));
        assert!(!hit("hello", "goodbye"));
    }

    #[test]
    fn visible_syntax_examples_match_the_documented_behavior() {
        for (query, matching, missing) in SEARCH_SYNTAX_EXAMPLES {
            assert!(hit(query, matching), "{query:?} should match {matching:?}");
            assert!(!hit(query, missing), "{query:?} should reject {missing:?}");
        }
    }

    #[test]
    fn whitespace_is_and_order_independent() {
        let mt = m("foo bar");
        assert_eq!(mt.simple_term(), None); // two terms → not simple
        assert!(hit("foo bar", "bar then foo"));
        assert!(hit("foo bar", "foo and bar"));
        assert!(!hit("foo bar", "only foo"));
    }

    #[test]
    fn or_keyword_splits_groups() {
        assert!(hit("cat OR dog", "i have a dog"));
        assert!(hit("cat OR dog", "i have a cat"));
        assert!(!hit("cat OR dog", "i have a fish"));
        // AND binds tighter than OR: "a b OR c" = (a AND b) OR c
        assert!(hit("apple pie OR cake", "cake"));
        assert!(hit("apple pie OR cake", "apple pie"));
        assert!(!hit("apple pie OR cake", "apple tart"));
    }

    #[test]
    fn negation_excludes() {
        assert!(hit("foo -bar", "foo only"));
        assert!(!hit("foo -bar", "foo and bar"));
        // A pure-negation query matches nothing (not everything).
        assert!(matches!(m("-bar"), Matcher::Empty));
        assert!(!hit("-bar", "anything"));
    }

    #[test]
    fn quoted_phrase_is_contiguous() {
        assert!(hit("\"foo bar\"", "a foo bar b"));
        assert!(!hit("\"foo bar\"", "foo x bar"));
        // Quoted single word is not "simple" (user opted into the grammar).
        assert_eq!(m("\"foo\"").simple_term(), None);
        // Negated phrase.
        assert!(!hit("-\"foo bar\"", "foo bar here"));
        assert!(hit("keep -\"foo bar\"", "keep foo x bar"));
    }

    #[test]
    fn regex_whole_query_case_sensitive() {
        assert!(hit("/[A-Z]{3}/", "abc ABC def"));
        assert!(!hit("/[A-Z]{3}/", "abc def"));
        assert!(hit("/^start/", "start of line"));
        assert!(!hit("/^start/", "not at start"));
    }

    #[test]
    fn invalid_regex_reports_and_matches_nothing() {
        let mt = m("/(unclosed/");
        assert!(matches!(mt, Matcher::InvalidRegex(_)));
        assert!(!mt.matches("(unclosed", "(unclosed"));
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(matches!(m("   "), Matcher::Empty));
        assert!(!hit("   ", "anything"));
    }

    #[test]
    fn score_name_prefers_prefix() {
        let mt = m("foo bar");
        // "foobar…" — a positive term prefixes the name → 1000.
        assert_eq!(mt.score_name("foobar baz", "foobar baz"), Some(1000));
        // both present but neither prefixes → 500.
        assert_eq!(mt.score_name("xbar yfoo", "xbar yfoo"), Some(500));
        assert_eq!(mt.score_name("only foo", "only foo"), None);
    }

    #[test]
    fn lone_slash_is_literal_not_regex() {
        // `//` is too short to be a regex → literal term.
        assert!(hit("//", "a // b"));
        assert!(!matches!(m("//"), Matcher::Regex(_)));
    }

    #[test]
    fn canonical_unicode_equivalence_and_default_accent_fold() {
        assert!(hit("café", "a cafe\u{301} here"));
        assert!(hit("cafe\u{301}", "a café here"));
        assert!(hit("\u{ac00}", "Hangul \u{1100}\u{1161}"));
        assert!(hit("i\u{307}", "\u{130}"));
        assert!(hit("cafe", "café"));
        assert!(hit("lodz", "Łódź"));
        assert!(hit("елка", "ёлка"));
        assert!(!hit("か", "が"));
        assert!(!hit("и", "й"));
        assert!(!hit("कु", "क"));
        // Regular expressions retain their original-text semantics.
        assert!(!hit("/café/", "cafe\u{301}"));
    }

    #[test]
    fn master_accent_fold_policy_keeps_letter_making_marks() {
        // a3e7bfda9: Latin/Greek/RTL accents, strokes, ignorable marks,
        // Cyrillic ё, and compatibility width fold; letter-making marks stay.
        for (raw, plain) in [
            ("café", "cafe"),
            ("Příliš žluťoučký kůň", "prilis zlutoucky kun"),
            ("γειά", "γεια"),
            ("שָׁלוֹם", "שלום"),
            ("مَرْحَبًا", "مرحبا"),
            ("ёлка", "елка"),
            ("Łódź", "lodz"),
            ("Øresund", "oresund"),
            ("Đà Nẵng", "da nang"),
            ("Ħal", "hal"),
            ("Ŧ", "t"),
            ("Ｔｉｎｅ", "tine"),
            ("a\u{034f}b", "ab"),
        ] {
            assert_eq!(canonical_fold(raw), canonical_fold(plain), "{raw:?}");
        }
        for (raw, plain) in [
            ("が", "か"),
            ("ぱ", "は"),
            ("क्", "क"),
            ("कु", "क"),
            ("กุ", "ก"),
            ("ກຸ", "ກ"),
            ("ཀི", "ཀ"),
            ("й", "и"),
            ("ї", "і"),
            ("ў", "у"),
            ("ѐ", "е"),
        ] {
            assert_ne!(canonical_fold(raw), canonical_fold(plain), "{raw:?}");
        }
        let (folded, spans) = canonical_fold_with_map("🧠 cafe\u{301} café");
        assert_eq!(folded, "🧠 cafe cafe");
        assert_eq!(spans[5], 6..8);
        assert_eq!(spans[6], 8..9);
    }

    #[test]
    fn master_a6_compatibility_fixtures_keep_source_ranges() {
        // 48321626a/6370058fa fold.rs accepted_a6_fixtures: ligatures,
        // width, Jamo composition, dotted I and contextual sigma.
        for (raw, folded, span_at) in [
            ("ofﬁce", "office", (2, 2..3)),
            ("😀Ｔｉｎｅ", "😀tine", (1, 2..3)),
            ("ㄱㅏ", "가", (0, 0..2)),
            ("İstanbul", "istanbul", (0, 0..1)),
            ("ΟΣ Σ", "ος σ", (1, 1..2)),
        ] {
            let (mapped, spans) = canonical_fold_with_map(raw);
            assert_eq!(canonical_fold(raw), folded, "{raw:?}");
            assert_eq!(mapped, folded, "{raw:?}");
            assert_eq!(spans[span_at.0], span_at.1, "{raw:?}");
        }
        assert_ne!(canonical_fold("Straße"), canonical_fold("STRASSE"));
    }

    #[test]
    fn master_a6_reordering_and_hangul_fixtures_match_mapped_text() {
        for (raw, expected) in [
            ("prefix ㄱ\u{301}ㅏ suffix", "prefix 가 suffix"),
            ("한글", "한글"),
            ("Kelvin", "kelvin"),
            ("Ｐｒｏｊｅｃｔ ｶﾞｲﾄﾞ 豈", "project ガイド 豈"),
            ("a\u{301}", "a"),
            ("ΟΣ Σ", "ος σ"),
        ] {
            let (mapped, spans) = canonical_fold_with_map(raw);
            assert_eq!(canonical_fold(raw), expected, "{raw:?}");
            assert_eq!(mapped, expected, "{raw:?}");
            assert_eq!(spans.len(), mapped.chars().count(), "{raw:?}");
        }
    }

    #[test]
    fn config_off_uses_literal_case_and_canonical_equivalence() {
        let match_off = Matcher::parse_with_policy("cafe", false);
        assert!(!match_off.matches(&literal_fold("café"), "café"));
        assert!(Matcher::parse_with_policy("café", false)
            .matches(&literal_fold("cafe\u{301}"), "cafe\u{301}"));
        assert_eq!(literal_fold("Ｔｉｎｅ"), "tine");
        assert_ne!(identity_fold("Ｔｉｎｅ"), identity_fold("Tine"));
        let (mapped, spans) = literal_fold_with_map("Ｔｉｎｅ");
        assert_eq!(mapped, "tine");
        assert_eq!(spans, vec![0..1, 1..2, 2..3, 3..4]);
    }

    #[test]
    fn erased_accent_terms_keep_boolean_semantics() {
        // 6370058fa lib.rs a6_erased_terms_keep_positive_false_and_negative_true_semantics.
        let mark = "\u{301}";
        assert_eq!(Matcher::parse(mark).simple_term(), None);
        assert!(!hit(mark, "anything"));
        assert!(!hit(&format!("{mark} alpha"), "alpha"));
        assert!(hit(&format!("{mark} OR alpha"), "alpha"));
        assert!(hit(&format!("alpha -{mark}"), "alpha"));
        assert!(matches!(
            Matcher::parse(&format!("-{mark}")),
            Matcher::Empty
        ));
    }
}

#[cfg(test)]
mod unicode_contract {
    use super::*;
    #[test]
    fn native_unicode_regex_fixture_matches_browser_contract() {
        let rows: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/search-regex-unicode.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let q = row["query"].as_str().unwrap();
            let text = row["text"].as_str().unwrap();
            let m = Matcher::parse(q);
            assert!(matches!(m, Matcher::Regex(_)), "{q}");
            assert_eq!(
                m.matches(&canonical_fold(text), text),
                row["match"].as_bool().unwrap(),
                "{q}"
            );
            let spans: Vec<_> = m
                .spans(text, 24, true)
                .into_iter()
                .map(|span| serde_json::json!({"start":span.start,"end":span.end}))
                .collect();
            assert_eq!(serde_json::json!(spans), row["spans"], "{q}");
        }
        assert!(matches!(
            Matcher::parse("/a{1000000}/"),
            Matcher::InvalidRegex(_)
        ));
    }
}
