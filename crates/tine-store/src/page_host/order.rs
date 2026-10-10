//! A host operation's caller reply and a running rename's write order
//! (STEP3-DESIGN "Martin decision (2026-10-10 evening): rename ordering =
//! option 2"). Both live in memory only: a stop or a launch ends them.
//!
//! Order: while Tine runs, a host rename publishes its destination, then
//! its rewritten referrers, then its source's deletion. A page's witness is
//! its own save's `Published` at the operation's version or later (the save
//! completed its directory sync), never a rename step, an observation or a
//! version advance. Across a crash only no-loss and completion at relaunch
//! hold (SPEC-s3 accepted effects).
//!
//! Replies (Finding B): every operation a caller (not the window) admits
//! answers exactly once into its caller's slot; the caller owns the slot and
//! removes it on every exit, so a late answer creates nothing.
use super::*;

/// A caller's reply slot, scoped to the host incarnation that admitted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ReplyId {
    pub incarnation: u64,
    n: u64,
}

/// Who an operation answers: the window's request, or a caller's slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Window(Request),
    Caller(ReplyId),
}

/// The operation's terminal: applied at these page versions, or its draft
/// is durably absent, so nothing changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OperationReply {
    Applied(BTreeMap<PageKey, u64>),
    DraftFailed,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Slot {
    pub reply: Option<OperationReply>,
    /// Answered pages with their witness.
    pub witnessed: BTreeSet<PageKey>,
    /// The first answered page discarded or resurrected before its witness.
    pub superseded: Option<PageKey>,
}

/// One running rename's order: the pages still unwitnessed (`open`) at the
/// versions the operation installed. A referrer waits for `dst`; `src` (its
/// deletion, or text a later submit resurrected) waits for every other page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Gates {
    dst: PageKey,
    src: PageKey,
    versions: BTreeMap<PageKey, u64>,
    open: BTreeSet<PageKey>,
}

impl Gates {
    /// A rename's order over its pages at their installed versions.
    pub fn new(dst: &str, src: &str, versions: BTreeMap<PageKey, u64>) -> Self {
        Self {
            dst: dst.into(),
            src: src.into(),
            open: versions.keys().cloned().collect(),
            versions,
        }
    }

    fn gates(&self, key: &str) -> bool {
        self.open.contains(key)
            && key != self.dst
            && (self.open.contains(&self.dst) || (key == self.src && self.open.len() > 1))
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Order {
    replies: BTreeMap<ReplyId, Slot>,
    next: u64,
    gates: Vec<Gates>,
}

impl Order {
    pub fn allocate(&mut self, incarnation: u64) -> ReplyId {
        self.next = self.next.checked_add(1).expect("reply ids exhausted");
        ReplyId {
            incarnation,
            n: self.next,
        }
    }

    pub fn register(&mut self, id: ReplyId) {
        self.replies.insert(id, Slot::default());
    }

    /// The caller's exit: false when the slot was already gone.
    pub fn remove(&mut self, id: ReplyId) -> bool {
        self.replies.remove(&id).is_some()
    }

    pub fn slot(&self, id: ReplyId) -> Option<&Slot> {
        self.replies.get(&id)
    }

    #[cfg(test)]
    pub fn slots(&self) -> usize {
        self.replies.len()
    }

    /// The running orders as (dst, src, versions, open), in install order:
    /// the conformance projection onto the model's `gates`.
    #[cfg(test)]
    pub fn view(
        &self,
    ) -> impl Iterator<
        Item = (
            &PageKey,
            &PageKey,
            &BTreeMap<PageKey, u64>,
            &BTreeSet<PageKey>,
        ),
    > {
        self.gates
            .iter()
            .map(|g| (&g.dst, &g.src, &g.versions, &g.open))
    }

    /// Answer `id`; false when its caller already left.
    pub fn answer(&mut self, id: ReplyId, reply: OperationReply) -> bool {
        match self.replies.get_mut(&id) {
            Some(slot) => {
                slot.reply = Some(reply);
                true
            }
            None => false,
        }
    }

    /// Install a rename's order at its terminal. No chaining: its pages
    /// touch no other running order (`touches`).
    pub fn install(&mut self, gates: Option<Gates>) {
        self.gates.extend(gates);
    }

    /// Whether `key`'s save waits for another page's witness.
    pub fn gated(&self, key: &str) -> bool {
        self.gates.iter().any(|g| g.gates(key))
    }

    /// D4 with no chaining: an operation over any page of a running order,
    /// witnessed or discarded ones included, waits until that order ends.
    pub fn touches(&self, keys: &BTreeSet<PageKey>) -> bool {
        keys.iter()
            .any(|key| self.gates.iter().any(|g| g.versions.contains_key(key)))
    }

    /// The page whose disk bytes a Discard of `key` also reads: a running
    /// rename's source, when `key` is its unwitnessed destination.
    pub fn partner(&self, key: &str) -> Option<&PageKey> {
        self.gates
            .iter()
            .find(|g| g.dst == key && g.open.contains(key))
            .map(|g| &g.src)
    }

    /// `key` published `version` (its save's directory sync completed).
    pub fn published(&mut self, key: &str, version: u64) {
        for slot in self.replies.values_mut() {
            if let Some(OperationReply::Applied(pages)) = &slot.reply {
                if pages.get(key).is_some_and(|v| version >= *v) {
                    slot.witnessed.insert(key.into());
                }
            }
        }
        for g in &mut self.gates {
            if g.versions.get(key).is_some_and(|v| version >= *v) {
                g.open.remove(key);
            }
        }
        self.gates.retain(|g| !g.open.is_empty());
    }

    /// An answered, unwitnessed `key` changed under its caller: a Discard,
    /// or a submit resurrecting a deletion.
    pub fn supersede(&mut self, key: &str) {
        for slot in self.replies.values_mut() {
            if let Some(OperationReply::Applied(pages)) = &slot.reply {
                if pages.contains_key(key) && !slot.witnessed.contains(key) {
                    slot.superseded.get_or_insert_with(|| key.into());
                }
            }
        }
    }

    /// A Discard of `key` took the disk's bytes. An unwitnessed destination
    /// ends its whole order (the caller reverts the source); a referrer or the
    /// source leaves its order.
    pub fn discarded(&mut self, key: &str) {
        self.supersede(key);
        self.gates
            .retain(|g| !(g.dst == key && g.open.contains(key)));
        for g in &mut self.gates {
            if g.dst != key {
                g.open.remove(key);
            }
        }
        self.gates.retain(|g| !g.open.is_empty());
    }

    /// A stop ends every order; slots stay with their callers, who see the
    /// stopped or relaunched host.
    pub fn stop(&mut self) {
        self.gates.clear();
    }
}

impl<F: HostIo> Host<F> {
    /// Discard (SPEC read table): the page takes its disk bytes, held at risk
    /// only while those are the bytes it or its draft already had. A running
    /// rename's unwitnessed destination also reverts its source when that is
    /// still the operation's deletion: the source keeps its disk bytes, so a
    /// cancelled rename never trashes it; newer input typed into the source
    /// stays. Refused when a read fails (SPEC-s2 §4.11 "discard failed"), the
    /// source's included. In-scope scenario: a disk error.
    pub(super) fn discard(&mut self, request: &Request) {
        let key = &request.page;
        let Some(page) = self.pages.get(key).cloned() else {
            return self.refused(request, key, Refusal::NotHeld);
        };
        let src = self
            .order
            .partner(key)
            .filter(|src| self.pages.get(*src).is_some_and(|p| p.buf.is_none()))
            .cloned();
        let src_read = src.as_ref().map(|src| self.fs.read_page(src));
        let reads = (self.fs.read_page(key), src_read);
        let (Ok(bytes), None | Some(Ok(_))) = reads.clone() else {
            return self.refused(request, key, Refusal::ReadFailed);
        };
        self.take_disk(key, page, bytes);
        // The discarded page first: a waiting caller hears the page the user
        // discarded, not the source it reverts.
        self.order.discarded(key);
        if let (Some(src), Some(Ok(bytes))) = (src, reads.1) {
            self.order.supersede(&src);
            self.take_disk(&src, self.pages[&src].clone(), bytes);
        }
        self.answer(request, key, false);
    }

    fn take_disk(&mut self, key: &str, page: Page, bytes: Text) {
        let drafts = self.logical_drafts();
        let hold = page.risk
            && (page.buf == bytes || drafts.get(key).is_some_and(|draft| draft.bytes == bytes));
        self.version = self.next_version();
        let mut page = Self::initial_page(bytes, self.version);
        page.risk = hold;
        self.set_page(key, Some(page));
    }

    /// Register the caller's slot for the operation this locked step just
    /// admitted (Finding B), before anything can answer it.
    pub(super) fn register_reply(&mut self) -> Option<ReplyId> {
        let worker = self.worker.as_ref()?;
        let Some(Application::Operation {
            reply: Reply::Caller(id),
            ..
        }) = &worker.application
        else {
            return None;
        };
        let id = *id;
        self.order.register(id);
        Some(id)
    }
}
