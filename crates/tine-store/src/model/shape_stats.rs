//! Graph-shape statistics for the diagnostics dump (GH #623 follow-up):
//! counts and quantiles over the pages Tine already holds in memory, so a
//! reporter who cannot share a graph can still show its shape.
//!
//! **Numbers only.** Nothing in this module returns, formats or hashes a page
//! name, a block's text or a path: names and text are read to be counted and
//! dropped. `shape_never_names_the_graph` plants a distinctive name and body
//! and asserts neither reaches the output (invariant I-5, the flight
//! recorder's privacy boundary).
//!
//! Cost: O(blocks) over the cached pages, no file reads. Blocks whose
//! projection is not yet memoized parse once (the same memo every reference
//! and search surface fills), so the first dump on a cold session can cost a
//! whole-graph projection pass; the caller measures it (`shapeMs`) and runs
//! it off the UI thread.

use super::*;
use crate::launch_diag::FileFacts;
use serde_json::{json, Value};
use std::collections::HashMap;
use tine_core::model::PageKind;

impl Graph {
    /// The built page cache, if one exists. Never builds one: a diagnostics
    /// dump must not start the whole-graph parse it is meant to explain.
    pub(crate) fn peek_pages(&self) -> Option<Arc<Pages>> {
        self.cache.read().unwrap().as_ref().map(Arc::clone)
    }
}

/// `{n, min, p10, p50, p90, p99, max, sum}` of `values` (nearest rank).
/// `n == 0` reports zeros rather than omitting the key.
pub(crate) fn quantiles(values: &mut [u64]) -> Value {
    values.sort_unstable();
    let n = values.len();
    let at = |per_cent: usize| -> u64 {
        if n == 0 {
            return 0;
        }
        values[(n * per_cent).div_ceil(100).clamp(1, n) - 1]
    };
    json!({
        "n": n,
        "min": values.first().copied().unwrap_or(0),
        "p10": at(10),
        "p50": at(50),
        "p90": at(90),
        "p99": at(99),
        "max": values.last().copied().unwrap_or(0),
        "sum": values.iter().sum::<u64>(),
    })
}

/// Per-page counters gathered in one walk of its blocks.
#[derive(Default)]
struct PageCounts {
    blocks: u64,
    depth: u64,
    links: u64,
    tags: u64,
    embeds: u64,
    block_refs: u64,
    queries: u64,
    properties: u64,
}

fn count_page(doc: &tine_core::doc::Document) -> PageCounts {
    let mut counts = PageCounts::default();
    // Explicit stack: nesting depth is user data, never recursed on.
    let mut stack: Vec<(&tine_core::doc::DocBlock, u64)> =
        doc.roots.iter().map(|root| (root, 1)).collect();
    while let Some((block, depth)) = stack.pop() {
        counts.blocks += 1;
        counts.depth = counts.depth.max(depth);
        let projection = block.projection();
        for reference in projection.reference_source().explicit {
            match reference.rule {
                "explicit_link" | "explicit_nested_link" => counts.links += 1,
                "explicit_tag" => counts.tags += 1,
                "explicit_embed" => counts.embeds += 1,
                _ => {}
            }
        }
        counts.block_refs += projection.block_refs().len() as u64;
        counts.properties += projection.properties().len() as u64;
        let raw = block.raw();
        if raw.contains("{{") {
            counts.queries += tine_core::query::macro_text::query_macro_extents(raw).len() as u64;
        }
        stack.extend(block.children.iter().map(|child| (child, depth + 1)));
    }
    counts
}

/// The `shape` section of the diagnostics dump. `crlf_files` is the load-time
/// count (`None` before the first load finished).
pub(crate) fn shape(pages: &Pages, facts: FileFacts, crlf_files: Option<u64>) -> Value {
    let (mut blocks, mut depth, mut links, mut tags) = (vec![], vec![], vec![], vec![]);
    let (mut embeds, mut block_refs, mut queries, mut props) = (vec![], vec![], vec![], vec![]);
    let mut namespace_depth = Vec::with_capacity(pages.len());
    let mut journals = 0u64;
    // Per distinct (referencing page, referenced key): `(count, last page)`
    // dedupes without allocating a set per page.
    let mut referrers: HashMap<&str, (u64, usize)> = HashMap::new();
    for (index, (entry, doc)) in pages.iter().enumerate() {
        journals += u64::from(entry.kind == PageKind::Journal);
        namespace_depth.push(entry.name.matches('/').count() as u64);
        let counts = count_page(doc);
        blocks.push(counts.blocks);
        depth.push(counts.depth);
        links.push(counts.links);
        tags.push(counts.tags);
        embeds.push(counts.embeds);
        block_refs.push(counts.block_refs);
        queries.push(counts.queries);
        props.push(counts.properties);
        let mut stack: Vec<&tine_core::doc::DocBlock> = doc.roots.iter().collect();
        while let Some(block) = stack.pop() {
            for key in block.projection().refs_norm() {
                let slot = referrers.entry(key.as_str()).or_insert((0, usize::MAX));
                if slot.1 != index {
                    *slot = (slot.0 + 1, index);
                }
            }
            stack.extend(block.children.iter());
        }
    }
    let mut in_degree: Vec<u64> = pages
        .iter()
        .map(|(entry, _)| {
            let key = tine_core::refs::normalize(&entry.name);
            referrers.get(key.as_str()).map_or(0, |slot| slot.0)
        })
        .collect();
    let mut lens = facts.lens;
    json!({
        "ready": true,
        "pages": pages.len(),
        "journals": journals,
        "crlfFilesAtLastLoad": crlf_files,
        "nonNfcFileNames": facts.non_nfc_named,
        "conflictNamedFiles": facts.conflict_named,
        "fileBytes": quantiles(&mut lens),
        "blocksPerPage": quantiles(&mut blocks),
        "maxNestingDepth": quantiles(&mut depth),
        "linksPerPage": quantiles(&mut links),
        "tagsPerPage": quantiles(&mut tags),
        // Page embeds only; a block embed `{{embed ((uuid))}}` is counted in
        // blockRefsPerPage because lsdoc projects it as a block reference.
        "embedsPerPage": quantiles(&mut embeds),
        "blockRefsPerPage": quantiles(&mut block_refs),
        "queriesPerPage": quantiles(&mut queries),
        "propertiesPerPage": quantiles(&mut props),
        "inDegree": quantiles(&mut in_degree),
        "namespaceDepth": quantiles(&mut namespace_depth),
    })
}

#[cfg(test)]
mod tests {
    use super::quantiles;

    #[test]
    fn quantiles_use_nearest_rank_and_survive_empty_input() {
        let mut ten: Vec<u64> = (1..=10).rev().collect();
        let q = quantiles(&mut ten);
        assert_eq!(
            (q["n"].as_u64(), q["min"].as_u64(), q["p10"].as_u64()),
            (Some(10), Some(1), Some(1))
        );
        assert_eq!(
            (q["p50"].as_u64(), q["p90"].as_u64(), q["p99"].as_u64()),
            (Some(5), Some(9), Some(10))
        );
        assert_eq!((q["max"].as_u64(), q["sum"].as_u64()), (Some(10), Some(55)));
        let empty = quantiles(&mut []);
        assert_eq!(
            (empty["n"].as_u64(), empty["p99"].as_u64()),
            (Some(0), Some(0))
        );
    }
}
