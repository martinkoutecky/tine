//! A feature's planned write as one host operation (host-only-writer P2,
//! DESIGN §3.1): the feature computes every page's new bytes from the bytes
//! it read, and the host admits the whole set at once only if each page
//! still holds those bytes, then drafts, versions and publishes it as a
//! rename's operation. Nothing is fenced while the feature plans.
use super::driver::{Deadline, State, Unanswered, Wait};
use super::retained::{self, ReplySlot, Settled, OPERATION_WAIT};
use super::*;

type HostState = State<ProductionIo, SystemClock>;

/// A page's new content in a [`HostOp`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The page's file holds these bytes (created when it had none).
    Write(Arc<[u8]>),
    /// The page's file is removed.
    Delete,
}

/// The page content a [`HostOp`] was planned from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    /// The page's file held these bytes.
    Bytes(Arc<[u8]>),
    /// The page had no file.
    Absent,
}

/// Held input used to select an operation's write set. Admission checks the
/// entire set again, including pages excluded because their input was unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInputs(u64, BTreeMap<PageKey, (PageId, Text, u64, bool)>);

impl HostInputs {
    /// Each held page's current spelling, buffer and saved-input status.
    pub fn pages(&self) -> impl Iterator<Item = (&PageId, Option<&[u8]>, bool)> {
        self.1
            .values()
            .map(|(id, text, _, clean)| (id, text.as_deref(), *clean))
    }

    pub(super) fn current<F: HostIo>(&self, host: &Host<F>) -> bool {
        self.0 == host.incarnation
            && self.1.len() == host.pages.len()
            && self.1.iter().all(|(key, (id, text, version, clean))| {
                host.pages.get(key).is_some_and(|page| {
                    page.buf == *text
                        && page.version == *version
                        && page.clean() == *clean
                        && host.fs.spelling(key) == id.as_str()
                })
            })
    }
}

/// One feature write (DESIGN §3.1).
#[derive(Clone, Debug)]
pub struct HostOp {
    /// Use distinct entries for an absent case-only target on a sensitive volume.
    pub separate_case_entries: bool,
    /// Optional held-input selection guard, checked before admission effects.
    pub inputs: Option<HostInputs>,
    /// Every page's effect, with the content it was computed from.
    pub pages: Vec<(PageId, Effect, Expected)>,
    /// The edit kinds recorded for its pages (OG-RULES Rule 8).
    pub kinds: Vec<EditKind>,
    /// The writer's contract for unsaved input on its pages.
    pub policy: Input,
    /// Each `(receiver, source)`: the receiver's new bytes took the
    /// source's, which the operation deletes; its writes publish in a
    /// rename's order (the receivers, then the other pages, then the
    /// sources' deletions). No page is in two moves (S2: a namespace rename
    /// moves every page of the namespace in one operation).
    pub moves: Vec<(PageId, PageId)>,
    /// In-memory publication receipts, recorded at the guarded live rename,
    /// before any later host action. Kept by admitted work even if its caller
    /// times out. Only publications of this operation's bytes at its version
    /// or later produce receipts. Admission refuses a path already in this
    /// set under the same driver state that records publication, so a stale
    /// retry must replan rather than publish that path again.
    pub publications: Option<Arc<Mutex<BTreeSet<PageId>>>>,
    /// Transaction creation preconditions: page, optional DTO name, optional
    /// move source. In-memory admission metadata; no persisted format changes.
    /// Hand-built operations use an empty vector and the raw-create checks.
    pub(crate) creation_checks: Vec<(PageId, Option<String>, Option<crate::FileId>)>,
    /// The planner's view (`Transaction::expect_view`, B2): admitted only
    /// while no publication or config delivery has happened since, else the
    /// feature replans (in-scope scenario: a sync service or an external
    /// editor changed a page or `config.edn` since the plan). In memory only.
    pub(crate) view: Option<(crate::GraphRev, Arc<tine_core::config::Config>)>,
    /// What the planner knows of each page it serialized (`PagePlan::plan`
    /// of a `save_page` step), in `pages` order. The host never reads it:
    /// a writer that checks what its bytes mean takes it before `apply`
    /// instead of parsing them again (PRINCIPLES 3). Hand-built operations
    /// leave it empty. Memory only.
    pub saved: Vec<(PageId, Saved)>,
}

/// One page a plan serialized: the name and kind its new bytes give it, and
/// the Document those bytes parse to (the serializer's own round-trip parse,
/// `Graph::prepare_page_content`), with the runtime ids of the plan's blocks.
#[derive(Clone, Debug)]
pub struct Saved {
    /// The page's name from its new bytes (`title::`, else its file name).
    pub name: String,
    /// Page or journal.
    pub kind: tine_core::model::PageKind,
    /// The new bytes' parse.
    pub document: tine_core::doc::Document,
}

/// Why a host operation did not apply, or did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// A page has unsaved input (`Input::Refuse`), or its save cannot
    /// complete (`Input::Flush`: a conflict or a persistent save error).
    Unsaved(PageId),
    /// A page no longer holds the content the operation was planned from,
    /// or could not be read: the feature replans. In-scope scenario: the
    /// window, an external editor or a sync service changed the file.
    Changed,
    /// Not admitted within `OPERATION_WAIT`: other work held its pages.
    Busy,
    /// Not an operation this host can run: a path that is not a page of
    /// this graph, a page named twice, a move that is not a move, no edit
    /// kind, or a host that stopped. Nothing changed.
    Refused,
    /// The operation's draft could not be written: nothing changed.
    /// In-scope scenario: a full disk or an app-data disk error.
    DraftFailed,
    /// Admitted, but this page could not be written (a conflict or a
    /// persistent save error); the operation completes once it can.
    Unwritten(PageId),
    /// Applied, but this page was discarded or edited back before it
    /// published: a later edit decides it.
    Superseded(PageId),
    /// No answer or no publication within `OPERATION_WAIT`, or the host
    /// stopped: the operation may still complete.
    Uncertain,
    /// A folded entry move may be live, but the whole rename did not
    /// confirm completion. Retry an ordinary rename to the intended name.
    SpellingIncomplete,
}

/// What one admission attempt came to.
enum Admitted {
    Slot(ReplySlot),
    /// These pages have unsaved input; with `Input::Flush`, wait for their
    /// saves holding nothing, then try again.
    Unsaved(BTreeSet<PageKey>, BTreeMap<PageKey, PageId>),
}

impl PageHost {
    pub(crate) const PLANNING_WAIT: std::time::Duration = OPERATION_WAIT;

    /// Capture held input without reading disk or saving typed text. A feature
    /// selecting from it attaches the snapshot to its resulting operation.
    pub fn inputs(&self) -> Result<HostInputs, ApplyRefusal> {
        self.driver
            .shared
            .read(|state: &HostState| {
                let host = &state.progress.host;
                HostInputs(
                    host.incarnation,
                    host.pages
                        .iter()
                        .map(|(key, page)| {
                            (
                                key.clone(),
                                (
                                    PageId::from(host.fs.spelling(key)),
                                    page.buf.clone(),
                                    page.version,
                                    page.clean(),
                                ),
                            )
                        })
                        .collect(),
                )
            })
            .map_err(|_| ApplyRefusal::Refused)
    }

    /// Run `op` as one host operation (DESIGN §3.1). Admission is one
    /// decision on the driver under the operation's path locks: unsaved
    /// input refuses (`Input::Refuse`) or is saved first (`Input::Flush`);
    /// a page busy in another operation waits, at most
    /// `OPERATION_WAIT` from entry (`Busy`), while a flush waits for the
    /// save as an operation's does (`bounded`: past the deadline
    /// only a page held by a rename refuses); a page whose content is not
    /// the operation's expected content refuses (`Changed`). Then waits
    /// for the operation's answer and every page's publication, as
    /// `rename` does.
    pub fn apply(&self, mut op: HostOp) -> Result<(), ApplyRefusal> {
        // The plan's knowledge of its pages is its caller's: not held through
        // the operation (a page's Document is as large as the page).
        drop(std::mem::take(&mut op.saved));
        // Validate kinds at their admission owner, including before a
        // folded wrapper can move an entry. The wrapper forwards them intact.
        if op.kinds.is_empty() {
            return Err(ApplyRefusal::Refused);
        }
        // A folding volume has one key for both spellings. Its content is
        // an ordinary operation; the byte-preserving entry move runs on the
        // driver before that operation is admitted (S3, no new draft record).
        let mut spellings = Vec::new();
        let mut distinct_case_entries = BTreeSet::new();
        for (dst, src) in &op.moves {
            let source = self.try_identify(src).ok_or(ApplyRefusal::Refused)?;
            let target = self.try_identify(dst).ok_or(ApplyRefusal::Refused)?;
            // Single-page policy keeps distinct entries on sensitive volumes,
            // including source-trash custody. Carry local absence through
            // registration; other operations retain their entry-first policy.
            let sensitive_case_move = crate::file_kind::case_only(&src.file(), &dst.file())
                && !target.2
                && self
                    .store
                    .page_plan(None)
                    .admission_slot(&dst.file(), None, Some(&src.file()))
                    .is_ok_and(|absent| absent);
            if op.separate_case_entries && sensitive_case_move && source.0 != target.0 {
                distinct_case_entries.extend([src.clone(), dst.clone()]);
            }
            if source.0 == target.0 || (sensitive_case_move && !op.separate_case_entries) {
                spellings.push((dst.clone(), src.clone(), source.0, source.1));
            }
        }
        if !spellings.is_empty() {
            return self.apply_spellings(op, spellings);
        }
        let ended = || ApplyRefusal::Refused;
        let mut effects = BTreeMap::new();
        let mut spellings = BTreeMap::new();
        for (page, effect, expected) in &op.pages {
            let (key, spelling, _, proved) = self.try_identify(page).ok_or_else(ended)?;
            // `open`'s invalid-target refusal: a path that is not a page file
            // of this graph, or reaches outside it through a symlink, writes
            // nothing. In-scope scenarios: a plugin or a frontend defect
            // passing a path; an imported graph holding a symlinked folder
            // (REVIEW P2 round 2, finding 1).
            if self.store.page_spot(&spelling).is_err() {
                return Err(ApplyRefusal::Refused);
            }
            // A path that only collides with a registered entry, unproved,
            // is not the page the feature planned from (DESIGN §3.1).
            if !proved && !distinct_case_entries.contains(page) {
                return Err(ApplyRefusal::Changed);
            }
            self.try_register(&key, &spelling).ok_or_else(ended)?;
            let new = match effect {
                Effect::Write(bytes) => Some(bytes.clone()),
                Effect::Delete => None,
            };
            let old = match expected {
                Expected::Bytes(bytes) => Some(bytes.clone()),
                Expected::Absent => None,
            };
            // Two spellings of one entry (a case-folding volume) are one page.
            if effects.insert(key.clone(), (new, old)).is_some() {
                return Err(ApplyRefusal::Refused);
            }
            spellings.insert(page.clone(), key);
        }
        // A move deletes its source, planned from its bytes, and writes its
        // receiver; the host checks the same shape, so a refusal under its
        // state is only ever a changed page.
        let mut moved = Vec::new();
        let mut in_a_move = BTreeSet::new();
        for (dst, src) in &op.moves {
            match (spellings.get(dst), spellings.get(src)) {
                (Some(dst), Some(src))
                    if dst != src
                        && matches!(effects.get(src), Some((None, Some(_))))
                        && matches!(effects.get(dst), Some((Some(_), _)))
                        && in_a_move.insert(dst.clone())
                        && in_a_move.insert(src.clone()) =>
                {
                    moved.push((dst.clone(), src.clone()));
                }
                _ => return Err(ApplyRefusal::Refused),
            }
        }
        // OG-RULES Rule 8 (A-K1): an operation records the edit kinds it names.
        if effects.is_empty() || op.kinds.is_empty() {
            return Err(ApplyRefusal::Refused);
        }
        let keys: BTreeSet<PageKey> = effects.keys().cloned().collect();
        let (policy, kinds) = (op.policy, op.kinds);
        let mut deadline = std::time::Instant::now() + OPERATION_WAIT;
        let shared = Arc::downgrade(&self.driver.shared);
        let effects = Arc::new(effects);
        let store = self.store.clone();
        let publications = op.publications.clone();
        let mut publication_names: BTreeMap<PageKey, Vec<PageId>> = BTreeMap::new();
        for (page, key) in &spellings {
            publication_names
                .entry(key.clone())
                .or_default()
                .push(page.clone());
        }
        let creations: Vec<_> = op
            .pages
            .iter()
            .filter_map(|(page, effect, expected)| {
                if !matches!((effect, expected), (Effect::Write(_), Expected::Absent)) {
                    return None;
                }
                let check = op.creation_checks.iter().find(|(id, ..)| id == page);
                let name = check.and_then(|(_, name, _)| name.clone());
                // Slot rules belong to the original transaction step. Move
                // ordering declared by a feature cannot replace a Save's rules.
                let source = check.and_then(|(_, _, source)| source.clone());
                Some((page.file(), name, source))
            })
            .collect();
        let save_rules: Vec<_> = creations
            .iter()
            .map(|(file, name, source)| {
                let key = spellings[&PageId::from(file.as_str())].clone();
                (
                    key.clone(),
                    effects[&key].0.clone(),
                    name.clone(),
                    source.clone(),
                )
            })
            .collect();
        let slot = loop {
            let (effects, keys, moved) = (effects.clone(), keys.clone(), moved.clone());
            let kinds = kinds.clone();
            let shared = shared.clone();
            let store = store.clone();
            let creations = creations.clone();
            let view = op.view.clone();
            let save_rules = save_rules.clone();
            let publications = publications.clone();
            let publication_names = publication_names.clone();
            let inputs = op.inputs.clone();
            let admit = move |state: &mut HostState| {
                let host = &state.progress.host;
                if !host.alive {
                    return Wait::Ready(Err(ApplyRefusal::Refused));
                }
                if inputs.as_ref().is_some_and(|inputs| !inputs.current(host)) {
                    return Wait::Ready(Err(ApplyRefusal::Changed));
                }
                // The planner's receipt snapshot may predate an older
                // Uncertain operation's publication and a window undo.
                // Admission and guarded publication hold this same driver
                // state, so an overlapping retry cannot admit stale work.
                if publications.as_ref().is_some_and(|sink| {
                    let paths = sink.lock().unwrap_or_else(|p| p.into_inner());
                    publication_names
                        .values()
                        .flatten()
                        .any(|page| paths.contains(page))
                }) {
                    return Wait::Ready(Err(ApplyRefusal::Changed));
                }
                if let Some((rev, config)) = &view {
                    if store.changes.rev() != *rev || !Arc::ptr_eq(&store.config().config, config) {
                        return Wait::Ready(Err(ApplyRefusal::Changed));
                    }
                }
                let unsaved = retained::unsaved(host, &keys);
                if !unsaved.is_empty() {
                    return Wait::Ready(Ok(Admitted::Unsaved(keys.clone(), unsaved)));
                }
                // C3 is a binding pre-admission refusal (s33 stutter). Run
                // under the same path locks, before Host::apply can record it.
                if !creations.is_empty() {
                    if state.progress.host.lacks_locks(&keys) {
                        return Wait::Until(None);
                    }
                    if let Some(refusal) = creation_refusal(&store, &creations) {
                        return Wait::Ready(Err(refusal));
                    }
                }
                let (disposition, reply) = state.progress.with_host(|host| {
                    let disposition = host.apply(&effects, &moved);
                    let pending = disposition == Disposition::Pending;
                    if pending {
                        for (key, bytes, name, source) in &save_rules {
                            host.fs
                                .note_creation(key, bytes.clone(), name.clone(), source.clone());
                        }
                    }
                    let reply = pending.then(|| host.register_reply()).flatten();
                    if let (Some(id), Some(sink)) = (reply, publications.clone()) {
                        host.order.watch_publications(
                            id,
                            sink,
                            publication_names.clone(),
                            effects
                                .iter()
                                .map(|(key, (bytes, _))| (key.clone(), bytes.clone()))
                                .collect(),
                        );
                    }
                    (disposition, reply)
                });
                if state.progress.host.lock_request.is_some() {
                    // It runs again under its path locks.
                    return Wait::Until(None);
                }
                match (disposition, reply) {
                    (Disposition::Pending, Some(id)) => {
                        // OG-RULES Rule 8 (A-K1): the feature's own kinds.
                        for kind in &kinds {
                            state.book.took(keys.iter().cloned(), *kind);
                        }
                        Wait::Ready(Ok(Admitted::Slot(ReplySlot::new(shared.clone(), id))))
                    }
                    (Disposition::Waiting | Disposition::Pending, _) => {
                        let now = std::time::Instant::now();
                        if now >= deadline {
                            return Wait::Ready(Err(ApplyRefusal::Busy));
                        }
                        let pause = now + std::time::Duration::from_millis(100);
                        Wait::Until(Some(pause.min(deadline)))
                    }
                    // Every key is registered and unsaved input was ruled out
                    // under this state: the host refuses a page whose content
                    // is not the expected one, or that it could not read.
                    _ => Wait::Ready(Err(ApplyRefusal::Changed)),
                }
            };
            let admitted = self
                .driver
                .shared
                .locked_wait(Deadline::At(deadline), admit);
            match admitted {
                Ok(Ok(Admitted::Slot(slot))) => break slot,
                Ok(Ok(Admitted::Unsaved(keys, unsaved))) => {
                    let pages: BTreeSet<PageKey> = unsaved.keys().cloned().collect();
                    if policy == Input::Refuse {
                        let page = unsaved.into_values().next().expect("unsaved");
                        return Err(ApplyRefusal::Unsaved(page));
                    }
                    match self.flush(&pages, &keys, deadline) {
                        // The flush waits for a save as long as it runs; the
                        // admission after it gets a whole budget again
                        // (REVIEW P2 round 2, finding 2).
                        Ok(()) => {
                            deadline = std::time::Instant::now() + OPERATION_WAIT;
                            continue;
                        }
                        Err(FlushRefusal::Unsaved(pages)) => {
                            let page = pages.into_iter().next().expect("unsaved");
                            return Err(ApplyRefusal::Unsaved(page));
                        }
                        Err(FlushRefusal::Busy) => return Err(ApplyRefusal::Busy),
                    }
                }
                Ok(Err(refusal)) => return Err(refusal),
                // Not admitted: other work held its pages, or a path lock.
                Err(Unanswered::Withdrawn) => return Err(ApplyRefusal::Busy),
                // Admission was under way: it may complete.
                Err(Unanswered::Late) => return Err(ApplyRefusal::Uncertain),
                Err(Unanswered::Ended) => return Err(ApplyRefusal::Refused),
            }
        };
        match self.settle(&slot, None, deadline) {
            Settled::Applied => Ok(()),
            Settled::DraftFailed => Err(ApplyRefusal::DraftFailed),
            Settled::Unwritten(page) => Err(ApplyRefusal::Unwritten(self.spelling(&page))),
            Settled::Superseded(page) => Err(ApplyRefusal::Superseded(self.spelling(&page))),
            Settled::Uncertain => Err(ApplyRefusal::Uncertain),
        }
    }

    /// Save `pages`' unsaved input now, waiting as `apply`'s
    /// `Input::Flush` does: a feature planning an operation from the store
    /// first makes every page it reads hold its saved bytes, because a page
    /// that exists only as typed input reads as absent (REVIEW P2). Then the
    /// plan's expected bytes are the page's, and input typed after this is
    /// `apply`'s to flush. `Unsaved` names a page whose save cannot
    /// complete (a conflict or a persistent save error).
    pub fn flush_input(&self, pages: &[PageId]) -> Result<(), ApplyRefusal> {
        // `apply`'s invalid-target refusal, as `open`'s.
        if pages.iter().any(|page| self.store.page_spot(page).is_err()) {
            return Err(ApplyRefusal::Refused);
        }
        let keys = self
            .register_all(pages.to_vec())
            .ok_or(ApplyRefusal::Refused)?;
        let deadline = std::time::Instant::now() + OPERATION_WAIT;
        self.flush(&keys, &keys, deadline)
            .map_err(|refusal| match refusal {
                FlushRefusal::Unsaved(pages) => {
                    ApplyRefusal::Unsaved(pages.into_iter().next().expect("unsaved"))
                }
                FlushRefusal::Busy => ApplyRefusal::Busy,
            })
    }
}

/// Binding C3: a creation whose name slot another file claims is refused.
/// The real binding and lockstep conformance both call this one predicate
/// before the inner host can draft or version an operation.
pub(in crate::page_host) fn creation_refusal(
    store: &crate::Store,
    creations: &[(crate::FileId, Option<String>, Option<crate::FileId>)],
) -> Option<ApplyRefusal> {
    let tx = store.page_plan(None);
    creations
        .iter()
        .any(|(file, name, source)| {
            tx.admission_slot(file, name.as_deref(), source.as_ref())
                .is_err()
        })
        .then_some(ApplyRefusal::Changed)
}
