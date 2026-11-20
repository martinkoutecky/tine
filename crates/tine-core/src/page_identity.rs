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
        return strip_boundary_slashes(trimmed).to_ascii_lowercase();
    }
    fold_literal(trimmed, true, true)
}

/// Strip the opening and closing slash of trimmed page-key input.
pub(crate) fn strip_boundary_slashes(text: &str) -> &str {
    let text = text.strip_prefix('/').unwrap_or(text);
    text.strip_suffix('/').unwrap_or(text)
}

/// Lowercase and compose one page-key literal. Only literals at a text
/// boundary can lose one slash.
pub(crate) fn fold_literal(text: &str, first: bool, last: bool) -> String {
    let lowered = text.to_lowercase();
    let text = if first {
        lowered.strip_prefix('/').unwrap_or(&lowered)
    } else {
        &lowered
    };
    let text = if last {
        text.strip_suffix('/').unwrap_or(text)
    } else {
        text
    };
    text.nfc().collect()
}
