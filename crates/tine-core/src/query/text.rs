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
/// Iterative greedy matching with a single `%` restart point (I-22: the query
/// text is hostile input). A later `%` subsumes every earlier one, so only the
/// most recent `%` ever needs retrying: O(haystack × pattern) time in the worst
/// case, O(haystack + pattern) space, no recursion.
pub fn like_matches(haystack: &str, pattern: &str) -> bool {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Token {
        Scalar(char),
        Any,
        One,
    }

    let mut tokens = Vec::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        tokens.push(match ch {
            '\\' => {
                let Some(next) = chars.next() else {
                    // SQLite `LIKE ... ESCAPE '\\'` rejects an unpaired
                    // trailing escape instead of discarding it.
                    return false;
                };
                Token::Scalar(next)
            }
            '%' => Token::Any,
            '_' => Token::One,
            other => Token::Scalar(other),
        });
    }

    let haystack = haystack.chars().collect::<Vec<_>>();
    let (mut at, mut next) = (0, 0);
    // (token index just after the last `%`, haystack index it is retried from)
    let mut restart: Option<(usize, usize)> = None;
    while at < haystack.len() {
        match tokens.get(next) {
            Some(Token::One) => {
                at += 1;
                next += 1;
            }
            Some(Token::Scalar(ch)) if *ch == haystack[at] => {
                at += 1;
                next += 1;
            }
            Some(Token::Any) => {
                next += 1;
                restart = Some((next, at));
            }
            _ => match restart {
                Some((after, from)) => {
                    // Let the last `%` swallow one more scalar and retry.
                    restart = Some((after, from + 1));
                    at = from + 1;
                    next = after;
                }
                None => return false,
            },
        }
    }
    tokens[next..].iter().all(|token| *token == Token::Any)
}

#[cfg(test)]
mod tests {
    use super::like_matches;

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
