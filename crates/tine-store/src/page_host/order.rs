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
use crate::PageId;

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
    /// Answered pages whose own save also reached the live index. The disk
    /// witness alone cannot vouch for a deferred or failed consumer delivery.
    pub indexed: BTreeSet<PageKey>,
    /// The first answered page discarded or resurrected before its witness.
    pub superseded: Option<PageKey>,
}

/// One running operation's order over its moves `(dst, src)`: the pages
/// still unwitnessed (`open`) at the versions the operation installed
/// (DESIGN §3.1). A receiver (`dst`) never waits; a page that is neither
/// waits while any receiver is open; a source (its deletion, or text a later
/// submit resurrected) waits while any non-source page is open. With one
/// move: a referrer waits for `dst`, and `src` for every other page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Gates {
    moves: Vec<(PageKey, PageKey)>,
    versions: BTreeMap<PageKey, u64>,
    open: BTreeSet<PageKey>,
}

impl Gates {
    /// An operation's order over its pages at their installed versions.
    /// `moves` is non-empty, and no page is in two moves (`Host::apply`).
    pub fn new(moves: Vec<(PageKey, PageKey)>, versions: BTreeMap<PageKey, u64>) -> Self {
        debug_assert!(!moves.is_empty());
        Self {
            moves,
            open: versions.keys().cloned().collect(),
            versions,
        }
    }

    fn receiver(&self, key: &str) -> bool {
        self.moves.iter().any(|(dst, _)| dst == key)
    }

    fn source(&self, key: &str) -> bool {
        self.moves.iter().any(|(_, src)| src == key)
    }

    fn gates(&self, key: &str) -> bool {
        self.open.contains(key)
            && !self.receiver(key)
            && (self.moves.iter().any(|(dst, _)| self.open.contains(dst))
                || (self.source(key) && self.open.iter().any(|k| !self.source(k))))
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Order {
    replies: BTreeMap<ReplyId, Slot>,
    next: u64,
    gates: Vec<Gates>,
    publication_pending: BTreeMap<ReplyId, PublicationWatch>,
    publication_versions: BTreeMap<PageKey, (u64, Arc<PublicationWatch>)>,
}

#[derive(Clone, Debug)]
struct PublicationWatch {
    sink: Arc<Mutex<BTreeSet<PageId>>>,
    names: BTreeMap<PageKey, Vec<PageId>>,
    bytes: BTreeMap<PageKey, Text>,
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

    /// The running orders as (moves, versions, open), in install order:
    /// the conformance projection onto the model's `gates`.
    #[cfg(test)]
    pub fn view(
        &self,
    ) -> impl Iterator<
        Item = (
            &[(PageKey, PageKey)],
            &BTreeMap<PageKey, u64>,
            &BTreeSet<PageKey>,
        ),
    > {
        self.gates
            .iter()
            .map(|g| (g.moves.as_slice(), &g.versions, &g.open))
    }

    /// Answer `id`; false when its caller already left.
    pub fn answer(&mut self, id: ReplyId, reply: OperationReply) -> bool {
        if let Some(watch) = self.publication_pending.remove(&id) {
            if let OperationReply::Applied(pages) = &reply {
                let watch = Arc::new(watch);
                for (key, version) in pages {
                    self.publication_versions
                        .insert(key.clone(), (*version, watch.clone()));
                }
            }
        }
        match self.replies.get_mut(&id) {
            Some(slot) => {
                slot.reply = Some(reply);
                true
            }
            None => false,
        }
    }

    pub fn watch_publications(
        &mut self,
        id: ReplyId,
        sink: Arc<Mutex<BTreeSet<PageId>>>,
        names: BTreeMap<PageKey, Vec<PageId>>,
        bytes: BTreeMap<PageKey, Text>,
    ) {
        self.publication_pending
            .insert(id, PublicationWatch { sink, names, bytes });
    }

    /// The guarded rename actually published this version, even if a later
    /// sync fails. Independent of reply slots: a caller may already be gone.
    pub fn renamed(&mut self, key: &str, version: u64, bytes: Option<&[u8]>) {
        if self
            .publication_versions
            .get(key)
            .is_some_and(|(v, watch)| {
                *v <= version && watch.bytes.get(key).map(|bytes| bytes.as_deref()) == Some(bytes)
            })
        {
            let (_, watch) = self.publication_versions.remove(key).unwrap();
            let mut sink = watch.sink.lock().unwrap_or_else(|p| p.into_inner());
            sink.extend(watch.names.get(key).into_iter().flatten().cloned());
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
    /// move's source, when `key` is its unwitnessed destination.
    pub fn partner(&self, key: &str) -> Option<&PageKey> {
        self.gates
            .iter()
            .filter(|g| g.open.contains(key))
            .find_map(|g| g.moves.iter().find(|(dst, _)| dst == key))
            .map(|(_, src)| src)
    }

    /// `key` published `version` (its save's directory sync completed).
    pub fn published(&mut self, key: &str, version: u64) {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::host_publication_probes(self.replies.len() + self.gates.len());
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

    /// The publication consumer successfully indexed an own save. Keep this
    /// separate from `published`: the rename's disk ordering uses that
    /// witness, while its caller waits for the live view as well.
    pub fn indexed(&mut self, key: &str, version: u64) {
        #[cfg(feature = "test-faults")]
        crate::cost_counters::host_publication_probes(self.replies.len());
        for slot in self.replies.values_mut() {
            if let Some(OperationReply::Applied(pages)) = &slot.reply {
                if pages.get(key).is_some_and(|v| version >= *v) {
                    slot.indexed.insert(key.into());
                }
            }
        }
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
    /// cancels its move: the move leaves its order, with its source (the
    /// caller reverts it), and an order left with no move ends. A referrer or
    /// a source leaves its order. With one move: an unwitnessed destination
    /// ends its whole order.
    pub fn discarded(&mut self, key: &str) {
        self.supersede(key);
        for g in &mut self.gates {
            if !g.open.contains(key) {
                continue;
            }
            if let Some(at) = g.moves.iter().position(|(dst, _)| dst == key) {
                let (_, src) = g.moves.remove(at);
                g.open.remove(&src);
            }
            g.open.remove(key);
        }
        self.gates
            .retain(|g| !g.open.is_empty() && !g.moves.is_empty());
    }

    /// A stop ends every order; slots stay with their callers, who see the
    /// stopped or relaunched host.
    pub fn stop(&mut self) {
        self.gates.clear();
        self.publication_pending.clear();
        self.publication_versions.clear();
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
        let hold = page.risk
            && (page.buf == bytes || self.draft_of(key).is_some_and(|draft| draft.bytes == bytes));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_receipt_survives_a_late_draft_answer_and_removed_caller() {
        let mut order = Order::default();
        let id = order.allocate(1);
        let sink = Arc::new(Mutex::new(BTreeSet::new()));
        order.register(id);
        order.watch_publications(
            id,
            sink.clone(),
            BTreeMap::from([("p".into(), vec![PageId::from("pages/p.md")])]),
            BTreeMap::from([("p".into(), Some(Arc::from(&b"U"[..])))]),
        );
        // Uncertain/Late caller leaves before the draft worker completes.
        assert!(order.remove(id));
        assert!(!order.answer(
            id,
            OperationReply::Applied(BTreeMap::from([("p".into(), 7)]))
        ));
        assert!(sink.lock().unwrap().is_empty());
        order.renamed("p", 7, Some(b"U"));
        assert_eq!(
            *sink.lock().unwrap(),
            BTreeSet::from([PageId::from("pages/p.md")])
        );
        assert!(order.publication_pending.is_empty());
        assert!(order.publication_versions.is_empty());
        assert_eq!(order.slots(), 0);
    }

    #[test]
    fn a_newer_publication_is_not_a_receipt_for_unpublished_pull_bytes() {
        let mut order = Order::default();
        let id = order.allocate(1);
        let sink = Arc::new(Mutex::new(BTreeSet::new()));
        order.watch_publications(
            id,
            sink.clone(),
            BTreeMap::from([("p".into(), vec![PageId::from("pages/p.md")])]),
            BTreeMap::from([("p".into(), Some(Arc::from(&b"U"[..])))]),
        );
        order.answer(
            id,
            OperationReply::Applied(BTreeMap::from([("p".into(), 7)])),
        );
        order.renamed("p", 8, Some(b"H"));
        assert!(sink.lock().unwrap().is_empty());
        // A coalesced/newer save of U still counts as U's publication.
        order.renamed("p", 9, Some(b"U"));
        assert_eq!(
            *sink.lock().unwrap(),
            BTreeSet::from([PageId::from("pages/p.md")])
        );
        assert!(order.publication_versions.is_empty());
    }

    fn order(moves: &[(&str, &str)], pages: &[&str]) -> Order {
        let versions = pages.iter().map(|p| (p.to_string(), 1)).collect();
        let moves = moves
            .iter()
            .map(|(d, s)| (d.to_string(), s.to_string()))
            .collect();
        let mut order = Order::default();
        order.install(Some(Gates::new(moves, versions)));
        order
    }

    /// With one move the rule is the pre-S2 one exactly: a referrer waits
    /// for `dst`, `src` waits for every other page. Every subset of
    /// witnessed pages of a move with two referrers.
    #[test]
    fn one_move_gates_as_before() {
        let pages = ["d", "s", "r1", "r2"];
        for witnessed in 0..16u32 {
            let open: BTreeSet<PageKey> = pages
                .iter()
                .enumerate()
                .filter(|(i, _)| witnessed & (1 << i) == 0)
                .map(|(_, p)| p.to_string())
                .collect();
            let mut o = order(&[("d", "s")], &pages);
            o.gates[0].open = open.clone();
            for key in pages {
                let before = open.contains(key)
                    && key != "d"
                    && (open.contains("d") || (key == "s" && open.len() > 1));
                assert_eq!(o.gated(key), before, "{key} with open {open:?}");
            }
        }
    }

    /// Two moves and a referrer (a namespace rename): no receiver waits, the
    /// referrer waits while any receiver is open, a source while any
    /// non-source page is open.
    #[test]
    fn several_moves_order_receivers_then_others_then_sources() {
        let pages = ["d1", "d2", "s1", "s2", "r"];
        let mut o = order(&[("d1", "s1"), ("d2", "s2")], &pages);
        assert!(!o.gated("d1") && !o.gated("d2"));
        assert!(o.gated("r") && o.gated("s1") && o.gated("s2"));
        o.published("d1", 1);
        assert!(o.gated("r"), "d2 is still open");
        o.published("d2", 1);
        assert!(!o.gated("r") && o.gated("s1") && o.gated("s2"));
        o.published("r", 1);
        assert!(!o.gated("s1") && !o.gated("s2"), "only sources are left");
        o.published("s1", 1);
        o.published("s2", 1);
        assert!(o.gates.is_empty());
    }

    /// A Discard of one unwitnessed receiver cancels its move only: its
    /// source is its partner and leaves the order; the other move keeps its
    /// order. With no move left, the order ends.
    #[test]
    fn a_discarded_receiver_cancels_its_own_move() {
        let pages = ["d1", "d2", "s1", "s2", "r"];
        let mut o = order(&[("d1", "s1"), ("d2", "s2")], &pages);
        assert_eq!(o.partner("d1").map(String::as_str), Some("s1"));
        assert_eq!(o.partner("d2").map(String::as_str), Some("s2"));
        assert_eq!(o.partner("r"), None);
        o.discarded("d1");
        assert_eq!(o.partner("d1"), None);
        assert!(!o.gated("s1"), "the reverted source left the order");
        assert!(o.gated("r") && o.gated("s2"), "d2 is still open");
        assert!(o.touches(&BTreeSet::from(["s1".to_string()])));
        o.discarded("d2");
        assert!(o.gates.is_empty(), "no move left: the order ends");
    }
}
