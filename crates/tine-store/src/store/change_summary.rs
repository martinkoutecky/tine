//! Transport-only invalidation evidence, derived from edited parsed pages.
//! Only mounted unlinked targets are checked; no graph traversal on publication.
use super::*;
use std::collections::BTreeSet;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::refs::page_key;

/// Mounted unlinked readers, with a registration counter so a publication can
/// tell which registrations arrived while it was being captured.
#[derive(Default)]
pub(super) struct UnlinkedRegistry {
    pub(super) generation: u64,
    pub(super) targets: BTreeMap<String, UnlinkedTarget>,
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
    props: BTreeMap<String, BTreeSet<String>>,
    inventory_changed: bool,
    full: bool,
    #[serde(skip)]
    day: Option<i64>,
}

impl ChangeSummary {
    /// A target registered after the publication snapshotted the registry:
    /// its reader may have read the snapshot this publication replaces, so
    /// the publication fires its key (over-invalidation, never staleness).
    pub(super) fn add_late_unlinked_target(&mut self, name: &str) {
        self.text_to.insert(page_key(name));
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
            return summary;
        };
        summary.full |= old.answers.2.day != summary.day || self.unreadable != old.unreadable;
        let Some(edits) = edits else {
            summary.full = true; // create, delete, rename, or unavailable old bytes
            return summary;
        };
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

    /// Native binding lifetime: revoked or dropped bindings stop contributing
    /// immediately, even when the same Store is adopted by a replacement slot.
    /// Dead registrations are pruned on publication; this holds no slot alive.
    pub fn register_unlinked_reference_while_live(
        &self,
        subscription: String,
        target: Option<String>,
        revoked: &Arc<AtomicBool>,
    ) {
        self.register_unlinked_target(subscription, target, Some(Arc::downgrade(revoked)));
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
