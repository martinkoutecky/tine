//! The graph-wide referenced page-name table (D-10; cost-model audit
//! 2026-11-02 rows 1 and 2). For every page key some page references, the
//! slots of the referencing pages (ascending) and the spelling the least slot
//! uses: exactly what `referenced_page_names` folds from every page (first
//! page in slot order wins, each page's own first spelling), kept across
//! generations by per-page deltas instead of a walk per call.
//!
//! In memory only, never persisted or checkpointed: built on first use from
//! the per-page referenced-name index (O(P + R) for R page-level names, no
//! parse) and inherited by every later generation, which patches only the
//! pages a publication changed: O(changed pages x their names x log V), plus
//! for each changed key a copy of its referrer list (8 bytes per referring
//! page) and, when its least referrer left, one lookup of the new least
//! referrer's spelling. Resident cost: one tree node per distinct name
//! (measured in the og-perf-names receipt).
use super::persistent::{Map as SharedMap, Pages};
use super::{
    collect_document_referenced_names_with_baseline, Document, PageEntry, ReadSnapshot,
    SnapshotPageDerivedIndex,
};
use std::collections::{BTreeMap, BTreeSet};

/// The referrers of one page key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyReferrers {
    /// Spelling the least referring slot uses.
    pub(crate) spelling: String,
    /// Referring page slots, ascending, never empty.
    slots: Vec<usize>,
}

impl KeyReferrers {
    pub(crate) fn least_slot(&self) -> usize {
        self.slots[0]
    }
    pub(crate) fn slots(&self) -> &[usize] {
        &self.slots
    }
}

#[derive(Clone, Default)]
pub(crate) struct ReferencedNameTable {
    keys: SharedMap<String, KeyReferrers>,
}

/// The spelling page `slot` uses for `key`, read from the per-page index.
pub(super) fn slot_spelling(
    pages: &Pages,
    index: &SnapshotPageDerivedIndex,
    slot: usize,
    key: &str,
) -> Option<String> {
    let (entry, _) = pages.get(slot)?;
    index
        .get(&entry.path)?
        .iter()
        .find(|name| tine_core::refs::page_key(name) == key)
        .cloned()
}

impl ReferencedNameTable {
    /// The table of every page in `pages`. O(P + R log R).
    pub(super) fn build(pages: &Pages, index: &SnapshotPageDerivedIndex) -> Self {
        let mut keys: BTreeMap<String, KeyReferrers> = BTreeMap::new();
        for (slot, (entry, _)) in pages.slots() {
            #[cfg(test)]
            probe(1);
            for name in index.get(&entry.path).into_iter().flatten() {
                keys.entry(tine_core::refs::page_key(name))
                    .and_modify(|referrers| referrers.slots.push(slot))
                    .or_insert_with(|| KeyReferrers {
                        spelling: name.clone(),
                        slots: vec![slot],
                    });
            }
        }
        Self {
            keys: SharedMap::from_sorted_unique(keys.into_iter().collect()),
        }
    }

    /// `old` patched for the pages a publication changed. Each change is
    /// `(slot before, names before, slot after, names after)` from the
    /// per-page index of the respective generation; `pages`/`index` are the
    /// new generation's.
    pub(super) fn patched<'a>(
        old: &Self,
        changes: impl IntoIterator<
            Item = (Option<(usize, &'a [String])>, Option<(usize, &'a [String])>),
        >,
        pages: &Pages,
        index: &SnapshotPageDerivedIndex,
    ) -> Self {
        let mut next = old.clone();
        let changes: Vec<_> = changes.into_iter().collect();
        let mut dirty = BTreeSet::new();
        // Removals first, then additions, then each touched key's spelling
        // from its final least referrer: a page that both leaves and joins a
        // key within one publication never reads a half-patched list.
        for (before, _) in &changes {
            let Some((slot, names)) = before else {
                continue;
            };
            #[cfg(test)]
            probe(1);
            for name in names.iter() {
                let key = tine_core::refs::page_key(name);
                let Some(referrers) = next.keys.get(&key) else {
                    continue;
                };
                let mut referrers = referrers.clone();
                if let Ok(at) = referrers.slots.binary_search(slot) {
                    referrers.slots.remove(at);
                }
                if referrers.slots.is_empty() {
                    next.keys.remove(&key);
                    dirty.remove(&key);
                } else {
                    next.keys.insert(key.clone(), referrers);
                    dirty.insert(key);
                }
            }
        }
        for (_, after) in &changes {
            let Some((slot, names)) = after else { continue };
            #[cfg(test)]
            probe(1);
            for name in names.iter() {
                let key = tine_core::refs::page_key(name);
                let mut referrers = next.keys.get(&key).cloned().unwrap_or(KeyReferrers {
                    spelling: name.clone(),
                    slots: Vec::new(),
                });
                if let Err(at) = referrers.slots.binary_search(slot) {
                    referrers.slots.insert(at, *slot);
                }
                next.keys.insert(key.clone(), referrers);
                dirty.insert(key);
            }
        }
        for key in dirty {
            let Some(referrers) = next.keys.get(&key) else {
                continue;
            };
            let least = referrers.least_slot();
            if let Some(spelling) = slot_spelling(pages, index, least, &key) {
                if spelling != referrers.spelling {
                    let mut referrers = referrers.clone();
                    referrers.spelling = spelling;
                    next.keys.insert(key, referrers);
                }
            }
        }
        next
    }

    pub(crate) fn get(&self, key: &str) -> Option<&KeyReferrers> {
        #[cfg(test)]
        NAME_LOOKUPS.with(|count| count.set(count.get() + 1));
        self.keys.get(key)
    }

    /// Every key with its referrers, in key order. O(V).
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &KeyReferrers)> {
        self.keys.iter().map(|(key, referrers)| (key, referrers))
    }

    /// The keys at least `from` with their referrers, in key order. O(log V)
    /// to start.
    pub(crate) fn iter_from(&self, from: &str) -> impl Iterator<Item = (&String, &KeyReferrers)> {
        self.keys.iter_from(from)
    }
}

#[cfg(test)]
thread_local! {
    static NAME_LOOKUPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn name_lookups() -> usize {
    NAME_LOOKUPS.with(std::cell::Cell::get)
}

/// Authored spellings for explicit keys, retaining a folded key when no
/// authored row exists. Join the resident indexes in O(E log 64 + V) merge
/// work, plus existing active-referrer checks. The spelling table is only
/// needed for digit-bearing keys; no new index is built.
pub(crate) fn explicit_reference_spellings(graph: &ReadSnapshot) -> impl Iterator<Item = &str> {
    use std::cmp::Reverse;
    use std::collections::binary_heap::PeekMut;
    let mut shards: Vec<_> = graph.explicit_reference_name_shards().collect();
    let mut heads: std::collections::BinaryHeap<_> = shards
        .iter_mut()
        .enumerate()
        .filter_map(|(at, shard)| shard.next().map(|name| Reverse((name, at))))
        .collect();
    let ordered = std::iter::from_fn(move || {
        let mut head = heads.peek_mut()?;
        let Reverse((name, at)) = *head;
        if let Some(next) = shards[at].next() {
            *head = Reverse((next, at));
        } else {
            PeekMut::pop(head);
        }
        Some(name)
    });
    let mut spellings = None;
    ordered.map(move |key| {
        if !key.bytes().any(|byte| byte.is_ascii_digit()) {
            return key;
        }
        let spellings =
            spellings.get_or_insert_with(|| graph.referenced_name_table().iter().peekable());
        while spellings
            .peek()
            .is_some_and(|(name, _)| name.as_str() < key)
        {
            spellings.next();
        }
        spellings
            .peek()
            .filter(|(name, _)| name.as_str() == key)
            .map_or(key, |(_, name)| name.spelling.as_str())
    })
}

/// The table and its readers (moved beside the table to keep `model.rs`
/// within its size ratchet).
impl ReadSnapshot {
    /// Explicit keys with active referrers: O(E) traversal plus the existing
    /// active-owner checks. Ordering stays private to the resident index.
    pub(crate) fn explicit_reference_names(&self) -> impl Iterator<Item = &str> {
        self.explicit_reference_name_shards().flatten()
    }

    /// Explicit-key shards, each in name order, with only active referrers.
    /// Flattening costs O(E) traversal plus the existing active-owner checks;
    /// authored-spelling readers can merge the same fixed 64 streams.
    fn explicit_reference_name_shards(&self) -> impl Iterator<Item = impl Iterator<Item = &str>> {
        self.explicit_index.shards.iter().map(|shard| {
            shard
                .iter()
                .filter(|(_, paths)| paths.iter().any(|(path, _)| self.is_active_path(path)))
                .map(|(name, _)| name.as_str())
        })
    }

    /// The graph-wide referenced-name table, built on first use (O(P + R))
    /// and inherited by later generations through per-page patches.
    pub(crate) fn referenced_name_table(&self) -> &ReferencedNameTable {
        self.referenced_name_table
            .get_or_init(|| ReferencedNameTable::build(&self.pages, self.referenced_name_index()))
    }

    /// The table only when this generation already has it (a publication
    /// never builds it: the first inventory or completion read does).
    pub(crate) fn referenced_name_table_built(&self) -> Option<&ReferencedNameTable> {
        self.referenced_name_table.get()
    }

    /// Test oracle: this generation's table built from scratch, as
    /// (key, spelling, referrer slots) rows.
    #[cfg(test)]
    pub(crate) fn referenced_name_rows(
        table: &ReferencedNameTable,
    ) -> Vec<(String, String, Vec<usize>)> {
        table
            .iter()
            .map(|(key, referrers)| {
                (
                    key.clone(),
                    referrers.spelling.clone(),
                    referrers.slots().to_vec(),
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn referenced_name_table_fresh(&self) -> ReferencedNameTable {
        ReferencedNameTable::build(&self.pages, self.referenced_name_index())
    }

    /// Test convenience over the typed fileless-page answer.
    #[cfg(test)]
    pub(crate) fn reference_only_page_names(&self) -> Vec<String> {
        self.reference_only_pages()
            .into_iter()
            .map(|answer| answer.name)
            .collect()
    }

    pub(crate) fn referenced_page_names(&self) -> Vec<String> {
        self.referenced_names
            .get_or_init(|| {
                self.referenced_name_table()
                    .iter()
                    .map(|(_, referrers)| referrers.spelling.clone())
                    .collect()
            })
            .clone()
    }

    /// One spelling per referenced page key, as `referenced_page_names`
    /// spells it, from the resident table. With an editor baseline `(owner,
    /// id, original)`, pages named `owner` read block `id` as `original`:
    /// completion sees the editing block's reference baseline even when its
    /// live text has already autosaved. Cost O(V) borrowed names plus the
    /// owner pages' names; no other page is visited.
    pub(crate) fn referenced_names_for_completion(
        &self,
        baseline: Option<(&str, &str, &str)>,
    ) -> Vec<std::borrow::Cow<'_, str>> {
        use std::borrow::Cow;
        let table = self.referenced_name_table();
        let Some((owner, id, original)) = baseline else {
            return table
                .iter()
                .map(|(_, referrers)| Cow::Borrowed(referrers.spelling.as_str()))
                .collect();
        };
        let index = self.referenced_name_index();
        // key -> owner slot -> that owner's baseline spelling (None: the
        // owner references the key now but not in the baseline).
        let mut affected: std::collections::BTreeMap<
            String,
            std::collections::BTreeMap<usize, Option<String>>,
        > = Default::default();
        for (slot, entry, doc) in self.pages_named(owner) {
            #[cfg(test)]
            probe(1);
            for name in index.get(&entry.path).into_iter().flatten() {
                affected
                    .entry(tine_core::refs::page_key(name))
                    .or_default()
                    .entry(slot)
                    .or_insert(None);
            }
            for name in collect_document_referenced_names_with_baseline(
                doc,
                Some((id, original)),
                &self.config,
            ) {
                let key = tine_core::refs::page_key(&name);
                *affected.entry(key).or_default().entry(slot).or_default() = Some(name);
            }
        }
        let mut names = Vec::with_capacity(table.iter().size_hint().0);
        let mut pending = affected.into_iter().peekable();
        type Owners = std::collections::BTreeMap<usize, Option<String>>;
        fn adjusted<'a>(
            pages: &Pages,
            index: &SnapshotPageDerivedIndex,
            key: &str,
            referrers: Option<&'a KeyReferrers>,
            owners: Owners,
        ) -> Option<Cow<'a, str>> {
            let other = referrers.and_then(|referrers| {
                referrers
                    .slots()
                    .iter()
                    .copied()
                    .find(|slot| !owners.contains_key(slot))
            });
            let owner = owners
                .into_iter()
                .find_map(|(slot, spelling)| spelling.map(|spelling| (slot, spelling)));
            match (other, owner) {
                (Some(other), Some((slot, spelling))) if slot < other => Some(Cow::Owned(spelling)),
                (None, Some((_, spelling))) => Some(Cow::Owned(spelling)),
                (Some(other), _) => {
                    let referrers = referrers?;
                    if other == referrers.least_slot() {
                        Some(Cow::Borrowed(referrers.spelling.as_str()))
                    } else {
                        slot_spelling(pages, index, other, key).map(Cow::Owned)
                    }
                }
                (None, None) => None,
            }
        }
        let pages = &self.pages;
        let adjusted =
            |key: &str, referrers, owners| adjusted(pages, index, key, referrers, owners);
        for (key, referrers) in table.iter() {
            while pending
                .peek()
                .is_some_and(|(pending, _)| pending.as_str() < key.as_str())
            {
                let (pending, owners) = pending.next().unwrap();
                names.extend(adjusted(&pending, None, owners));
            }
            if pending.peek().is_some_and(|(pending, _)| pending == key) {
                let (_, owners) = pending.next().unwrap();
                names.extend(adjusted(key, Some(referrers), owners));
            } else {
                names.push(Cow::Borrowed(referrers.spelling.as_str()));
            }
        }
        for (pending, owners) in pending {
            names.extend(adjusted(&pending, None, owners));
        }
        names
    }

    /// Every page whose name is exactly `name`, with its slot: through the
    /// real-name table and the entry list (O(claimants log P)); a page the
    /// lookup cannot place falls back to one pass over the pages.
    fn pages_named(&self, name: &str) -> Vec<(usize, &PageEntry, &Document)> {
        let mut found = Vec::new();
        for (path, claimed) in self
            .real_page_names
            .owners(&tine_core::refs::page_key(name))
        {
            if !self.is_active_path(path) {
                continue;
            }
            if claimed != name {
                continue;
            }
            let placed = self
                .list
                .get_path(path)
                .and_then(|entry| self.pages.positions.get(entry.rel_path_str()))
                .and_then(|&slot| {
                    self.pages
                        .get(slot)
                        .filter(|(entry, _)| &entry.path == path)
                        .map(|(entry, doc)| (slot, entry, doc.as_ref()))
                });
            match placed {
                Some(page) => found.push(page),
                None => {
                    #[cfg(test)]
                    probe(self.pages.len());
                    return self
                        .pages
                        .slots()
                        .filter(|(_, (entry, _))| entry.name == name)
                        .map(|(slot, (entry, doc))| (slot, entry, doc.as_ref()))
                        .collect();
                }
            }
        }
        found
    }

    /// Test oracle: the pre-table answer (the union over every page in slot
    /// order, `owner` pages read with the baseline), kept verbatim from the
    /// removed `referenced_page_names_for_editor`.
    #[cfg(test)]
    pub(crate) fn referenced_page_names_for_editor_walk(
        &self,
        baseline: Option<(&str, &str, &str)>,
    ) -> Vec<String> {
        let index = self.referenced_name_index();
        let mut seen = std::collections::HashMap::new();
        for (entry, doc) in self.pages.iter() {
            probe(1);
            let names = match baseline {
                Some((owner, id, original)) if entry.name == owner => {
                    collect_document_referenced_names_with_baseline(
                        doc,
                        Some((id, original)),
                        &self.config,
                    )
                }
                _ => index.get(&entry.path).cloned().unwrap_or_default(),
            };
            for name in names {
                seen.entry(tine_core::refs::page_key(&name)).or_insert(name);
            }
        }
        seen.into_values().collect()
    }
}

#[cfg(test)]
thread_local! {
    static PAGE_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test probe: pages whose referenced names a name-table answer visited on
/// this thread (a build visits every page, a patch or an editor baseline only
/// the pages it names).
#[cfg(test)]
pub(crate) fn probe(count: usize) {
    PAGE_PROBES.with(|probes| probes.set(probes.get() + count));
}

#[cfg(test)]
pub(crate) fn take_page_probes() -> usize {
    PAGE_PROBES.with(|probes| probes.replace(0))
}
