//! What removing blocks does to block references: one rule for the app's
//! reference notice (wasm) and `tine delete` (native).

use serde::Serialize;

/// The persisted `id::`s a removal takes away (`targets`, sorted), and how
/// many of their referrers go with it (`removed_with`: each removed block
/// counted once per target, as the graph's block-ref counter counts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemovalRefs {
    pub targets: Vec<String>,
    pub removed_with: usize,
}

/// `removed` is every removed block's `(raw, is_org)`, subtrees included;
/// `with_content` is false when the blocks' text survives elsewhere (a
/// merge), so no referrer goes with them. Cost O(removed blocks).
pub fn removal_refs(removed: &[(&str, bool)], with_content: bool) -> RemovalRefs {
    let mut targets: Vec<String> = removed
        .iter()
        .filter_map(|(raw, is_org)| crate::block_regions::parse(raw, *is_org).id)
        .map(|property| property.value.trim().to_string())
        .filter(|id| is_block_uuid(id))
        .collect();
    targets.sort();
    targets.dedup();
    let removed_with = if with_content {
        removed
            .iter()
            .map(|(raw, is_org)| {
                // Sorted and de-duplicated: one referrer per target.
                crate::lsdoc_block_parse::parse_projection(raw, *is_org)
                    .refs
                    .block
                    .iter()
                    .filter(|id| targets.binary_search(id).is_ok())
                    .count()
            })
            .sum()
    } else {
        0
    };
    RemovalRefs {
        targets,
        removed_with,
    }
}

fn is_block_uuid(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.iter().map(|g| g.len()).eq([8, 4, 4, 4, 12])
        && groups
            .iter()
            .all(|g| g.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";

    #[test]
    fn a_referrer_removed_with_its_target_counts_once() {
        let target = format!("one\nid:: {ID}");
        let inside = format!("(({ID})) and (({ID}))");
        let refs = removal_refs(&[(&target, false), (&inside, false)], true);
        assert_eq!(refs.targets, vec![ID.to_string()]);
        assert_eq!(refs.removed_with, 1);
        assert_eq!(
            removal_refs(&[(&target, false), (&inside, false)], false).removed_with,
            0
        );
    }

    #[test]
    fn a_non_uuid_id_is_no_target() {
        assert!(removal_refs(&[("one\nid:: abc", false)], true)
            .targets
            .is_empty());
    }
}
