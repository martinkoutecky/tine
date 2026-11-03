//! I-12/I-25: the publication owns derived-answer deltas. Compare only changed
//! parsed pages through the existing native answerers; never rebuild inventory.
use super::*;
use serde::ser::SerializeStruct;
use std::collections::BTreeSet;

// Change serializes only its bounded answer signal. This is the save/watcher
// transport contract: rev orders per-target updates, zero clears a badge, and
// inventoryChanged invalidates names when their source contributions change.
impl serde::Serialize for Change {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("GraphAnswers", 4)?;
        wire.serialize_field("rev", &self.graph_rev.0.to_string())?;
        wire.serialize_field("inventoryChanged", &self.answers.0)?;
        wire.serialize_field("blockRefCounts", &self.answers.1)?;
        wire.serialize_field("summary", &self.answers.2)?;
        wire.end()
    }
}

impl Snapshot {
    /// Whether the inventory changed, its delta when expressible (see
    /// [`InventoryDelta`]), and the final changed-target block-ref counts.
    /// A saved page's own alias/reference-name change is an inventory change
    /// only when the graph-wide answer for one of its keys moved: typing
    /// inside `[[Existing Page]]` changes nothing graph-wide. Cost O(changed
    /// pages x their names x log V) with the resident name table and alias
    /// relation; without either (not built for this generation) any own-set
    /// change refetches, as before, and nothing is built here.
    pub(super) fn answer_changes(
        &self,
        old: Option<&Self>,
        paths: &[String],
        names_changed: bool,
    ) -> (bool, Option<Arc<InventoryDelta>>, BTreeMap<String, usize>) {
        let Some(old) = old else {
            return (
                true,
                None,
                self.graph
                    .block_ref_counts()
                    .iter()
                    .map(|(id, count)| (id.clone(), *count))
                    .collect(),
            );
        };
        let full = names_changed || self.unreadable != old.unreadable;
        let mut own_names_changed = false;
        let mut keys: BTreeSet<String> = BTreeSet::new();
        let mut targets = HashSet::new();
        for path in paths {
            let page = |snapshot: &Self| {
                snapshot
                    .graph
                    .pages
                    .positions
                    .get(path)
                    .and_then(|&slot| snapshot.graph.pages.get(slot))
                    .map(|(_, doc)| Arc::clone(doc))
            };
            let before = page(old);
            let after = page(self);
            let counts = |doc: Option<&tine_core::doc::Document>| {
                doc.map(crate::model::document_block_ref_counts)
                    .unwrap_or_default()
            };
            let before_counts = counts(before.as_deref());
            let after_counts = counts(after.as_deref());
            targets.extend(
                before_counts
                    .keys()
                    .chain(after_counts.keys())
                    .filter(|id| before_counts.get(*id) != after_counts.get(*id))
                    .cloned(),
            );
            if full {
                continue;
            }
            // Each key this page aliases or references, with the page's own
            // spelling of a reference (the spelling the graph-wide table may use).
            let names = |doc: Option<&tine_core::doc::Document>,
                         config: &tine_core::config::Config| {
                let mut aliases = BTreeSet::new();
                let mut references = BTreeMap::new();
                if let Some(doc) = doc {
                    aliases.extend(
                        crate::query::document_aliases(doc)
                            .iter()
                            .map(|alias| tine_core::refs::page_key(alias)),
                    );
                    for name in crate::model::collect_document_referenced_names(doc, config) {
                        references.insert(tine_core::refs::page_key(&name), name);
                    }
                }
                (aliases, references)
            };
            let (aliases_before, references_before) = names(before.as_deref(), old.graph.config());
            let (aliases_after, references_after) = names(after.as_deref(), self.graph.config());
            keys.extend(aliases_before.symmetric_difference(&aliases_after).cloned());
            // A page that moved slot can change which page spells a key.
            let moved = old.graph.pages.positions.get(path) != self.graph.pages.positions.get(path);
            // A changed name also changes its namespace parents' entries
            // (`model::namespace_parents`): a parent appears with its first
            // child, goes with its last, and can change spelling.
            for (key, name) in references_before
                .iter()
                .chain(references_after.iter())
                .filter(|(key, _)| {
                    moved || references_before.get(*key) != references_after.get(*key)
                })
            {
                keys.insert(key.clone());
                if name.contains('/') {
                    keys.extend(
                        crate::model::namespace_parents::parent_prefixes(name).map(|(_, key)| key),
                    );
                }
            }
        }
        own_names_changed |= !keys.is_empty();
        // The delta reads only what both generations already hold: the name
        // table and the alias relation (`name_entry` asks the relation for a
        // key's owners). `[[` completion builds the table without the
        // relation; building the relation here would walk every page inside
        // the publication, so a generation without it refetches instead.
        fn resident(
            snapshot: &Snapshot,
        ) -> Option<&crate::model::referenced_names::ReferencedNameTable> {
            snapshot
                .graph
                .referenced_name_table_built()
                .filter(|_| snapshot.graph.alias_list_built())
        }
        let (inventory, delta) = if full {
            (true, None)
        } else if !own_names_changed {
            (false, None)
        } else if let (Some(before), Some(after)) = (resident(old), resident(self)) {
            let mut delta = InventoryDelta::default();
            for key in keys {
                let now = self.name_entry(after, &key);
                if old.name_entry(before, &key) != now {
                    delta.keys.push(key);
                    delta.entries.0.extend(now);
                }
            }
            if delta.keys.is_empty() {
                (false, None)
            } else {
                (true, Some(Arc::new(delta)))
            }
        } else {
            (true, None)
        };
        let before = old.graph.block_ref_counts();
        let after = self.graph.block_ref_counts();
        let counts = targets
            .into_iter()
            .filter_map(|id| {
                let count = after.get(&id).copied().unwrap_or(0);
                (count != before.get(&id).copied().unwrap_or(0)).then_some((id, count))
            })
            .collect();
        (inventory, delta, counts)
    }
}
