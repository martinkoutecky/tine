//! `PageIds`, the one owner of which runtime key each block of a physical
//! page file carries (`RUNTIME-ID-MODEL.md` v3; phase A, store side). Every
//! install of a page's keyed document asks [`Graph::page_ids`] with one
//! [`Input`] and the path's prior S(P) (its row, else its claim or seed).
//!
//! **Witness W**, one counter per session: a path's S(P) takes a fresh value
//! when it is created and when an installed keyed outline differs from the
//! prior's ([`Document::same_keyed_outline`]); its entry leaves when the path
//! ends. A path a launch built and nothing changed since has W 0. Phase A
//! issues W; the window adopts it in phase B.
//!
//! **Foreign-key registry**: the page each moved key lives on (`move_blocks`
//! transfers, the same-file spelling move), so allocation on any other page
//! skips it (A3). Empty after a restart, free while empty; an entry leaves
//! when its page no longer holds the key or ends. Memory: one u64 per live
//! path created or changed this session, O(moved keys).

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tine_core::projection::{assign_runtime_ids_reserving, for_each_runtime_key, key_census};
use tine_core::BlockId;

/// What a page's keys are decided from. Each carries the bytes' parse.
pub(crate) enum Input<'a> {
    /// The serializer's keyed parse of exactly the published bytes (D4):
    /// installed whole, its holes filled collision-safe. Untrusted keys are
    /// validated first: a repeated key, or one another page holds as a
    /// runtime key ([`PageIds::held_elsewhere`]), becomes a hole (intake, A2).
    Pair(Arc<Document>, Trust<'a>),
    /// Bytes the owner did not save, parsed without keys: the prior is kept
    /// when they hold its outline, else positional keys (also with no prior:
    /// launch, cold build, discovery).
    Observed(Document),
}

/// The serializer's keyed parse of a save (D4), with the exact bytes it is
/// the parse of: a publication installs it only for those bytes.
#[derive(Clone)]
pub(crate) struct SaveHandoff {
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) doc: Arc<Document>,
}

impl SaveHandoff {
    /// The document, when `published` are its bytes. O(bytes), no parse.
    pub(crate) fn of(self, published: Option<&Arc<[u8]>>) -> Option<Arc<Document>> {
        let same = published
            .is_some_and(|bytes| Arc::ptr_eq(bytes, &self.bytes) || **bytes == *self.bytes);
        same.then_some(self.doc)
    }
}

/// Who supplied a [`Input::Pair`]'s keys.
pub(crate) enum Trust<'a> {
    /// The desktop window's submission: one node per key in its `byId`.
    Window,
    /// An entrance whose keys are unproven, with the generation it submits
    /// against: each key is checked for a repeat and for another page's
    /// runtime ownership ([`PageIds::held_elsewhere`]).
    Untrusted(&'a ReadSnapshot),
}

/// W and the foreign-key registry (module doc). Private to the model.
#[derive(Default)]
pub(crate) struct PageIds {
    counter: AtomicU64,
    witness: Mutex<HashMap<PathBuf, u64>>,
    foreign: RwLock<Registry>,
}

/// The foreign-key registry: each moved key's page, and each page's moved
/// keys, so a page's cleanup visits only its own entries. Every mutation
/// updates both.
#[derive(Default)]
struct Registry {
    owner: HashMap<BlockId, PathBuf>,
    keys: HashMap<PathBuf, HashSet<BlockId>>,
}

impl Registry {
    fn insert(&mut self, key: BlockId, to: &Path) {
        match self.owner.insert(key.clone(), to.to_path_buf()) {
            Some(from) if from != to => self.unlist(&from, &key),
            _ => {}
        }
        self.keys.entry(to.to_path_buf()).or_default().insert(key);
    }

    fn unlist(&mut self, path: &Path, key: &BlockId) {
        if let Some(keys) = self.keys.get_mut(path) {
            keys.remove(key);
            if keys.is_empty() {
                self.keys.remove(path);
            }
        }
    }

    /// `path`'s entries leave: O(its moved keys).
    fn end(&mut self, path: &Path) {
        for key in self.keys.remove(path).into_iter().flatten() {
            #[cfg(test)]
            count_registry_visit();
            self.owner.remove(&key);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Per-class decisions on this thread: (fresh, full, mixed, kept).
    pub(crate) static CLASSES: std::cell::Cell<[u64; 4]> = const { std::cell::Cell::new([0; 4]) };
    /// Validation sets allocated on this thread.
    pub(crate) static VALIDATION_SETS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Registry entries this thread's cleanups visited.
    pub(crate) static REGISTRY_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Runtime holders this thread's intake ownership probes visited.
    pub(crate) static HOLDER_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_class(at: usize) {
    CLASSES.with(|classes| {
        let mut now = classes.get();
        now[at] += 1;
        classes.set(now);
    });
}

#[cfg(test)]
fn count_registry_visit() {
    REGISTRY_VISITS.with(|visits| visits.set(visits.get() + 1));
}

impl PageIds {
    /// The witness of `path`'s installed keyed outline (module doc).
    #[cfg(test)]
    pub(crate) fn witness(&self, path: &Path) -> u64 {
        self.witness.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    /// Witness entries: one per live path created or changed this session.
    #[cfg(test)]
    pub(crate) fn witnesses(&self) -> usize {
        self.witness.lock().unwrap().len()
    }

    fn advance(&self, path: &Path) {
        let next = self.counter.fetch_add(1, Ordering::AcqRel) + 1;
        self.witness
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), next);
    }

    /// `after` was installed for `path` over `before`, its prior S(P) (None:
    /// the path's S(P) is created, and takes a fresh W).
    pub(crate) fn installed(
        &self,
        path: &Path,
        before: Option<&Arc<Document>>,
        after: &Arc<Document>,
    ) {
        let same = before
            .is_some_and(|before| Arc::ptr_eq(before, after) || before.same_keyed_outline(after));
        if !same {
            self.advance(path);
        }
        if self.foreign.read().unwrap().keys.contains_key(path) {
            self.retain_held(path, after);
        }
    }

    /// `path`'s S(P) ended (confirmed deletion): its witness and its moved
    /// keys leave; a row created there later takes a fresh W.
    pub(crate) fn ended(&self, path: &Path) {
        self.witness.lock().unwrap().remove(path);
        self.foreign.write().unwrap().end(path);
    }

    /// The same file now lives at `to` (a confirmed same-file spelling move):
    /// its keys go with it, as moved keys, and W advances there.
    pub(crate) fn respelled(&self, from: &Path, to: &Path, doc: &Document) {
        self.ended(from);
        self.advance(to);
        let mut foreign = self.foreign.write().unwrap();
        for_each_runtime_key(&doc.roots, &mut |key| foreign.insert(key.clone(), to));
    }

    /// `keys` moved to `to` with a `move_blocks` transfer or an accepted
    /// foreign submission (guard 1 of phase A: accepted and registered, as
    /// moves keep their keys today).
    pub(crate) fn moved(&self, keys: impl IntoIterator<Item = BlockId>, to: &Path) {
        let mut foreign = self.foreign.write().unwrap();
        for key in keys {
            foreign.insert(key, to);
        }
    }

    /// The one ownership view (the model's `owner_of`): whether a page other
    /// than `path` holds `key` as a runtime key. Its holders are each runtime
    /// entry of `at`'s block index for `key` (an authored `id::` is none)
    /// and the page a moved key is registered to, which may be hidden (a
    /// claim, a seed). O(entries for `key`); no page is walked.
    pub(crate) fn held_elsewhere(&self, key: &BlockId, path: &Path, at: &ReadSnapshot) -> bool {
        let index = at.block_index.get().expect("an exact block index");
        let shown = (index.owners(&key.text()).into_iter()).flat_map(BlockOwners::runtime_slots);
        let shown = shown.filter_map(|slot| at.physical_pages.get(slot));
        let registry = self.foreign.read().unwrap();
        let moved = registry.owner.get(key).map(PathBuf::as_path);
        let mut holders = shown.map(|(entry, _)| entry.path.as_path()).chain(moved);
        holders.any(|holder| {
            #[cfg(test)]
            HOLDER_VISITS.with(|visits| visits.set(visits.get() + 1));
            holder != path
        })
    }

    /// The page a moved `key` lives on, if one does.
    #[cfg(test)]
    pub(crate) fn owner_of(&self, key: &BlockId) -> Option<PathBuf> {
        self.foreign.read().unwrap().owner.get(key).cloned()
    }

    #[cfg(test)]
    pub(crate) fn registry_len(&self) -> usize {
        self.foreign.read().unwrap().owner.len()
    }

    /// `path`'s entries whose key `doc` no longer holds leave: O(its moved
    /// keys + its blocks), only for a page that has moved keys.
    fn retain_held(&self, path: &Path, doc: &Document) {
        let mut held = HashSet::new();
        for_each_runtime_key(&doc.roots, &mut |key| {
            held.insert(key.clone());
        });
        let mut foreign = self.foreign.write().unwrap();
        let Some(keys) = foreign.keys.get_mut(path) else {
            return;
        };
        let mut gone = Vec::new();
        keys.retain(|key| {
            #[cfg(test)]
            count_registry_visit();
            held.contains(key) || {
                gone.push(key.clone());
                false
            }
        });
        if keys.is_empty() {
            foreign.keys.remove(path);
        }
        for key in gone {
            foreign.owner.remove(&key);
        }
    }
}

impl Graph {
    /// The one decision of which key each block of the page file `rel`
    /// (graph-relative) carries (module doc). `prior` is the path's S(P)
    /// document: its visible row, else its claim or seed. Primes the
    /// installed document's projections. O(blocks); positional hashes only
    /// for a fresh or changed outline and for holes; registry probes only
    /// while it is non-empty.
    pub(crate) fn page_ids(
        &self,
        path: &Path,
        rel: &str,
        input: Input<'_>,
        prior: Option<&Arc<Document>>,
    ) -> Arc<Document> {
        let mut doc = match input {
            Input::Observed(doc) => {
                if let Some(prior) = prior.filter(|prior| ***prior == doc) {
                    #[cfg(test)]
                    count_class(3);
                    return Arc::clone(prior);
                }
                doc
            }
            Input::Pair(mut doc, trust) => {
                if let Trust::Untrusted(visible) = trust {
                    let roots = &mut Arc::make_mut(&mut doc).roots;
                    let elsewhere = |key: &BlockId| self.ids.held_elsewhere(key, path, visible);
                    let accepted = validate_keys(roots, &elsewhere, prior.map(|p| &**p));
                    if !accepted.is_empty() {
                        self.ids.moved(accepted, path);
                    }
                }
                if let Some(prior) = prior.filter(|prior| prior.same_keyed_outline(&doc)) {
                    #[cfg(test)]
                    count_class(3);
                    return Arc::clone(prior);
                }
                if !key_census(&doc.roots).0 {
                    // Fully keyed (an ordinary own save): nothing assigned,
                    // nothing allocated, the handoff's document installed.
                    #[cfg(test)]
                    count_class(1);
                    page_parse::prime_projections(&doc);
                    return doc;
                }
                Arc::unwrap_or_clone(doc)
            }
        };
        page_parse::shrink_blocks(&mut doc.roots);
        #[cfg(test)]
        count_class(match key_census(&doc.roots) {
            (false, _) => 1,
            (true, false) => 0,
            (true, true) => 2,
        });
        {
            let registry = self.ids.foreign.read().unwrap();
            let foreign = &registry.owner;
            let reserved = |key: &BlockId| foreign.get(key).is_some_and(|owner| owner != path);
            let reserved: Option<&dyn Fn(&BlockId) -> bool> =
                (!foreign.is_empty()).then_some(&reserved);
            assign_runtime_ids_reserving(&mut doc.roots, rel, reserved);
        }
        page_parse::prime_projections(&doc);
        Arc::new(doc)
    }

    /// The witness of `path`'s installed keyed outline (module doc).
    #[cfg(test)]
    pub(crate) fn witness(&self, path: &Path) -> u64 {
        self.ids.witness(path)
    }
}

/// Intake of unproven keys: one hash insert per block and one ownership
/// probe per key. A repeated key, or one `owned_elsewhere`, becomes a hole
/// (filled by the caller). Returns the accepted keys `base` (the page's
/// prior) does not hold: the page's runtime ownership from now on.
fn validate_keys(
    blocks: &mut [DocBlock],
    owned_elsewhere: &dyn Fn(&BlockId) -> bool,
    base: Option<&Document>,
) -> Vec<BlockId> {
    fn walk(
        blocks: &mut [DocBlock],
        (seen, base): (&mut HashSet<BlockId>, &HashSet<BlockId>),
        owned_elsewhere: &dyn Fn(&BlockId) -> bool,
        accepted: &mut Vec<BlockId>,
    ) {
        for block in blocks {
            if block.uuid.is_empty() {
                // A hole already: filled by the caller.
            } else if !seen.insert(block.uuid.clone()) || owned_elsewhere(&block.uuid) {
                block.uuid = BlockId::default();
            } else if !base.contains(&block.uuid) {
                accepted.push(block.uuid.clone());
            }
            walk(&mut block.children, (seen, base), owned_elsewhere, accepted);
        }
    }
    #[cfg(test)]
    VALIDATION_SETS.with(|sets| sets.set(sets.get() + 1));
    let mut held = HashSet::new();
    if let Some(base) = base {
        for_each_runtime_key(&base.roots, &mut |key| {
            held.insert(key.clone());
        });
    }
    let mut accepted = Vec::new();
    walk(
        blocks,
        (&mut HashSet::new(), &held),
        owned_elsewhere,
        &mut accepted,
    );
    accepted
}
