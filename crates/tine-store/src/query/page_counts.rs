//! The All pages count (D-20261102-CM4 CM-COUNT): per page, how many blocks
//! its unfiltered Linked References panel lists, for every page at once.
//!
//! One forward walk over every block's parser-owned reference spans:
//! 1. per block, the distinct literal targets `visit_reference_targets` yields
//!    (the panel's rule, `property_page_key_enabled` included);
//! 2. each distinct target folded once to the panel it opens, through the
//!    panel's own resolver (`query::PanelResolver`, the one setup the link
//!    table shares); the fold table is a local of this one computation and
//!    is dropped with it, not a cache (D-10);
//! 3. per block, the folded panels deduplicated, the source page's own panel
//!    and the Favorites arrangement page skipped
//!    (`refs::ReferenceSourceExclusions`), each remaining panel incremented.
//!
//! I-12 exception (CM step 2, coordinator 2026-11-03): this counter and the
//! panel's `backlinks_bounded` are two drivers of one reference rule; the
//! conformance harness keeps them equal. Retire: one forward target map (D3).
//!
//! Cost O(P + B + R + T) for P pages, B blocks, R references and T distinct
//! targets (one resolve each); transient memory O(T + E) for E distinct
//! (block, target) pairs, all dropped on return; nothing persisted (I-25).
//! Measured on the realistic 10k graph: about 100 ms (CM step 2 batch 5
//! receipt).

use std::collections::HashMap;

use tine_core::doc::DocBlock;
use tine_core::reference_evidence::{visit_reference_targets, ReferenceTarget};
use tine_core::refs;

use super::{page_property_block_with_config, PanelResolver};
use crate::model::ReadSnapshot;

/// Referring blocks per panel, by the panel's canonical page key. A key that
/// is absent has no referring block.
pub(crate) fn referring_block_counts(graph: &ReadSnapshot) -> HashMap<String, u32> {
    let config = graph.config();
    let resolver = PanelResolver::new(graph);
    let resolve = |target: &str| resolver.resolve(target);
    let favorites = config
        .favorites_page
        .as_deref()
        .map(refs::page_key)
        .filter(|key| !key.is_empty());
    let parse_config = tine_core::query::atom::ParseConfig::from_config(config);

    // Step 1. Distinct targets are interned without allocating for one seen
    // before; `spellings[t]` is target t's first spelling (what a page
    // without a file is named after, as in the link table).
    let mut page_ids: HashMap<String, u32> = HashMap::new();
    let mut property_ids: HashMap<String, u32> = HashMap::new();
    let mut spellings: Vec<String> = Vec::new();
    let mut source_keys: Vec<String> = Vec::new();
    // `(source page, block ordinal, target)`, one entry per distinct target
    // of each block.
    let mut pending: Vec<(u32, u32, u32)> = Vec::new();
    let mut blocks = 0u32;
    graph.with_pages(|pages| {
        for (entry, doc) in pages.iter() {
            let key = refs::page_key(&entry.name);
            if favorites.as_deref() == Some(key.as_str()) {
                continue;
            }
            let source = source_keys.len() as u32;
            source_keys.push(key);
            let mut seen: Vec<u32> = Vec::new();
            let mut collect = |block: &DocBlock| {
                seen.clear();
                let ordinal = blocks;
                blocks += 1;
                for reference in block.projection().reference_source().explicit {
                    visit_reference_targets(reference, config, |target| {
                        let id = match target {
                            ReferenceTarget::PageKey(key) => match page_ids.get(key) {
                                Some(&id) => id,
                                None => {
                                    let id = spellings.len() as u32;
                                    spellings.push(if key == refs::page_key(&reference.name) {
                                        reference.name.trim().to_owned()
                                    } else {
                                        key.to_owned()
                                    });
                                    page_ids.insert(key.to_owned(), id);
                                    id
                                }
                            },
                            ReferenceTarget::PropertyKey(folded) => {
                                match property_ids.get(folded) {
                                    Some(&id) => id,
                                    None => {
                                        let id = spellings.len() as u32;
                                        spellings.push(reference.name.trim().to_owned());
                                        property_ids.insert(folded.to_owned(), id);
                                        id
                                    }
                                }
                            }
                        };
                        // A block's targets are few: a linear scan beats a set.
                        if !seen.contains(&id) {
                            seen.push(id);
                            pending.push((source, ordinal, id));
                        }
                        false
                    });
                }
            };
            if let Some(block) = doc
                .pre_block
                .as_deref()
                .and_then(|pre| page_property_block_with_config(entry, pre, &parse_config))
            {
                collect(&block);
            }
            // Explicit stack: an outline's depth never recurses here.
            let mut stack: Vec<&DocBlock> = doc.roots.iter().rev().collect();
            while let Some(block) = stack.pop() {
                collect(block);
                stack.extend(block.children.iter().rev());
            }
        }
    });
    drop((page_ids, property_ids));

    // Step 2. Each distinct target to its panel, once.
    let mut panel_ids: HashMap<String, u32> = HashMap::new();
    let mut panel_keys: Vec<String> = Vec::new();
    let mut panel_self: Vec<String> = Vec::new();
    let fold: Vec<u32> = spellings
        .iter()
        .map(|spelling| {
            let (canonical, _, self_page) = resolve(spelling);
            let key = refs::page_key(&canonical);
            *panel_ids.entry(key.clone()).or_insert_with(|| {
                panel_keys.push(key);
                panel_self.push(refs::page_key(&self_page));
                (panel_keys.len() - 1) as u32
            })
        })
        .collect();
    drop((spellings, panel_ids));

    // Step 3. A block once per panel; never the panel's own page.
    let mut counts = vec![0u32; panel_keys.len()];
    let mut counted = vec![u32::MAX; panel_keys.len()];
    for &(source, block, target) in &pending {
        let panel = fold[target as usize] as usize;
        if source_keys[source as usize] == panel_self[panel] || counted[panel] == block {
            continue;
        }
        counted[panel] = block;
        counts[panel] += 1;
    }
    panel_keys
        .into_iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .collect()
}
