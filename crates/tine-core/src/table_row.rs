//! What a children table's "Add row" writes on its new row: `false` for each
//! of the table's checkbox columns that has a plain, ordinary name, so a query
//! for `false` finds the row the table shows unchecked (tine#1047).

use crate::asset_refs::is_asset_pointer_key;
use crate::block_regions::{self, Edit};
use crate::doc::property_key_norm;
use crate::reference_evidence::is_og_built_in_property;
use crate::tine_keys::canonical_key;
use std::collections::HashSet;

#[cfg(test)]
thread_local! {
    /// Writer calls the seed made (a cost test's count).
    static WRITES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The raw text of a new row whose checkbox columns are `keys` (the table's
/// schema order): an empty title line, then `key:: false` (Org: a drawer row
/// `:key: false`) for each admitted key, written by the block's one property
/// writer; `""` (an empty row) when none is admitted. A key is admitted when it
/// is plain (ASCII letter first, then letters, digits, `_` or `-`; never `end`,
/// which closes an Org drawer), names no Logseq built-in, Tine key or the asset
/// pointer, and is not the engine's same key as an earlier admitted one (first
/// spelling wins). Every other column stays unset. O(key bytes) plus one writer
/// call on a one-line block; no parse when nothing is admitted.
pub fn new_checkbox_row_raw(org: bool, keys: &[String]) -> String {
    let mut seen = HashSet::new();
    let values: Vec<(String, String)> = keys
        .iter()
        .filter(|key| plain(key) && ordinary(key) && seen.insert(property_key_norm(key)))
        .map(|key| (key.clone(), "false".to_string()))
        .collect();
    if values.is_empty() {
        return String::new();
    }
    #[cfg(test)]
    WRITES.with(|n| n.set(n.get() + 1));
    // The writer refuses only keys plain spelling already excludes, or a
    // quarantined block, which a blank line is not: an empty row then.
    block_regions::edit("\n", org, Edit::Properties { values }).unwrap_or_default()
}

/// `[A-Za-z][A-Za-z0-9_-]*` and not `end` in any case: a name every reader in
/// both formats takes as itself. O(key bytes).
fn plain(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
        && !key.eq_ignore_ascii_case("end")
}

/// Not a key whose value means something beyond the column. O(key bytes).
fn ordinary(key: &str) -> bool {
    !is_og_built_in_property(key)
        && !canonical_key(key).starts_with("tine.")
        && !is_asset_pointer_key(key)
}

#[cfg(test)]
#[path = "table_row_tests.rs"]
mod tests;
