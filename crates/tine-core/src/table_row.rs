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

/// The raw text of a new row whose checkbox columns are `keys` and whose other
/// property columns are `others`: every property column the table draws, in
/// its order (declared, observed on rows, added). An empty title line, then `key:: false` (Org: a drawer row
/// `:key: false`) for each admitted key, written by the block's one property
/// writer; `""` (an empty row) when none is admitted. A key is admitted when it
/// is plain (ASCII letter first, then letters, digits, `_` or `-`; never `end`
/// or `properties`, which spell an Org drawer's own lines) and names no Logseq
/// built-in, Tine key or the asset pointer. The whole row stays empty, as
/// before this seed, when two of all its property columns (`keys` and
/// `others`) are the engine's same key (a later edit of one would write or read
/// the other's line), or, in Org, when any of them is named `end` or
/// `properties` (a later edit of it would put that entry in the seeded drawer,
/// where the drawer's own lines are mistaken for it). Every
/// other column stays unset. O(key bytes of all columns) plus one writer call on a one-line
/// block; no parse when nothing is admitted.
pub fn new_checkbox_row_raw(org: bool, keys: &[String], others: &[String]) -> String {
    let columns = || keys.iter().chain(others);
    let mut norms = HashSet::new();
    if !columns().all(|key| norms.insert(property_key_norm(key)))
        || (org && columns().any(|key| drawer_word(key)))
    {
        return String::new();
    }
    let values: Vec<(String, String)> = keys
        .iter()
        .filter(|key| plain(key) && ordinary(key))
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

/// `[A-Za-z][A-Za-z0-9_-]*` and not `end` or `properties` in any case (they
/// spell a drawer's closer and opener): a name every reader in both formats
/// takes as itself. O(key bytes).
fn plain(key: &str) -> bool {
    let bytes = key.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
        && !drawer_word(key)
}

/// `end` or `properties` in any ASCII case, around any spaces. O(key bytes).
fn drawer_word(key: &str) -> bool {
    let key = key.trim();
    key.eq_ignore_ascii_case("end") || key.eq_ignore_ascii_case("properties")
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
