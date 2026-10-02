//! The plain-text (unlinked) reference matcher: the one answerer of "where in
//! this source span does this page name occur outside explicit link syntax"
//! (I-12). A match is a span whose `nfc(lowercase(span))` equals the folded
//! name, starting and ending on grapheme boundaries (I-4), then filtered by the
//! OG edge rules (`og_prefix_allows`, adjacent ASCII alphanumerics).
//!
//! GH #623 made this the hot loop of every unlinked-references query (Ellis's
//! 440k-block graph spent 0.5-3.7 s here), so the scan is built to touch each
//! source byte about once and to allocate nothing per block:
//!
//! * The name is folded once per query (`Needle`), not once per block.
//! * An ASCII name can only be matched by ASCII characters and by the three
//!   non-ASCII characters whose NFC-lowercase form is a single ASCII character
//!   (`fold_non_ascii`); candidate starts are found with `memchr`.
//! * A non-ASCII name finds its candidate starts by the necessary start-
//!   character condition (`match_may_start_with`), with a lazily built
//!   per-256-character table of characters for which that condition is just
//!   `c == base`.
//! * Every candidate is verified without allocation when the source spells
//!   the name literally (up to ASCII case), and by the original
//!   allocation-per-character comparison otherwise.
//!
//! The accepted spans are exactly those of the original per-grapheme scan; the
//! tests below keep that matcher verbatim as the oracle.

use memchr::{memchr2, memchr3};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// One page name, folded and classified once per query.
pub(super) struct Needle {
    /// `nfc(lowercase(name))`, the string a source span must fold to.
    folded: String,
    first_requires_boundary: bool,
    last_requires_boundary: bool,
    /// First character of `nfd(folded)` when it is a starter (see
    /// `match_may_start_with`); `None` disables start-character rejection.
    base: Option<char>,
    ascii: bool,
    /// Every char prefix `p` of `folded` satisfies `nfc(lowercase(p)) == p`,
    /// which makes a literal (up to ASCII case) spelling of `folded` in the
    /// source provably a match without running the allocating comparison.
    literal_ok: bool,
    /// Memo of `prefix_may_start` for UTF-8 lead bytes `0xC2..=0xEF` (see there).
    prefix_known: [AtomicU64; PREFIX_LEADS],
    prefix_value: [AtomicU64; PREFIX_LEADS],
}

/// Lead bytes `0xC2..=0xEF`: two- and three-byte UTF-8 sequences.
const PREFIX_LEADS: usize = 0xF0 - 0xC2;

/// Longest folded name for which `literal_ok` is computed (it is quadratic).
const LITERAL_CHECK_MAX_BYTES: usize = 512;

impl Needle {
    /// `None` for an empty name, which matches nothing.
    pub(super) fn new(name: &str) -> Option<Self> {
        if name.is_empty() {
            return None;
        }
        let folded: String = name.to_lowercase().nfc().collect();
        let first_requires_boundary = folded.chars().next().is_some_and(|ch| ch.is_alphanumeric());
        let last_requires_boundary = folded
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric());
        let base = needle_base_starter(&folded);
        let ascii = folded.is_ascii();
        let literal_ok = !ascii
            && folded.len() <= LITERAL_CHECK_MAX_BYTES
            && folded.char_indices().all(|(index, ch)| {
                let prefix = &folded[..index + ch.len_utf8()];
                prefix.to_lowercase().nfc().eq(prefix.chars())
            });
        Some(Self {
            folded,
            first_requires_boundary,
            last_requires_boundary,
            base,
            ascii,
            literal_ok,
            prefix_known: [const { AtomicU64::new(0) }; PREFIX_LEADS],
            prefix_value: [const { AtomicU64::new(0) }; PREFIX_LEADS],
        })
    }

    /// Whether any character whose UTF-8 encoding begins `lead, second`
    /// (exactly one for a two-byte lead, 64 for a three-byte lead) may start a
    /// match. Memoized per prefix: CJK and kana text touches a few hundred
    /// prefixes, so a query pays for the prefixes its text uses, not for the
    /// 65 thousand characters of the three-byte range.
    fn prefix_may_start(&self, lead: u8, second: u8) -> bool {
        let slot = usize::from(lead - 0xC2);
        let bit = 1u64 << (second & 0x3F);
        if self.prefix_known[slot].load(Ordering::Relaxed) & bit != 0 {
            return self.prefix_value[slot].load(Ordering::Relaxed) & bit != 0;
        }
        let high = u32::from(second & 0x3F) << 6;
        let may = if lead < 0xE0 {
            char::from_u32((u32::from(lead & 0x1F) << 6) | u32::from(second & 0x3F))
                .is_some_and(|ch| self.may_start(ch))
        } else {
            let base = (u32::from(lead & 0x0F) << 12) | high;
            // Unassigned or surrogate code points are not source characters.
            (0..64).any(|low| char::from_u32(base | low).is_some_and(|ch| self.may_start(ch)))
        };
        if may {
            self.prefix_value[slot].fetch_or(bit, Ordering::Relaxed);
        }
        self.prefix_known[slot].fetch_or(bit, Ordering::Relaxed);
        may
    }

    /// Necessary condition for a match to start at source character `ch`.
    fn may_start(&self, ch: char) -> bool {
        let Some(base) = self.base else {
            return true;
        };
        if ch.is_ascii() {
            return ch.to_ascii_lowercase() == base;
        }
        if is_start_inert(ch) {
            return ch == base;
        }
        match_may_start_with(ch, Some(base))
    }
}

/// The first character of `nfd(needle)` when it is a starter (combining class
/// 0), else `None` (no start-character pre-rejection is sound then).
fn needle_base_starter(needle: &str) -> Option<char> {
    let base = needle.chars().next()?.nfd().next()?;
    (canonical_combining_class(base) == 0).then_some(base)
}

/// Necessary condition for a match to start at a source character `first`: a
/// match requires `nfc(lower(span)) == needle`, hence
/// `nfd(lower(span)) == nfd(needle)`. Canonical reordering never moves a
/// starter, so when the first character of `nfd(lower(first))` is a starter it
/// is also the first character of `nfd(lower(span))` and must equal the
/// needle's base. A non-starter there (an orphan combining mark) keeps the
/// candidate, so only provably impossible starts are skipped and the accepted
/// matches are exactly those of the unfiltered scan.
fn match_may_start_with(first: char, needle_base: Option<char>) -> bool {
    let Some(base) = needle_base else {
        return true;
    };
    if first.is_ascii() {
        // ASCII has no decompositions and lowercases within ASCII.
        return first.to_ascii_lowercase() == base;
    }
    match first.to_lowercase().nfd().next() {
        Some(starter) => canonical_combining_class(starter) != 0 || starter == base,
        None => true,
    }
}

/// `c` is a starter whose lowercase-then-decomposition begins with `c` itself,
/// so `match_may_start_with(c, Some(base))` is exactly `c == base`.
fn is_start_inert_slow(c: char) -> bool {
    canonical_combining_class(c) == 0 && c.to_lowercase().nfd().next() == Some(c)
}

/// `is_start_inert_slow`, memoized for the Basic Multilingual Plane in 256
/// character pages built on first touch (about 15 us each): CJK and kana text
/// touches a handful of pages, so no process pays for the whole table.
fn is_start_inert(c: char) -> bool {
    static PAGES: [OnceLock<[u64; 4]>; 256] = [const { OnceLock::new() }; 256];
    let code = c as u32;
    if code >= 0x1_0000 {
        return is_start_inert_slow(c);
    }
    let page = PAGES[(code >> 8) as usize].get_or_init(|| {
        let mut bits = [0u64; 4];
        for low in 0..256u32 {
            let inert = char::from_u32((code & !0xff) | low).is_some_and(is_start_inert_slow);
            if inert {
                bits[(low >> 6) as usize] |= 1 << (low & 63);
            }
        }
        bits
    });
    let low = code & 0xff;
    page[(low >> 6) as usize] >> (low & 63) & 1 == 1
}

/// The ASCII byte a non-ASCII source character folds to under
/// `nfc(lowercase(.))`, if any. Exhaustively checked against the real fold by
/// `non_ascii_fold_table_is_exhaustive`.
fn fold_non_ascii(ch: char) -> Option<u8> {
    match ch {
        '\u{212A}' => Some(b'k'),
        '\u{37E}' => Some(b';'),
        '\u{1FEF}' => Some(b'`'),
        _ => None,
    }
}

/// UTF-8 lead byte of the non-ASCII character that folds to ASCII `byte`.
fn non_ascii_fold_lead(byte: u8) -> Option<u8> {
    match byte {
        b'k' => Some(0xE2),
        b';' => Some(0xCD),
        b'`' => Some(0xE1),
        _ => None,
    }
}

/// Whether `at` (a char boundary of `source`) is a grapheme boundary.
fn is_grapheme_boundary(source: &str, at: usize) -> bool {
    let bytes = source.as_bytes();
    if at == 0 || at >= bytes.len() {
        return true;
    }
    let (before, after) = (bytes[at - 1], bytes[at]);
    if before < 0x80 && after < 0x80 {
        return !(before == b'\r' && after == b'\n');
    }
    GraphemeCursor::new(at, bytes.len(), true)
        .is_boundary(source, 0)
        .expect("the whole source is the chunk")
}

/// The original comparison: grow the candidate one character at a time,
/// folding the whole candidate each step, until it equals `needle` at a grapheme
/// edge (I-4) or can no longer be a prefix of it. Allocates per character, so
/// it only runs for candidates the cheaper checks cannot decide.
fn slow_match_len(source: &str, offset: usize, needle: &str) -> Option<usize> {
    let mut candidate_raw = String::new();
    let mut boundaries = source[offset..]
        .grapheme_indices(true)
        .map(|(relative, grapheme)| relative + grapheme.len());
    let mut boundary = boundaries.next().expect("nonempty suffix");
    for (relative, ch) in source[offset..].char_indices() {
        candidate_raw.push(ch);
        let end = relative + ch.len_utf8();
        if end > boundary {
            boundary = boundaries.next().expect("next grapheme");
        }
        let candidate: String = candidate_raw.to_lowercase().nfc().collect();
        if candidate == needle && end == boundary {
            return Some(end);
        }
        // Accept only at a grapheme edge (I-4), while rejecting incompatible
        // prefixes early without allocating an arbitrarily long grapheme.
        let without_last = candidate
            .char_indices()
            .next_back()
            .map_or("", |(index, _)| &candidate[..index]);
        if !needle.starts_with(&candidate) && !needle.starts_with(without_last) {
            return None;
        }
    }
    None
}

/// Length of the match of `needle` starting at char boundary `offset`, with
/// both ends on grapheme boundaries, for a non-ASCII name.
fn general_match_len(source: &str, offset: usize, needle: &Needle) -> Option<usize> {
    if needle.literal_ok {
        let mut chars = source[offset..].chars();
        let mut consumed = 0usize;
        let mut spelled = true;
        for want in needle.folded.chars() {
            let Some(have) = chars.next() else {
                spelled = false;
                break;
            };
            if have == want || (have.is_ascii() && have.to_ascii_lowercase() == want) {
                consumed += have.len_utf8();
            } else if have.is_ascii() && want.is_ascii() {
                // An ASCII character no ASCII-only fold can repair: a later
                // combining mark could only compose it into a non-ASCII
                // character, never into `want`.
                return None;
            } else {
                spelled = false;
                break;
            }
        }
        if spelled && is_grapheme_boundary(source, offset + consumed) {
            return Some(consumed);
        }
    }
    slow_match_len(source, offset, &needle.folded)
}

/// Length of the match of an ASCII `needle` starting at `offset`: every source
/// character must fold to the matching ASCII byte, one for one.
fn ascii_match_len(source: &str, offset: usize, needle: &[u8]) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut pos = offset;
    for &want in needle {
        let &have = bytes.get(pos)?;
        if have < 0x80 {
            if have.to_ascii_lowercase() != want {
                return None;
            }
            pos += 1;
        } else {
            let ch = source[pos..].chars().next()?;
            if fold_non_ascii(ch) != Some(want) {
                return None;
            }
            pos += ch.len_utf8();
        }
    }
    (is_grapheme_boundary(source, offset) && is_grapheme_boundary(source, pos))
        .then_some(pos - offset)
}

/// Visit source-order matches with memory bounded by the target name, not the
/// number or size of matches in the block. Edge rules and accepted spans are
/// identical to the original per-grapheme scan.
pub(super) fn visit_plain_matches(
    raw: &str,
    range: &Range<usize>,
    needle: &Needle,
    mut visit: impl FnMut(Range<usize>) -> bool,
) {
    let Some(source) = raw.get(range.clone()) else {
        return;
    };
    let mut accept = |offset: usize, len: usize| -> bool {
        let start = range.start + offset;
        let end = start + len;
        let before = raw
            .get(..start)
            .and_then(|prefix| prefix.chars().next_back());
        let after = raw.get(end..).and_then(|suffix| suffix.chars().next());
        // Exact OG edge semantics: only adjacent ASCII alphanumerics exclude
        // an unlinked match. `_` and continuous CJK are valid boundaries.
        if super::og_prefix_allows(raw, start)
            && (!needle.first_requires_boundary || !super::is_og_edge_alphanumeric(before))
            && (!needle.last_requires_boundary || !super::is_og_edge_alphanumeric(after))
        {
            return visit(start..end);
        }
        true
    };
    if needle.ascii {
        let name = needle.folded.as_bytes();
        let lower = name[0];
        let upper = lower.to_ascii_uppercase();
        let lead = non_ascii_fold_lead(lower);
        let bytes = source.as_bytes();
        let mut from = 0usize;
        while from < bytes.len() {
            let found = match lead {
                Some(lead) => memchr3(lower, upper, lead, &bytes[from..]),
                None => memchr2(lower, upper, &bytes[from..]),
            };
            let Some(relative) = found else {
                return;
            };
            let offset = from + relative;
            from = offset + 1;
            if let Some(len) = ascii_match_len(source, offset, name) {
                if !accept(offset, len) {
                    return;
                }
            }
        }
        return;
    }
    if needle.base.is_none() {
        for (offset, _) in source.char_indices() {
            if !is_grapheme_boundary(source, offset) {
                continue;
            }
            if let Some(len) = general_match_len(source, offset, needle) {
                if !accept(offset, len) {
                    return;
                }
            }
        }
        return;
    }
    // Byte-level skip to plausible start characters (GH #623): CJK text is
    // three bytes a character, and nearly every character is provably unable
    // to start a match, so decode only where a lead-byte prefix says one can.
    let bytes = source.as_bytes();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let lead = bytes[offset];
        let plausible = if lead < 0x80 {
            needle.may_start(char::from(lead))
        } else if lead < 0xC2 {
            false
        } else if lead < 0xF0 {
            bytes
                .get(offset + 1)
                .is_some_and(|&second| needle.prefix_may_start(lead, second))
        } else {
            true
        };
        if plausible {
            let Some(ch) = source[offset..].chars().next() else {
                return;
            };
            if needle.may_start(ch) && is_grapheme_boundary(source, offset) {
                if let Some(len) = general_match_len(source, offset, needle) {
                    if !accept(offset, len) {
                        return;
                    }
                }
            }
            offset += ch.len_utf8();
        } else {
            offset += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_evidence::{is_og_edge_alphanumeric, og_prefix_allows};

    // GH #623: the start-character pre-rejection must not change which spans
    // match. `reference_visit_plain_matches` is the matcher as it was before the
    // pre-rejection (verbatim); every (text, needle) pair over an alphabet of
    // the awkward cases (case pairs, composed/decomposed accents, final sigma,
    // dotted I, sharp s, Hangul jamo, orphan combining marks, CJK, ZWJ emoji)
    // must give the identical match list.
    fn reference_visit_plain_matches(
        raw: &str,
        range: &Range<usize>,
        needle: &str,
        mut visit: impl FnMut(Range<usize>) -> bool,
    ) {
        let Some(source) = raw.get(range.clone()) else {
            return;
        };
        if needle.is_empty() {
            return;
        }
        let needle: String = needle.to_lowercase().nfc().collect();
        let first_requires_boundary = needle.chars().next().is_some_and(|ch| ch.is_alphanumeric());
        let last_requires_boundary = needle
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_alphanumeric());
        let ascii_first = needle
            .chars()
            .next()
            .filter(char::is_ascii)
            .map(|ch| (ch.to_ascii_lowercase(), ch.to_ascii_uppercase()));
        for (offset, grapheme) in source.grapheme_indices(true) {
            let first = grapheme.chars().next().expect("nonempty grapheme");
            if let Some((lower, upper)) = ascii_first {
                if first.is_ascii() && first != lower && first != upper {
                    continue;
                }
            }
            let start = range.start + offset;
            let mut end = start;
            let mut candidate_raw = String::new();
            let mut matched = false;
            let mut boundaries = source[offset..]
                .grapheme_indices(true)
                .map(|(offset, grapheme)| start + offset + grapheme.len());
            let mut boundary = boundaries.next().expect("nonempty suffix");
            for (relative, ch) in source[offset..].char_indices() {
                candidate_raw.push(ch);
                end = start + relative + ch.len_utf8();
                if end > boundary {
                    boundary = boundaries.next().expect("next grapheme");
                }
                let candidate: String = candidate_raw.to_lowercase().nfc().collect();
                if candidate == needle && end == boundary {
                    matched = true;
                    break;
                }
                // Accept only at a grapheme edge (I-4), while rejecting incompatible
                // prefixes early without allocating an arbitrarily long grapheme.
                let without_last = candidate
                    .char_indices()
                    .next_back()
                    .map_or("", |(index, _)| &candidate[..index]);
                if !needle.starts_with(&candidate) && !needle.starts_with(without_last) {
                    break;
                }
            }
            if !matched {
                continue;
            }
            let before = raw
                .get(..start)
                .and_then(|prefix| prefix.chars().next_back());
            let after = raw.get(end..).and_then(|suffix| suffix.chars().next());
            // Exact OG edge semantics: only adjacent ASCII alphanumerics exclude
            // an unlinked match. `_` and continuous CJK are valid boundaries.
            if og_prefix_allows(raw, start)
                && (!first_requires_boundary || !is_og_edge_alphanumeric(before))
                && (!last_requires_boundary || !is_og_edge_alphanumeric(after))
                && !visit(start..end)
            {
                return;
            }
        }
    }

    fn new_matches(raw: &str, range: &Range<usize>, name: &str) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        if let Some(needle) = Needle::new(name) {
            visit_plain_matches(raw, range, &needle, |range| {
                found.push(range);
                true
            });
        }
        found
    }

    fn old_matches(raw: &str, range: &Range<usize>, name: &str) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        reference_visit_plain_matches(raw, range, name, |range| {
            found.push(range);
            true
        });
        found
    }

    // GH #623: the matcher must accept exactly the spans the original scan
    // accepted. `reference_visit_plain_matches` is that scan (verbatim, as it
    // was before the start-character pre-rejection); every (text, needle) pair
    // over an alphabet of the awkward cases (case pairs, Kelvin and the other
    // ASCII-folding singletons, composed/decomposed accents, final sigma, dotted
    // I, sharp s, Hangul jamo, kana with dakuten, Indic conjuncts, orphan
    // combining marks, CJK, ZWJ and regional-indicator emoji, CR LF) must give
    // the identical match list. A second loop plants (case-mangled) copies of
    // the needle in the text so the literal fast paths see real matches.
    const ALPHABET: &[&str] = &[
        "a",
        "A",
        "e",
        "E",
        "r",
        "R",
        "i",
        "I",
        "s",
        "S",
        "k",
        "K",
        "\u{212A}",
        "\u{37e}",
        "\u{1fef}",
        ";",
        "`",
        "\u{e9}",
        "\u{c9}",
        "e\u{301}",
        "E\u{301}",
        "\u{159}",
        "r\u{30c}",
        "\u{158}",
        "\u{3a3}",
        "\u{3c3}",
        "\u{3c2}",
        "\u{130}",
        "i\u{307}",
        "\u{df}",
        "\u{1e9e}",
        "\u{1c5}",
        "ss",
        "\u{1100}",
        "\u{1161}",
        "\u{11a8}",
        "\u{ac00}",
        "\u{301}",
        "\u{30a}",
        "\u{3099}",
        "\u{306f}",
        "\u{306f}\u{3099}",
        "\u{3070}",
        "\u{ff21}",
        "\u{915}\u{94d}\u{937}",
        "\u{915}\u{94d}",
        "\u{937}",
        "\u{4e2d}",
        "\u{6587}",
        "\u{f900}",
        "\u{8c48}",
        "\u{1f468}\u{200d}\u{1f469}",
        "\u{200d}",
        "\u{1f1e6}",
        "\u{1f1e7}",
        "\u{1f44d}\u{1f3fb}",
        "\u{f8}",
        "\u{d8}",
        "o\u{338}",
        "\u{c5}",
        "A\u{30a}",
        "\u{212b}",
        " ",
        "-",
        "_",
        "1",
        "/",
        "\r\n",
        "\r",
        "\n",
        "[",
        "#",
    ];

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn build(&mut self, max: u64) -> String {
            (0..=self.next() % max)
                .map(|_| ALPHABET[(self.next() % ALPHABET.len() as u64) as usize])
                .collect()
        }
    }

    fn mangle_case(rng: &mut Rng, text: &str) -> String {
        text.chars()
            .map(|ch| match rng.next() % 4 {
                0 => ch.to_uppercase().collect::<String>(),
                1 => ch.to_lowercase().collect::<String>(),
                _ => ch.to_string(),
            })
            .collect()
    }

    #[test]
    fn start_character_prerejection_matches_exactly_what_the_unfiltered_scan_matches() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut matched_any = 0usize;
        let mut matched_literal = 0usize;
        for round in 0..120_000 {
            let needle = rng.build(3);
            let raw = if round % 2 == 0 {
                rng.build(14)
            } else {
                let planted = mangle_case(&mut rng, &needle);
                format!("{}{}{}", rng.build(4), planted, rng.build(4))
            };
            let whole = 0..raw.len();
            let new = new_matches(&raw, &whole, &needle);
            let old = old_matches(&raw, &whole, &needle);
            assert_eq!(new, old, "raw={raw:?} needle={needle:?}");
            matched_any += usize::from(!old.is_empty());
            matched_literal += usize::from(!old.is_empty() && round % 2 == 1);
        }
        assert!(
            matched_any > 5_000,
            "alphabet must produce matches: {matched_any}"
        );
        assert!(
            matched_literal > 2_000,
            "planted needles must match: {matched_literal}"
        );
    }

    #[test]
    fn matcher_agrees_with_the_original_on_sub_ranges_and_long_needles() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for _ in 0..30_000 {
            let needle = rng.build(7);
            let planted = mangle_case(&mut rng, &needle);
            let raw = format!("{}{}{}", rng.build(6), planted, rng.build(6));
            // Any char-boundary window is a legal plain range (parser claims
            // split a block into several).
            let bounds: Vec<usize> = raw
                .char_indices()
                .map(|(i, _)| i)
                .chain([raw.len()])
                .collect();
            let a = bounds[(rng.next() % bounds.len() as u64) as usize];
            let b = bounds[(rng.next() % bounds.len() as u64) as usize];
            let range = a.min(b)..a.max(b);
            assert_eq!(
                new_matches(&raw, &range, &needle),
                old_matches(&raw, &range, &needle),
                "raw={raw:?} range={range:?} needle={needle:?}"
            );
        }
    }

    #[test]
    fn matcher_agrees_with_the_original_on_realistic_names() {
        let names = [
            "lantern",
            "Lantern Kelp",
            "harbor 55f2",
            "k",
            "K",
            "a;",
            "x`y",
            "Anchor \u{5287}\u{5352}",
            "\u{5662}\u{57f6}",
            "\u{5662}",
            "caf\u{e9}",
            "cafe\u{301}",
            "stra\u{df}e",
            "\u{3a3}\u{3b1}\u{3c2}",
            "\u{c5}ngstr\u{f6}m",
            "ab\u{3099}",
            "\u{306f}\u{3099}",
            // Ellis-shaped corpus names (ds/ellis-graph): hex-suffixed ASCII
            // phrases, hyphenated slugs, one-word and CJK-bigram names.
            "Basalt isle glacier 4b6",
            "pewter-upland-knoll-flint 27a1",
            "Kelp fjord 7f23",
            "Wren-haven 5233",
            "kelp",
            "mesa",
            "\u{4fde}\u{5765}\u{56e0}",
            "\u{5ec3}\u{5723}",
            "\u{564d}\u{57f6}\u{4f37}",
            "Pewter \u{535e}\u{53dc}\u{5a27}",
        ];
        let texts = [
            "The Lantern kelp and LANTERN KELP, lanterns; lantern_x x_lantern 1lantern",
            "harbor 55f2 Harbor 55F2 harbor 55f2x",
            "k K \u{212a} \u{212A}elvin kK",
            "a; A\u{37e} x`y x\u{1fef}Y",
            "Anchor \u{5287}\u{5352} anchor \u{5287}\u{5352}\u{5287} ANCHOR \u{5287}",
            "\u{5662}\u{57f6}\u{4f37} \u{5662} \u{f900}\u{8c48} \u{5662}\u{57f6}",
            "caf\u{e9} cafe\u{301} CAF\u{c9} cafe\u{301}\u{301} caf\u{e9}\u{301}",
            "stra\u{df}e STRASSE strasse STRA\u{1e9e}E",
            "\u{3a3}\u{391}\u{3a3} \u{3c3}\u{3b1}\u{3c2} \u{3a3}\u{3b1}\u{3a3} \u{3c3}\u{3b1}\u{3c3}",
            "\u{c5}ngstr\u{f6}m A\u{30a}ngstro\u{308}m \u{212b}ngstr\u{f6}m",
            "\u{3070} \u{306f}\u{3099} \u{306f} ab\u{3099} \u{3070}\u{3099}",
            "see Basalt Isle Glacier 4B6, basalt isle glacier 4b60; Kelp Fjord 7F23 mesa Mesas MESA",
            "pewter-upland-knoll-flint 27a1 Pewter-Upland-Knoll-Flint 27A1 wren-haven 5233 Wren-Haven 5233x kelp",
            "\u{4fde}\u{5765}\u{56e0}\u{5ec3}\u{5723} \u{5ec3}\u{5723}\u{5ec3} \u{564d}\u{57f6}\u{4f37} Pewter \u{535e}\u{53dc}\u{5a27}",
        ];
        for text in texts {
            for name in names {
                let whole = 0..text.len();
                assert_eq!(
                    new_matches(text, &whole, name),
                    old_matches(text, &whole, name),
                    "text={text:?} name={name:?}"
                );
            }
        }
    }

    /// The ASCII fast path is only sound if the ASCII characters and the three
    /// singletons in `fold_non_ascii` are the only characters whose
    /// `nfd(lowercase(.))` is entirely ASCII, and each folds to one byte.
    #[test]
    fn non_ascii_fold_table_is_exhaustive() {
        for code in 0..=0x10_FFFFu32 {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let folded: String = ch.to_string().to_lowercase().nfc().collect();
            let decomposed: String = ch.to_string().to_lowercase().nfd().collect();
            let ascii_only = decomposed.is_ascii();
            if ch.is_ascii() {
                assert_eq!(folded, ch.to_ascii_lowercase().to_string());
                continue;
            }
            match fold_non_ascii(ch) {
                Some(byte) => {
                    assert_eq!(folded.as_bytes(), [byte], "U+{code:04X}");
                    assert_eq!(
                        non_ascii_fold_lead(byte),
                        Some(ch.to_string().as_bytes()[0])
                    );
                }
                None => assert!(!ascii_only, "U+{code:04X} folds to ASCII {decomposed:?}"),
            }
        }
    }

    /// `is_start_inert` replaces `match_may_start_with` by `c == base`; check
    /// the memoized table against the real predicate for every scalar value and
    /// that the replacement is exact.
    #[test]
    fn start_inert_table_is_exact() {
        let bases = [
            Some('a'),
            Some('\u{4e2d}'),
            Some('\u{5662}'),
            Some('\u{306f}'),
            None,
        ];
        for code in 0..=0x10_FFFFu32 {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            assert_eq!(is_start_inert(ch), is_start_inert_slow(ch), "U+{code:04X}");
            if is_start_inert(ch) && !ch.is_ascii() {
                for base in bases {
                    let expected = base.is_none_or(|base| base == ch);
                    assert_eq!(match_may_start_with(ch, base), expected, "U+{code:04X}");
                }
            }
        }
    }

    #[test]
    fn grapheme_boundary_helper_agrees_with_segmentation() {
        let mut rng = Rng(0xdead_beef_cafe_f00d);
        for _ in 0..20_000 {
            let text = rng.build(12);
            let starts: std::collections::BTreeSet<usize> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            for (at, _) in text.char_indices().chain([(text.len(), ' ')]) {
                assert_eq!(
                    is_grapheme_boundary(&text, at),
                    starts.contains(&at),
                    "{text:?} @{at}"
                );
            }
        }
    }

    // GH #623: the block signature filter must admit every block the exact
    // matcher accepts a span in (no false negatives), on the same hostile
    // alphabet, over whole blocks and sub-ranges, and on the realistic names.
    fn filter_admits(raw: &str, name: &str) -> bool {
        crate::reference_evidence::ReferenceFilter::new(&[name.to_string()]).is_none_or(|filter| {
            filter.admits(&crate::reference_evidence::BlockSignature::of_text(raw))
        })
    }

    #[test]
    fn signature_filter_never_rejects_a_block_the_matcher_matches() {
        let mut rng = Rng(0x0bad_cafe_f00d_1234);
        let mut matched = 0usize;
        let mut rejected = 0usize;
        for round in 0..120_000 {
            let name = rng.build(4);
            let raw = if round % 2 == 0 {
                rng.build(14)
            } else {
                let planted = mangle_case(&mut rng, &name);
                format!("{}{}{}", rng.build(4), planted, rng.build(4))
            };
            let bounds: Vec<usize> = raw
                .char_indices()
                .map(|(i, _)| i)
                .chain([raw.len()])
                .collect();
            let a = bounds[(rng.next() % bounds.len() as u64) as usize];
            let b = bounds[(rng.next() % bounds.len() as u64) as usize];
            let range = if round % 3 == 0 {
                a.min(b)..a.max(b)
            } else {
                0..raw.len()
            };
            if old_matches(&raw, &range, &name).is_empty() {
                rejected += usize::from(!filter_admits(&raw, &name));
                continue;
            }
            matched += 1;
            assert!(
                filter_admits(&raw, &name),
                "raw={raw:?} range={range:?} name={name:?}"
            );
        }
        assert!(matched > 5_000, "alphabet must produce matches: {matched}");
        assert!(
            rejected > 5_000,
            "the filter must actually reject: {rejected}"
        );
    }

    #[test]
    fn signature_filter_admits_realistic_matches_and_rejects_unrelated_text() {
        let names = [
            "lantern",
            "Lantern Kelp",
            "harbor 55f2",
            "Anchor \u{5287}\u{5352}",
            "\u{5662}\u{57f6}",
            "\u{5662}",
            "caf\u{e9}",
            "cafe\u{301}",
            "stra\u{df}e",
            "\u{3a3}\u{3b1}\u{3c2}",
            "\u{c5}ngstr\u{f6}m",
            "\u{306f}\u{3099}",
        ];
        let texts = [
            "The Lantern kelp and LANTERN KELP, lanterns; lantern_x x_lantern 1lantern",
            "harbor 55f2 Harbor 55F2 harbor 55f2x",
            "Anchor \u{5287}\u{5352} anchor \u{5287}\u{5352}\u{5287} ANCHOR \u{5287}",
            "\u{5662}\u{57f6}\u{4f37} \u{5662} \u{f900}\u{8c48} \u{5662}\u{57f6}",
            "caf\u{e9} cafe\u{301} CAF\u{c9} cafe\u{301}\u{301} caf\u{e9}\u{301}",
            "stra\u{df}e STRASSE strasse STRA\u{1e9e}E",
            "\u{3a3}\u{391}\u{3a3} \u{3c3}\u{3b1}\u{3c2} \u{3a3}\u{3b1}\u{3a3} \u{3c3}\u{3b1}\u{3c3}",
            "\u{c5}ngstr\u{f6}m A\u{30a}ngstro\u{308}m \u{212b}ngstr\u{f6}m",
            "\u{3070} \u{306f}\u{3099} \u{306f} ab\u{3099} \u{3070}\u{3099}",
        ];
        for text in texts {
            for name in names {
                if !old_matches(text, &(0..text.len()), name).is_empty() {
                    assert!(filter_admits(text, name), "text={text:?} name={name:?}");
                }
            }
        }
        assert!(!filter_admits(
            "nothing relevant in this block",
            "lantern kelp"
        ));
        assert!(!filter_admits("\u{4e2d}\u{6587}", "\u{5662}\u{57f6}"));
    }
}
