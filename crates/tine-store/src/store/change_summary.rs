//! Transport-only invalidation evidence, derived from edited parsed pages.
//! Only mounted query/unlinked targets are checked; no graph traversal on publication.
use super::*;
use crate::query::{exec::Plan, index::PageFacts};
use std::collections::BTreeSet;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::query::{
    atom::ParseConfig,
    ir::{ExecutionContext, Query},
};
use tine_core::refs::page_key;

/// In-memory, binding-owned readers. These never enter a checkpoint.
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum QuerySubscription {
    /// A mounted unlinked-reference answer over a page's names.
    Unlinked {
        /// The page whose plain-text mentions the reader displays.
        name: String,
    },
    /// A query resolved through the same front door as execution.
    Query {
        /// Parsed query IR, including its original advanced source.
        query: Query,
        /// Host-page inputs such as current-page.
        context: ExecutionContext,
    },
    /// A workspace's complete macro argument, parsed by the execution parser.
    Source {
        /// Query source without outer macro delimiters.
        source: String,
        /// The same wire dialect supplied to query_parse.
        dialect: tine_core::query::wire_parse::QueryTextDialect,
    },
    /// Builder property/key vocabulary.
    Registry,
    /// A reader without a classifiable plan (for example graph search).
    Full,
}

/// Classification and immutable-snapshot baseline for a mounted reader.
#[derive(serde::Serialize)]
pub struct QueryRegistration {
    /// `narrow`, `quiet`, or conservative `full` scheduling.
    pub mode: String,
    /// Prevents a newly mounted reader sharing an answer from an older snapshot.
    pub rev: String,
}

#[derive(Default)]
pub(super) struct RegisteredTargets {
    pub(super) unlinked: Vec<String>,
    pub(super) queries: Vec<(String, Arc<Mutex<QueryTarget>>)>,
    unlinked_generation: u64,
    query_generation: u64,
}

impl ChangeFeed {
    pub(super) fn registered_targets(&self) -> RegisteredTargets {
        let (unlinked, unlinked_generation) = self.unlinked_targets.lock().unwrap().live();
        let (queries, query_generation, retired) = self.query_targets.lock().unwrap().live();
        // The registry guard from the preceding statement is gone. A revoked
        // plan may own graph-sized tag/registry data; destroy it outside the
        // lock so cleanup cannot block new registrations.
        drop(retired);
        RegisteredTargets {
            unlinked,
            queries,
            unlinked_generation,
            query_generation,
        }
    }
    pub(super) fn fire_registered_since(
        &self,
        targets: &RegisteredTargets,
        snapshot: &mut Snapshot,
    ) {
        self.unlinked_targets
            .lock()
            .unwrap()
            .fire_since(targets.unlinked_generation, &mut snapshot.answers.2);
        self.query_targets
            .lock()
            .unwrap()
            .fire_since(targets.query_generation, &mut snapshot.answers.2);
    }
}

#[derive(Default)]
pub(super) struct QueryRegistry {
    generation: u64,
    targets: BTreeMap<String, Arc<Mutex<QueryTarget>>>,
}

pub(super) struct QueryTarget {
    request: QuerySubscription,
    plan: Option<Plan>,
    added: u64,
    revoked: Option<std::sync::Weak<AtomicBool>>,
    dirty: bool,
}

impl QueryTarget {
    fn live(&self) -> bool {
        self.revoked.as_ref().map_or(true, |weak| {
            weak.upgrade()
                .is_some_and(|flag| !flag.load(Ordering::Acquire))
        })
    }
    fn plan(request: &QuerySubscription, graph: &ReadSnapshot) -> Option<Plan> {
        match request {
            QuerySubscription::Query { query, context } => {
                let today = tine_core::date::JournalDate::today();
                let resolved = tine_core::query::resolve_for_execution(query, context, today);
                (!resolved.query().is_invalid())
                    .then(|| crate::query::exec::plan(graph, resolved.query(), today, false))
            }
            QuerySubscription::Source { source, dialect } => {
                let parsed = graph.with_pages(|pages| {
                    tine_core::query::wire_parse::parse_query_pair(
                        source,
                        *dialect,
                        &[],
                        &graph.query_index().registry(pages),
                    )
                });
                Self::plan(
                    &QuerySubscription::Query {
                        query: parsed.query,
                        context: ExecutionContext::none(),
                    },
                    graph,
                )
            }
            _ => None,
        }
    }
    fn mode(&self) -> &'static str {
        match &self.request {
            QuerySubscription::Registry => "quiet",
            QuerySubscription::Full | QuerySubscription::Unlinked { .. } => "full",
            QuerySubscription::Query { .. } | QuerySubscription::Source { .. }
                if self.plan.is_none() =>
            {
                "full"
            }
            QuerySubscription::Query { .. } | QuerySubscription::Source { .. }
                if self.plan.as_ref().is_some_and(|p| {
                    p.reads_links() || p.reads_tag_targets() || p.registry().is_some()
                }) =>
            {
                "quiet"
            }
            _ => "narrow",
        }
    }
}

impl QueryRegistry {
    pub(super) fn live(
        &mut self,
    ) -> (
        Vec<(String, Arc<Mutex<QueryTarget>>)>,
        u64,
        Vec<Arc<Mutex<QueryTarget>>>,
    ) {
        let mut retired = Vec::new();
        self.targets.retain(|_, target| {
            if target.lock().unwrap().live() {
                true
            } else {
                // Retain one owner until the caller has released the lock.
                retired.push(Arc::clone(target));
                false
            }
        });
        (
            self.targets
                .iter()
                .map(|(id, target)| (id.clone(), Arc::clone(target)))
                .collect(),
            self.generation,
            retired,
        )
    }
    pub(super) fn fire_since(&self, generation: u64, summary: &mut ChangeSummary) {
        for (id, target) in &self.targets {
            let mut target = target.lock().unwrap();
            if target.added > generation && target.live() {
                summary.query_to.insert(id.clone());
                target.dirty = true;
            }
        }
    }
}

fn dirty_query_plans(targets: &[(String, Arc<Mutex<QueryTarget>>)]) {
    for (_, target) in targets {
        let mut target = target.lock().unwrap();
        target.dirty = true;
    }
}

#[cfg(test)]
mod retirement_tests {
    use super::*;

    #[test]
    fn revoked_query_destruction_is_deferred_until_after_registry_unlock() {
        let revoked = Arc::new(AtomicBool::new(true));
        let target = Arc::new(Mutex::new(QueryTarget {
            request: QuerySubscription::Full,
            plan: None,
            added: 0,
            revoked: Some(Arc::downgrade(&revoked)),
            dirty: false,
        }));
        let weak = Arc::downgrade(&target);
        let registry = Mutex::new(QueryRegistry {
            generation: 0,
            targets: BTreeMap::from([("revoked".into(), target)]),
        });
        let mut guard = registry.lock().unwrap();
        let captured = guard.live();
        assert!(guard.targets.is_empty());
        assert!(weak.upgrade().is_some(), "pruning must move ownership out instead of destroying a potentially graph-sized plan under the lock");
        drop(guard);
        assert!(registry.try_lock().is_ok());
        drop(captured);
        assert!(
            weak.upgrade().is_none(),
            "retired plans are released after unlocking"
        );
    }
}

/// Mounted unlinked readers, with a registration counter so a publication can
/// tell which registrations arrived while it was being captured.
#[derive(Default)]
pub(super) struct UnlinkedRegistry {
    pub(super) generation: u64,
    pub(super) targets: BTreeMap<String, UnlinkedTarget>,
}

impl UnlinkedRegistry {
    /// Prune dead leases; the live target names and the current counter.
    pub(super) fn live(&mut self) -> (Vec<String>, u64) {
        self.targets.retain(|_, target| target.live());
        let names: BTreeSet<_> = self.targets.values().map(|t| t.name.clone()).collect();
        (names.into_iter().collect(), self.generation)
    }

    /// A target registered after the publication snapshotted the registry:
    /// its reader may have read the snapshot this publication replaces, so
    /// the publication fires its key (over-invalidation, never staleness).
    pub(super) fn fire_since(&self, generation: u64, summary: &mut ChangeSummary) {
        for target in self.targets.values() {
            if target.added > generation && target.live() {
                summary.text_to.insert(page_key(&target.name));
            }
        }
    }
}

pub(super) struct UnlinkedTarget {
    pub(super) name: String,
    /// `generation` when this registration was made.
    pub(super) added: u64,
    revoked: Option<std::sync::Weak<AtomicBool>>,
}
impl UnlinkedTarget {
    pub(super) fn live(&self) -> bool {
        self.revoked.as_ref().map_or(true, |weak| {
            weak.upgrade()
                .is_some_and(|flag| !flag.load(Ordering::Acquire))
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChangeSummary {
    rev: String,
    pages: BTreeSet<String>,
    blocks: BTreeSet<String>,
    links_to: BTreeSet<String>,
    text_to: BTreeSet<String>,
    tags_to: BTreeSet<String>,
    tag_table_to: BTreeSet<String>,
    refs_to: BTreeSet<String>,
    query_to: BTreeSet<String>,
    props: BTreeMap<String, BTreeSet<String>>,
    inventory_changed: bool,
    full: bool,
    #[serde(skip)]
    day: Option<i64>,
}

impl ChangeSummary {
    #[cfg(test)]
    pub(super) fn fires_query(&self, key: &str) -> bool {
        self.query_to.contains(key)
    }
    #[cfg(test)]
    pub(super) fn fires_text(&self, key: &str) -> bool {
        self.text_to.contains(key)
    }
}

impl Snapshot {
    pub(super) fn change_summary(
        &self,
        old: Option<&Self>,
        paths: &[String],
        names_changed: bool,
        inventory_changed: bool,
        edits: Option<
            &[(
                PageEntry,
                Arc<tine_core::doc::Document>,
                Arc<tine_core::doc::Document>,
            )],
        >,
        unlinked_targets: &[String],
        query_targets: &[(String, Arc<Mutex<QueryTarget>>)],
    ) -> ChangeSummary {
        let mut summary = ChangeSummary {
            rev: self.rev.0.to_string(),
            inventory_changed,
            full: names_changed || paths.len() > 32,
            day: Some(tine_core::date::JournalDate::today().ordinal_key()),
            ..Default::default()
        };
        // Build this existing lazy owner at initial publication (or rebuild),
        // so later edits inherit its per-page patch instead of doing a cold walk.
        self.graph.alias_edges();
        let Some(old) = old else {
            summary.full = true;
            dirty_query_plans(query_targets);
            return summary;
        };
        summary.full |= old.answers.2.day != summary.day || self.unreadable != old.unreadable;
        let Some(edits) = edits else {
            summary.full = true; // create, delete, rename, or unavailable old bytes
            dirty_query_plans(query_targets);
            return summary;
        };
        let config = ParseConfig::from_config(self.graph.config());
        // No query work when no reader is registered. Otherwise derive one
        // facts pair per edited page, shared by all mounted queries.
        let facts: Vec<_> = if query_targets.is_empty() {
            Vec::new()
        } else {
            edits
                .iter()
                .map(|(entry, before, after)| {
                    (
                        PageFacts::of(entry, before, &config),
                        PageFacts::of(entry, after, &config),
                    )
                })
                .collect()
        };
        for (id, target) in query_targets {
            let mut target = target.lock().unwrap();
            if !target.live() {
                continue;
            }
            let dirty = summary.full
                || facts.iter().any(|(was, now)| {
                    target.plan.as_ref().is_some_and(|plan| {
                        plan.registry().is_some() && was.registry_rows_differ(now)
                            || plan.reads_tag_targets() && was.tags() != now.tags()
                    })
                });
            // Time can change a displayed answer even when this publication
            // has no changed document to pass to the shared edit predicate.
            let touched = target.plan.as_ref().is_some_and(Plan::reads_clock)
                || edits
                    .iter()
                    .zip(&facts)
                    .any(|((entry, before, after), (was, now))| {
                        if before == after {
                            return false;
                        }
                        match &target.request {
                            QuerySubscription::Full | QuerySubscription::Unlinked { .. } => true,
                            QuerySubscription::Registry => was.registry_rows_differ(now),
                            QuerySubscription::Query { .. } | QuerySubscription::Source { .. } => {
                                target.plan.as_ref().is_none_or(|plan| {
                                    plan.touches_edit(entry, before, after, was, now, &config)
                                })
                            }
                        }
                    });
            if touched || target.dirty || dirty {
                summary.query_to.insert(id.clone());
            }
            // A stale global dependency cannot classify future edits safely.
            // Keep firing until the scheduled reader renews its plan. In
            // particular, never rebuild the O(graph) tag-target set here.
            target.dirty |= dirty;
        }
        for (entry, before, after) in edits {
            if before == after {
                continue;
            }
            let key = page_key(&entry.name);
            summary.pages.insert(key.clone());
            // `(tag P)` reads path refs, including the source page and its
            // namespace ancestors. Backlinks exclude self; keep these keys
            // separate so typing in P does not refresh P's reference sections.
            for graph in [&old.graph, &self.graph] {
                for name in tine_core::refs::namespace_refs(&entry.name) {
                    summary
                        .tag_table_to
                        .extend(crate::query::reference_invalidation_names(graph, &name));
                }
            }
            changed_blocks(&before, &after, &mut summary.blocks);
            summary.full |= crate::query::document_alias_spellings(&before)
                != crate::query::document_alias_spellings(&after);
            let properties = |doc: &Document| {
                let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for (key, value) in crate::query::page_properties::page_facets(doc).0 {
                    result
                        .entry(property_key_norm(&key))
                        .or_default()
                        .push(value);
                }
                result
            };
            let was = properties(&before);
            let now = properties(&after);
            let keys: BTreeSet<_> = was
                .keys()
                .chain(now.keys())
                .filter(|key| was.get(*key) != now.get(*key))
                .cloned()
                .collect();
            if before.pre_block != after.pre_block {
                // Page rows expose all page properties, so edits to any of
                // them can change an already-tagged row, not just membership.
                for (graph, doc) in [(&old.graph, before), (&self.graph, after)] {
                    for tag in crate::query::page_properties::page_facets(doc).1 {
                        summary
                            .tags_to
                            .extend(crate::query::reference_invalidation_names(graph, &tag));
                    }
                }
            }
            if !keys.is_empty() {
                // Icon inheritance and tag readers may use an alias of the
                // source page. Reuse the answerer's component/day closure.
                let names = crate::query::reference_invalidation_names(&self.graph, &entry.name);
                for name in names {
                    summary
                        .props
                        .entry(name)
                        .or_default()
                        .extend(keys.iter().cloned());
                }
                summary.props.entry(key).or_default().extend(keys);
            }
            // The union, rather than symmetric difference, is intentional:
            // editing the text of an existing referrer changes its answer too.
            // The existing explicit-reference owner includes property policy,
            // tags and namespace ancestors; the answerer widens aliases/days.
            for (graph, doc) in [(&old.graph, &before), (&self.graph, &after)] {
                let header_changed = before.pre_block != after.pre_block;
                summary
                    .refs_to
                    .extend(crate::query::changed_block_reference_targets(
                        doc,
                        &summary.blocks,
                        header_changed,
                    ));
                let (header, selected) = crate::query::changed_reference_source_blocks(
                    entry,
                    doc,
                    graph.config(),
                    &summary.blocks,
                    header_changed,
                );
                for target in unlinked_targets {
                    if crate::query::changed_page_affects_unlinked(
                        graph,
                        target,
                        entry,
                        &selected,
                        header.as_ref(),
                    ) {
                        summary.text_to.insert(page_key(target));
                    }
                }
                for target in crate::query::changed_explicit_reference_names(
                    &entry,
                    doc,
                    graph.config(),
                    &summary.blocks,
                    before.pre_block != after.pre_block,
                ) {
                    let names = crate::query::reference_invalidation_names(graph, &target);
                    summary.links_to.extend(names);
                }
            }
        }
        summary
            .tag_table_to
            .extend(summary.links_to.iter().cloned());
        summary
    }
}

impl Store {
    /// One lifetime-bound registration door for mounted reference and query
    /// readers. Its typed dependency replaces the old unlinked-only parameter.
    /// Revoked/dropped bindings stop contributing without holding a slot alive.
    pub fn register_reader_while_live(
        &self,
        subscription: String,
        request: Option<QuerySubscription>,
        revoked: &Arc<AtomicBool>,
    ) -> Result<QueryRegistration, LoadError> {
        let weak = Some(Arc::downgrade(revoked));
        match request {
            Some(QuerySubscription::Unlinked { name }) => {
                let reply =
                    self.register_query_target(subscription.clone(), None, Some(revoked))?;
                self.register_unlinked_target(subscription, Some(name), weak);
                Ok(reply)
            }
            request => {
                self.register_unlinked_target(subscription.clone(), None, weak);
                self.register_query_target(subscription, request, Some(revoked))
            }
        }
    }

    /// Store-only oracle for query registration without a native binding.
    /// Production uses the lifetime-bound reader registration operation.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn register_query_subscription(
        &self,
        subscription: String,
        request: Option<QuerySubscription>,
        revoked: Option<&Arc<AtomicBool>>,
    ) -> Result<QueryRegistration, LoadError> {
        self.register_query_target(subscription, request, revoked)
    }

    /// Register/release a binding-owned live query. Returns its refresh mode;
    /// the registration precedes the reader's initial immutable-snapshot read.
    fn register_query_target(
        &self,
        subscription: String,
        request: Option<QuerySubscription>,
        revoked: Option<&Arc<AtomicBool>>,
    ) -> Result<QueryRegistration, LoadError> {
        let request = request.filter(|_| revoked.is_none_or(|flag| !flag.load(Ordering::Acquire)));
        let view = if request.is_some() {
            Some(self.whole_graph()?)
        } else {
            None
        };
        // Compile/capture global inputs before either shared lock: a requested
        // tag-target scan must not hold publication's registry or snapshot lock.
        let prepared_rev = view.as_ref().map(|view| view.rev.0);
        let mut target = request.map(|request| QueryTarget {
            plan: view
                .as_ref()
                .and_then(|view| QueryTarget::plan(&request, &view.graph)),
            request,
            added: 0,
            revoked: revoked.map(Arc::downgrade),
            dirty: false,
        });
        let mode = target.as_ref().map_or("full", QueryTarget::mode).to_owned();
        // Read-before-registry lock order matches publication. A capture/install
        // race fires late registrations; a newer installed snapshot makes this
        // prepared plan conservatively dirty until the reader next renews it.
        let current = self.changes.snapshot.read().unwrap();
        let current_rev = current.as_ref().map_or(0, |s| s.rev.0);
        let mut registry = self.changes.query_targets.lock().unwrap();
        registry.generation += 1;
        let previous = if target.as_ref().is_some_and(QueryTarget::live) {
            let mut next = target.take().unwrap();
            next.added = registry.generation;
            next.dirty = prepared_rev != Some(current_rev);
            registry
                .targets
                .insert(subscription, Arc::new(Mutex::new(next)))
        } else {
            registry.targets.remove(&subscription)
        };
        drop(registry);
        drop(current);
        // Dropping an obsolete tag-target set can also be O(graph).
        drop(previous);
        drop(target);
        Ok(QueryRegistration {
            mode,
            rev: current_rev.to_string(),
        })
    }

    /// Register or release a mounted unlinked reader. Transport-only, per
    /// binding; never persisted. Never waits on a writer (reads never wait on
    /// writes): a registration that lands while a publication is being
    /// captured is caught by that publication's install step, which runs
    /// under the snapshot write lock, so the reader's first read either sees
    /// the new snapshot or is invalidated by it.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn register_unlinked_reference(&self, subscription: String, target: Option<String>) {
        self.register_unlinked_target(subscription, target, None);
    }

    fn register_unlinked_target(
        &self,
        subscription: String,
        target: Option<String>,
        revoked: Option<std::sync::Weak<AtomicBool>>,
    ) {
        let mut registry = self.changes.unlinked_targets.lock().unwrap();
        registry.generation += 1;
        let added = registry.generation;
        if let Some(target) = target {
            let target = UnlinkedTarget {
                name: target,
                added,
                revoked,
            };
            if target.live() {
                registry.targets.insert(subscription, target);
            }
        } else {
            registry.targets.remove(&subscription);
        }
    }
}

// Align by the store's runtime identity, not a content hash or persisted id::.
// Include structural changes and ancestors so block embeds remain sound, and
// both runtime and external identities so resolve_blocks consumers can use it.
fn changed_blocks(before: &Document, after: &Document, out: &mut BTreeSet<String>) {
    fn flatten<'a>(
        roots: &'a [DocBlock],
        parent: &'a str,
        map: &mut HashMap<&'a str, (&'a DocBlock, &'a str, usize)>,
    ) {
        for (position, block) in roots.iter().enumerate() {
            map.insert(&block.uuid, (block, parent, position));
            flatten(&block.children, &block.uuid, map);
        }
    }
    let (mut was, mut now) = (HashMap::new(), HashMap::new());
    flatten(&before.roots, "", &mut was);
    flatten(&after.roots, "", &mut now);
    let mut changed = HashSet::new();
    for id in was.keys().chain(now.keys()) {
        let equal = match (was.get(id), now.get(id)) {
            (Some((a, ap, ai)), Some((b, bp, bi))) => {
                a.raw() == b.raw()
                    && a.is_org() == b.is_org()
                    && ap == bp
                    && ai == bi
                    && a.children
                        .iter()
                        .map(|b| &b.uuid)
                        .eq(b.children.iter().map(|b| &b.uuid))
            }
            _ => false,
        };
        if !equal {
            changed.insert(*id);
        }
    }
    fn emit(roots: &[DocBlock], changed: &HashSet<&str>, out: &mut BTreeSet<String>) -> bool {
        let mut any = false;
        for block in roots {
            let descendants = emit(&block.children, changed, out);
            if descendants || changed.contains(block.uuid.as_str()) {
                out.insert(block.uuid.clone());
                if let Some(id) = tine_core::refs::block_id(block.raw(), block.is_org()) {
                    out.insert(id);
                }
                any = true;
            }
        }
        any
    }
    emit(&before.roots, &changed, out);
    emit(&after.roots, &changed, out);
}
