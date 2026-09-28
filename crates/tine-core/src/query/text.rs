//! Exact text helpers the structured-query evaluator shares.
//!
//! The source limits (`QUERY_SOURCE_MAX_BYTES`, nesting) live in the parent
//! `query` module, og's one answerer for them; master's SQL framing helpers
//! and raw-path visible projections are execution-side (SQL) and not ported.

/// SQL `LIKE` over an already-folded haystack. `%` matches any run, `_` one
/// scalar, and `\` escapes the following scalar.
pub fn like_matches(haystack: &str, pattern: &str) -> bool {
    #[derive(Debug)]
    enum Part {
        Literal(String),
        Any,
        One,
    }

    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut chars = pattern.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let Some(next) = chars.next() else {
                    // SQLite `LIKE ... ESCAPE '\\'` rejects an unpaired
                    // trailing escape instead of discarding it.
                    return false;
                };
                literal.push(next);
            }
            '%' | '_' => {
                if !literal.is_empty() {
                    parts.push(Part::Literal(std::mem::take(&mut literal)));
                }
                parts.push(if ch == '%' { Part::Any } else { Part::One });
            }
            other => literal.push(other),
        }
    }
    if !literal.is_empty() {
        parts.push(Part::Literal(literal));
    }

    let haystack = haystack.chars().collect::<Vec<_>>();
    fn matches(parts: &[Part], haystack: &[char], at: usize) -> bool {
        match parts.first() {
            None => at == haystack.len(),
            Some(Part::One) => at < haystack.len() && matches(&parts[1..], haystack, at + 1),
            Some(Part::Any) => {
                (at..=haystack.len()).any(|next| matches(&parts[1..], haystack, next))
            }
            Some(Part::Literal(text)) => {
                let literal = text.chars().collect::<Vec<_>>();
                haystack.get(at..at.saturating_add(literal.len())) == Some(literal.as_slice())
                    && matches(&parts[1..], haystack, at + literal.len())
            }
        }
    }
    matches(&parts, &haystack, 0)
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
}
