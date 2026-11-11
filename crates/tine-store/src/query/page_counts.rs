//! CM-COUNT projects the Linked References membership owner's cardinalities.
//! Cold construction belongs to that owner; subsequent reads copy O(P) output.
use crate::model::ReadSnapshot;
use std::collections::HashMap;

/// Nonzero panel cardinalities by (journal kind, canonical page key).
pub(crate) fn referring_block_counts(graph: &ReadSnapshot) -> HashMap<(bool, String), u32> {
    graph.scan_all_backlink_memberships().scan_all_counts()
}
