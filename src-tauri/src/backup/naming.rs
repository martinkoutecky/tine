//! Snapshot directory names, `<UTC stamp>[-<suffix>][-<k>]`: their
//! chronological order and the same-second counter a new one reserves.

use std::io::ErrorKind;

/// The highest same-second counter `name` already has among `dir`'s
/// entries, published or `.partial-`: `name` itself is 1, `name-k` is k;
/// 0 when there is none.
pub(super) fn highest_counter(dir: &std::path::Path, name: &str) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(dir) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        entries => entries?,
    };
    let mut highest = 0;
    for entry in entries {
        let entry = entry?.file_name();
        let entry = entry.to_str().unwrap_or_default();
        let counter = match entry
            .strip_prefix(".partial-")
            .unwrap_or(entry)
            .strip_prefix(name)
        {
            Some("") => Some(1),
            Some(rest) => rest.strip_prefix('-').and_then(|k| k.parse().ok()),
            None => None,
        };
        highest = highest.max(counter.unwrap_or(0));
    }
    Ok(highest)
}

/// Chronological order of snapshot names, `<stamp>[-<suffix>][-<k>]`: the
/// stamp, then the same-second counter `k` as a number (none is 1), then the
/// name (REVIEW N5: `-10` is newer than `-9`).
pub(super) fn snapshot_order(name: &str) -> (&str, u64, &str) {
    let (stamp, rest) = match (name.get(..19), name.get(19..)) {
        (Some(stamp), Some(rest)) => (stamp, rest),
        _ => (name, ""),
    };
    let counter = rest
        .rsplit_once('-')
        .and_then(|(_, k)| k.parse().ok())
        .unwrap_or(1);
    (stamp, counter, name)
}
