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
    /// Both families of a friendly Search tab, using execution's text plan.
    Search {
        /// Complete friendly search source.
        source: String,
        /// Which physical pages may match through their content.
        page_match_scope: tine_core::query::ir::FriendlyPageMatchScope,
        /// Include the same page-name expansion as the displayed answer.
        transliteration: bool,
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
    search_plan: Option<crate::query_plan::QueryPlan>,
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
            QuerySubscription::Search { .. } => "narrow",
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
            search_plan: None,
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
    days: bool,
    journals: bool,
    templates: bool,
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

/// A publication changing more paths than this publishes `full`; a page
/// lifecycle decides that before any per-path preparation (`snapshot.rs`).
pub(super) const PRECISE_PATHS: usize = 32;

/// One page's active entry and document on one side of a publication.
type PageSide = (PageEntry, Arc<Document>);

/// A publication that moved page names, as the edit list the summary already
/// reads: a page that left is an edit to the empty document, one that came
/// is an edit from it, so its own blocks, references, properties, tags and
/// templates are named as for any edit (one producer, AR-1).
struct Lifecycle {
    edits: Vec<(PageEntry, Arc<Document>, Arc<Document>)>,
    /// Per edit: a page that came or left, which counts as changed even with
    /// an empty document (a created empty page still changes its readers).
    moved: Vec<bool>,
    /// The entries whose name came (`true`, this generation) or left.
    names: Vec<(bool, PageEntry)>,
}

impl Lifecycle {
    fn of(changes: &[(Option<PageSide>, Option<PageSide>)]) -> Self {
        let empty = Arc::new(Document::default());
        let mut lifecycle = Lifecycle {
            edits: Vec::new(),
            moved: Vec::new(),
            names: Vec::new(),
        };
        for change in changes {
            match change {
                (Some((was, before)), Some((now, after)))
                    if was.name == now.name && was.kind == now.kind =>
                {
                    lifecycle
                        .edits
                        .push((now.clone(), Arc::clone(before), Arc::clone(after)));
                    lifecycle.moved.push(false);
                }
                (before, after) => {
                    if let Some((was, doc)) = before {
                        lifecycle
                            .edits
                            .push((was.clone(), Arc::clone(doc), Arc::clone(&empty)));
                        lifecycle.moved.push(true);
                        lifecycle.names.push((false, was.clone()));
                    }
                    if let Some((now, doc)) = after {
                        lifecycle
                            .edits
                            .push((now.clone(), Arc::clone(&empty), Arc::clone(doc)));
                        lifecycle.moved.push(true);
                        lifecycle.names.push((true, now.clone()));
                    }
                }
            }
        }
        lifecycle
    }

    /// An ordinary edit list, owned so fileless rows can join it.
    fn of_edits(edits: &[(PageEntry, Arc<Document>, Arc<Document>)]) -> Self {
        Lifecycle {
            moved: vec![false; edits.len()],
            edits: edits.to_vec(),
            names: Vec::new(),
        }
    }
}

/// The pages without a file a publication brought or removed, as `(came,
/// row)`: a namespace child brings its parent and its last one removes it, a
/// page's references bring reference-only pages, and a file taking or leaving
/// a name removes or brings that name's fileless row (review F1). Candidates:
/// a moved page's name and references, an edited page's references that came
/// or went (none while typing plain text), and the namespace prefixes of
/// both, each looked up on both sides. `None` when there are candidates but a
/// side's fileless inputs are not resident (never built here): the caller then
/// treats every reader that can list one as touched.
fn fileless_changes(
    old: &ReadSnapshot,
    new: &ReadSnapshot,
    edits: &[(PageEntry, Arc<Document>, Arc<Document>)],
    moved: impl Fn(usize) -> bool,
) -> Option<Vec<(bool, crate::model::fileless_names::FilelessName)>> {
    let mut spellings = BTreeSet::new();
    for (i, (entry, before, after)) in edits.iter().enumerate() {
        let moved = moved(i);
        if before == after && !moved {
            continue;
        }
        let refs = |graph: &ReadSnapshot, doc: &Document| -> BTreeSet<String> {
            crate::model::collect_document_referenced_names(doc, graph.config())
                .into_iter()
                .collect()
        };
        let (was, now) = (refs(old, before), refs(new, after));
        let mut names: Vec<String> = if moved {
            was.union(&now).cloned().collect()
        } else {
            was.symmetric_difference(&now).cloned().collect()
        };
        if moved {
            names.push(entry.name.clone());
        }
        for name in names {
            spellings.extend(
                crate::model::namespace_parents::parent_prefixes(&name)
                    .map(|(prefix, _)| prefix.to_owned()),
            );
            spellings.insert(name);
        }
    }
    if spellings.is_empty() {
        return Some(Vec::new());
    }
    let rows = |graph: &ReadSnapshot| {
        let mut rows = BTreeMap::new();
        for name in &spellings {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::lifecycle_probe();
            if let Some(row) = graph.resident_reference_only_page(name)? {
                rows.insert(page_key(&row.name), row);
            }
        }
        Some(rows)
    };
    let (was, now) = (rows(old)?, rows(new)?);
    let mut changes = Vec::new();
    for key in was.keys().chain(now.keys()).collect::<BTreeSet<_>>() {
        let (was, now) = (was.get(key), now.get(key));
        if was != now {
            changes.extend(was.map(|row| (false, row.clone())));
            changes.extend(now.map(|row| (true, row.clone())));
        }
    }
    Some(changes)
}

impl Snapshot {
    /// `page_changes`: when `names_changed`, both sides of every changed path
    /// (`ReadSnapshot::page_changes`); `None` publishes `full`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn change_summary(
        &self,
        old: Option<&Self>,
        paths: &[String],
        names_changed: bool,
        inventory_changed: bool,
        moved_keys: &[String],
        edits: Option<
            &[(
                PageEntry,
                Arc<tine_core::doc::Document>,
                Arc<tine_core::doc::Document>,
            )],
        >,
        page_changes: Option<&[(Option<PageSide>, Option<PageSide>)]>,
        unlinked_targets: &[String],
        query_targets: &[(String, Arc<Mutex<QueryTarget>>)],
    ) -> ChangeSummary {
        let page_changes = page_changes.filter(|_| names_changed);
        let mut summary = ChangeSummary {
            rev: self.rev.0.to_string(),
            inventory_changed,
            full: names_changed && page_changes.is_none() || paths.len() > PRECISE_PATHS,
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
        let mut lifecycle = page_changes.map(Lifecycle::of);
        let Some(current) = lifecycle.as_ref().map_or(edits, |l| Some(&l.edits[..])) else {
            summary.full = true; // create, delete, rename, or unavailable old bytes
            dirty_query_plans(query_targets);
            return summary;
        };
        // Each fileless row that came or left is a moved edit of the empty
        // document under its fileless entry (the row query execution
        // evaluates, `exec_candidates.rs`), so the same classifier names its
        // readers, and its name is a name that came or left. Not compared
        // when already `full` (every reader fires).
        let fileless = if summary.full {
            Some(Vec::new())
        } else {
            fileless_changes(&old.graph, &self.graph, current, |i| {
                lifecycle.as_ref().is_some_and(|l| l.moved[i])
            })
        };
        let fileless_unknown = fileless.is_none();
        if let Some(rows) = fileless.filter(|rows| !rows.is_empty()) {
            if lifecycle.is_none() {
                lifecycle = edits.map(Lifecycle::of_edits);
            }
            let lifecycle = lifecycle.as_mut().expect("an edit list");
            let empty = Arc::new(Document::default());
            for (came, row) in rows {
                let entry = row.page_entry();
                lifecycle
                    .edits
                    .push((entry.clone(), Arc::clone(&empty), Arc::clone(&empty)));
                lifecycle.moved.push(true);
                lifecycle.names.push((came, entry));
            }
        }
        let edits = lifecycle
            .as_ref()
            .map_or_else(|| edits.unwrap_or_default(), |l| &l.edits[..]);
        let moved = |i: usize| lifecycle.as_ref().is_some_and(|l| l.moved[i]);
        // An alias edit regroups pages: a reference query matches a page's
        // whole alias group (D-20261019-Q8), so it can change an answer on a
        // page the edit never touched. Decided before the query readers are
        // classified, so they fire and renew their plans (CM step 2 batch 9).
        summary.full |= edits.iter().any(|(_, before, after)| {
            crate::query::document_alias_spellings(before)
                != crate::query::document_alias_spellings(after)
        });
        // A name that came or left. Its own page's readers, and every reader
        // of a name in its group (old and new generation), are named; an
        // alias another page declares for it regroups pages (`full`, as an
        // alias edit). A name that spells a journal day (a journal, or an
        // ordinary page named like a date) reshapes the day classes that
        // reference queries hold (`ref_classes.rs`), so those plans renew.
        let mut day_keys = Vec::new();
        for (_, entry) in lifecycle.iter().flat_map(|l| &l.names) {
            let key = page_key(&entry.name);
            summary.full |= [&old.graph, &self.graph]
                .iter()
                .any(|graph| graph.alias_edges().neighbors(&key).next().is_some());
            // A fileless day is a virtual journal: a day, not a journal file.
            let day =
                entry.kind == PageKind::Journal || self.journal_format.parse(&entry.name).is_some();
            let journal = entry.kind == PageKind::Journal && entry.rel_path.is_some();
            if day {
                day_keys.push(key.clone());
            }
            summary.journals |= journal;
            summary.days |= day;
            summary.pages.insert(key);
            for graph in [&old.graph, &self.graph] {
                for name in crate::query::reference_invalidation_names(graph, &entry.name) {
                    summary.links_to.insert(name.clone());
                    summary.text_to.insert(name.clone());
                    summary.tags_to.insert(name);
                }
            }
        }
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
                || !day_keys.is_empty()
                    && target.plan.as_ref().is_some_and(|plan| {
                        plan.reference_classes_reshaped_by(&old.graph, &day_keys)
                    })
                || facts.iter().any(|(was, now)| {
                    target.plan.as_ref().is_some_and(|plan| {
                        plan.registry().is_some() && was.registry_rows_differ(now)
                            || plan.reads_tag_targets() && was.tags() != now.tags()
                    })
                });
            // A page that started or stopped existing without a file of its
            // own (answer_changes' moved keys) is in no edited document.
            let listed = target
                .plan
                .as_ref()
                .is_some_and(|plan| plan.may_list_fileless(moved_keys, &self.journal_format));
            // Time can change a displayed answer even when this publication
            // has no changed document to pass to the shared edit predicate.
            // Fileless rows that could not be compared (their inputs not
            // resident): every query that can list one is touched. (Search
            // already offers an edited page's referenced names itself.)
            let touched = target.plan.as_ref().is_some_and(Plan::reads_clock)
                || fileless_unknown
                    && matches!(
                        target.request,
                        QuerySubscription::Query { .. } | QuerySubscription::Source { .. }
                    )
                    && target.plan.as_ref().is_none_or(Plan::lists_fileless_pages)
                || edits.iter().zip(&facts).enumerate().any(
                    |(i, ((entry, before, after), (was, now)))| {
                        if before == after && !moved(i) {
                            return false;
                        }
                        match &target.request {
                            QuerySubscription::Full | QuerySubscription::Unlinked { .. } => true,
                            QuerySubscription::Registry => was.registry_rows_differ(now),
                            QuerySubscription::Search { .. } => {
                                target.search_plan.as_ref().is_none_or(|plan| {
                                    plan.touches_search_document(entry, before, &old.graph)
                                        || plan.touches_search_document(entry, after, &self.graph)
                                })
                            }
                            QuerySubscription::Query { .. } | QuerySubscription::Source { .. } => {
                                target.plan.as_ref().is_none_or(|plan| {
                                    plan.touches_edit(
                                        &self.graph,
                                        entry,
                                        before,
                                        after,
                                        was,
                                        now,
                                        &config,
                                    )
                                })
                            }
                        }
                    },
                );
            if touched || listed || target.dirty || dirty {
                summary.query_to.insert(id.clone());
            }
            // A stale global dependency cannot classify future edits safely.
            // Keep firing until the scheduled reader renews its plan. In
            // particular, never rebuild the O(graph) tag-target set here.
            target.dirty |= dirty;
        }
        for (i, (entry, before, after)) in edits.iter().enumerate() {
            if before == after && !moved(i) {
                continue;
            }
            let key = page_key(&entry.name);
            summary.pages.insert(key.clone());
            let day = entry.kind == PageKind::Journal;
            // `journals` means journal files (the calendar dots); a fileless
            // day row is a virtual journal and sets only `days`.
            summary.journals |= day && entry.rel_path.is_some();
            // `days` is the feed's day set and Upcoming (files listed, days
            // referenced, planning). Editing an existing journal changes none
            // of them by itself; its body reaches a loaded feed day through
            // the page's own reload (D-20261102-PERF, audit row 9). A journal
            // page that came or left (create, delete, rename, or a fileless
            // day row) changes the listed days.
            summary.days |= day && moved(i);
            //
            // Inspect parsed facts of this edited page only, on both sides so
            // removing planning/template metadata invalidates its old answer.
            for doc in [before, after] {
                let (planning, templates) = document_reader_facts(doc);
                summary.days |= planning;
                summary.templates |= templates;
            }
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
                        graph, target, entry, &selected, &header,
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
        // The feed's referenced-day inventory uses these same explicit names
        // and journal parser. Alias changes are full; the widened closure also
        // covers journal aliases and configured/ISO day spellings. No registry
        // or referenced-day inventory is scanned during publication.
        summary.days |= summary
            .links_to
            .iter()
            .any(|name| self.journal_format.parse(name).is_some());
        summary
    }
}

// Presence is conservative: editing any part of a template-bearing document
// can change its insertion DTO. Do not build/copy template subtrees here.
fn document_reader_facts(doc: &Document) -> (bool, bool) {
    fn blocks(roots: &[DocBlock], planning: &mut bool, templates: &mut bool) {
        for block in roots {
            *planning |= block.scheduled().is_some() || block.deadline().is_some();
            *templates |= block
                .property("template")
                .is_some_and(|name| !name.is_empty());
            blocks(&block.children, planning, templates);
        }
    }
    let mut templates = doc.pre_block.as_deref().is_some_and(|pre| {
        tine_core::template::may_define_template(pre.as_bytes())
            && tine_core::page_properties::document_page_property_block(doc).is_some_and(|block| {
                block
                    .property("template")
                    .is_some_and(|name| !name.is_empty())
            })
    });
    let mut planning = false;
    blocks(&doc.roots, &mut planning, &mut templates);
    (planning, templates)
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
            search_plan: view.as_ref().and_then(|view| match &request {
                QuerySubscription::Search {
                    source,
                    page_match_scope,
                    transliteration,
                } => {
                    let mut plan = crate::query_plan::QueryPlan::friendly_with_scope(
                        source,
                        1,
                        1,
                        view.graph.config().enable_search_remove_accents,
                        *page_match_scope,
                    )
                    .with_transliteration(*transliteration);
                    plan.name_keys = Some(Arc::clone(&view.graph.transliteration_names));
                    Some(plan)
                }
                _ => None,
            }),
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
