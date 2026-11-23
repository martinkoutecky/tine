//! `PageIds`, the one owner of which runtime key each block of a physical
//! page file carries (`RUNTIME-ID-MODEL.md` v3; phase A, store side). Every
//! install of a page's keyed document asks [`Graph::page_ids`] with one
//! [`Input`] and the path's prior S(P) (its row, else its claim or seed).
//!
//! **Witness W**, one counter per session: it advances when a path is
//! created again after it ended, and when an installed keyed outline differs
//! from the prior's ([`Document::same_keyed_outline`]); a path unchanged
//! this session has W 0. Phase A issues W; the window adopts it in phase B.
//!
//! **Foreign-key registry**: the page each moved key lives on (`move_blocks`
//! transfers, the same-file spelling move), so allocation on any other page
//! skips it (A3). Empty after a restart, free while empty; an entry leaves
//! when its page no longer holds the key. Memory: one u64 per changed path,
//! O(moved keys).

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tine_core::projection::{assign_runtime_ids_reserving, for_each_runtime_key, key_census};
use tine_core::BlockId;

/// What a page's keys are decided from. Each carries the bytes' parse.
pub(crate) enum Input<'a> {
    /// The serializer's keyed parse of exactly the published bytes (D4):
    /// installed whole, its holes filled collision-safe. Untrusted keys are
    /// validated first: a repeated key, or one another page owns, becomes a
    /// hole (intake, A2).
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
    /// An entrance whose keys are unproven: each key is checked for a repeat
    /// and against `owned_elsewhere` (another page's runtime ownership).
    Untrusted(&'a dyn Fn(&BlockId) -> bool),
}

/// W and the foreign-key registry (module doc). Private to the model.
#[derive(Default)]
pub(crate) struct PageIds {
    counter: AtomicU64,
    witness: Mutex<HashMap<PathBuf, u64>>,
    foreign: RwLock<HashMap<BlockId, PathBuf>>,
}

#[cfg(test)]
thread_local! {
    /// Per-class decisions on this thread: (fresh, full, mixed, kept).
    pub(crate) static CLASSES: std::cell::Cell<[u64; 4]> = const { std::cell::Cell::new([0; 4]) };
    /// Validation sets allocated on this thread.
    pub(crate) static VALIDATION_SETS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_class(at: usize) {
    CLASSES.with(|classes| {
        let mut now = classes.get();
        now[at] += 1;
        classes.set(now);
    });
}

impl PageIds {
    /// The witness of `path`'s installed keyed outline (module doc).
    #[cfg(test)]
    pub(crate) fn witness(&self, path: &Path) -> u64 {
        self.witness.lock().unwrap().get(path).copied().unwrap_or(0)
    }

    fn advance(&self, path: &Path) {
        let next = self.counter.fetch_add(1, Ordering::AcqRel) + 1;
        self.witness
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), next);
    }

    /// `after` was installed for `path` over `before` (its prior S(P)).
    pub(crate) fn installed(
        &self,
        path: &Path,
        before: Option<&Arc<Document>>,
        after: &Arc<Document>,
    ) {
        let same = before
            .is_some_and(|before| Arc::ptr_eq(before, after) || before.same_keyed_outline(after));
        if !same && (before.is_some() || self.witness.lock().unwrap().contains_key(path)) {
            self.advance(path);
        }
        if !self.foreign.read().unwrap().is_empty() {
            self.retain_held(path, after);
        }
    }

    /// `path`'s S(P) ended (confirmed deletion): an old witness never
    /// validates a row created there later, and its moved keys are free.
    pub(crate) fn ended(&self, path: &Path) {
        self.advance(path);
        let mut foreign = self.foreign.write().unwrap();
        foreign.retain(|_, owner| owner != path);
    }

    /// The same file now lives at `to` (a confirmed same-file spelling move):
    /// its keys go with it, as moved keys, and W advances there.
    pub(crate) fn respelled(&self, from: &Path, to: &Path, doc: &Document) {
        self.advance(to);
        self.advance(from);
        let mut foreign = self.foreign.write().unwrap();
        foreign.retain(|_, owner| owner != from);
        for_each_runtime_key(&doc.roots, &mut |key| {
            foreign.insert(key.clone(), to.to_path_buf());
        });
    }

    /// `keys` moved to `to` with a `move_blocks` transfer (guard 1 of phase
    /// A: accepted and registered, as moves keep their keys today).
    pub(crate) fn moved(&self, keys: impl IntoIterator<Item = BlockId>, to: &Path) {
        let mut foreign = self.foreign.write().unwrap();
        for key in keys {
            foreign.insert(key, to.to_path_buf());
        }
    }

    /// The page a moved `key` lives on, if one does.
    pub(crate) fn owner_of(&self, key: &BlockId) -> Option<PathBuf> {
        self.foreign.read().unwrap().get(key).cloned()
    }

    #[cfg(test)]
    pub(crate) fn registry_len(&self) -> usize {
        self.foreign.read().unwrap().len()
    }

    /// Entries for `path` whose key `doc` no longer holds leave.
    fn retain_held(&self, path: &Path, doc: &Document) {
        let mut foreign = self.foreign.write().unwrap();
        if !foreign.values().any(|owner| owner == path) {
            return;
        }
        let mut held = HashSet::new();
        for_each_runtime_key(&doc.roots, &mut |key| {
            held.insert(key.clone());
        });
        foreign.retain(|key, owner| owner != path || held.contains(key));
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
                if let Trust::Untrusted(owned_elsewhere) = trust {
                    validate_keys(&mut Arc::make_mut(&mut doc).roots, owned_elsewhere);
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
            let foreign = self.ids.foreign.read().unwrap();
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
/// (filled by the caller).
fn validate_keys(blocks: &mut [DocBlock], owned_elsewhere: &dyn Fn(&BlockId) -> bool) {
    fn walk(
        blocks: &mut [DocBlock],
        seen: &mut HashSet<BlockId>,
        owned_elsewhere: &dyn Fn(&BlockId) -> bool,
    ) {
        for block in blocks {
            if !block.uuid.is_empty()
                && (!seen.insert(block.uuid.clone()) || owned_elsewhere(&block.uuid))
            {
                block.uuid = BlockId::default();
            }
            walk(&mut block.children, seen, owned_elsewhere);
        }
    }
    #[cfg(test)]
    VALIDATION_SETS.with(|sets| sets.set(sets.get() + 1));
    walk(blocks, &mut HashSet::new(), owned_elsewhere);
}
