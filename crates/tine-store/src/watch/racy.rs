//! §5.4 racy-observation helpers for the watcher's full diff.
use super::{stamp_metadata, Stamp};
use crate::store::FileRev;
use std::path::Path;

/// Hashes `path` for the observation `value`, re-statting afterwards: a write
/// landing between the stat and the read (external-editor race) would pair an
/// old stamp with new bytes and publish a spurious second change once the
/// next diff sees the new stamp. On a moved stat the fresh one is adopted and
/// hashed again; returns whether it moved. A stamp still moving after that is
/// fresh, hence racy (§5.4), so the next full diff rereads it.
pub(super) fn hash_settled(path: &Path, value: &mut Stamp) -> bool {
    let mut moved = false;
    for _ in 0..3 {
        value.rev = FileRev::from_file(path).ok();
        let Some(fresh) = stamp_metadata(path) else {
            return moved;
        };
        if fresh.modified == value.modified
            && fresh.len == value.len
            && fresh.identity == value.identity
            && fresh.changed == value.changed
        {
            return moved;
        }
        moved = true;
        *value = fresh;
    }
    value.rev = FileRev::from_file(path).ok();
    moved
}
