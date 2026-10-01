//! Page-name identity shared by refs::page_key and the synchronous WASM adapter.
//! O(name bytes), no I/O or failure. Rust Unicode whitespace, contextual lowercase,
//! one slash at each boundary, then NFC; display spellings are never changed.

use unicode_normalization::UnicodeNormalization;

/// The ONE page-name identity key: trimmed + **Unicode** lowercase + NFC (the
/// OG/Logseq fold). Use this — never a bare
/// `to_ascii_lowercase`/`eq_ignore_ascii_case` on a
/// page name — so the ref/backlink index and the file/cache resolution agree on
/// identity (a non-ASCII name like `Über` must resolve the same everywhere). Display
/// uses the original casing.
pub fn page_key(name: &str) -> String {
    // Preserve Tine's historical surrounding-whitespace tolerance. Otherwise
    // this is OG page-name-sanity-lc: lowercase, remove one slash at each
    // boundary, then NFC (never NFKC or accent folding).
    let trimmed = name.trim();
    // ASCII is already NFC; boundary slashes cannot affect ASCII case folding.
    // Allocate only the returned key, including on hot namespace lookups.
    if trimmed.is_ascii() {
        let without_leading = trimmed.strip_prefix('/').unwrap_or(trimmed);
        return without_leading
            .strip_suffix('/')
            .unwrap_or(without_leading)
            .to_ascii_lowercase();
    }
    let lowered = trimmed.to_lowercase();
    let without_leading = lowered.strip_prefix('/').unwrap_or(&lowered);
    let without_boundaries = without_leading.strip_suffix('/').unwrap_or(without_leading);
    without_boundaries.nfc().collect()
}
