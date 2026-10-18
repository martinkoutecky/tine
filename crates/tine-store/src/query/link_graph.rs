//! The link table behind TQL `links`, `linked_by` and `near`
//! (`docs/contracts/tql-links-near.md`).
//!
//! A pair (S, T) is a link exactly when page S is a group of page T's Linked
//! References panel (unfiltered, T opened under its own name). The table is
//! built from the panel's own pieces so the two cannot drift (I-12):
//! - the names a link to T may use are [`graph_equivalent_page_names_with`]
//!   (the body behind `graph_equivalent_page_names`), with the panel's
//!   O(journals) day lookup replaced by one precomputed map;
//! - what a block references is the parser-owned explicit reference list of
//!   every block and of the page-properties header, read through
//!   `reference_evidence::visit_reference_targets` (the rule behind the panel's
//!   `projected_reference_matches`);
//! - excluded sources are `refs::ReferenceSourceExclusions`' (T itself, the
//!   Favorites arrangement page).
//!
//! The table lives in memory, one per graph snapshot, built on the first query
//! that reads it (`ReadSnapshot::link_graph`) and never persisted (I-25: no new
//! record, file or transport byte per edit). Build: one pass over every block's
//! reference list, O(P + B + R); memory O(P + E).

use std::collections::HashMap;
use std::sync::Arc;

use tine_core::doc::{property_key_norm, DocBlock};
use tine_core::model::PageKind;
use tine_core::query::ir::{Leaf, NearDir, Quant, Rel};
use tine_core::reference_evidence::{visit_reference_targets, ReferenceTarget};
use tine_core::refs;

use super::{
    graph_equivalent_page_names_with, journal_format, page_property_block_with_config,
    real_page_names,
};
use crate::model::ReadSnapshot;

/// A page without a file that some page links to: the panel exists for it, so
/// it is a node (name, journal day when the name is a date; no properties, no
/// blocks, no outgoing links).
#[derive(Debug, Clone)]
pub(crate) struct VirtualNode {
    pub(crate) name: String,
    pub(crate) journal_day: Option<i64>,
}

/// One snapshot's link table. Nodes `0..physical` are the physical pages (one
/// node per page key; duplicate files of one name share it), the rest are
/// [`VirtualNode`]s.
#[derive(Debug, Default)]
pub(crate) struct LinkGraph {
    by_key: HashMap<String, u32>,
    physical: usize,
    virtuals: Vec<VirtualNode>,
    out: Vec<Vec<u32>>,
    inn: Vec<Vec<u32>>,
}

enum PendingTarget {
    Page { key: String, spelling: String },
    PropertyKey { folded: String, spelling: String },
}

impl LinkGraph {
    pub(crate) fn build(graph: &ReadSnapshot) -> LinkGraph {
        let config = graph.config();
        let format = journal_format(config);
        let real = real_page_names(graph);
        let edges = graph.alias_edges();
        // The panel's `find` over the page list, once: the FIRST journal page
        // of each day in page-list order.
        let mut first_journal: HashMap<i64, String> = HashMap::new();
        for entry in graph.page_list_arc().iter() {
            if entry.kind == PageKind::Journal {
                if let Some(day) = format.parse(&entry.name) {
                    first_journal
                        .entry(day.ordinal_key())
                        .or_insert_with(|| entry.name.clone());
                }
            }
        }
        let resolve = |target: &str| {
            graph_equivalent_page_names_with(&real, &edges, &format, target, |day| {
                first_journal.get(&day.ordinal_key()).cloned()
            })
        };
        let favorites = config
            .favorites_page
            .as_deref()
            .map(refs::page_key)
            .filter(|key| !key.is_empty());
        let parse_config = tine_core::query::atom::ParseConfig::from_config(config);

        let mut table = LinkGraph::default();
        // Per node: its own key and the key the panel excludes as `self_page`.
        let mut node_keys: Vec<String> = Vec::new();
        let mut self_keys: Vec<String> = Vec::new();
        // Name key / property-key fold -> nodes whose panel that name reaches.
        let mut by_name: HashMap<String, Vec<u32>> = HashMap::new();
        let mut by_property_key: HashMap<String, Vec<u32>> = HashMap::new();
        let register = |node: u32,
                        names: &[String],
                        by_name: &mut HashMap<String, Vec<u32>>,
                        by_property_key: &mut HashMap<String, Vec<u32>>| {
            for name in names {
                by_name.entry(name.clone()).or_default().push(node);
                let folded = property_key_norm(name);
                let nodes = by_property_key.entry(folded).or_default();
                if nodes.last() != Some(&node) {
                    nodes.push(node);
                }
            }
        };
        let mut pending: Vec<(u32, PendingTarget)> = Vec::new();
        graph.with_pages(|pages| {
            for (entry, _) in pages.iter() {
                let key = refs::page_key(&entry.name);
                if table.by_key.contains_key(&key) {
                    continue;
                }
                let node = node_keys.len() as u32;
                let (_, names, self_page) = resolve(&entry.name);
                register(node, &names, &mut by_name, &mut by_property_key);
                table.by_key.insert(key.clone(), node);
                node_keys.push(key);
                self_keys.push(refs::page_key(&self_page));
            }
            table.physical = node_keys.len();
            for (entry, doc) in pages.iter() {
                let source = table.by_key[&refs::page_key(&entry.name)];
                if favorites
                    .as_deref()
                    .is_some_and(|favorites| refs::page_key(&entry.name) == favorites)
                {
                    continue;
                }
                let mut collect = |block: &DocBlock| {
                    for reference in block.projection().reference_source().explicit {
                        visit_reference_targets(reference, config, |target| {
                            pending.push((
                                source,
                                match target {
                                    ReferenceTarget::PageKey(key) => PendingTarget::Page {
                                        key: key.to_owned(),
                                        spelling: if key == refs::page_key(&reference.name) {
                                            reference.name.trim().to_owned()
                                        } else {
                                            key.to_owned()
                                        },
                                    },
                                    ReferenceTarget::PropertyKey(folded) => {
                                        PendingTarget::PropertyKey {
                                            folded: folded.to_owned(),
                                            spelling: reference.name.trim().to_owned(),
                                        }
                                    }
                                },
                            ));
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
        // A target no node answers to is a page without a file: one node per
        // panel, registered under every name that panel answers to.
        for (_, target) in &pending {
            let (missing, spelling) = match target {
                PendingTarget::Page { key, spelling } => (!by_name.contains_key(key), spelling),
                PendingTarget::PropertyKey { folded, spelling } => {
                    (!by_property_key.contains_key(folded), spelling)
                }
            };
            if !missing {
                continue;
            }
            let node = node_keys.len() as u32;
            let (canonical, names, self_page) = resolve(spelling);
            register(node, &names, &mut by_name, &mut by_property_key);
            node_keys.push(refs::page_key(&canonical));
            self_keys.push(refs::page_key(&self_page));
            table.virtuals.push(VirtualNode {
                journal_day: format.parse(&canonical).map(|day| day.ordinal_key()),
                name: canonical,
            });
        }
        let count = node_keys.len();
        table.out = vec![Vec::new(); count];
        table.inn = vec![Vec::new(); count];
        let empty = Vec::new();
        for (source, target) in &pending {
            let targets = match target {
                PendingTarget::Page { key, .. } => by_name.get(key),
                PendingTarget::PropertyKey { folded, .. } => by_property_key.get(folded),
            }
            .unwrap_or(&empty);
            for &target in targets {
                // `ReferenceSourceExclusions`: T's panel never lists T itself.
                if node_keys[*source as usize] == self_keys[target as usize] {
                    continue;
                }
                table.out[*source as usize].push(target);
            }
        }
        for (source, targets) in table.out.iter_mut().enumerate() {
            targets.sort_unstable();
            targets.dedup();
            for &target in targets.iter() {
                table.inn[target as usize].push(source as u32);
            }
        }
        table
    }

    pub(crate) fn node_count(&self) -> usize {
        self.out.len()
    }

    pub(crate) fn edge_count(&self) -> usize {
        self.out.iter().map(Vec::len).sum()
    }

    /// Heap bytes held (allocated capacity; hash-map slots estimated at their
    /// key, value and one control byte).
    pub(crate) fn estimated_bytes(&self) -> usize {
        let adjacency = |lists: &Vec<Vec<u32>>| {
            lists.capacity() * std::mem::size_of::<Vec<u32>>()
                + lists.iter().map(|list| list.capacity() * 4).sum::<usize>()
        };
        let keys = self.by_key.capacity() * (std::mem::size_of::<(String, u32)>() + 1)
            + self.by_key.keys().map(String::capacity).sum::<usize>();
        let virtuals = self.virtuals.capacity() * std::mem::size_of::<VirtualNode>()
            + self
                .virtuals
                .iter()
                .map(|node| node.name.capacity())
                .sum::<usize>();
        adjacency(&self.out) + adjacency(&self.inn) + keys + virtuals
    }

    /// The node of the physical page whose key is `page_key`.
    pub(crate) fn node(&self, page_key: &str) -> Option<u32> {
        self.by_key.get(page_key).copied()
    }

    /// The physical nodes' count; virtual node `i` is node `physical + i`.
    pub(crate) fn physical(&self) -> usize {
        self.physical
    }

    pub(crate) fn virtuals(&self) -> &[VirtualNode] {
        &self.virtuals
    }

    /// The pages `node` links to.
    pub(crate) fn links(&self, node: u32) -> &[u32] {
        &self.out[node as usize]
    }

    /// The pages that link to `node`.
    pub(crate) fn linked_by(&self, node: u32) -> &[u32] {
        &self.inn[node as usize]
    }

    /// `near`: the seeds and every node within `k` steps, one breadth-first
    /// search per direction (each path keeps its direction). A node failing
    /// `through` is selected when reached but not expanded; seeds always are.
    /// Cost O(nodes + edges followed); no recursion.
    pub(crate) fn ball(
        &self,
        seeds: &[bool],
        k: u32,
        dir: NearDir,
        through: Option<&[bool]>,
    ) -> Vec<bool> {
        let mut ball = seeds.to_vec();
        let directions: &[&Vec<Vec<u32>>] = match dir {
            NearDir::Out => &[&self.out],
            NearDir::In => &[&self.inn],
            NearDir::Both => &[&self.out, &self.inn],
        };
        let start: Vec<u32> = (0..seeds.len() as u32)
            .filter(|&node| seeds[node as usize])
            .collect();
        for adjacency in directions {
            let mut seen = seeds.to_vec();
            let mut frontier = start.clone();
            let mut step = 0u32;
            while step < k && !frontier.is_empty() {
                step += 1;
                let mut next = Vec::new();
                for &node in &frontier {
                    let expands = seeds[node as usize]
                        || through.is_none_or(|through| through[node as usize]);
                    if !expands {
                        continue;
                    }
                    for &neighbour in &adjacency[node as usize] {
                        if !seen[neighbour as usize] {
                            seen[neighbour as usize] = true;
                            ball[neighbour as usize] = true;
                            next.push(neighbour);
                        }
                    }
                }
                frontier = next;
            }
        }
        ball
    }
}

/// One execution's answers for its link leaves, computed innermost first and
/// keyed by the leaf's address inside the plan's filter (the plan owns the
/// filter immutably for the whole execution).
#[derive(Default)]
pub(crate) struct LinkAnswers {
    table: Option<Arc<LinkGraph>>,
    /// Per leaf, one bit per node: the element condition (`links`,
    /// `linked_by`) or the selected ball (`near`).
    sets: HashMap<usize, Vec<bool>>,
}

impl LinkAnswers {
    pub(crate) fn new(table: Arc<LinkGraph>) -> Self {
        Self {
            table: Some(table),
            sets: HashMap::new(),
        }
    }

    pub(crate) fn none() -> &'static LinkAnswers {
        static NONE: std::sync::OnceLock<LinkAnswers> = std::sync::OnceLock::new();
        NONE.get_or_init(LinkAnswers::default)
    }

    pub(crate) fn insert(&mut self, leaf: &Leaf, set: Vec<bool>) {
        self.sets.insert(leaf as *const Leaf as usize, set);
    }

    fn set(&self, leaf: &Leaf) -> Option<(&LinkGraph, &[bool])> {
        let table = self.table.as_deref()?;
        let set = self.sets.get(&(leaf as *const Leaf as usize))?;
        Some((table, set.as_slice()))
    }

    /// `near` for the page row that is link-table node `node` when known,
    /// else the physical page whose key is `page_key`.
    pub(crate) fn near(&self, leaf: &Leaf, node: Option<u32>, page_key: &str) -> bool {
        self.set(leaf).is_some_and(|(table, ball)| {
            node.or_else(|| table.node(page_key))
                .is_some_and(|node| ball[node as usize])
        })
    }

    /// `quant(links | linked_by, …)` for the page row that is node `node` when
    /// known, else the physical page whose key is `page_key`.
    /// A row with no computed answer (a memo probe, which never carries a link
    /// answer) has no links.
    pub(crate) fn related(
        &self,
        leaf: &Leaf,
        rel: Rel,
        quant: Quant,
        node: Option<u32>,
        page_key: &str,
    ) -> bool {
        let elements: &[u32];
        let set: &[bool];
        match self.set(leaf) {
            Some((table, bits)) => {
                set = bits;
                elements = match node.or_else(|| table.node(page_key)) {
                    Some(node) if rel == Rel::Links => table.links(node),
                    Some(node) => table.linked_by(node),
                    None => &[],
                };
            }
            None => {
                set = &[];
                elements = &[];
            }
        }
        let mut hits = elements.iter().map(|&node| set[node as usize]);
        match quant {
            Quant::Any => hits.any(|hit| hit),
            Quant::None => !hits.any(|hit| hit),
            Quant::Every => hits.all(|hit| hit),
        }
    }
}
