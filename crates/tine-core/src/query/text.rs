//! Exact text helpers the structured-query evaluator shares.
//!
//! The source limits (`QUERY_SOURCE_MAX_BYTES`, nesting) live in the parent
//! `query` module, og's one answerer for them; master's SQL framing helpers
//! and raw-path visible projections are execution-side (SQL) and not ported.

/// SQL `LIKE` over an already-folded haystack; the caller folds the pattern
/// the same way. `%` matches any run (including empty), `_` exactly one
/// Unicode scalar, `\` escapes the following scalar, and the match is anchored
/// at both ends. An unpaired trailing `\` makes the pattern match nothing
/// (SQLite's `ESCAPE` rejects it).
///
/// Compiles the pattern and runs [`LikePattern::matches`]; an evaluator that
/// tests one pattern against many haystacks compiles it once instead.
pub fn like_matches(haystack: &str, pattern: &str) -> bool {
    LikePattern::compile(pattern).matches(haystack)
}

/// Encode literal data in a LIKE pattern; `%`, `_` and `\` are escaped.
/// Linear in input scalars, with no I/O or failure path.
pub fn escape_like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A compiled SQL `LIKE` pattern (semantics of [`like_matches`]).
///
/// The pattern is split on `%` into segments; greedy leftmost placement of
/// each `%`-separated segment is exact (a later placement never helps an
/// earlier one). A segment of plain scalars is found with the standard
/// library's linear-time substring search, so a `%`-only pattern costs
/// O(haystack + pattern). A segment containing `_` is found with bit-parallel
/// Shift-And: O(haystack × ⌈segment / 64⌉), memory O(segment) words. A
/// haystack shorter than the pattern's fixed length is rejected before any
/// scan (I-22: the query text is hostile input; this replaces a matcher whose
/// worst case was O(haystack × pattern) per comparison).
#[derive(Debug, Clone)]
pub struct LikePattern {
    /// `None` for an unpaired trailing `\`: matches nothing.
    shape: Option<Shape>,
}

#[derive(Debug, Clone)]
struct Shape {
    /// Pattern had a `%` at all (else the one segment is anchored at both ends).
    any: bool,
    leading_any: bool,
    trailing_any: bool,
    segments: Vec<Segment>,
    /// Scalars the haystack must have at least (sum of segment lengths).
    min_chars: usize,
}

#[derive(Debug, Clone)]
struct Segment {
    /// `None` is `_`.
    tokens: Vec<Option<char>>,
    /// The segment as a string when it has no `_`.
    literal: Option<String>,
    /// Shift-And tables when it has a `_`.
    shift_and: Option<ShiftAnd>,
}

#[derive(Debug, Clone)]
struct ShiftAnd {
    words: usize,
    /// Positions holding `_`.
    wild: Vec<u64>,
    /// Scalars occurring at more than `words` positions, as position bitsets.
    dense: std::collections::HashMap<char, Vec<u64>>,
    /// Every other scalar, as its (fewer than `words`) positions.
    sparse: std::collections::HashMap<char, Vec<usize>>,
}

impl ShiftAnd {
    fn new(tokens: &[Option<char>]) -> ShiftAnd {
        let words = tokens.len().div_ceil(64);
        let mut wild = vec![0u64; words];
        let mut positions: std::collections::HashMap<char, Vec<usize>> = Default::default();
        for (at, token) in tokens.iter().enumerate() {
            match token {
                None => wild[at / 64] |= 1 << (at % 64),
                Some(ch) => positions.entry(*ch).or_default().push(at),
            }
        }
        let mut dense = std::collections::HashMap::new();
        let mut sparse = std::collections::HashMap::new();
        for (ch, at) in positions {
            if at.len() > words {
                let mut bits = vec![0u64; words];
                for p in at {
                    bits[p / 64] |= 1 << (p % 64);
                }
                dense.insert(ch, bits);
            } else {
                sparse.insert(ch, at);
            }
        }
        ShiftAnd {
            words,
            wild,
            dense,
            sparse,
        }
    }
}

impl Segment {
    fn new(tokens: Vec<Option<char>>) -> Segment {
        let literal = tokens.iter().copied().collect::<Option<String>>();
        let shift_and = literal.is_none().then(|| ShiftAnd::new(&tokens));
        Segment {
            tokens,
            literal,
            shift_and,
        }
    }

    /// Whether the segment matches `text` starting at its first scalar;
    /// returns the byte length it covered.
    fn match_prefix(&self, text: &str) -> Option<usize> {
        let mut chars = text.char_indices();
        for token in &self.tokens {
            let (_, ch) = chars.next()?;
            work(1);
            if token.is_some_and(|want| want != ch) {
                return None;
            }
        }
        Some(chars.next().map_or(text.len(), |(at, _)| at))
    }

    /// The byte offset just past the leftmost occurrence in `text`.
    fn find_end(&self, text: &str) -> Option<usize> {
        if let Some(literal) = &self.literal {
            work(text.len() + literal.len());
            return text.find(literal.as_str()).map(|at| at + literal.len());
        }
        let table = self.shift_and.as_ref()?;
        let last = self.tokens.len() - 1;
        let (top_word, top_bit) = (last / 64, 1u64 << (last % 64));
        let mut state = vec![0u64; table.words];
        let mut shifted = vec![0u64; table.words];
        for (at, ch) in text.char_indices() {
            work(table.words);
            let mut carry = 1u64;
            for (word, out) in state.iter().zip(shifted.iter_mut()) {
                *out = (word << 1) | carry;
                carry = word >> 63;
            }
            match table.dense.get(&ch) {
                Some(bits) => {
                    for ((word, &s), (&w, &b)) in state
                        .iter_mut()
                        .zip(shifted.iter())
                        .zip(table.wild.iter().zip(bits.iter()))
                    {
                        *word = s & (w | b);
                    }
                }
                None => {
                    for ((word, &s), &w) in state.iter_mut().zip(shifted.iter()).zip(&table.wild) {
                        *word = s & w;
                    }
                    for &p in table.sparse.get(&ch).map_or(&[][..], Vec::as_slice) {
                        state[p / 64] |= shifted[p / 64] & (1 << (p % 64));
                    }
                }
            }
            if state[top_word] & top_bit != 0 {
                return Some(at + ch.len_utf8());
            }
        }
        None
    }
}

impl LikePattern {
    /// Literal prefix when the compiled pattern is exactly `literal%`.
    /// Uses the same decoded tokens as matching; no independent escape grammar.
    pub fn starts_with_prefix(&self) -> Option<String> {
        let shape = self.shape.as_ref()?;
        if !shape.any || shape.leading_any || !shape.trailing_any || shape.segments.len() != 1 {
            return None;
        }
        shape.segments[0].literal.clone()
    }

    pub fn compile(pattern: &str) -> LikePattern {
        let mut segments = Vec::new();
        let mut current: Vec<Option<char>> = Vec::new();
        let (mut any, mut leading_any, mut trailing_any) = (false, false, false);
        let mut chars = pattern.chars();
        while let Some(ch) = chars.next() {
            trailing_any = false;
            match ch {
                '\\' => {
                    let Some(next) = chars.next() else {
                        return LikePattern { shape: None };
                    };
                    current.push(Some(next));
                }
                '%' => {
                    if !any && current.is_empty() {
                        leading_any = true;
                    }
                    any = true;
                    trailing_any = true;
                    if !current.is_empty() {
                        segments.push(Segment::new(std::mem::take(&mut current)));
                    }
                }
                '_' => current.push(None),
                other => current.push(Some(other)),
            }
        }
        if !current.is_empty() {
            segments.push(Segment::new(current));
        }
        let min_chars = segments.iter().map(|s| s.tokens.len()).sum();
        LikePattern {
            shape: Some(Shape {
                any,
                leading_any,
                trailing_any,
                segments,
                min_chars,
            }),
        }
    }

    pub fn matches(&self, haystack: &str) -> bool {
        let Some(shape) = &self.shape else {
            return false;
        };
        // Bytes bound scalars from above: most haystacks shorter than a long
        // pattern are rejected without counting.
        if haystack.len() < shape.min_chars
            || (shape.min_chars > 0 && haystack.chars().count() < shape.min_chars)
        {
            return false;
        }
        let segments = &shape.segments[..];
        if !shape.any {
            return match segments.first() {
                None => haystack.is_empty(),
                Some(only) => only.match_prefix(haystack) == Some(haystack.len()),
            };
        }
        let (mut from, mut to) = (0usize, haystack.len());
        let mut middle = segments;
        if !shape.leading_any {
            let Some((first, rest)) = middle.split_first() else {
                return true;
            };
            match first.match_prefix(haystack) {
                Some(len) => from = len,
                None => return false,
            }
            middle = rest;
        }
        if !shape.trailing_any {
            let Some((last, rest)) = middle.split_last() else {
                // The anchored first segment was also the last one.
                return false;
            };
            // Its start: `last.tokens.len()` scalars before the end.
            let start = haystack
                .char_indices()
                .rev()
                .nth(last.tokens.len() - 1)
                .map(|(at, _)| at);
            match start {
                Some(start) if start >= from => {
                    if last.match_prefix(&haystack[start..]).is_none() {
                        return false;
                    }
                    to = start;
                }
                _ => return false,
            }
            middle = rest;
        }
        for segment in middle {
            match segment.find_end(&haystack[from..to]) {
                Some(end) => from += end,
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
thread_local! {
    static LIKE_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test-only work accounting: scalars and Shift-And words touched.
#[inline(always)]
fn work(_units: usize) {
    #[cfg(test)]
    LIKE_WORK.with(|w| w.set(w.get() + _units));
}

#[cfg(test)]
mod tests {
    use super::{like_matches, LikePattern};

    #[test]
    fn like_matches_sql_escape_semantics() {
        assert!(!like_matches("abc", "abc\\"));
        assert!(!like_matches("", "\\"));
        assert!(like_matches("a_b", "a\\_b"));
        assert!(!like_matches("axb", "a\\_b"));
        assert!(like_matches("100%", "100\\%"));
        assert!(!like_matches("1000", "100\\%"));
    }

    /// Run `like_matches` on another thread and fail (rather than hang the
    /// suite) when it does not answer within `limit`.
    fn within(limit: std::time::Duration, haystack: String, pattern: String) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(like_matches(&haystack, &pattern));
        });
        rx.recv_timeout(limit)
            .expect("like_matches must answer a hostile pattern within its bound (I-22)")
    }

    /// I-22 hostile case: repeated `%a` against a long all-`a` haystack with a
    /// suffix that cannot match. Backtracking explores every split point of
    /// every `%`; a bounded matcher answers in time linear-ish in the input.
    #[test]
    fn a_pathological_wildcard_pattern_answers_within_a_bound() {
        let limit = std::time::Duration::from_secs(2);
        // Shallow enough not to overflow a recursive matcher's stack, so this
        // one fails by TIME: C(400, 40) split points to try.
        assert!(!within(
            limit,
            "a".repeat(400),
            format!("{}b", "%a".repeat(40))
        ));
        let haystack = "a".repeat(20_000);
        let miss = format!("{}b", "%a".repeat(5_000));
        assert!(!within(limit, haystack.clone(), miss));
        let hit = format!("{}%", "%a".repeat(5_000));
        assert!(within(limit, haystack.clone(), hit));
        let underscores = format!("{}%b", "%_".repeat(5_000));
        assert!(!within(limit, haystack, underscores));
    }

    /// Benign extreme paired with the hostile case: large inputs that must
    /// still be ACCEPTED, so the bound is not a refusal in disguise.
    #[test]
    fn a_large_benign_pattern_is_still_matched() {
        let limit = std::time::Duration::from_secs(2);
        let haystack = format!("{}needle{}", "x".repeat(500_000), "y".repeat(500_000));
        assert!(within(limit, haystack.clone(), "%needle%".into()));
        assert!(!within(limit, haystack.clone(), "%needles%".into()));
        assert!(within(
            limit,
            haystack.clone(),
            format!("{}needle%", "_".repeat(500_000))
        ));
        let literal = "ab_%\\".repeat(10_000);
        let escaped = literal
            .chars()
            .flat_map(|ch| match ch {
                '%' | '_' | '\\' => vec!['\\', ch],
                other => vec![other],
            })
            .collect::<String>();
        assert!(within(limit, literal.clone(), escaped.clone()));
        assert!(!within(limit, format!("{literal}!"), escaped));
    }

    fn work_of(haystack: &str, pattern: &LikePattern) -> (bool, usize) {
        super::LIKE_WORK.with(|w| w.set(0));
        let hit = pattern.matches(haystack);
        (hit, super::LIKE_WORK.with(std::cell::Cell::get))
    }

    /// G3 (og 14 Q2) I-22/I-25: a pattern at the admitted source limit
    /// (64 KiB) against a long body. The old greedy-restart matcher retried the
    /// suffix after the last `%` at every haystack position, O(body × pattern)
    /// (~1.7e10 steps here). Plain segments must cost O(body + pattern); a
    /// `_` segment O(body × ⌈segment/64⌉).
    #[test]
    fn an_admitted_limit_pattern_costs_linear_work_on_a_long_body() {
        let limit = std::time::Duration::from_secs(5);
        let max = crate::query::QUERY_SOURCE_MAX_BYTES;
        let body = format!("{}c", "a".repeat(256 * 1024));
        let run = "a".repeat(max - 3);
        for pattern in [format!("%{run}b"), format!("%{run}b%"), format!("{run}%b")] {
            assert!(pattern.len() <= max);
            let compiled = LikePattern::compile(&pattern);
            let (hit, units) = work_of(&body, &compiled);
            assert!(!hit, "{}", &pattern[..8]);
            assert!(
                units <= 4 * (body.len() + pattern.len()),
                "LIKE work {units} is not linear in body + pattern"
            );
            assert!(!within(limit, body.clone(), pattern));
        }
        // Accepted counterpart: the same size of pattern that DOES occur.
        let hit = format!("%{run}c");
        assert!(within(limit, body.clone(), hit));
        // `_` segment: bounded by body × words, not body × pattern scalars.
        let wild = format!("%{}b%", "a_".repeat((max - 3) / 2));
        let compiled = LikePattern::compile(&wild);
        let short_body = format!("{}c", "a".repeat(96 * 1024));
        let (hit, units) = work_of(&short_body, &compiled);
        assert!(!hit);
        let words = wild.len().div_ceil(64);
        assert!(
            units <= 2 * short_body.len() * words,
            "Shift-And work {units}"
        );
        // Benign: a short `_` pattern over the long body still finds its hit.
        assert!(like_matches(&body, "%a_c"));
        assert!(!like_matches(&body, "%a_b%"));
    }

    #[test]
    fn like_matches_wildcard_semantics() {
        assert!(like_matches("", ""));
        assert!(like_matches("", "%"));
        assert!(like_matches("", "%%"));
        assert!(!like_matches("", "_"));
        assert!(like_matches("abc", "a%c"));
        assert!(like_matches("ac", "a%c"));
        assert!(!like_matches("ab", "a%c"));
        assert!(like_matches("abcbc", "%bc"));
        assert!(like_matches("abcbd", "a%b_"));
        assert!(!like_matches("abc", "a_"));
        assert!(like_matches("čau", "_au"));
        assert!(like_matches("mississippi", "m%iss%ppi"));
        assert!(!like_matches("mississippi", "m%iss%ppx"));
        assert!(like_matches("a%b", "a\\%b"));
        assert!(like_matches("xa%b", "%\\%b"));
    }

    /// The iterative matcher agrees with the obvious recursive definition on
    /// every pattern of up to five tokens over a two-letter alphabet.
    #[test]
    fn like_matches_agrees_with_the_recursive_definition() {
        fn reference(h: &[char], p: &[char]) -> bool {
            match p.split_first() {
                None => h.is_empty(),
                Some(('%', rest)) => (0..=h.len()).any(|i| reference(&h[i..], rest)),
                Some(('_', rest)) => !h.is_empty() && reference(&h[1..], rest),
                Some((c, rest)) => h.first() == Some(c) && reference(&h[1..], rest),
            }
        }
        fn words(alphabet: &[char], max: usize) -> Vec<String> {
            let mut all = vec![String::new()];
            let mut layer = vec![String::new()];
            for _ in 0..max {
                layer = layer
                    .iter()
                    .flat_map(|w| alphabet.iter().map(move |c| format!("{w}{c}")))
                    .collect();
                all.extend(layer.iter().cloned());
            }
            all
        }
        let haystacks = words(&['a', 'b'], 5);
        for pattern in words(&['a', 'b', '%', '_'], 5) {
            let p = pattern.chars().collect::<Vec<_>>();
            for haystack in &haystacks {
                let h = haystack.chars().collect::<Vec<_>>();
                assert_eq!(
                    like_matches(haystack, &pattern),
                    reference(&h, &p),
                    "{haystack:?} LIKE {pattern:?}"
                );
            }
        }
    }
}
