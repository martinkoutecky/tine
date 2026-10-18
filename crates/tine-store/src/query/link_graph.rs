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
//! reference list, O(P + B + R); memory O(P + E) once built (the adjacency
//! lists are shrunk after deduplication, so a page that names one target many
//! times holds one entry, not one per occurrence; the transient pending list
//! holds one entry per distinct target of each page, and the adjacency lists
//! one per link, so the build's peak is O(R + E), not R times the alias
//! fan-out).
//!
//! Every pass is cooperatively cancellable through the caller's `cancelled`
//! check (the `Fn() -> bool` shape interactive search uses behind
//! `store::Cancel`): the build checks it once per page and once per
//! [`CHECK_EVERY`] references and links expanded, a `near` search once per
//! [`CHECK_EVERY`] edges scanned. A cancelled pass returns `None` and leaves nothing behind.

use std::collections::{HashMap, HashSet};
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

/// How many references, links or edges a pass handles between two
/// cancellation checks.
pub(crate) const CHECK_EVERY: usize = 1024;

enum PendingTarget {
    Page { key: String, spelling: String },
    PropertyKey { folded: String, spelling: String },
}

impl LinkGraph {
    /// The table, or `None` once `cancelled` reports true.
    pub(crate) fn build(graph: &ReadSnapshot, cancelled: &dyn Fn() -> bool) -> Option<LinkGraph> {
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
        // References read so far, across pages: the collection checks the
        // caller once per CHECK_EVERY of them, so one block naming many
        // targets is not one unbounded step.
        let mut references = 0usize;
        let finished = graph.with_pages(|pages| {
            for (entry, _) in pages.iter() {
                if cancelled() {
                    return false;
                }
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
                if cancelled() {
                    return false;
                }
                let source = table.by_key[&refs::page_key(&entry.name)];
                if favorites
                    .as_deref()
                    .is_some_and(|favorites| refs::page_key(&entry.name) == favorites)
                {
                    continue;
                }
                // One pending entry per distinct target of this page: a page
                // naming one target R times holds one entry, not R (review r1:
                // the occurrences were expanded before deduplication).
                let mut seen_keys: HashSet<String> = HashSet::new();
                let mut seen_folds: HashSet<String> = HashSet::new();
                // False once the caller cancelled.
                let mut collect = |block: &DocBlock| -> bool {
                    for reference in block.projection().reference_source().explicit {
                        references += 1;
                        if references.is_multiple_of(CHECK_EVERY) && cancelled() {
                            return false;
                        }
                        visit_reference_targets(reference, config, |target| {
                            let fresh = match target {
                                ReferenceTarget::PageKey(key) => {
                                    !seen_keys.contains(key) && seen_keys.insert(key.to_owned())
                                }
                                ReferenceTarget::PropertyKey(folded) => {
                                    !seen_folds.contains(folded)
                                        && seen_folds.insert(folded.to_owned())
                                }
                            };
                            if !fresh {
                                return false;
                            }
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
                    true
                };
                if let Some(block) = doc
                    .pre_block
                    .as_deref()
                    .and_then(|pre| page_property_block_with_config(entry, pre, &parse_config))
                {
                    if !collect(&block) {
                        return false;
                    }
                }
                // Explicit stack: an outline's depth never recurses here.
                let mut stack: Vec<&DocBlock> = doc.roots.iter().rev().collect();
                let mut visited = 0usize;
                while let Some(block) = stack.pop() {
                    visited += 1;
                    if visited.is_multiple_of(CHECK_EVERY) && cancelled() {
                        return false;
                    }
                    if !collect(block) {
                        return false;
                    }
                    stack.extend(block.children.iter().rev());
                }
            }
            true
        });
        if !finished {
            return None;
        }
        // A target no node answers to is a page without a file: one node per
        // panel, registered under every name that panel answers to.
        for (at, (_, target)) in pending.iter().enumerate() {
            if at.is_multiple_of(CHECK_EVERY) && cancelled() {
                return None;
            }
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
        // `added[target]` is the last source that linked to `target`: within
        // one source's run of entries a link is pushed once, so the lists
        // hold O(links) before the final sort and dedup (duplicate files of
        // one name, whose entries are not adjacent, add at most one copy per
        // file).
        let mut added: Vec<u32> = vec![u32::MAX; count];
        let mut work = 0usize;
        for (source, target) in pending.iter() {
            let targets = match target {
                PendingTarget::Page { key, .. } => by_name.get(key),
                PendingTarget::PropertyKey { folded, .. } => by_property_key.get(folded),
            }
            .unwrap_or(&empty);
            for &target in targets {
                // Checked per target scanned, not per entry: one entry may
                // fan out to every owner of an alias.
                work += 1;
                if work.is_multiple_of(CHECK_EVERY) && cancelled() {
                    return None;
                }
                if added[target as usize] == *source {
                    continue;
                }
                // `ReferenceSourceExclusions`: T's panel never lists T itself.
                if node_keys[*source as usize] == self_keys[target as usize] {
                    continue;
                }
                added[target as usize] = *source;
                table.out[*source as usize].push(target);
            }
        }
        drop(added);
        drop(pending);
        if cancelled() {
            return None;
        }
        table.index_links(cancelled)?;
        Some(table)
    }

    /// Sorts and dedups each out-list and fills the reverse lists from them.
    /// A list's sort counts its length once and indexing counts each edge,
    /// so even one hub page's million links meet a check every CHECK_EVERY.
    fn index_links(&mut self, cancelled: &dyn Fn() -> bool) -> Option<()> {
        let mut work = 0usize;
        let mut due = |n: usize| {
            let before = work / CHECK_EVERY;
            work += n;
            work / CHECK_EVERY > before
        };
        for (source, targets) in self.out.iter_mut().enumerate() {
            if due(targets.len()) && cancelled() {
                return None;
            }
            targets.sort_unstable();
            targets.dedup();
            // A page naming one target many times kept one slot per
            // occurrence until here; hold only the distinct links.
            targets.shrink_to_fit();
            for &target in targets.iter() {
                if due(1) && cancelled() {
                    return None;
                }
                self.inn[target as usize].push(source as u32);
            }
        }
        for sources in &mut self.inn {
            sources.shrink_to_fit();
        }
        Some(())
    }

    pub(crate) fn node_count(&self) -> usize {
        self.out.len()
    }

    #[cfg(any(test, feature = "test-faults"))]
    pub(crate) fn edge_count(&self) -> usize {
        self.out.iter().map(Vec::len).sum()
    }

    /// Heap bytes held (allocated capacity; hash-map slots estimated at their
    /// key, value and one control byte). A test probe.
    #[cfg(any(test, feature = "test-faults"))]
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
    /// search per direction (each path keeps its direction). Cost O(nodes + edges followed); no recursion. `None` once `cancelled`
    /// reports true (checked once per [`CHECK_EVERY`] edges scanned).
    pub(crate) fn ball(
        &self,
        seeds: &[bool],
        k: u32,
        dir: NearDir,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<bool>> {
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
            // Edges scanned in this direction: checked per CHECK_EVERY of
            // them, so one hub's adjacency list is not one unbounded step.
            let mut scanned = 0usize;
            while step < k && !frontier.is_empty() {
                step += 1;
                let mut next = Vec::new();
                for &node in &frontier {
                    for &neighbour in &adjacency[node as usize] {
                        scanned += 1;
                        if scanned.is_multiple_of(CHECK_EVERY) && cancelled() {
                            return None;
                        }
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
        Some(ball)
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

#[cfg(test)]
mod cancellation_tests {
    //! Review r1: the table build, the element and seed conditions and the
    //! `near` searches stop once the caller's cancellation check reports true.
    use std::cell::Cell;

    use tine_core::query::ir::{ExecutionContext, QueryRows};
    use tine_core::query::registry::Registry;
    use tine_core::query::{parse_query_input, QueryInput};

    use super::{LinkGraph, CHECK_EVERY};
    use tine_core::query::ir::NearDir;

    const CHAIN: usize = 3 * CHECK_EVERY;

    /// A chain p0 → p1 → … → p{CHAIN-1}.
    fn chain() -> (
        tempfile::TempDir,
        std::sync::Arc<crate::model::ReadSnapshot>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pages")).unwrap();
        std::fs::create_dir_all(dir.path().join("logseq")).unwrap();
        for at in 0..CHAIN {
            std::fs::write(
                dir.path().join("pages").join(format!("p{at}.md")),
                format!("- to [[p{}]]\n", at + 1),
            )
            .unwrap();
        }
        let (store, _, _) =
            crate::store::Store::open(dir.path(), crate::store::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap().test_read_snapshot();
        (dir, graph)
    }

    /// A check that reports true from its `after`-th call on, counting calls.
    fn stop_after(after: usize, calls: &Cell<usize>) -> impl Fn() -> bool + '_ {
        move || {
            calls.set(calls.get() + 1);
            calls.get() >= after
        }
    }

    #[test]
    fn a_cancelled_link_table_build_stops_and_leaves_nothing_behind() {
        let (_dir, graph) = chain();
        let calls = Cell::new(0);
        assert!(LinkGraph::build(&graph, &stop_after(5, &calls)).is_none());
        assert_eq!(calls.get(), 5, "the build stops at the first true check");
        assert!(graph.link_graph(&stop_after(1, &Cell::new(0))).is_none());
        // Nothing was cached: an uncancelled caller builds the whole table.
        let table = graph.link_graph(&|| false).unwrap();
        assert_eq!(
            table.node_count(),
            CHAIN + 1,
            "p0..p{{CHAIN-1}} and fileless p{{CHAIN}}"
        );
        assert_eq!(table.edge_count(), CHAIN);
    }

    #[test]
    fn a_cancelled_near_search_stops_inside_its_frontier_walk() {
        let (_dir, graph) = chain();
        let table = graph.link_graph(&|| false).unwrap();
        let mut seeds = vec![false; table.node_count()];
        seeds[table.node("p0").unwrap() as usize] = true;
        let whole = table
            .ball(&seeds, CHAIN as u32, NearDir::Out, &|| false)
            .unwrap();
        assert_eq!(whole.iter().filter(|&&hit| hit).count(), CHAIN + 1);
        let calls = Cell::new(0);
        assert!(table
            .ball(&seeds, CHAIN as u32, NearDir::Out, &stop_after(1, &calls))
            .is_none());
        // One check per CHECK_EVERY edges scanned (one per chain node here):
        // stopped after the first CHECK_EVERY of the CHAIN edges.
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn a_cancelled_link_query_returns_nothing_and_is_not_memoized() {
        let tql = "@page and near(name = 'p0', 5000, 'out') and any(linked_by, true)";
        let (query, view) = parse_query_input(
            tql,
            QueryInput::Tql,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert!(!query.is_invalid(), "{:?}", query.diagnostics);
        let context = ExecutionContext::default();
        let request = || crate::IrRequest::Run {
            query: &query,
            view: &view,
            context: &context,
        };
        // How many checks a whole run makes (on its own graph: this run's
        // answer is memoized there).
        let total = {
            let (_dir, graph) = chain();
            let calls = Cell::new(0);
            crate::query::exec::query_ir(&graph, request(), &stop_after(usize::MAX, &calls))
                .unwrap()
                .unwrap();
            calls.get()
        };
        // The build checks once per page twice, the seed condition and the
        // element condition once per node each: the run spans all of them.
        assert!(total > 4 * CHAIN, "checks in a whole run: {total}");
        // A fresh graph per stage: a run stopped after the build keeps the
        // (complete) table, so a later run would make fewer checks.
        for after in [3, total / 4, total / 2, 3 * total / 4, total - 1] {
            let (_dir, graph) = chain();
            let calls = Cell::new(0);
            let answer =
                crate::query::exec::query_ir(&graph, request(), &stop_after(after, &calls))
                    .unwrap();
            assert!(answer.is_none(), "stopped after {after} of {total} checks");
            assert_eq!(calls.get(), after, "no check after the first true one");
        }
        // Nothing of a stopped run is memoized: stop one, then run it whole.
        let (_dir, graph) = chain();
        let stopped =
            crate::query::exec::query_ir(&graph, request(), &stop_after(total / 2, &Cell::new(0)))
                .unwrap();
        assert!(stopped.is_none());
        let crate::IrAnswer::Result(result) =
            crate::query::exec::query_ir(&graph, request(), &|| false)
                .unwrap()
                .unwrap()
        else {
            panic!("a run answers a result");
        };
        let QueryRows::Page { pages } = result.rows else {
            panic!("page rows");
        };
        // p1..p{CHAIN-1} (p0 has no referrer; the fileless end is no row).
        assert_eq!(pages.len(), CHAIN - 1);
    }

    /// One whole run on `graph` of `tql`, counting its checks.
    fn checks_of_a_whole_run(graph: &crate::model::ReadSnapshot, tql: &str) -> usize {
        let (query, view) = parse_query_input(
            tql,
            QueryInput::Tql,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        assert!(!query.is_invalid(), "{:?}", query.diagnostics);
        let context = ExecutionContext::default();
        let calls = Cell::new(0);
        crate::query::exec::query_ir(
            graph,
            crate::IrRequest::Run {
                query: &query,
                view: &view,
                context: &context,
            },
            &stop_after(usize::MAX, &calls),
        )
        .unwrap()
        .expect("never cancelled");
        calls.get()
    }

    fn run_with(
        graph: &crate::model::ReadSnapshot,
        tql: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<crate::IrAnswer> {
        let (query, view) = parse_query_input(
            tql,
            QueryInput::Tql,
            tine_core::date::JournalDate::today(),
            Registry::none(),
        );
        let context = ExecutionContext::default();
        crate::query::exec::query_ir(
            graph,
            crate::IrRequest::Run {
                query: &query,
                view: &view,
                context: &context,
            },
            cancelled,
        )
        .unwrap()
    }

    #[test]
    fn a_cancelled_run_is_never_served_from_the_memo() {
        let (_dir, graph) = chain();
        for tql in [
            "@page and near(name = 'p0', 2, 'out')",
            "@page and not name = 'p5'",
        ] {
            // A whole run is memoized ...
            assert!(run_with(&graph, tql, &|| false).is_some());
            // ... and a cancelled run is answered nothing before the memo is
            // read: one check, no served answer.
            let calls = Cell::new(0);
            assert!(
                run_with(&graph, tql, &stop_after(1, &calls)).is_none(),
                "{tql}"
            );
            assert_eq!(calls.get(), 1, "{tql}");
            // The memo itself is untouched: an uncancelled run is served.
            let calls = Cell::new(0);
            assert!(run_with(&graph, tql, &stop_after(usize::MAX, &calls)).is_some());
            assert_eq!(
                calls.get(),
                1,
                "{tql}: a memo hit makes only the entry check"
            );
        }
    }

    #[test]
    fn a_run_cancelled_as_it_completes_stores_nothing_in_the_memo() {
        for tql in [
            "@page and near(name = 'p0', 2, 'out')",
            "@page and not name = 'p5'",
        ] {
            let total = {
                let (_dir, graph) = chain();
                checks_of_a_whole_run(&graph, tql)
            };
            let (_dir, graph) = chain();
            // The last check comes after the computation finished: the run is
            // cancelled then, so its complete answer is not stored either.
            let calls = Cell::new(0);
            assert!(
                run_with(&graph, tql, &stop_after(total, &calls)).is_none(),
                "{tql}"
            );
            assert_eq!(calls.get(), total, "{tql}");
            // Nothing was stored: the next run computes (many checks), it is
            // not a memo hit (one check).
            assert!(checks_of_a_whole_run(&graph, tql) > 1, "{tql}");
        }
    }

    #[test]
    fn a_query_without_link_conditions_stops_in_its_page_scan() {
        let (_dir, graph) = chain();
        let tql = "@page and not name = 'p5'";
        let total = checks_of_a_whole_run(&chain().1, tql);
        // The entry check, one per candidate page, the completion check.
        assert!(total >= CHAIN, "checks in a whole run: {total}");
        for after in [2, total / 2, total - 1] {
            let calls = Cell::new(0);
            assert!(
                run_with(&graph, tql, &stop_after(after, &calls)).is_none(),
                "stopped after {after} of {total} checks"
            );
            assert_eq!(calls.get(), after, "no check after the first true one");
        }
    }

    /// Review r1: a hub's adjacency list is one node but many edges. A
    /// search at k = 1 from a hub with a million out-edges, cancelled from
    /// its first check, stops within the first CHECK_EVERY edges.
    #[test]
    fn a_cancelled_near_search_stops_inside_one_hubs_edges() {
        const SPOKES: usize = 1 << 20;
        let mut table = LinkGraph::default();
        table.physical = SPOKES + 1;
        table.out = vec![Vec::new(); SPOKES + 1];
        table.out[0] = (1..=SPOKES as u32).collect();
        table.inn = vec![Vec::new(); SPOKES + 1];
        for spoke in 1..=SPOKES {
            table.inn[spoke].push(0);
        }
        let mut seeds = vec![false; SPOKES + 1];
        seeds[0] = true;
        let calls = Cell::new(0);
        assert!(table
            .ball(&seeds, 1, NearDir::Out, &stop_after(1, &calls))
            .is_none());
        assert_eq!(calls.get(), 1, "stopped at the first check");
        // Uncancelled, the same search reaches every spoke and checks once
        // per CHECK_EVERY edges.
        let calls = Cell::new(0);
        let ball = table
            .ball(&seeds, 1, NearDir::Out, &stop_after(usize::MAX, &calls))
            .unwrap();
        assert!(ball.iter().all(|&hit| hit));
        assert_eq!(calls.get(), SPOKES / CHECK_EVERY);
    }

    /// Review r2: the final pass checks inside one hub's edges too. A hub
    /// with a million out-links, cancelled at its second check, stops
    /// within the first CHECK_EVERY edges it indexes.
    #[test]
    fn a_cancelled_reverse_index_stops_inside_one_hubs_edges() {
        const SPOKES: usize = 1 << 20;
        let hub = || {
            let mut table = LinkGraph::default();
            table.out = vec![Vec::new(); SPOKES + 1];
            table.out[0] = (1..=SPOKES as u32).collect();
            table.inn = vec![Vec::new(); SPOKES + 1];
            table
        };
        let mut table = hub();
        let calls = Cell::new(0);
        assert!(table.index_links(&stop_after(2, &calls)).is_none());
        assert_eq!(calls.get(), 2, "stopped at the second check");
        let indexed = table.inn.iter().filter(|list| !list.is_empty()).count();
        assert!(indexed <= CHECK_EVERY, "{indexed} edges indexed");
        // Uncancelled: one check for the sort, then one per CHECK_EVERY edges.
        let mut table = hub();
        let calls = Cell::new(0);
        assert!(table.index_links(&stop_after(usize::MAX, &calls)).is_some());
        assert!(table.inn[1..].iter().all(|list| list == &[0]));
        assert_eq!(calls.get(), 1 + SPOKES / CHECK_EVERY);
    }

    /// Review r1: the build checks per link expanded, not per reference.
    /// Each of OWNERS pages aliased `shared` names `[[shared]]` once, a
    /// reference that reaches every owner: OWNERS references expand to about
    /// OWNERS² links, and the checks follow the links.
    #[test]
    fn a_link_table_build_checks_per_link_expanded() {
        const OWNERS: usize = 200;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pages")).unwrap();
        std::fs::create_dir_all(dir.path().join("logseq")).unwrap();
        for at in 0..OWNERS {
            std::fs::write(
                dir.path().join("pages").join(format!("o{at}.md")),
                "alias:: shared\n\n- see [[shared]]\n",
            )
            .unwrap();
        }
        let (store, _, _) =
            crate::store::Store::open(dir.path(), crate::store::OpenOptions::default()).unwrap();
        let graph = store.whole_graph().unwrap().test_read_snapshot();
        let calls = Cell::new(0);
        let table = LinkGraph::build(&graph, &stop_after(usize::MAX, &calls)).unwrap();
        let links = table.edge_count();
        assert!(links >= OWNERS * (OWNERS - 1), "links: {links}");
        // Two per-page passes check once per page; the expansion adds one
        // check per CHECK_EVERY links it pushes or skips.
        let per_page = 2 * OWNERS;
        assert!(
            calls.get() >= per_page + links / CHECK_EVERY,
            "{} checks for {OWNERS} pages and {links} links",
            calls.get()
        );
    }
}
