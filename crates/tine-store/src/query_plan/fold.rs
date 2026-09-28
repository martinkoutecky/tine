use super::*;

/// Search-folded text plus one original UTF-16 span per output scalar.
/// The first pass normalizes each extended grapheme cluster and maps output
/// scalars to their original spans. A final composition pass can join adjacent
/// source graphemes, including compatibility-decomposed Hangul Jamo, and unions
/// their spans.
fn folded_with_map(original: &str, remove_accents: bool) -> (Vec<char>, Vec<MatchSpan>) {
    let (folded, map) = if remove_accents {
        tine_core::search_query::canonical_fold_with_map(original)
    } else {
        tine_core::search_query::literal_fold_with_map(original)
    };
    (
        folded.chars().collect(),
        map.into_iter()
            .map(|span| MatchSpan {
                start: span.start,
                end: span.end,
            })
            .collect(),
    )
}

fn folded_chars(value: &str, remove_accents: bool) -> Vec<char> {
    (if remove_accents {
        canonical_fold(value)
    } else {
        literal_fold(value)
    })
    .chars()
    .collect()
}

fn merge_spans(spans: impl IntoIterator<Item = MatchSpan>) -> Vec<MatchSpan> {
    let mut out: Vec<MatchSpan> = Vec::new();
    for span in spans {
        if let Some(last) = out.last_mut() {
            if last.end == span.start {
                last.end = span.end;
                continue;
            }
            if *last == span {
                continue;
            }
        }
        out.push(span);
        if out.len() == MAX_EVIDENCE_SPANS {
            break;
        }
    }
    out
}

pub(super) fn casefold_substring_spans(
    original: &str,
    needle: &str,
    remove_accents: bool,
) -> Vec<MatchSpan> {
    let (hay, map) = folded_with_map(original, remove_accents);
    let needle = folded_chars(needle, remove_accents);
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for start in 0..=hay.len() - needle.len() {
        if hay[start..start + needle.len()] == needle {
            let first = map[start];
            let last = map[start + needle.len() - 1];
            spans.push(MatchSpan {
                start: first.start,
                end: last.end,
            });
            if spans.len() == MAX_EVIDENCE_SPANS {
                break;
            }
        }
    }
    spans
}

pub(super) fn fuzzy_evidence(
    pred: &TextPredicate,
    original: &str,
    remove_accents: bool,
) -> Option<MatchEvidence> {
    let (hay, map) = folded_with_map(original, remove_accents);
    let needle = folded_chars(&pred.value, remove_accents);
    if needle.is_empty() {
        return Some(MatchEvidence {
            clause_id: pred.clause_id,
            field: pred.field,
            mode: pred.mode,
            spans: Vec::new(),
            score: Some(0),
        });
    }
    if needle.len() <= hay.len() {
        for start in 0..=hay.len() - needle.len() {
            if hay[start..start + needle.len()] == needle {
                let first = map[start];
                let last = map[start + needle.len() - 1];
                return Some(MatchEvidence {
                    clause_id: pred.clause_id,
                    field: pred.field,
                    mode: pred.mode,
                    spans: vec![MatchSpan {
                        start: first.start,
                        end: last.end,
                    }],
                    score: Some(if start == 0 { 1000 } else { 500 }),
                });
            }
        }
    }
    let mut at = 0;
    let mut picked = Vec::new();
    for (i, ch) in hay.iter().enumerate() {
        if needle.get(at) == Some(ch) {
            picked.push(map[i]);
            at += 1;
            if at == needle.len() {
                return Some(MatchEvidence {
                    clause_id: pred.clause_id,
                    field: pred.field,
                    mode: pred.mode,
                    spans: merge_spans(picked),
                    score: Some(100),
                });
            }
        }
    }
    None
}
