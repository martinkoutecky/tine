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
        let mut wire = serializer.serialize_struct("GraphAnswers", 6)?;
        wire.serialize_field("rev", &self.graph_rev.0.to_string())?;
        wire.serialize_field("inventoryChanged", &self.answers.0)?;
        wire.serialize_field("blockRefCounts", &self.answers.1)?;
        wire.serialize_field("summary", &self.answers.2)?;
        wire.serialize_field("pageModified", &self.answers.4.modified)?;
        wire.serialize_field("pageBacklinkCounts", &self.answers.4.counts)?;
        wire.end()
    }
}

impl Snapshot {
    /// Whether the inventory changed, its delta when expressible (see
    /// [`InventoryDelta`]), the final changed-target block-ref counts, and
    /// the page keys whose inventory entry moved: the delta's keys, or every
    /// changed own name when the delta is not expressible (conservative), and
    /// none when the answer is full or nothing moved. A page that starts or
    /// stops existing without a file of its own (a reference-only page, a
    /// namespace parent) is among them, for the readers that can list it.
    /// A saved page's own alias/reference-name change is an inventory change
    /// only when the graph-wide answer for one of its keys moved: typing
    /// inside `[[Existing Page]]` changes nothing graph-wide. Cost O(changed
    /// pages x their names x log V) with the resident name table and alias
    /// relation, a namespace parent's entry being one read of the resident
    /// parents support (`name_entry`); without either (not built for this
    /// generation) any own-set change refetches, as before, and nothing is
    /// built here.
    pub(super) fn answer_changes(
        &self,
        old: Option<&Self>,
        paths: &[String],
        names_changed: bool,
    ) -> (
        bool,
        Option<Arc<InventoryDelta>>,
        BTreeMap<String, usize>,
        Vec<String>,
    ) {
        let Some(old) = old else {
            return (
                true,
                None,
                self.graph
                    .block_ref_counts()
                    .iter()
                    .map(|(id, count)| (id.clone(), *count))
                    .collect(),
                Vec::new(),
            );
        };
        let full = names_changed || self.unreadable != old.unreadable;
        let mut own_names_changed = false;
        let mut keys: BTreeSet<String> = BTreeSet::new();
        let mut days = BTreeMap::new();
        let remember_day = |days: &mut BTreeMap<String, JournalDate>, name: &str| {
            let answer =
                crate::model::fileless_names::FilelessName::new(&self.journal_format, name);
            if let Some(day) = answer.day {
                days.insert(tine_core::refs::page_key(&answer.name), day);
            }
        };
        let mut targets = HashSet::new();
        for path in paths {
            let page = |snapshot: &Self| {
                snapshot
                    .graph
                    .pages
                    .positions
                    .get(path)
                    .and_then(|&slot| snapshot.graph.pages.get(slot))
                    .map(|(entry, doc)| (entry.name.clone(), Arc::clone(doc)))
            };
            let (before_name, before) = page(old).unzip();
            let (after_name, after) = page(self).unzip();
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
            for key in aliases_before.symmetric_difference(&aliases_after) {
                keys.insert(key.clone());
            }
            for doc in [before.as_deref(), after.as_deref()].into_iter().flatten() {
                for name in crate::query::document_alias_spellings(doc) {
                    if keys.contains(&tine_core::refs::page_key(&name)) {
                        remember_day(&mut days, &name);
                    }
                }
            }
            // A page file's own name seeds its namespace parents only while
            // its content is not blank, so gaining its first text or losing
            // its last starts or ends them (`model::namespace_parents`).
            let supplies = |doc: Option<&Arc<tine_core::doc::Document>>| {
                doc.is_some_and(|doc| {
                    crate::model::namespace_parents::document_supplies_parents(doc)
                })
            };
            if supplies(before.as_ref()) != supplies(after.as_ref()) {
                for name in before_name.iter().chain(&after_name) {
                    for (prefix, key) in crate::model::namespace_parents::parent_prefixes(name) {
                        keys.insert(key);
                        remember_day(&mut days, prefix);
                    }
                }
            }
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
                remember_day(&mut days, name);
                if name.contains('/') {
                    for (prefix, key) in crate::model::namespace_parents::parent_prefixes(name) {
                        keys.insert(key);
                        remember_day(&mut days, prefix);
                    }
                }
            }
        }
        // A raw date spelling supports its canonical day entity. Include
        // that key even when this edit removed its last raw spelling.
        // Keep the typed day from the original spelling: configured literal
        // tokens can be case-sensitive, so a lowercased key is not a date input.
        keys.extend(days.keys().cloned());
        own_names_changed |= !keys.is_empty();
        // The delta reads only resident inputs. Completion can build the
        // name table without day support: ordinary edits still get deltas,
        // while a date edit refetches rather than building support by a
        // graph walk inside publication. Alias owners must also be resident.
        fn resident(
            snapshot: &Snapshot,
            needs_days: bool,
        ) -> Option<&crate::model::referenced_names::ReferencedNameTable> {
            snapshot.graph.referenced_name_table_built().filter(|_| {
                snapshot.graph.alias_list_built()
                    && (!needs_days || snapshot.graph.day_support_built())
            })
        }
        let (inventory, delta, moved) = if full {
            (true, None, Vec::new())
        } else if !own_names_changed {
            (false, None, Vec::new())
        } else if let (Some(before), Some(after)) = (
            resident(old, !days.is_empty()),
            resident(self, !days.is_empty()),
        ) {
            let mut delta = InventoryDelta::default();
            for key in keys {
                let day = days.get(&key).copied();
                let now = self.name_entry(after, &key, day);
                if old.name_entry(before, &key, day) != now {
                    delta.keys.push(key);
                    delta.entries.0.extend(now);
                }
            }
            if delta.keys.is_empty() {
                (false, None, Vec::new())
            } else {
                let moved = delta.keys.clone();
                (true, Some(Arc::new(delta)), moved)
            }
        } else {
            (true, None, keys.into_iter().collect())
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
        (inventory, delta, counts, moved)
    }
}
