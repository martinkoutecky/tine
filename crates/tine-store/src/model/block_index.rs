//! The snapshot block index, including exact and case-insensitive candidates.
use super::*;

/// Block id (runtime uuid or persisted `id::`) → the pages that hold it.
/// Exact for the pages it was captured against: a full build walks every
/// page, and each publication swaps a changed path's previous ids for its
/// current ones, so an id the index lacks is held by no page (I-13: a lookup
/// never needs a whole-graph scan). Slots are `Pages` slots, which never move
/// for a live path, are never reused, and ascend in page order. This rests on
/// the same publication delta (`changed_paths` names every path whose
/// document changed) that the reference and explicit indexes rest on.
#[derive(Clone)]
pub(super) struct SnapshotBlockIndex {
    pub(super) uuids: inline_map::InlineMap<u128, BlockOwners>,
    pub(super) uuid_overlay: inline_map::InlineMap<u128, Option<BlockOwners>>,
    pub(super) base: Arc<SharedMap<(String, String), BlockOwners>>,
    /// Ids changed since the last fold; `None`: no page holds it any more.
    pub(super) overlay: Arc<SharedMap<(String, String), Option<BlockOwners>>>,
}

impl SnapshotBlockIndex {
    // Persistent roots share untouched ids; folding N/8 changed ids into the
    // base updates those tree paths without copying all N base entries.

    pub(super) fn authored_id(block: &DocBlock) -> Option<&str> {
        block
            .projection()
            .properties()
            .iter()
            .find(|(key, _)| key.trim().eq_ignore_ascii_case("id"))
            .map(|(_, value)| value.as_str())
            .filter(|id| !id.is_empty())
    }

    pub(super) fn for_each_block_id<'a>(doc: &'a Document, mut visit: impl FnMut(&'a str)) {
        pub(super) fn walk<'a>(blocks: &'a [DocBlock], visit: &mut dyn FnMut(&'a str)) {
            for block in blocks {
                if !block.uuid.is_empty() {
                    visit(&block.uuid);
                }
                if let Some(id) = SnapshotBlockIndex::authored_id(block) {
                    visit(id);
                }
                walk(&block.children, visit);
            }
        }
        walk(&doc.roots, &mut visit);
    }

    #[cfg(test)]
    pub(super) fn block_ids(doc: &Document) -> std::collections::HashSet<String> {
        let mut ids = std::collections::HashSet::new();
        Self::for_each_block_id(doc, |id| {
            ids.insert(id.to_string());
        });
        ids
    }

    pub(super) fn fold_limit(&self) -> usize {
        (self.len() / 8).max(1)
    }

    pub(super) fn len(&self) -> usize {
        self.uuids.len() + self.base.len()
    }

    // Index keys and authored values are case-sensitive. UUID parsing alone
    // would collapse uppercase, simple and URN spellings that were distinct.
    pub(super) fn compact_uuid(id: &str) -> Option<u128> {
        if id.len() != 36 || id.bytes().any(|b| b.is_ascii_uppercase()) {
            return None;
        }
        uuid::Uuid::try_parse(id).ok().map(|id| id.as_u128())
    }

    pub(super) fn rows(&self) -> impl Iterator<Item = (String, &BlockOwners)> {
        let uuids = self
            .uuids
            .iter()
            .filter(|(id, _)| self.uuid_overlay.get(id).is_none())
            .chain(
                self.uuid_overlay
                    .iter()
                    .filter_map(|(id, owners)| owners.as_ref().map(|owners| (id, owners))),
            )
            .map(|(id, owners)| (uuid::Uuid::from_u128(*id).to_string(), owners));
        let strings = self
            .base
            .iter()
            .filter(|(id, _)| self.overlay.get(id).is_none())
            .chain(
                self.overlay
                    .iter()
                    .filter_map(|(id, owners)| owners.as_ref().map(|owners| (id, owners))),
            )
            .map(|(id, owners)| (id.1.clone(), owners));
        uuids.chain(strings)
    }

    pub(super) fn owners(&self, id: &str) -> Option<&BlockOwners> {
        if let Some(id) = Self::compact_uuid(id) {
            return match self.uuid_overlay.get(&id) {
                Some(changed) => changed.as_ref(),
                None => self.uuids.get(&id),
            };
        }
        let key = (id.trim().to_ascii_lowercase(), id.to_owned());
        match self.overlay.get(&key) {
            Some(changed) => changed.as_ref(),
            None => self.base.get(&key),
        }
    }

    /// Apply `change` to `id`'s ascending owner slots, recording only a change.
    pub(super) fn edit(&mut self, id: &str, change: impl FnOnce(&mut Vec<usize>)) {
        let prior = self.owners(id);
        let mut slots = prior.map_or_else(Vec::new, |owners| owners.slots().to_vec());
        change(&mut slots);
        let next = BlockOwners::from_slots(slots);
        if next.as_ref() != prior {
            if let Some(id) = Self::compact_uuid(id) {
                self.uuid_overlay.insert(id, next);
            } else {
                Arc::make_mut(&mut self.overlay)
                    .insert((id.trim().to_ascii_lowercase(), id.to_owned()), next);
            }
        }
    }

    /// Full build over `pages`, or `old` carried across a publication that
    /// changed `changed_paths`: one cold sort, changed blocks' tree paths live.
    pub(super) fn capture(
        old: Option<(&Self, &Arc<Pages>)>,
        pages: &Arc<Pages>,
        changed_paths: &[String],
    ) -> Self {
        let slot_page = |pages: &'_ Pages, rel: &str| {
            pages
                .positions
                .get(rel)
                .and_then(|&slot| pages.get(slot).map(|(_, doc)| (slot, Arc::clone(doc))))
        };
        if let Some((previous, previous_pages)) = old.filter(|(_, previous_pages)| {
            !changed_paths.is_empty() || Arc::ptr_eq(previous_pages, pages)
        }) {
            let mut index = previous.clone();
            for rel_path in changed_paths {
                if let Some((slot, doc)) = slot_page(previous_pages, rel_path) {
                    Self::for_each_block_id(&doc, |id| {
                        index.edit(id, |slots| slots.retain(|owner| *owner != slot));
                    });
                }
                if let Some((slot, doc)) = slot_page(pages, rel_path) {
                    Self::for_each_block_id(&doc, |id| {
                        index.edit(id, |slots| {
                            if let Err(at) = slots.binary_search(&slot) {
                                slots.insert(at, slot);
                            }
                        });
                    });
                }
            }
            if index.overlay.len() + index.uuid_overlay.len() >= index.fold_limit() {
                for (id, owners) in index.uuid_overlay.iter() {
                    match owners {
                        Some(owners) => index.uuids.insert(*id, owners.clone()),
                        None => index.uuids.remove(id),
                    }
                }
                index.uuid_overlay = inline_map::InlineMap::new();
                let base = Arc::make_mut(&mut index.base);
                for (id, owners) in index.overlay.iter() {
                    match owners {
                        Some(owners) => base.insert(id.clone(), owners.clone()),
                        None => base.remove(id),
                    };
                }
                index.overlay = Arc::new(SharedMap::new());
            }
            index
        } else {
            Self::build(pages)
        }
    }

    /// Borrow ids until sorting/deduplication is complete, then allocate each
    /// distinct key once and consume the sorted rows in one AVL build.
    pub(super) fn build(pages: &Pages) -> Self {
        let mut uuids = Vec::new();
        let mut strings = Vec::new();
        for (slot, (_, doc)) in pages.slots() {
            Self::for_each_block_id(doc, |id| {
                if let Some(id) = Self::compact_uuid(id) {
                    uuids.push((id, slot));
                } else {
                    strings.push((id, slot));
                }
            });
        }
        Self::from_rows(uuids, strings)
    }

    pub(super) fn from_rows<K: AsRef<str> + Ord + Into<String>>(
        mut uuids: Vec<(u128, usize)>,
        strings: Vec<(K, usize)>,
    ) -> Self {
        pub(super) fn merge<K: Eq>(rows: Vec<(K, usize)>) -> Vec<(K, BlockOwners)> {
            let mut entries: Vec<(K, BlockOwners)> = Vec::with_capacity(rows.len());
            for (id, slot) in rows {
                if let Some((_, owners)) = entries.last_mut().filter(|(key, _)| *key == id) {
                    if owners.slots().last() != Some(&slot) {
                        match owners {
                            BlockOwners::One(first) => {
                                *owners = BlockOwners::Many(Arc::new(vec![*first, slot]))
                            }
                            BlockOwners::Many(slots) => Arc::make_mut(slots).push(slot),
                        }
                    }
                } else {
                    entries.push((id, BlockOwners::One(slot)));
                }
            }
            entries
        }
        uuids.sort_unstable();
        let mut strings: Vec<_> = strings
            .into_iter()
            .map(|(id, slot)| {
                let id: String = id.into();
                ((id.trim().to_ascii_lowercase(), id), slot)
            })
            .collect();
        strings.sort_unstable();
        Self {
            uuids: inline_map::InlineMap::from_sorted_unique(merge(uuids)),
            uuid_overlay: inline_map::InlineMap::new(),
            base: Arc::new(SharedMap::from_sorted_unique(merge(strings))),
            overlay: Arc::new(SharedMap::new()),
        }
    }

    #[cfg(test)]
    pub(super) fn first_owner(&self, id: &str) -> Option<usize> {
        self.owners(id).map(|owners| owners.slots()[0])
    }

    /// Case-insensitive candidate owners from one keyed UUID probe and one
    /// range of the existing string tree, ordered by (folded ID, exact ID).
    /// Exact spelling remains available to ordinary block resolution.
    /// Cost O(log I + matching string spellings + candidate owner slots).
    pub(super) fn owners_ignoring_case(&self, id: &str) -> Vec<usize> {
        let lower = id.trim().to_ascii_lowercase();
        let mut slots: Vec<usize> = self
            .owners(&lower)
            .map_or_else(Vec::new, |owners| owners.slots().to_vec());
        let start = (lower.clone(), String::new());
        let strings = self
            .base
            .iter_from(&start)
            .take_while(|((folded, _), _)| folded == &lower)
            .filter(|(key, _)| self.overlay.get(*key).is_none())
            .chain(
                self.overlay
                    .iter_from(&start)
                    .take_while(|((folded, _), _)| folded == &lower)
                    .filter_map(|(key, owners)| owners.as_ref().map(|owners| (key, owners))),
            );
        for ((_, exact), owners) in strings {
            #[cfg(feature = "test-faults")]
            crate::cost_counters::query_uuid_index_key(id);
            if exact != &lower {
                slots.extend_from_slice(owners.slots());
            }
        }
        slots.sort_unstable();
        slots.dedup();
        slots
    }
}
