//! Transport-only invalidation evidence, derived from edited parsed pages.
//! No reader subscriptions or graph traversal on an ordinary publication.
use super::*;
use std::collections::BTreeSet;
use tine_core::doc::{property_key_norm, DocBlock, Document};
use tine_core::refs::page_key;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChangeSummary {
    rev: String,
    pages: BTreeSet<String>,
    blocks: BTreeSet<String>,
    links_to: BTreeSet<String>,
    props: BTreeMap<String, BTreeSet<String>>,
    inventory_changed: bool,
    full: bool,
    #[serde(skip)]
    day: Option<i64>,
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
