//! `PageIds`, the one owner of which runtime key each block of a physical
//! page file carries (`RUNTIME-ID-MODEL.md` v3, D-20261120-I12 and
//! D-20261120-IDM; phase A, store side).
//!
//! **Writer.** Every place that installs a page's keyed document asks
//! [`Graph::page_ids`] with one input and the path's prior S(P) (its visible
//! row, else its claim or seed):
//! - [`Input::Fresh`]: no prior (launch, cold build, discovery): positional keys.
//! - [`Input::Pair`]: the serializer's keyed parse of exactly the published
//!   bytes, for this target and version (D4): installed whole, its holes filled
//!   collision-safe. A window submission is trusted (its keys come from one
//!   `byId`, so no validation is allocated); an untrusted entrance (the
//!   browser's `submit_page`) validates every key: a repeated key, or one
//!   another page owns, becomes a hole and is filled (intake, A2).
//! - [`Input::Observed`]: bytes the owner did not save: a keyed outline equal
//!   to the prior's (keys aside) keeps the prior's document, keys and
//!   witness; else positional keys, collision-safe against the registry.
//!
//! **Witness W.** One monotonic counter per graph session. W advances when a
//! path's row is created again after it ended, and whenever an installed
//! keyed outline differs from the path's prior (tree, raw text, or which
//! block holds which key: [`Document::same_keyed_outline`]); it stays when
//! the prior is kept, whatever happens to the revision. A path never
//! changed this session has the implicit W 0. Phase A issues W; the window
//! adopts it in phase B.
//!
//! **Ownership.** A structural key is a hash of (path, position): two pages
//! share one only when a block moved with its key. The foreign-key registry
//! names the page each moved key lives on (accepted `move_blocks` transfers,
//! the same-file spelling move), so positional and fill allocation on any
//! other page skips it (A3). It is empty after a restart, and costs nothing
//! while empty. Entries leave when their page no longer holds the key.
//!
//! Memory: one u64 per path changed this session, and O(moved keys).

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tine_core::projection::assign_runtime_ids_reserving;
#[cfg(test)]
use tine_core::projection::Assigned;
use tine_core::BlockId;

/// What a page's keys are decided from. Each carries the bytes' parse.
pub(crate) enum Input<'a> {
    /// No prior identity: positional keys.
    Fresh(Document),
    /// The serializer's keyed parse of exactly the published bytes (D4),
    /// shared with the host's handoff: a fully keyed one is installed as is.
    Pair(Arc<Document>, Trust<'a>),
    /// Bytes the owner did not save, parsed without keys.
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
    /// The witness of `path`'s installed keyed outline (0: unchanged since
    /// this session first installed it). Phase B's window reads it; phase A
    /// issues it.
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
        if !foreign.is_empty() {
            foreign.retain(|_, owner| owner != path);
        }
    }

    /// The same file now lives at `to` (a confirmed same-file spelling move):
    /// its keys go with it, as moved keys, and W advances there.
    pub(crate) fn respelled(&self, from: &Path, to: &Path, doc: &Document) {
        self.advance(to);
        self.advance(from);
        let mut foreign = self.foreign.write().unwrap();
        foreign.retain(|_, owner| owner != from);
        doc.for_each_uuid(&mut |key| {
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
        let foreign = self.foreign.read().unwrap();
        if foreign.is_empty() {
            return None;
        }
        foreign.get(key).cloned()
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
        doc.for_each_uuid(&mut |key| {
            held.insert(key.clone());
        });
        foreign.retain(|key, owner| owner != path || held.contains(key));
    }
}

fn shrink(blocks: &mut Vec<DocBlock>) {
    blocks.shrink_to_fit();
    for block in blocks.iter_mut() {
        shrink(&mut block.children);
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
            Input::Fresh(doc) => doc,
            Input::Pair(mut doc, trust) => {
                if let Trust::Untrusted(owned_elsewhere) = trust {
                    validate_keys(&mut Arc::make_mut(&mut doc).roots, owned_elsewhere);
                }
                if let Some(prior) = prior.filter(|prior| prior.same_keyed_outline(&doc)) {
                    #[cfg(test)]
                    count_class(3);
                    return Arc::clone(prior);
                }
                if doc.fully_keyed() {
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
        shrink(&mut doc.roots);
        let _class = {
            let foreign = self.ids.foreign.read().unwrap();
            let reserved = |key: &BlockId| foreign.get(key).is_some_and(|owner| owner != path);
            let reserved: Option<&dyn Fn(&BlockId) -> bool> =
                (!foreign.is_empty()).then_some(&reserved);
            assign_runtime_ids_reserving(&mut doc.roots, rel, reserved)
        };
        #[cfg(test)]
        count_class(match _class {
            Assigned::Fresh => 0,
            Assigned::Full => 1,
            Assigned::Mixed => 2,
        });
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
