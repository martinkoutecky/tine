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

    pub fn moves(&self) -> &[(PageKey, PageKey)] {
        &self.moves
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

/// Where one durable pair's source removal stands (the design's "may Tine
/// remove this source now, and what is left owed"). Only `Order` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PairPhase {
    /// Owed: the source may be removed once its receiver is exact.
    Armed,
    /// A receiver check failed: no removal until the receiver is exact
    /// again; `removed` when the source's removal already published.
    Held { removed: bool },
    /// The exact receiver was checked after the source's durability steps.
    Settled,
    /// A durable Use Disk on the receiver disarmed the source for good.
    Cancelled,
    /// A decision this code cannot act on (unreadable, or a choice of a
    /// later slice), or a draft store that is down: the destructive step
    /// stays blocked; nothing settles.
    Blocked,
    /// The source took newer text (its own save published it): the removal
    /// is superseded, as for an ordinary page. Owes nothing.
    Disarmed,
}

impl PairPhase {
    /// The pair may still remove its source, or holds its text as custody:
    /// its operation's pages take no new operation (no chaining).
    pub fn unresolved(self) -> bool {
        matches!(self, Self::Armed | Self::Held { .. } | Self::Blocked)
    }
}

/// One durable pair: its receiver's certified image (the receiver record's
/// proposal, the exact bytes the planner certified carry the source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pair {
    pub member: u32,
    pub receiver: PageKey,
    pub source: PageKey,
    pub image: Text,
    pub phase: PairPhase,
}

/// A paired operation whose durable draft is on disk: its pairs, every
/// page it names, and the decision records that retire after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PairedOp {
    pub id: [u8; 16],
    pub pairs: Vec<Pair>,
    pub pages: BTreeSet<PageKey>,
    pub decisions: BTreeSet<String>,
    /// A decision this code cannot act on blocks the operation: only a
    /// durable Use Disk on a receiver releases a blocked pair then.
    pub unreadable: bool,
}

impl PairedOp {
    /// The pairing of a paired draft's records.
    pub fn new(id: [u8; 16], records: &[Record], pairs: &[(u32, u32)]) -> Self {
        Self {
            id,
            pairs: pairs
                .iter()
                .map(|&(receiver, source)| Pair {
                    member: receiver,
                    receiver: records[receiver as usize].page.clone(),
                    source: records[source as usize].page.clone(),
                    image: records[receiver as usize].bytes.clone(),
                    phase: PairPhase::Armed,
                })
                .collect(),
            pages: records.iter().map(|r| r.page.clone()).collect(),
            decisions: BTreeSet::new(),
            unreadable: false,
        }
    }

    /// Its draft must stay whole: something is owed, or a cancelled pair's
    /// records would replay as a lone deletion if split into page drafts.
    pub fn immutable(&self) -> bool {
        self.owes() || self.pairs.iter().any(|p| p.phase == PairPhase::Cancelled)
    }

    /// Something is still owed: a pair neither settled, cancelled nor
    /// disarmed.
    fn owes(&self) -> bool {
        self.pairs.iter().any(|p| p.phase.unresolved())
    }
}

/// What a source's save may do about its removal (`Order::removal`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Removal {
    /// Not a paired source: the ordinary save and its base guard.
    Ordinary,
    /// Remove only after checking `receiver` holds exactly `image`.
    Checked { receiver: PageKey, image: Text },
    /// No removal now (held, blocked or cancelled).
    Refused,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Order {
    replies: BTreeMap<ReplyId, Slot>,
    next: u64,
    gates: Vec<Gates>,
    /// Durable paired operations, by operation vehicle name, from admission
    /// or launch until their vehicle is durably gone.
    paired: BTreeMap<String, PairedOp>,
    /// Applied moves' receivers, by source, until the source's actual
    /// eviction. Separate from disk ordering: cleanup can outlive its witnesses.
    release_receivers: BTreeMap<PageKey, PageKey>,
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
        if let Some(gates) = &gates {
            for (dst, src) in &gates.moves {
                // A later move back into an old source replaces that
                // handoff; do not retain a cycle between reused keys.
                self.release_receivers.remove(dst);
                self.release_receivers.insert(src.clone(), dst.clone());
            }
        }
        self.gates.extend(gates);
    }

    pub fn releases_to(&self, source: &str, receiver: &str) -> bool {
        self.release_receivers
            .get(source)
            .is_some_and(|dst| dst == receiver)
    }

    pub fn released(&mut self, key: &str) {
        self.release_receivers.remove(key);
    }

    /// Whether a running order still waits for `key`'s witness.
    pub fn awaits(&self, key: &str) -> bool {
        self.gates.iter().any(|g| g.open.contains(key))
    }

    /// Whether `key`'s save waits for another page's witness.
    pub fn gated(&self, key: &str) -> bool {
        self.gates.iter().any(|g| g.gates(key))
    }

    /// D4 with no chaining: an operation over any page of a running order,
    /// witnessed or discarded ones included, waits until that order ends;
    /// over any page of a durable paired operation, while one of its pairs
    /// is unresolved. A settled, cancelled or disarmed pair releases them
    /// at once, before its draft retires.
    pub fn touches(&self, keys: &BTreeSet<PageKey>) -> bool {
        keys.iter().any(|key| {
            self.gates.iter().any(|g| g.versions.contains_key(key))
                || self.paired.values().any(|op| {
                    op.pages.contains(key) && op.pairs.iter().any(|p| p.phase.unresolved())
                })
        })
    }

    /// The unresolved pair whose source is `source`, with its operation's
    /// vehicle name (a resolved pair of an older operation is history).
    fn pairs(&self) -> impl Iterator<Item = (&String, &Pair)> {
        self.paired
            .iter()
            .flat_map(|(name, op)| op.pairs.iter().map(move |p| (name, p)))
    }

    fn active(&self, source: &str) -> Option<(&String, &Pair)> {
        self.pairs()
            .find(|(_, p)| p.source == source && p.phase.unresolved())
    }

    fn pair_mut(&mut self, source: &str) -> Option<&mut Pair> {
        self.paired
            .values_mut()
            .flat_map(|op| op.pairs.iter_mut())
            .find(|p| p.source == source && p.phase.unresolved())
    }

    /// Adopt a durable paired operation (admission's terminal, or launch).
    pub fn adopt(&mut self, name: String, op: PairedOp) {
        self.paired.insert(name, op);
    }

    /// Its operation vehicle is durably gone: nothing more is owed.
    pub fn forget(&mut self, name: &str) {
        self.paired.remove(name);
    }

    pub fn paired(&self, name: &str) -> Option<&PairedOp> {
        self.paired.get(name)
    }

    /// The operation still owes a source settlement: its draft retires
    /// only once every pair is settled or cancelled.
    pub fn holds(&self, name: &str) -> bool {
        self.paired.get(name).is_some_and(PairedOp::owes)
    }

    /// The pair a Use Disk on `receiver` cancels, as (vehicle name, member,
    /// id, source): an armed pair whose receiver is still unwitnessed
    /// (`unwitnessed`), a pair held before its source's removal (the
    /// receiver diverged from its certified image), or a blocked pair. Its
    /// decision must be durable first. None for a pair whose source already
    /// moved: its text stays in the draft until the receiver is exact again.
    pub fn cancellable(
        &self,
        receiver: &str,
        unwitnessed: bool,
    ) -> Option<(String, u32, [u8; 16], PageKey)> {
        self.paired.iter().find_map(|(name, op)| {
            op.pairs
                .iter()
                .find(|p| {
                    p.receiver == receiver
                        && match p.phase {
                            PairPhase::Armed => unwitnessed,
                            PairPhase::Held { removed } => !removed,
                            PairPhase::Blocked => true,
                            _ => false,
                        }
                })
                .map(|p| (name.clone(), p.member, op.id, p.source.clone()))
        })
    }

    /// A decision on `member` of `name` is (or may be) durable: the pair
    /// is cancelled, permanently. Sibling pairs keep their state.
    pub fn cancel(&mut self, name: &str, member: u32, decision: Option<String>) {
        if let Some(op) = self.paired.get_mut(name) {
            if let Some(pair) = op.pairs.iter_mut().find(|p| p.member == member) {
                pair.phase = PairPhase::Cancelled;
            }
            op.decisions.extend(decision);
        }
    }

    /// A decision this code does not act on: block the operation's
    /// destructive step.
    pub fn block(&mut self, name: &str, decision: String) {
        debug_assert!(self.paired.contains_key(name));
        if let Some(op) = self.paired.get_mut(name) {
            for pair in &mut op.pairs {
                if pair.phase.unresolved() {
                    pair.phase = PairPhase::Blocked;
                }
            }
            op.decisions.insert(decision);
            op.unreadable = true;
        }
    }

    /// Block every adopted operation (a down draft store).
    pub fn block_all(&mut self) {
        for op in self.paired.values_mut() {
            for pair in &mut op.pairs {
                if pair.phase.unresolved() {
                    pair.phase = PairPhase::Blocked;
                }
            }
        }
    }

    /// The draft store is up again (a successful Retry): pairs a down store
    /// blocked are armed again; an unreadable decision still blocks its
    /// operation. Returns the operations that changed.
    pub fn unblock_store(&mut self) -> Vec<String> {
        let mut changed = vec![];
        for (name, op) in self.paired.iter_mut().filter(|(_, op)| !op.unreadable) {
            for pair in op
                .pairs
                .iter_mut()
                .filter(|p| p.phase == PairPhase::Blocked)
            {
                pair.phase = PairPhase::Armed;
                changed.push(name.clone());
            }
        }
        changed.dedup();
        changed
    }

    /// Launch: the order of each adopted operation over its held pages,
    /// with its uncancelled moves (none when every pair was cancelled).
    pub fn recovered_gates(&self, pages: &BTreeMap<PageKey, Page>) -> Vec<Gates> {
        self.paired
            .values()
            .filter_map(|op| {
                let moves: Vec<_> = (op.pairs.iter())
                    .filter(|p| p.phase.unresolved())
                    .filter(|p| pages.contains_key(&p.receiver) || pages.contains_key(&p.source))
                    .map(|p| (p.receiver.clone(), p.source.clone()))
                    .collect();
                let versions: BTreeMap<PageKey, u64> = (op.pages.iter())
                    .filter_map(|key| Some((key.clone(), pages.get(key)?.version)))
                    .collect();
                (!moves.is_empty() && !versions.is_empty()).then(|| Gates::new(moves, versions))
            })
            .collect()
    }

    /// What `source`'s removal may do now.
    pub fn removal(&self, source: &str) -> Removal {
        match self.active(source) {
            // None unresolved: settled (the source is gone and its receiver
            // was exact), disarmed, or cancelled (its page took the disk).
            None => Removal::Ordinary,
            Some((_, p)) if p.phase == PairPhase::Armed => Removal::Checked {
                receiver: p.receiver.clone(),
                image: p.image.clone(),
            },
            Some(_) => Removal::Refused,
        }
    }

    /// A receiver check failed around `source`'s removal: hold the pair.
    /// Its operation (vehicle name) keeps its draft. Returns that name.
    pub fn hold(&mut self, source: &str, removed: bool) -> Option<String> {
        let name = self.active(source)?.0.clone();
        let pair = self.pair_mut(source)?;
        if matches!(pair.phase, PairPhase::Armed | PairPhase::Held { .. }) {
            pair.phase = PairPhase::Held { removed };
        }
        Some(name)
    }

    /// The exact receiver was checked after `source`'s durability steps.
    pub fn settle(&mut self, source: &str) -> Option<String> {
        self.resolve(source, PairPhase::Settled)
    }

    /// `source` published newer text before its removal: the removal is
    /// superseded (the newer text's own draft was durable before that save).
    /// A source already moved keeps its custody: its original text stays in
    /// the draft until the receiver is exact again.
    pub fn disarm(&mut self, source: &str) -> Option<String> {
        let pending = |p: &Pair| {
            matches!(
                p.phase,
                PairPhase::Armed | PairPhase::Held { removed: false }
            )
        };
        self.active(source).filter(|(_, p)| pending(p))?;
        self.resolve(source, PairPhase::Disarmed)
    }

    fn resolve(&mut self, source: &str, phase: PairPhase) -> Option<String> {
        let name = self.active(source)?.0.clone();
        let pair = self.pair_mut(source)?;
        if matches!(pair.phase, PairPhase::Armed | PairPhase::Held { .. }) {
            pair.phase = phase;
            return Some(name);
        }
        None
    }

    /// `receiver`'s disk bytes were read as `bytes`: a held pair whose
    /// receiver is exact again re-arms (or settles, its source already
    /// removed). Returns the operations whose state changed.
    pub fn receiver_read(&mut self, receiver: &str, bytes: &Text) -> Vec<String> {
        let mut changed = vec![];
        for (name, op) in &mut self.paired {
            for pair in &mut op.pairs {
                if pair.receiver == receiver && pair.image == *bytes {
                    match pair.phase {
                        PairPhase::Held { removed: true } => pair.phase = PairPhase::Settled,
                        PairPhase::Held { removed: false } => pair.phase = PairPhase::Armed,
                        _ => continue,
                    }
                    changed.push(name.clone());
                }
            }
        }
        changed
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

    /// The source a Discard of `key` also reverts: a running move's, when
    /// `key` is its unwitnessed destination, or the source of the durable
    /// pair that Use Disk on `key` cancels.
    pub fn reverts(&self, key: &str) -> Option<PageKey> {
        let unwitnessed = self.partner(key).is_some();
        (self.partner(key).cloned()).or_else(|| Some(self.cancellable(key, unwitnessed)?.3))
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

    /// A durable Use Disk cancelled `receiver`'s pair: its move leaves its
    /// running order with its source (the caller reverts it), even when the
    /// receiver was already witnessed; an order left with no move ends.
    pub fn cancel_move(&mut self, receiver: &str) {
        for g in &mut self.gates {
            if let Some(at) = g.moves.iter().position(|(dst, _)| dst == receiver) {
                let (_, src) = g.moves.remove(at);
                g.open.remove(&src);
            }
        }
        self.gates
            .retain(|g| !g.open.is_empty() && !g.moves.is_empty());
    }

    /// A stop ends every order; slots stay with their callers, who see the
    /// stopped or relaunched host.
    pub fn stop(&mut self) {
        self.gates.clear();
        // Launch reinstalls every durable pairing from the draft store.
        self.paired.clear();
        self.release_receivers.clear();
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
        // Use Disk on a durable pair's receiver cancels that pair while its
        // source is still on disk: an unwitnessed receiver, a receiver that
        // diverged from its certified image (a held pair), or a blocked
        // pair. The decision is durable before the choice takes effect or is
        // acknowledged, so no relaunch replays the removal it cancelled.
        let durable = self
            .order
            .cancellable(key, self.order.partner(key).is_some());
        // The source the choice reverts while its page holds the removal.
        let src = (self.order.reverts(key))
            .filter(|src| self.pages.get(src).is_some_and(|p| p.buf.is_none()));
        let src_read = src.as_ref().map(|src| self.fs.read_page(src));
        let reads = (self.fs.read_page(key), src_read);
        let (Ok(bytes), None | Some(Ok(_))) = reads.clone() else {
            return self.refused(request, key, Refusal::ReadFailed);
        };
        if let Some((name, member, id, _)) = &durable {
            let written = self.write_decision(key, name, *member, *id);
            self.repair_op(name);
            if !written {
                return self.refused(request, key, Refusal::DraftFailed);
            }
        }
        self.take_disk(key, page, bytes);
        // The discarded page first: a waiting caller hears the page the user
        // discarded, not the source it reverts.
        self.order.discarded(key);
        if durable.is_some() {
            self.order.cancel_move(key);
        }
        if let (Some(src), Some(Ok(bytes))) = (src, reads.1) {
            self.order.supersede(&src);
            self.take_disk(&src, self.pages[&src].clone(), bytes);
        }
        self.answer(request, key, false);
    }

    /// Write a Use Disk decision on `member` (receiver `key`) of paired
    /// operation `name` through the guarded vehicle path (fresh no-replace
    /// name, temp, fsync, rename, private-directory sync). True when it is
    /// durable. Any write that may have reached the disk disarms the pair
    /// in `Order`: a removal is never the result of an uncertain choice.
    fn write_decision(&mut self, key: &str, name: &str, member: u32, id: [u8; 16]) -> bool {
        self.wseq = self.wseq.checked_add(1).expect("draft sequence exhausted");
        let decision = drafts::Decision {
            op: id,
            member,
            wseq: self.wseq,
            choice: drafts::Choice::TakeDisk,
        };
        let file = drafts::decision_vehicle(key, &id, member);
        let mut vehicle = Vehicle::write_decision(file.clone(), &decision);
        // Bounded: temp, rename, up to three syncs, cleanup unlink and sync.
        for _ in 0..8 {
            if matches!(vehicle.stage, Stage::Present | Stage::Absent) {
                break;
            }
            vehicle.advance(&mut self.fs);
        }
        self.sync_drafts();
        let durable = vehicle.stage == Stage::Present;
        // Absent is durably absent; any other stage may leave it on disk.
        if vehicle.stage != Stage::Absent {
            self.order.cancel(name, member, Some(file));
        }
        if !durable {
            self.events.push(Event::DraftError {
                effect: name.into(),
                pages: BTreeSet::from([key.into()]),
                refresh: None,
                failures: vehicle.failures,
            });
        }
        durable
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
