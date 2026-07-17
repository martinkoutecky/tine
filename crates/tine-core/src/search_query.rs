//! Shared parser for the Ctrl-K quick-search query dialect (GH #44).
//!
//! Mirrors `src/editor/searchQuery.ts` on the frontend — keep the two in sync
//! (the grammar, the `is_simple` rule, and the match semantics must agree, or a
//! query would filter differently in the page list than in the block list).
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

use unicode_normalization::UnicodeNormalization;

/// Canonical comparison representation for non-regex search. Lowercasing is
/// locale-independent; NFC makes canonically equivalent spellings compare
/// alike without compatibility folding or removing accents.
pub fn canonical_fold(value: &str) -> String {
    value.to_lowercase().nfc().collect()
}

/// One AND-term: a substring to test (already canonically folded) plus whether it is
/// negated (`-term` → must NOT be present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Lowercase-plus-NFC needle for `visible_lower.contains(..)`.
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
        let q = query.trim();
        if q.is_empty() {
            return Matcher::Empty;
        }
        // Whole-query regex: `/pattern/` with a non-empty pattern. (A lone `//`
        // would be an empty pattern that matches everything — not useful, so we
        // treat it as a literal boolean term instead.)
        if q.len() >= 3 && q.starts_with('/') && q.ends_with('/') {
            let pat = &q[1..q.len() - 1];
            return match regex::Regex::new(pat) {
                Ok(re) => Matcher::Regex(re),
                Err(e) => Matcher::InvalidRegex(e.to_string()),
            };
        }
        let groups = parse_boolean(q);
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

    /// Does `visible` match? `lower` is the pre-folded lowercase-plus-NFC body
    /// (hot path for boolean terms); `orig` is the original body (needed by regex).
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
                (!t.negated && !t.quoted).then_some(t.text.as_str())
            }
            _ => None,
        }
    }

    /// Rank a page name (already lowercase-plus-NFC in `lower`, original in `orig`) for
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
fn parse_boolean(q: &str) -> Vec<AndGroup> {
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
            text: canonical_fold(&tok.text),
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
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let mut negated = false;
        // A leading `-` negates, but only when something follows it (a lone `-`
        // is treated as a literal term).
        if chars[i] == '-' && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
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
            while i < chars.len() && !chars[i].is_whitespace() {
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
    fn canonical_unicode_equivalence_does_not_fold_accents() {
        assert!(hit("café", "a cafe\u{301} here"));
        assert!(hit("cafe\u{301}", "a café here"));
        assert!(hit("\u{ac00}", "Hangul \u{1100}\u{1161}"));
        assert!(hit("i\u{307}", "\u{130}"));
        assert!(!hit("cafe", "café"));
        // Regular expressions retain their original-text semantics.
        assert!(!hit("/café/", "cafe\u{301}"));
    }
}
