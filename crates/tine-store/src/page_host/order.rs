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
    /// A durable Use Disk disarmed the source for good, and the source's
    /// original text is still anchored: its preimage on disk, or its
    /// certified receiver.
    Cancelled,
    /// A durable Use Disk after the source's removal published, with the
    /// receiver not exact: the admitted preimage is selectable for guarded
    /// restoration (`Host::restore_source`), the operation draft keeps it.
    Restore,
    /// A durable Use Disk, or newer source text, left neither endpoint
    /// anchoring the source's original text, and no restoration is possible
    /// now (its slot is occupied): the operation draft keeps the preimage as
    /// custody, and the original removal stays disarmed.
    Retained,
    /// A decision this code cannot act on (unreadable, or a choice of a
    /// later slice), or a draft store that is down: the destructive step
    /// stays blocked; nothing settles.
    Blocked,
    /// The source took newer text (its own save published it, after its
    /// durable superseding choice) while the original text is anchored (the
    /// newer text is the preimage, or the receiver is certified): the
    /// removal is superseded, as for an ordinary page. Owes nothing.
    Disarmed,
}

impl PairPhase {
    /// The pair may still remove its source, or holds its text as custody:
    /// its operation's pages take no new operation (no chaining).
    pub fn unresolved(self) -> bool {
        matches!(
            self,
            Self::Armed | Self::Held { .. } | Self::Blocked | Self::Restore | Self::Retained
        )
    }

    /// The pair still orders its operation's saves (launch reinstalls its
    /// gate): a choice-free pair that may still remove its source.
    fn gated(self) -> bool {
        matches!(self, Self::Armed | Self::Held { .. } | Self::Blocked)
    }
}

/// The durable Use Disk recorded on one pair (the model's `decision`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PairChoice {
    None,
    /// Use Disk on the source: its removal is cancelled.
    SourceDisk,
    /// Use Disk on the receiver: the move is cancelled.
    ReceiverDisk,
    /// Newer editor input on the source replaces its removal (the
    /// superseding decision, durable before that input publishes).
    Superseded,
}

/// Which endpoint of a pair a Use Disk chose (`Order::use_disk`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Chosen {
    /// The operation vehicle name and its request id.
    pub name: String,
    pub id: [u8; 16],
    /// The chosen endpoint's record index: the decision's member.
    pub member: u32,
    pub receiver: PageKey,
    pub source: PageKey,
    /// The receiver was chosen (else the source).
    pub on_receiver: bool,
    pub image: Text,
    pub preimage: Text,
    /// The source's removal already published (its text is in the trash).
    pub removed: bool,
}

/// One durable pair: its receiver's certified image (the receiver record's
/// proposal, the exact bytes the planner certified carry the source).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pair {
    /// The receiver's and the source's record indexes.
    pub member: u32,
    pub source_member: u32,
    pub receiver: PageKey,
    pub source: PageKey,
    pub image: Text,
    /// The source's admitted text (its record's base).
    pub preimage: Text,
    pub phase: PairPhase,
    pub choice: PairChoice,
}

/// A paired operation whose durable draft is on disk: its pairs, every
/// page it names, and the decision records that retire after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PairedOp {
    pub id: [u8; 16],
    pub pairs: Vec<Pair>,
    pub pages: BTreeSet<PageKey>,
    /// Decision vehicle names, with the decision when this code acts on it
    /// (a settlement rewrite carries a sibling's decision to its new draft).
    pub decisions: BTreeMap<String, Option<drafts::Decision>>,
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
                    source_member: source,
                    receiver: records[receiver as usize].page.clone(),
                    source: records[source as usize].page.clone(),
                    image: records[receiver as usize].bytes.clone(),
                    preimage: match &records[source as usize].base {
                        Base::Known(text) => text.clone(),
                        Base::Unknown => None,
                    },
                    phase: PairPhase::Armed,
                    choice: PairChoice::None,
                })
                .collect(),
            pages: records.iter().map(|r| r.page.clone()).collect(),
            decisions: BTreeMap::new(),
            unreadable: false,
        }
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
                || (self.paired.values()).any(|op| op.pages.contains(key) && op.owes())
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

    /// A settlement rewrite of `old` made its new draft `new` (with
    /// `records` and the remapped `pairs`) and its surviving pairs'
    /// `decisions` durable: those pairs move to `new` with their phase and
    /// choice, before `old`'s unlink starts. `old` keeps only the retired
    /// pair, which leaves with it.
    pub fn rewrite(
        &mut self,
        old: &str,
        new: String,
        id: [u8; 16],
        records: &[Record],
        pairs: &[(u32, u32)],
        decisions: BTreeMap<String, Option<drafts::Decision>>,
    ) {
        let Some(prev) = self.paired.get_mut(old) else {
            return;
        };
        let mut op = PairedOp::new(id, records, pairs);
        for pair in &mut op.pairs {
            let same = |p: &Pair| p.receiver == pair.receiver && p.source == pair.source;
            if let Some(at) = prev.pairs.iter().position(same) {
                let moved = prev.pairs.remove(at);
                pair.phase = moved.phase;
                pair.choice = moved.choice;
            }
        }
        op.decisions = decisions;
        self.paired.insert(new, op);
    }

    /// Its operation vehicle is durably gone: nothing more is owed.
    pub fn forget(&mut self, name: &str) {
        self.paired.remove(name);
    }

    pub fn paired(&self, name: &str) -> Option<&PairedOp> {
        self.paired.get(name)
    }

    /// The live pair with these endpoints in any operation (a conformance
    /// projection across a settlement rewrite).
    #[cfg(test)]
    pub fn live_pair(&self, receiver: &str, source: &str) -> Option<&Pair> {
        self.pairs()
            .map(|(_, p)| p)
            .find(|p| p.receiver == receiver && p.source == source)
    }

    /// `key` is the receiver of an unresolved pair, whose checks ask for its
    /// durability witness (`Host::receiver_certified`).
    pub fn needs_witness(&self, key: &str) -> bool {
        self.pairs()
            .any(|(_, p)| p.receiver == key && p.phase.unresolved())
    }

    /// The operation still owes a source settlement: its draft retires
    /// only once every pair is settled or cancelled.
    pub fn holds(&self, name: &str) -> bool {
        self.paired.get(name).is_some_and(PairedOp::owes)
    }

    /// The pair a Use Disk on `key` decides (the model's `decideDisk`),
    /// read as `bytes`; its decision must be durable first.
    ///
    /// On a receiver: a choice-free unresolved pair whose receiver is
    /// unwitnessed (`unwitnessed`), not at its certified image, held (its
    /// source moved or not) or blocked; or, in any phase, a pair whose
    /// source took its disk while the receiver is still unwitnessed. On a
    /// source: a choice-free unresolved pair. None otherwise. `certified`
    /// answers whether the receiver holds a pair's image
    /// (`Host::receiver_certified`, the model's `certified` in `partner`).
    pub fn use_disk(
        &self,
        key: &str,
        unwitnessed: bool,
        mut certified: impl FnMut(&PageKey, &Text) -> bool,
    ) -> Option<Chosen> {
        self.paired.iter().find_map(|(name, op)| {
            op.pairs.iter().find_map(|p| {
                let on_receiver = p.receiver == key
                    && match p.choice {
                        PairChoice::None | PairChoice::Superseded => {
                            p.phase.unresolved()
                                && (unwitnessed
                                    || p.phase != PairPhase::Armed
                                    || !certified(&p.receiver, &p.image))
                        }
                        // The move itself is still cancellable while its
                        // receiver is unwitnessed, whatever the source chose.
                        PairChoice::SourceDisk => unwitnessed,
                        PairChoice::ReceiverDisk => false,
                    };
                let on_source =
                    p.source == key && p.phase.unresolved() && p.choice == PairChoice::None;
                (on_receiver || on_source).then(|| Chosen {
                    name: name.clone(),
                    id: op.id,
                    member: if on_receiver {
                        p.member
                    } else {
                        p.source_member
                    },
                    receiver: p.receiver.clone(),
                    source: p.source.clone(),
                    on_receiver,
                    image: p.image.clone(),
                    preimage: p.preimage.clone(),
                    removed: matches!(
                        p.phase,
                        PairPhase::Held { removed: true } | PairPhase::Restore
                    ),
                })
            })
        })
    }

    /// A Use Disk decision on `member` of `name` is (or may be) durable:
    /// a receiver's cancels the move, a source's its removal, for good.
    /// `phase` is where the pair's original text stands then (the model's
    /// `cancelPhase`); None keeps the phase (launch decides it later).
    /// Sibling pairs keep their state.
    pub fn decide(
        &mut self,
        name: &str,
        member: u32,
        phase: Option<PairPhase>,
        decision: Option<(String, drafts::Decision)>,
    ) {
        if let Some(op) = self.paired.get_mut(name) {
            for pair in &mut op.pairs {
                let choice = if pair.member == member {
                    PairChoice::ReceiverDisk
                } else if pair.source_member == member && pair.choice == PairChoice::None {
                    PairChoice::SourceDisk
                } else {
                    continue;
                };
                pair.choice = choice;
                pair.phase = phase.unwrap_or(PairPhase::Cancelled);
            }
            op.decisions
                .extend(decision.map(|(name, d)| (name, Some(d))));
        }
    }

    /// The one answer to "which pair's removal does newer text on `source`
    /// supersede now": its unresolved pair, while that removal may still
    /// run (Armed, or Held before the source moved). Both questions below
    /// ask it: whether a superseding decision must be made durable before
    /// the text saves (`supersedes`), and which pair the published text
    /// disarms or retains (`disarming`, `disarm`).
    fn superseded(&self, source: &str) -> Option<(&String, &PairedOp, &Pair)> {
        let (name, pair) = self.active(source)?;
        matches!(
            pair.phase,
            PairPhase::Armed | PairPhase::Held { removed: false }
        )
        .then(|| (name, &self.paired[name], pair))
    }

    /// The superseded pair (`superseded`) whose superseding decision is not
    /// known durable yet (its choice is still None): newer text saved on
    /// `source` first writes that decision (`Host::start_save`). As
    /// (vehicle name, request id, source member).
    pub fn supersedes(&self, source: &str) -> Option<(String, [u8; 16], u32)> {
        self.superseded(source)
            .filter(|(_, _, p)| p.choice == PairChoice::None)
            .map(|(name, op, p)| (name.clone(), op.id, p.source_member))
    }

    /// A superseding decision on `member` (a source) of `name` may be on
    /// disk: its vehicle is registered (it retires with the operation, and
    /// a settlement rewrite carries it). Only a decision known durable
    /// (`durable`: its write reached Present, or launch read it) gives the
    /// pair its Superseded choice; an uncertain write leaves the choice
    /// None, so the next save writes the decision again before its text
    /// may publish. The phase is unchanged until the newer text publishes
    /// (`disarm`), or launch decides it.
    pub fn decide_superseded(
        &mut self,
        name: &str,
        member: u32,
        decision: (String, drafts::Decision),
        durable: bool,
    ) {
        if let Some(op) = self.paired.get_mut(name) {
            if durable {
                for pair in op.pairs.iter_mut().filter(|p| p.source_member == member) {
                    if pair.choice == PairChoice::None {
                        pair.choice = PairChoice::Superseded;
                    }
                }
            }
            op.decisions.insert(decision.0, Some(decision.1));
        }
    }

    /// Launch: each decided pair's phase from its endpoints' disk bytes
    /// (`reads`), judged by `judge` (the pair, the source's and the
    /// receiver's bytes): whether the source durably holds the original
    /// (`Host::source_original`) and whether the receiver is certified
    /// (`Host::receiver_certified`). A failed read keeps custody.
    pub fn decided_phases(
        &mut self,
        reads: &BTreeMap<PageKey, Option<Text>>,
        mut judge: impl FnMut(&Pair, &Text, &Text) -> (Original, bool),
    ) {
        for op in self.paired.values_mut() {
            for pair in op.pairs.iter_mut().filter(|p| p.choice != PairChoice::None) {
                let read = |key: &PageKey| reads.get(key).cloned().flatten();
                let phase = match (read(&pair.source), read(&pair.receiver)) {
                    (Some(source), Some(receiver)) => {
                        let (original, certified) = judge(pair, &source, &receiver);
                        cancel_phase(original, &source, certified, true)
                    }
                    _ => PairPhase::Retained,
                };
                pair.phase = phase;
            }
        }
    }

    /// Both endpoints of every decided pair (launch reads them once).
    pub fn decided_pages(&self) -> BTreeSet<PageKey> {
        self.pairs()
            .filter(|(_, p)| p.choice != PairChoice::None)
            .flat_map(|(_, p)| [p.receiver.clone(), p.source.clone()])
            .collect()
    }

    /// Each decided pair's source (once) whose launch read `bytes` gives
    /// `Some` (given the source and its preimage): launch witnesses those
    /// in its one directory pass (`HostIo::graph_launch`).
    pub fn decided_sources(
        &self,
        mut bytes: impl FnMut(&PageKey, &Text) -> Option<Text>,
    ) -> Vec<(PageKey, Text)> {
        let mut seen = BTreeSet::new();
        self.pairs()
            .filter(|(_, p)| p.choice != PairChoice::None)
            .filter_map(|(_, p)| Some((p.source.clone(), bytes(&p.source, &p.preimage)?)))
            .filter(|(source, _)| seen.insert(source.clone()))
            .collect()
    }

    /// A decision this code does not act on: block the operation's
    /// destructive step.
    pub fn block(&mut self, name: &str, decision: String) {
        debug_assert!(self.paired.contains_key(name));
        if let Some(op) = self.paired.get_mut(name) {
            for pair in &mut op.pairs {
                if pair.phase.gated() {
                    pair.phase = PairPhase::Blocked;
                }
            }
            op.decisions.insert(decision, None);
            op.unreadable = true;
        }
    }

    /// Block every adopted operation (a down draft store).
    pub fn block_all(&mut self) {
        for op in self.paired.values_mut() {
            for pair in &mut op.pairs {
                if pair.phase.gated() {
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
    /// The order covers the endpoints of those moves and the operation's
    /// referrers, never the endpoints of a resolved or retained pair.
    pub fn recovered_gates(&self, pages: &BTreeMap<PageKey, Page>) -> Vec<Gates> {
        self.paired
            .values()
            .filter_map(|op| {
                let gated: Vec<_> = (op.pairs.iter()).filter(|p| p.phase.gated()).collect();
                let moves: Vec<_> = (gated.iter())
                    .filter(|p| pages.contains_key(&p.receiver) || pages.contains_key(&p.source))
                    .map(|p| (p.receiver.clone(), p.source.clone()))
                    .collect();
                let endpoint = |key: &PageKey, pairs: &[&Pair]| {
                    pairs.iter().any(|p| &p.receiver == key || &p.source == key)
                };
                let all: Vec<_> = op.pairs.iter().collect();
                let versions: BTreeMap<PageKey, u64> = (op.pages.iter())
                    .filter(|key| endpoint(key, &gated) || !endpoint(key, &all))
                    .filter_map(|key| Some((key.clone(), pages.get(key)?.version)))
                    .collect();
                (!moves.is_empty() && !versions.is_empty()).then(|| Gates::new(moves, versions))
            })
            .collect()
    }

    /// The phase and choice of the (first) pair whose source is `source`.
    #[cfg(test)]
    pub fn source_state(&self, source: &str) -> Option<(PairPhase, PairChoice)> {
        (self.pairs())
            .find(|(_, p)| p.source == source)
            .map(|(_, p)| (p.phase, p.choice))
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

    /// The pair whose removal newer text on `source` supersedes now
    /// (`superseded`), as (vehicle name, receiver, image, preimage): its
    /// receiver is read to place the original text (`disarm`).
    pub fn disarming(&self, source: &str) -> Option<(String, PageKey, Text, Text)> {
        self.superseded(source).map(|(name, _, p)| {
            (
                name.clone(),
                p.receiver.clone(),
                p.image.clone(),
                p.preimage.clone(),
            )
        })
    }

    /// `source` published newer text before its removal (the model's
    /// `publishedPair`): the removal is superseded. Its superseding choice
    /// was durable before that save (`supersedes`), so no relaunch replays
    /// the removal. When the original text is anchored (`anchored`, the
    /// module's `anchored` over the published text) the pair is
    /// disarmed; else it is retained, and the operation draft keeps the
    /// admitted preimage as the only copy of that text. A source already
    /// moved keeps its custody: its original text stays in the draft until
    /// the receiver is exact again.
    pub fn disarm(&mut self, source: &str, anchored: bool) -> Option<String> {
        self.superseded(source)?;
        let phase = if anchored {
            PairPhase::Disarmed
        } else {
            PairPhase::Retained
        };
        let name = self.resolve(source, phase)?;
        // `resolve` found the pair by source; it is no longer unresolved
        // only when disarmed, so look it up by name.
        if let Some(op) = self.paired.get_mut(&name) {
            for pair in op.pairs.iter_mut().filter(|p| p.source == source) {
                if pair.phase == phase && pair.choice == PairChoice::None {
                    pair.choice = PairChoice::Superseded;
                }
            }
        }
        Some(name)
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

    /// `receiver` was read: a held pair whose receiver holds its image
    /// again (`certified`, given the image: `Host::receiver_certified`)
    /// re-arms, or settles with its source already removed (the model's
    /// `resumePair`). Returns the operations whose state changed.
    pub fn receiver_read(
        &mut self,
        receiver: &str,
        mut certified: impl FnMut(&Text) -> bool,
    ) -> Vec<String> {
        let mut changed = vec![];
        for (name, op) in &mut self.paired {
            for pair in &mut op.pairs {
                let PairPhase::Held { removed } = pair.phase else {
                    continue;
                };
                if pair.receiver == receiver && certified(&pair.image) {
                    pair.phase = if removed {
                        PairPhase::Settled
                    } else {
                        PairPhase::Armed
                    };
                    changed.push(name.clone());
                }
            }
        }
        changed
    }

    /// The source's own save published: a pair awaiting its guarded
    /// restoration settles as cancelled once the source holds the preimage
    /// again (`original`, given the preimage: `Host::source_original`, with
    /// a durable or weak witness). A failed witness leaves the pair awaiting
    /// its restoration, as other bytes do. Returns the operations that
    /// changed.
    pub fn restored(
        &mut self,
        source: &str,
        mut original: impl FnMut(&Text) -> Original,
    ) -> Vec<String> {
        let mut changed = vec![];
        for (name, op) in &mut self.paired {
            for pair in &mut op.pairs {
                if pair.source == source
                    && pair.phase == PairPhase::Restore
                    && original(&pair.preimage).holds()
                {
                    pair.phase = PairPhase::Cancelled;
                    changed.push(name.clone());
                }
            }
        }
        changed
    }

    /// The decided pair whose source `key` may be restored from its
    /// admitted preimage, as (vehicle name, preimage).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn restorable(&self, key: &str) -> Option<(String, Text)> {
        self.pairs()
            .find(|(_, p)| {
                p.source == key
                    && matches!(p.phase, PairPhase::Restore | PairPhase::Retained)
                    && matches!(p.choice, PairChoice::SourceDisk | PairChoice::ReceiverDisk)
                    && p.preimage.is_some()
            })
            .map(|(name, p)| (name.clone(), p.preimage.clone()))
    }

    /// A restoration of `key` was selected: its pair awaits the guarded
    /// create (the model's `selectPair(.., "restore")`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn select_restore(&mut self, key: &str) -> Option<String> {
        let name = self.restorable(key)?.0;
        let pair = self.pair_mut(key)?;
        pair.phase = PairPhase::Restore;
        Some(name)
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

    /// The other page a Discard of `key` locks: a running move's source,
    /// when `key` is its unwitnessed destination, or the other endpoint of
    /// the durable pair a Use Disk on `key` may decide (it reads both).
    pub fn reverts(&self, key: &str) -> Option<PageKey> {
        let unwitnessed = self.partner(key).is_some();
        (self.partner(key).cloned()).or_else(|| {
            // Unread: judged not certified, so both pages are locked.
            let chosen = self.use_disk(key, unwitnessed, |_, _| false)?;
            Some(if chosen.on_receiver {
                chosen.source
            } else {
                chosen.receiver
            })
        })
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

/// The source's bytes are its admitted text, the pair's preimage: an
/// absent source holds none. Bytes alone anchor nothing; only
/// `Host::source_original` asks this, with the source's durability witness.
pub(super) fn source_present(preimage: &Text, source: &Text) -> bool {
    source.is_some() && source == preimage
}

/// Whether a source holds its pair's original text, as the one answerer
/// `Host::source_original` judges it (the model's `sourcePresent`), from
/// the source's bytes and its durability witness (`HostIo::source_witnessed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Original {
    /// The preimage, and the witness flushed that very file and synced its
    /// directory entries up to the graph root (D).
    Durable,
    /// The preimage on a filesystem that offers no directory sync, or a
    /// Windows chain (W): the bytes rule every save there already gets.
    Weak,
    /// The preimage, but a witness step failed (U): a power cut may still
    /// drop or revert the file, so it anchors nothing.
    Failed,
    /// Other bytes, or no file.
    No,
}

impl Original {
    /// The source anchors the original text (D or W; develop's byte match).
    pub(super) fn holds(self) -> bool {
        matches!(self, Self::Durable | Self::Weak)
    }
}

/// The one answer to "is a pair's original text still anchored outside its
/// operation draft" (the model's `sourcePresent or certified`): the source
/// holds its preimage with a durable or weak witness (`original`:
/// `Host::source_original`), or the receiver is certified (`certified`:
/// `Host::receiver_certified`). Launch and Use Disk ask it through
/// `cancel_phase`; a published newer source text asks it directly
/// (`Order::disarm`'s `anchored`).
pub(super) fn anchored(original: Original, certified: bool) -> bool {
    original.holds() || certified
}

/// Where a decided pair's original text stands (the model's
/// `cancelPhase`): anchored (`anchored`) cancels the pair; else it is
/// restorable from its preimage when the source's slot (bytes `source`) is
/// free after its removal (`removed`); else custody.
pub(super) fn cancel_phase(
    original: Original,
    source: &Text,
    certified: bool,
    removed: bool,
) -> PairPhase {
    if anchored(original, certified) {
        PairPhase::Cancelled
    } else if source.is_none() && removed {
        PairPhase::Restore
    } else {
        PairPhase::Retained
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
        let Ok(bytes) = self.fs.read_page(key) else {
            return self.refused(request, key, Refusal::ReadFailed);
        };
        // Use Disk on a durable pair's endpoint decides that pair (the
        // model's `decideDisk`): on the receiver it cancels the move, on the
        // source its removal. The decision is durable before the choice
        // takes effect or is acknowledged, so no relaunch replays the
        // removal it cancelled.
        let unwitnessed = self.order.partner(key).is_some();
        let fs = &mut self.fs;
        let chosen = (self.order).use_disk(key, unwitnessed, |receiver, image| {
            Self::receiver_certified(fs, receiver, &bytes, image)
        });
        // The source the choice reverts while its page holds the removal.
        let src = (self.order.partner(key).cloned())
            .or_else(|| {
                chosen
                    .as_ref()
                    .filter(|c| c.on_receiver)
                    .map(|c| c.source.clone())
            })
            .filter(|src| self.pages.get(src).is_some_and(|p| p.buf.is_none()));
        let src_bytes = match &src {
            Some(src) => match self.fs.read_page(src) {
                Ok(bytes) => Some(bytes),
                Err(_) => return self.refused(request, key, Refusal::ReadFailed),
            },
            None => None,
        };
        if let Some(chosen) = &chosen {
            // Where the original text stands: the other endpoint's bytes.
            let source = match (&src_bytes, chosen.on_receiver) {
                (_, false) => Ok(bytes.clone()),
                (Some(source), true) => Ok(source.clone()),
                (None, true) => self.fs.read_page(&chosen.source),
            };
            let Ok(source) = source else {
                return self.refused(request, key, Refusal::ReadFailed);
            };
            let fs = &mut self.fs;
            let original = Self::source_original(&chosen.preimage, &source, || {
                fs.source_witnessed(&chosen.source, &source)
            });
            // Develop's receiver evidence: on the source, a held original
            // (D or W) needs no receiver read; otherwise the receiver is
            // read, and a certified one still cancels. Only a failed witness
            // (U) tolerates an unreadable receiver: the choice is recorded
            // as Retained, keeping the draft, instead of being refused.
            let certified = if chosen.on_receiver {
                Self::receiver_certified(&mut self.fs, &chosen.receiver, &bytes, &chosen.image)
            } else if original.holds() {
                false
            } else {
                match self.fs.read_page(&chosen.receiver) {
                    Ok(receiver) => Self::receiver_certified(
                        &mut self.fs,
                        &chosen.receiver,
                        &receiver,
                        &chosen.image,
                    ),
                    Err(_) if original == Original::Failed => false,
                    Err(_) => return self.refused(request, key, Refusal::ReadFailed),
                }
            };
            let phase = cancel_phase(original, &source, certified, chosen.removed);
            let written = self.write_decision(key, chosen, phase);
            self.repair_op(&chosen.name);
            if !written {
                return self.refused(request, key, Refusal::DraftFailed);
            }
        }
        self.take_disk(key, page, bytes);
        // The discarded page first: a waiting caller hears the page the user
        // discarded, not the source it reverts.
        self.order.discarded(key);
        if chosen.as_ref().is_some_and(|c| c.on_receiver) {
            self.order.cancel_move(key);
        }
        if let (Some(src), Some(bytes)) = (src, src_bytes) {
            self.order.supersede(&src);
            self.take_disk(&src, self.pages[&src].clone(), bytes);
        }
        self.answer(request, key, false);
    }

    /// Write a Use Disk decision on `chosen`'s endpoint (page `key`) through
    /// the guarded vehicle path (fresh no-replace name, temp, fsync,
    /// rename, private-directory sync). True when it is durable. Any write
    /// that may have reached the disk decides the pair in `Order` (at
    /// `phase`): a removal is never the result of an uncertain choice.
    fn write_decision(&mut self, key: &str, chosen: &order::Chosen, phase: PairPhase) -> bool {
        let (stage, decision) = self.decision_vehicle(
            key,
            &chosen.name,
            chosen.id,
            chosen.member,
            drafts::Choice::TakeDisk,
        );
        // Absent is durably absent; any other stage may leave it on disk.
        if stage != Stage::Absent {
            self.order
                .decide(&chosen.name, chosen.member, Some(phase), Some(decision));
        }
        stage == Stage::Present
    }

    /// Newer text on `key`, the source of a pair that may still remove it,
    /// is about to save (`Host::start_save`): write its superseding choice
    /// (the decision's choice `2`, with the text) through the guarded
    /// vehicle path first, so no relaunch replays the removal over it. True
    /// when there is nothing to write or it is durable. A write that may
    /// have reached the disk registers its vehicle but leaves the choice
    /// None (`Order::decide_superseded`): the save waits, and its retry
    /// writes the decision again until one is known durable.
    pub(super) fn write_superseding(&mut self, key: &str, page: &Page) -> bool {
        let Some((name, id, member)) = self.order.supersedes(key) else {
            return true;
        };
        let choice = drafts::Choice::Text {
            superseding: true,
            version: page.version,
            base: page.base.clone(),
            bytes: page.buf.clone(),
        };
        let (stage, decision) = self.decision_vehicle(key, &name, id, member, choice);
        // Absent is durably absent; any other stage may leave it on disk,
        // but only Present lets the text publish over the removal.
        if stage != Stage::Absent {
            let durable = stage == Stage::Present;
            self.order
                .decide_superseded(&name, member, decision, durable);
        }
        stage == Stage::Present
    }

    /// One decision vehicle on endpoint `key` (`member`) of paired operation
    /// `name` (`id`), written now: fresh no-replace name, temp, fsync,
    /// rename, private-directory sync. Its last stage, its file name and
    /// body; a draft error when it is not durable.
    fn decision_vehicle(
        &mut self,
        key: &str,
        name: &str,
        id: [u8; 16],
        member: u32,
        choice: drafts::Choice,
    ) -> (Stage, (String, drafts::Decision)) {
        self.wseq = self.wseq.checked_add(1).expect("draft sequence exhausted");
        let decision = drafts::Decision {
            op: id,
            member,
            wseq: self.wseq,
            choice,
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
        if vehicle.stage != Stage::Present {
            self.events.push(Event::DraftError {
                effect: name.into(),
                pages: BTreeSet::from([key.into()]),
                refresh: None,
                failures: vehicle.failures,
            });
        }
        (vehicle.stage, (file, decision))
    }

    /// Restore `key`'s admitted preimage after its pair's Use Disk left the
    /// text only in the operation draft (the model's `restoreSource`): the
    /// page takes the preimage at risk over an absent base, so its own draft
    /// and its guarded create publish it, and that publication cancels the
    /// pair. Refused unless the slot is free on disk (an occupied slot keeps
    /// the preimage as custody) and the page holds no unsaved text; a
    /// claimant or twin fails the create, as for any creation.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn restore_source(&mut self, key: &str) -> Disposition {
        if !self.alive {
            return Disposition::Disabled;
        }
        let Some((name, preimage)) = self.order.restorable(key) else {
            return Disposition::Refused;
        };
        if self.busy(key) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        if self.pages.get(key).is_some_and(|p| !p.clean()) {
            return Disposition::Refused;
        }
        let keys = BTreeSet::from([key.to_owned()]);
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            // The slot is absent and no other spelling claims the page
            // (the creation guard's twin check); its save re-checks both.
            if !matches!(host.fs.read_page(key), Ok(None))
                || !matches!(host.fs.page_twin(key), Ok(None))
            {
                return Disposition::Refused;
            }
            #[cfg(test)]
            host.events.push(Event::OperationRead {
                page: key.to_owned(),
                base: Base::Known(None),
            });
            host.order.select_restore(key);
            host.repair_op(&name);
            host.version = host.next_version();
            let page = Page {
                buf: preimage,
                base: Base::Known(None),
                version: host.version,
                typed: true,
                risk: true,
                conflict: false,
                // It read the slot absent.
                obs: Some(None),
            };
            host.set_page(key, Some(page));
            Disposition::Applied
        })
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
