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

/// One feature write (DESIGN §3.1).
#[derive(Clone, Debug)]
pub struct HostOp {
    /// Every page's effect, with the content it was computed from.
    pub pages: Vec<(PageId, Effect, Expected)>,
    /// The edit kinds recorded for its pages (OG-RULES Rule 8).
    pub kinds: Vec<EditKind>,
    /// The writer's contract for unsaved input on its pages.
    pub policy: Input,
    /// `(receiver, source)`: the receiver's new bytes took the source's,
    /// which the operation deletes; its writes publish in a rename's order
    /// (the receiver, then the other pages, then the source's deletion).
    /// One move per operation in P2.
    pub moved: Option<(PageId, PageId)>,
    /// Transaction creation preconditions: page, optional DTO name, optional
    /// move source. In-memory admission metadata; no persisted format changes.
    /// Hand-built operations use an empty vector and the raw-create checks.
    pub(crate) creation_checks: Vec<(PageId, Option<String>, Option<crate::FileId>)>,
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
}

/// What one admission attempt came to.
enum Admitted {
    Slot(ReplySlot),
    /// These pages have unsaved input; with `Input::Flush`, wait for their
    /// saves holding nothing, then try again.
    Unsaved(BTreeSet<PageKey>, BTreeMap<PageKey, PageId>),
}

impl PageHost {
    /// Run `op` as one host operation (DESIGN §3.1). Admission is one
    /// decision on the driver under the operation's path locks: unsaved
    /// input refuses (`Input::Refuse`) or is saved first (`Input::Flush`);
    /// a page busy in another operation or reservation waits, at most
    /// `OPERATION_WAIT` from entry (`Busy`), while a flush waits for the
    /// save as a reservation's does (`bounded`: past the deadline
    /// only a page held by a rename refuses); a page whose content is not
    /// the operation's expected content refuses (`Changed`). Then waits
    /// for the operation's answer and every page's publication, as
    /// `rename` does.
    pub fn apply(&self, op: HostOp) -> Result<(), ApplyRefusal> {
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
            if !proved {
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
        let moved = match &op.moved {
            Some((dst, src)) => match (spellings.get(dst), spellings.get(src)) {
                (Some(dst), Some(src))
                    if dst != src
                        && matches!(effects.get(src), Some((None, Some(_))))
                        && matches!(effects.get(dst), Some((Some(_), _))) =>
                {
                    Some((dst.clone(), src.clone()))
                }
                _ => return Err(ApplyRefusal::Refused),
            },
            None => None,
        };
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
        let slot = loop {
            let (effects, keys, moved) = (effects.clone(), keys.clone(), moved.clone());
            let kinds = kinds.clone();
            let shared = shared.clone();
            let store = store.clone();
            let creations = creations.clone();
            let admit = move |state: &mut HostState| {
                let host = &state.progress.host;
                if !host.alive {
                    return Wait::Ready(Err(ApplyRefusal::Refused));
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
                    let moved = moved.as_ref().map(|(d, s)| (d.as_str(), s.as_str()));
                    let disposition = host.apply(&effects, moved);
                    let pending = disposition == Disposition::Pending;
                    (
                        disposition,
                        pending.then(|| host.register_reply()).flatten(),
                    )
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
                        Err(ReserveRefusal::Unsaved(pages)) => {
                            let page = pages.into_iter().next().expect("unsaved");
                            return Err(ApplyRefusal::Unsaved(page));
                        }
                        Err(ReserveRefusal::Busy) => return Err(ApplyRefusal::Busy),
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
                ReserveRefusal::Unsaved(pages) => {
                    ApplyRefusal::Unsaved(pages.into_iter().next().expect("unsaved"))
                }
                ReserveRefusal::Busy => ApplyRefusal::Busy,
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
    let tx = store.transaction(None);
    creations
        .iter()
        .any(|(file, name, source)| {
            tx.admission_slot(file, name.as_deref(), source.as_ref())
                .is_err()
        })
        .then_some(ApplyRefusal::Changed)
}
