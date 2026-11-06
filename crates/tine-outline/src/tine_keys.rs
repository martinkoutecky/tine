//! Tine's own property keys (D-20261102-PNS, Discussion #825): every property
//! with a Tine-specific meaning is `tine.<name>`, the dot form of OG's own
//! `logseq.order-list-type::`, so it never collides with a user's property.
//!
//! The page markers Tine wrote on the stable line in a slash form
//! (`tine/favorites:: true` …) are still read: [`LEGACY_KEY_ALIASES`] is the one
//! alias table every reader (native and, through `lsdoc-wasm`, the window)
//! consults. Tine writes only the canonical spelling. Keys that existed only in
//! Betas (`quote::`, `quote-prefix::`, `quote-suffix::`, `author::`, `export::`)
//! were renamed with no alias, because a reader for the old name would keep the
//! collision with users' own `quote::` / `author::` / `export::` properties.

use crate::property_visibility::normalize;

/// A margin comment's quoted text (ADR 0075; was the Beta-only `quote::`).
pub const COMMENT_QUOTE: &str = "tine.quote";
/// The quote's left context (was the Beta-only `quote-prefix::`).
pub const COMMENT_QUOTE_PREFIX: &str = "tine.quote-prefix";
/// The quote's right context (was the Beta-only `quote-suffix::`).
pub const COMMENT_QUOTE_SUFFIX: &str = "tine.quote-suffix";
/// Who wrote a comment, reply or CLI-added block (was the Beta-only `author::`).
pub const AUTHOR: &str = "tine.author";
/// A folder export definition's destination (was the Beta-only `export::`).
pub const EXPORT: &str = "tine.export";

/// The Favorites arrangement page's marker (stable wrote `tine/favorites`).
pub const FAVORITES_MARKER: &str = "tine.favorites";
/// Today's sections page's marker (stable wrote `tine/journal-sections`).
pub const JOURNAL_SECTIONS_MARKER: &str = "tine.journal-sections";
/// The exports page's marker (`tine/exports`; listed with the stable markers by
/// D-20261102-PNS, so its slash spelling is read too).
pub const EXPORTS_MARKER: &str = "tine.exports";

/// `(legacy spelling, canonical key)`, both normalized. The ONE alias table.
pub const LEGACY_KEY_ALIASES: &[(&str, &str)] = &[
    ("tine/favorites", FAVORITES_MARKER),
    ("tine/journal-sections", JOURNAL_SECTIONS_MARKER),
    ("tine/exports", EXPORTS_MARKER),
];

/// `key` normalized ([`normalize`]), with a legacy spelling mapped to the key
/// it stands for. O(key length).
pub fn canonical_key(key: &str) -> String {
    let norm = normalize(key);
    LEGACY_KEY_ALIASES
        .iter()
        .find(|(legacy, _)| *legacy == norm)
        .map(|(_, canonical)| (*canonical).to_string())
        .unwrap_or(norm)
}

/// Every spelling a reader accepts for `canonical`: the key itself first, then
/// its legacy spellings. For queries that match keys literally.
pub fn spellings(canonical: &str) -> Vec<&str> {
    std::iter::once(canonical)
        .chain(
            LEGACY_KEY_ALIASES
                .iter()
                .filter(|(_, c)| *c == canonical)
                .map(|(legacy, _)| *legacy),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_marker_spellings_read_as_the_dot_key() {
        assert_eq!(canonical_key("tine/favorites"), "tine.favorites");
        assert_eq!(
            canonical_key(" Tine/Journal_Sections "),
            "tine.journal-sections"
        );
        assert_eq!(canonical_key("tine/exports"), "tine.exports");
        assert_eq!(canonical_key("tine.exports"), "tine.exports");
        assert_eq!(spellings(EXPORTS_MARKER), ["tine.exports", "tine/exports"]);
        assert_eq!(spellings(AUTHOR), ["tine.author"]);
    }

    #[test]
    fn beta_only_keys_have_no_alias() {
        // A reader for the old name would keep the collision (D-20261102-PNS).
        for old in ["quote", "quote-prefix", "quote-suffix", "author", "export"] {
            assert_eq!(canonical_key(old), old);
        }
        for (legacy, canonical) in LEGACY_KEY_ALIASES {
            assert!(canonical.starts_with("tine."), "{canonical}");
            assert_eq!(normalize(legacy), *legacy);
        }
    }
}
