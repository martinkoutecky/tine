//! I-12/I-25: the publication owns derived-answer deltas. Compare only changed
//! parsed pages through the existing native answerers; never rebuild inventory.
use super::*;
use serde::ser::SerializeStruct;

// Change serializes only its bounded answer signal. This is the save/watcher
// transport contract: rev orders per-target updates, zero clears a badge, and
// inventoryChanged invalidates names when their source contributions change.
impl serde::Serialize for Change {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("GraphAnswers", 3)?;
        wire.serialize_field("rev", &self.graph_rev.0.to_string())?;
        wire.serialize_field("inventoryChanged", &self.answers.0)?;
        wire.serialize_field("blockRefCounts", &self.answers.1)?;
        wire.end()
    }
}

impl Snapshot {
    pub(super) fn answer_changes(
        &self,
        old: Option<&Self>,
        paths: &[String],
        names_changed: bool,
    ) -> (bool, BTreeMap<String, usize>) {
        let Some(old) = old else {
            return (
                true,
                self.graph
                    .block_ref_counts()
                    .iter()
                    .map(|(id, count)| (id.clone(), *count))
                    .collect(),
            );
        };
        let mut inventory = names_changed || self.unreadable != old.unreadable;
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
            let names = |doc: Option<&tine_core::doc::Document>| {
                doc.map(|doc| {
                    (
                        crate::query::document_aliases(doc)
                            .into_iter()
                            .collect::<HashSet<_>>(),
                        crate::model::collect_document_referenced_names(doc)
                            .into_iter()
                            .collect::<HashSet<_>>(),
                    )
                })
                .unwrap_or_default()
            };
            inventory |= names(before.as_deref()) != names(after.as_deref());
        }
        let before = old.graph.block_ref_counts();
        let after = self.graph.block_ref_counts();
        let counts = targets
            .into_iter()
            .filter_map(|id| {
                let count = after.get(&id).copied().unwrap_or(0);
                (count != before.get(&id).copied().unwrap_or(0)).then_some((id, count))
            })
            .collect();
        (inventory, counts)
    }
}
