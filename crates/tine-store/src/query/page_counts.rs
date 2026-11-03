//! The All pages count (D-20261102-CM4 CM-COUNT): per page, how many blocks
//! its unfiltered Linked References panel lists, for every page at once.
//!
//! The panel rule runs from the panel to its names: a page's panel lists
//! every block naming one of the names `PanelResolver::resolve` gives that
//! page (its alias group, the group's journal-date spellings). One name can
//! belong to several panels (an alias member, a chain, two declarers, a date
//! spelling), so a target is never folded forward to one panel (the batch 5
//! regression UI-CM-ALLPAGES-COUNT-ZERO-FOR-ALIAS-GROUP did that). Instead,
//! as the link table does:
//! 0. every listed page is a panel, registered under each name its
//!    resolve gives (`by_name`, and by property-key fold);
//! 1. one forward walk: per block, the distinct literal targets
//!    `visit_reference_targets` yields (the panel's rule,
//!    `property_page_key_enabled` included), interned;
//! 2. a target no panel answers to opens a page without a file: one panel per
//!    such target, registered under the names its own resolve gives;
//! 3. per block, each panel a target reaches counted once, never the panel's
//!    own page (`self_page`) nor from the Favorites arrangement page
//!    (`refs::ReferenceSourceExclusions`).
//!
//! The tables are locals of this one computation, dropped with it, not a
//! cache (D-10).
//!
//! I-12 exception (CM step 2, coordinator 2026-11-03): this counter and the
//! panel's `backlinks_bounded` are two drivers of one reference rule; the
//! conformance harness keeps them equal. Retire: one forward target map (D3).
//!
//! Cost O(P + B + R + T + F) for P pages, B blocks, R references, T distinct
//! targets and F (block, panel) pairs reached; one resolve per page and per
//! fileless target; transient memory O(P + T + E) for E distinct
//! (block, target) pairs, all dropped on return; nothing persisted (I-25).
//! Measured on the realistic 10k graph: see the fix-cm-counts-alias receipt.

use std::collections::HashMap;

use tine_core::doc::{property_key_norm, DocBlock};
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
    // Step 0. Panels: every listed page (the first entry of a key), its
    // own key, the key its panel excludes as `self_page`, and the names it
    // answers to.
    let mut panel_ids: HashMap<String, u32> = HashMap::new();
    let mut panel_keys: Vec<String> = Vec::new();
    let mut panel_self: Vec<String> = Vec::new();
    let mut by_name: HashMap<String, Vec<u32>> = HashMap::new();
    let mut by_property_key: HashMap<String, Vec<u32>> = HashMap::new();
    let mut targets: Vec<Target> = Vec::new();
    graph.with_pages(|pages| {
        for (entry, _) in pages.iter() {
            let key = refs::page_key(&entry.name);
            if panel_ids.contains_key(&key) {
                continue;
            }
            let panel = panel_keys.len() as u32;
            let (_, names, self_page) = resolve(&entry.name);
            register(panel, &names, &mut by_name, &mut by_property_key);
            panel_ids.insert(key.clone(), panel);
            panel_keys.push(key);
            panel_self.push(refs::page_key(&self_page));
        }
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
                                    targets.push(Target::Page(key.to_owned()));
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
                                        targets.push(Target::PropertyKey(folded.to_owned()));
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

    // Step 2. A target no panel answers to opens a page without a file: one
    // panel per such target, in first-seen order (a later target may be
    // answered by an earlier one's names), keyed by its canonical name.
    for (target, spelling) in targets.iter().zip(&spellings) {
        let answered = match target {
            Target::Page(key) => by_name.contains_key(key),
            Target::PropertyKey(folded) => by_property_key.contains_key(folded),
        };
        if answered {
            continue;
        }
        let panel = panel_keys.len() as u32;
        let (canonical, names, self_page) = resolve(spelling);
        register(panel, &names, &mut by_name, &mut by_property_key);
        let key = refs::page_key(&canonical);
        panel_ids.entry(key.clone()).or_insert(panel);
        panel_keys.push(key);
        panel_self.push(refs::page_key(&self_page));
    }
    drop(spellings);
    let empty = Vec::new();
    let reach: Vec<&Vec<u32>> = targets
        .iter()
        .map(|target| {
            match target {
                Target::Page(key) => by_name.get(key),
                Target::PropertyKey(folded) => by_property_key.get(folded),
            }
            .unwrap_or(&empty)
        })
        .collect();

    // Step 3. A block once per panel; never the panel's own page.
    let mut counts = vec![0u32; panel_keys.len()];
    let mut counted = vec![u32::MAX; panel_keys.len()];
    for &(source, block, target) in &pending {
        for &panel in reach[target as usize] {
            let panel = panel as usize;
            if source_keys[source as usize] == panel_self[panel] || counted[panel] == block {
                continue;
            }
            counted[panel] = block;
            counts[panel] += 1;
        }
    }
    // A key names the panel its first entry registered (the link table's
    // `by_key`).
    panel_ids
        .into_iter()
        .map(|(key, panel)| (key, counts[panel as usize]))
        .filter(|(_, n)| *n > 0)
        .collect()
}

/// `panel` answers to each of `names` (by key, and by property-key fold).
fn register(
    panel: u32,
    names: &[String],
    by_name: &mut HashMap<String, Vec<u32>>,
    by_property_key: &mut HashMap<String, Vec<u32>>,
) {
    for name in names {
        by_name.entry(name.clone()).or_default().push(panel);
        let panels = by_property_key.entry(property_key_norm(name)).or_default();
        if panels.last() != Some(&panel) {
            panels.push(panel);
        }
    }
}

enum Target {
    Page(String),
    PropertyKey(String),
}
