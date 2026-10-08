//! Per-block candidate signatures for the plain-text (unlinked) reference
//! matcher (GH #623, I-12, I-25).
//!
//! A `BlockSignature` is a 256-bit, two-hash bloom filter over the adjacent
//! character pairs of one block's source text (plus single non-ASCII
//! characters), taken in the *key* alphabet below. `ReferenceFilter` turns the
//! page names of one query into the same kind of mask; a block can only contain
//! a match when its signature covers a name's mask. Unlinked references on
//! Ellis's 440k-block graph used to read every block of every page for any name
//! with a non-ASCII alias or a common word; with the signatures only blocks that
//! may match are touched, and the verdict of the exact matcher (`plain_match`)
//! is unchanged because the filter has no false negatives.
//!
//! # Why there are no false negatives
//!
//! The matcher accepts a span `s` of the block text exactly when
//! `nfc(lowercase(s)) == nfc(lowercase(name))`. Define the key sequence
//! `K(x)` as the concatenation, over the characters `c` of `x`, of the
//! characters of `nfd(lowercase(c))` whose combining class is 0, with the two
//! sigmas identified:
//!
//! * `lowercase` is context-free per character except for the final sigma,
//!   which the sigma identification removes, so `K` is a per-character map and
//!   `K(s)` is a contiguous run of `K(block text)` whenever `s` is a substring
//!   of the block text;
//! * `nfd(nfc(y)) == nfd(y)`, and canonical reordering moves only characters
//!   with a non-zero combining class, so the combining-class-0 characters of
//!   `nfd(nfc(lowercase(x)))` are exactly `K(x)`.
//!
//! Hence `F(s) == F(name)` implies `K(s) == K(name)`, so every key of `K(name)`
//! and every adjacent pair of it occurs in `K(block text)`. A name with no key
//! at all makes the whole filter vacuous and callers must scan everything.
//! The property test in `plain_match` checks all of this against the verbatim original matcher on hostile Unicode.

use unicode_normalization::char::{canonical_combining_class, decompose_canonical};

const WORDS: usize = 4;
const BITS: u64 = (WORDS as u64) * 64;

/// 40 bytes per block: the 256-bit pair/unigram bloom, and one word marking
/// which ASCII keys occur (it is what lets a one-letter name prune at all).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct BlockSignature {
    bloom: [u64; WORDS],
    ascii: u64,
}

impl BlockSignature {
    /// Whether every bit of `mask` is set here.
    pub fn covers(&self, mask: &BlockSignature) -> bool {
        self.ascii & mask.ascii == mask.ascii
            && self
                .bloom
                .iter()
                .zip(&mask.bloom)
                .all(|(have, need)| have & need == *need)
    }

    fn set_feature(&mut self, feature: u64) {
        if feature & ASCII_TAG != 0 {
            self.ascii |= 1u64 << (feature % 64);
            return;
        }
        let hash = mix(feature);
        for bit in [hash % BITS, (hash >> 32) % BITS] {
            self.bloom[(bit / 64) as usize] |= 1u64 << (bit % 64);
        }
    }

    /// The signature of one block's source text.
    pub fn of_text(text: &str) -> Self {
        let mut signature = Self::default();
        for_each_feature(text, |feature| signature.set_feature(feature));
        signature
    }
}

/// SplitMix64 finalizer.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

const UNIGRAM_TAG: u64 = 1 << 60;
const ASCII_TAG: u64 = 1 << 61;

fn pair_feature(a: char, b: char) -> u64 {
    (u64::from(u32::from(a)) << 24) | u64::from(u32::from(b))
}

fn key_char(c: char) -> char {
    if c == '\u{3c2}' {
        '\u{3c3}'
    } else {
        c
    }
}

/// `K(text)`, one key at a time.
fn for_each_key(text: &str, mut emit: impl FnMut(char)) {
    for c in text.chars() {
        if c.is_ascii() {
            emit(c.to_ascii_lowercase());
            continue;
        }
        for lower in c.to_lowercase() {
            decompose_canonical(lower, |d| {
                if canonical_combining_class(d) == 0 {
                    emit(key_char(d));
                }
            });
        }
    }
}

/// Every feature of a block text: adjacent key pairs and non-ASCII keys.
fn for_each_feature(text: &str, mut feature: impl FnMut(u64)) {
    let mut previous: Option<char> = None;
    for_each_key(text, |key| {
        let code = u64::from(u32::from(key));
        feature(if key.is_ascii() {
            ASCII_TAG | code
        } else {
            UNIGRAM_TAG | code
        });
        if let Some(before) = previous {
            feature(pair_feature(before, key));
        }
        previous = Some(key);
    });
}

/// The features a block must have to contain a match of `name`; `None` when
/// the name offers none (no key at all). Every key of `K(name)` must occur in
/// the block (a unigram feature) and so must every adjacent pair: the extra
/// unigram bits are what keep a two-character CJK name from being admitted by
/// every block that merely holds a few of its pairs' hash bits.
fn required_mask(name: &str) -> Option<BlockSignature> {
    let mut keys = Vec::new();
    for_each_key(name, |key| keys.push(key));
    if keys.is_empty() {
        return None;
    }
    let mut mask = BlockSignature::default();
    for key in &keys {
        let code = u64::from(u32::from(*key));
        mask.set_feature(if key.is_ascii() {
            ASCII_TAG | code
        } else {
            UNIGRAM_TAG | code
        });
    }
    for pair in keys.windows(2) {
        mask.set_feature(pair_feature(pair[0], pair[1]));
    }
    Some(mask)
}

/// The query's masks, one per page name (a block may match any of them).
#[derive(Clone, Debug)]
pub struct ReferenceFilter {
    masks: Vec<BlockSignature>,
}

impl ReferenceFilter {
    /// `None` when some name cannot be filtered: the caller must then treat
    /// every block as a candidate.
    pub fn new(names: &[String]) -> Option<Self> {
        let masks = names
            .iter()
            .map(|name| required_mask(name))
            .collect::<Option<Vec<_>>>()?;
        (!masks.is_empty()).then_some(Self { masks })
    }

    /// Whether the block signature may belong to a block containing a match.
    pub fn admits(&self, signature: &BlockSignature) -> bool {
        self.masks.iter().any(|mask| signature.covers(mask))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_identify_case_sigma_marks_and_compatibility_singletons() {
        let keys = |s: &str| {
            let mut out = String::new();
            for_each_key(s, |k| out.push(k));
            out
        };
        assert_eq!(keys("Kelp"), "kelp");
        assert_eq!(keys("\u{212a}elp"), "kelp");
        assert_eq!(keys("ΟΔΟΣ"), keys("οδος"));
        assert_eq!(keys("e\u{301}"), "e");
        assert_eq!(keys("\u{e9}"), "e");
        assert_eq!(keys("\u{f900}"), keys("\u{8c48}"));
        // Hangul syllable and its jamo agree.
        assert_eq!(keys("한"), keys("\u{1112}\u{1161}\u{11ab}"));
    }

    #[test]
    fn a_name_is_admitted_by_a_block_that_contains_it() {
        let block = BlockSignature::of_text("see the Kelp fjord 7f23 notes");
        let name = |n: &str| ReferenceFilter::new(&[n.to_string()]).unwrap();
        assert!(name("kelp fjord").admits(&block));
        assert!(!name("zzqx probe").admits(&block));
        // One letter prunes by the ASCII word.
        assert!(name("k").admits(&block));
        assert!(!name("q").admits(&block));
        assert!(ReferenceFilter::new(&[String::new()]).is_none());
        assert!(ReferenceFilter::new(&["kelp".to_string(), String::new()]).is_none());
        assert!(ReferenceFilter::new(&[]).is_none());
    }
}
