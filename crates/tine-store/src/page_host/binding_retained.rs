//! Retained writers' reservations, the single-page rename, and the
//! switch/restore stop (STEP3 §6–§7): the binding commands that fence
//! pages away from the host or run a page operation for a writer.
use super::driver::{Deadline, Shared, State, Unanswered, Wait};
use super::*;
use crate::model::entry_identity::Identity;
use crate::transaction::validation::{
    prepared as reuse_rewrite, rewrite as rewrite_refs, rewrite_move,
};
use crate::RenameMap;

/// Why a host rename did not run (§7, F10), or did not finish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameRefusal {
    /// `target` is another spelling of `source`'s own directory entry (a
    /// case-folding volume, Q4): the caller moves it as a retained
    /// transaction under a reservation of the page and `respell`s it.
    Alias,
    /// A page the rename would rewrite has unsaved input.
    Unsaved(PageId),
    /// A page the rename would change cannot be rewritten: an Org file
    /// that does not round-trip, or bytes that are not UTF-8.
    Unwritable(PageId),
    /// The operation was admitted but this page could not be written (a
    /// conflict or a persistent save error); the operation keeps its
    /// custody and completes once the page can be written.
    Unwritten(PageId),
    /// The target has a file, or the host is stopped.
    Refused,
    /// The operation's draft could not be written: nothing changed. In-scope
    /// scenario: a full disk or an app-data disk error.
    DraftFailed,
    /// Applied, but this page was discarded or edited back before it
    /// published: a later edit decides it.
    Superseded(PageId),
    /// No answer or no publication within `OPERATION_WAIT`, or the host
    /// stopped: the operation may still complete; Tine keeps trying.
    Uncertain,
    /// Not admitted within `OPERATION_WAIT`: other work held its pages
    /// (another rename still running over them, or draft recovery).
    Busy,
}

/// Why a retained writer was given no reservation (§7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReserveRefusal {
    /// These pages have unsaved input (`Input::Refuse`), or their save
    /// cannot complete (`Input::Flush`), named by current spelling.
    Unsaved(BTreeSet<PageId>),
    /// A rename still finishing held one of the pages for `OPERATION_WAIT`
    /// from the writer's start (M6); nothing is reserved.
    Busy,
}

/// The planner's rewrite of a referrer, when it was computed from exactly
/// these bytes under this file-name format (GH #623).
pub type PreparedRewrite =
    dyn Fn(&PageId, &[u8], tine_core::config::FileNameFormat) -> Option<Vec<u8>> + Send + Sync;

type HostShared = Shared<ProductionIo, SystemClock>;
pub(super) type HostState = State<ProductionIo, SystemClock>;

/// The longest a host operation's caller waits for admission, then for its
/// answer and every page's publication (Finding B, R1).
pub const OPERATION_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// What a caller's admitted operation came to, by precedence: Superseded,
/// Applied, Unwritten, Uncertain (STEP3-DESIGN option 2; plan v3 N6).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Settled {
    Applied,
    DraftFailed,
    Unwritten(PageKey),
    Superseded(PageKey),
    Uncertain,
}

/// A caller's reply slot. Only its holder removes it, here, on every exit
/// (an unwind included), so a late answer creates nothing (R2). Made on
/// the driver with the operation's admission: a slot whose caller already
/// left is dropped there, and removed the same way.
pub(super) struct ReplySlot {
    shared: std::sync::Weak<HostShared>,
    id: ReplyId,
}

impl ReplySlot {
    pub(super) fn new(shared: std::sync::Weak<HostShared>, id: ReplyId) -> Self {
        Self { shared, id }
    }
}

impl Drop for ReplySlot {
    fn drop(&mut self) {
        // A notice: a driver that ended has no slot left to answer.
        let id = self.id;
        if let Some(shared) = self.shared.upgrade() {
            shared.notify(move |state| {
                let _ = state.progress.host.order.remove(id);
            });
        }
    }
}

/// The pages a retained writer holds (§7 step 3): none of them can be
/// opened, loaded, saved or drafted until it is dropped. Dropping it ends
/// the retained transaction (§7 step 5), on every exit, a panic's unwind
/// included (A-R1): the driver unreserves the keys and observes each,
/// retrying a failed read with the save backoff. A save before that
/// observation is safe: its guard check reads the writer's change. The
/// transaction's index stands until that observation succeeds (V2).
#[must_use = "dropping a reservation releases its pages"]
pub struct Reservation {
    keys: BTreeSet<PageKey>,
    shared: Arc<HostShared>,
    /// Whether the writer may have written: false while `reserve` still
    /// checks the set, when a withdrawal hands nothing over.
    handover: bool,
}

impl Reservation {
    /// The reserved keys.
    pub(crate) fn keys(&self) -> &BTreeSet<PageKey> {
        &self.keys
    }
}

impl std::fmt::Debug for Reservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reservation")
            .field("keys", &self.keys)
            .finish()
    }
}

// Runs in `Reservation::drop` just before it takes the state lock (E90's
// test plants a poison there).
#[cfg(test)]
thread_local! {
    pub(super) static BEFORE_RELEASE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let keys = std::mem::take(&mut self.keys);
        #[cfg(test)]
        if let Some(hook) = BEFORE_RELEASE.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
        // A notice, so it never waits or panics, an unwind included (E90):
        // a driver that ended drops it with every page it held.
        let handover = self.handover;
        self.shared.notify(move |state| {
            for key in keys {
                state.progress.host.retained.remove(&key);
                if handover {
                    state.book.handover.insert(key.clone());
                    state.observe.entry(key).or_default();
                }
            }
        });
    }
}

/// One step of a retained writer's reservation, decided on the driver
/// ([`PageHost::bounded`]).
enum Step<R> {
    Done(R),
    /// Waiting on a busy page; `renaming`: on a rename still finishing.
    Wait {
        renaming: bool,
    },
    /// The host cannot serve the writer.
    Failed,
}

/// The pages among `keys` with unsaved input (Q3), including an admitted
/// operation's pages before its draft applies (a retiring draft is not
/// input: its page is saved), each with its current spelling.
pub(in crate::page_host) fn unsaved<F: HostIo>(
    host: &Host<F>,
    keys: &BTreeSet<PageKey>,
) -> BTreeMap<PageKey, PageId> {
    keys.iter()
        .filter(|key| match host.logical(key) {
            Logical::Settled(page) => !page.clean(),
            Logical::Admitted { .. } => true,
            Logical::Absent => false,
        })
        .map(|key| (key.clone(), PageId::from(host.fs.spelling(key))))
        .collect()
}

/// What the rename policy recorded for one attempt (pages by spelling).
#[derive(Default)]
struct Rewrites {
    changed: BTreeSet<String>,
    skipped: BTreeSet<String>,
    unwritable: Option<String>,
}

impl PageHost {
    /// Reserve the complete page set `discover` names for a retained
    /// writer (§7 steps 2–3, R6). Waiting reservations hold no lock. Under
    /// the reservation discovery runs again; a set it does not cover starts
    /// over with the union fenced, never shrinking (Q-P2b-1): each restart
    /// strictly grows a set bounded by the pages discovery can name, so a
    /// discovery that alternates (a config flipping between plans) ends,
    /// with a superset reserved. Then,
    /// under that final reservation, the writer's input contract is checked
    /// on every page in it (Q3): unsaved input is a buffer that is not
    /// clean (typed, at risk or on an unknown base) or a submit or move the
    /// host admitted but has not applied. `Refuse` returns those pages;
    /// `Flush` releases, waits for their saves holding nothing, and retries,
    /// returning the pages whose save cannot complete. A returned page is
    /// named by its current spelling, never by its opaque key. The reservation
    /// exists from the fence on, so an unwind out of the rediscovery or the
    /// check withdraws it (A-R1). Every step, the fence, the check and the
    /// flush, goes through [`Self::bounded`] with one deadline,
    /// `OPERATION_WAIT` from entry (M6, REVIEW-3b-P2b-2 N1).
    pub fn reserve(
        &self,
        mut discover: impl FnMut() -> Vec<PageId>,
        input: Input,
    ) -> Result<Reservation, ReserveRefusal> {
        let deadline = std::time::Instant::now() + OPERATION_WAIT;
        let mut keys = self.register_all(discover()).ok_or(ReserveRefusal::Busy)?;
        loop {
            let fence = keys.clone();
            self.bounded(deadline, move |state| {
                match state.progress.host.reserve(&fence) {
                    Disposition::Applied => Step::Done(()),
                    Disposition::Waiting => Step::Wait {
                        renaming: state.progress.host.renaming(&fence),
                    },
                    // `Refused`, unreachable: every key was registered first.
                    // Were it not, nothing would be reserved, so it refuses as a
                    // host that stopped answering does.
                    _ => Step::Failed,
                }
            })?;
            let mut reservation = Reservation {
                keys: keys.clone(),
                shared: Arc::clone(&self.driver.shared),
                handover: false,
            };
            let found = self.register_all(discover()).ok_or(ReserveRefusal::Busy)?;
            if !found.is_subset(reservation.keys()) {
                drop(reservation);
                keys.extend(found);
                continue;
            }
            let held = reservation.keys().clone();
            let unsaved = self.bounded(deadline, move |state| {
                Step::Done(unsaved(&state.progress.host, &held))
            })?;
            if unsaved.is_empty() {
                reservation.handover = true;
                return Ok(reservation);
            }
            drop(reservation);
            if input == Input::Refuse {
                return Err(ReserveRefusal::Unsaved(unsaved.into_values().collect()));
            }
            self.flush(&unsaved.into_keys().collect(), &keys, deadline)?;
        }
    }

    /// `pages`' keys, each registered; None when the host state is
    /// poisoned.
    pub(super) fn register_all(&self, pages: Vec<PageId>) -> Option<BTreeSet<PageKey>> {
        pages
            .iter()
            .map(|page| {
                let (key, spelling, ..) = self.try_identify(page)?;
                self.try_register(&key, &spelling)?;
                Some(key)
            })
            .collect()
    }

    /// Every wait and check of a retained writer's reservation (M6,
    /// REVIEW-3b-P2b-2 N1): the driver runs `step` until it is done, again
    /// after each of its steps. With each step: a host that cannot answer
    /// refuses `Busy` (the driver ended, or the host is down; in-scope
    /// scenario: a host that failed on a disk error), and past `deadline` a
    /// step waiting on a rename still finishing refuses `Busy` (in-scope
    /// scenario: a disk error keeps failing its drafts or writes, or a sync
    /// service or external editor conflicted one of its pages). A step
    /// waiting on any other busy page keeps waiting.
    fn bounded<R: Send + 'static>(
        &self,
        deadline: std::time::Instant,
        mut step: impl FnMut(&mut HostState) -> Step<R> + Send + 'static,
    ) -> Result<R, ReserveRefusal> {
        let wait = move |state: &mut HostState| {
            if !state.progress.host.alive {
                return Wait::Ready(Err(ReserveRefusal::Busy));
            }
            match step(state) {
                Step::Done(done) => Wait::Ready(Ok(done)),
                Step::Failed => Wait::Ready(Err(ReserveRefusal::Busy)),
                Step::Wait { renaming } => {
                    let now = std::time::Instant::now();
                    if now >= deadline && renaming {
                        return Wait::Ready(Err(ReserveRefusal::Busy));
                    }
                    Wait::Until((now < deadline).then_some(deadline))
                }
            }
        };
        let answer = self.driver.shared.wait(Deadline::Never, wait);
        answer.unwrap_or(Err(ReserveRefusal::Busy))
    }

    /// `page_rename` (§7, F10): rename `source`'s page to the absent
    /// `target` as one host operation, rewriting `referrers` (the index's
    /// explicit referrers; held buffers are scanned too) with `map`. The
    /// policy is the rename transaction's own: the moving page's refs and
    /// own title are rebound (`rewrite_move`), every other page's refs are
    /// rewritten, a changed Org file that does not round-trip refuses the
    /// whole operation, and a page carrying VCS conflict markers is left
    /// byte-identical (moved verbatim) and reported. A `source` with no file
    /// rewrites references only. Admitted only while the graph and config are
    /// still `view`'s, the context the caller planned in (A-W1, B2); otherwise
    /// `Refused`, and the caller replans. The operation's final pages are
    /// decided under the host lock from `referrers` and the held buffers
    /// (`rename_refs`); its drafts, flush and report cover exactly those.
    /// Waits while a page is busy, then until the operation's writes
    /// complete. Returns the rewritten referrers and the pages left for their
    /// markers; with nothing to write (a references-only rerun whose
    /// referrers already say the new name), both are empty. `prepared`
    /// returns the planner's rewrite of a referrer when it was computed from
    /// exactly the given bytes under the given file-name format; the host
    /// reuses it instead of rewriting under the driver's state lock
    /// (GH #623), and rewrites otherwise. Admission runs on the driver; a
    /// caller past `OPERATION_WAIT` before it is `Busy`, during it
    /// `Uncertain`.
    pub fn rename(
        &self,
        source: &PageId,
        target: &PageId,
        referrers: &[PageId],
        map: &RenameMap,
        view: &crate::WholeGraph,
        prepared: Arc<PreparedRewrite>,
    ) -> Result<(Vec<PageId>, Vec<PageId>), RenameRefusal> {
        crate::bench_events::mark("rename_enter", referrers.len() as u64);
        // A host that ended renames nothing.
        let ended = || RenameRefusal::Refused;
        let (src, src_spelling, ..) = self.try_identify(source).ok_or_else(ended)?;
        let (dst, dst_spelling, ..) = self.try_identify(target).ok_or_else(ended)?;
        if src == dst {
            return Err(RenameRefusal::Alias);
        }
        self.try_register(&src, &src_spelling).ok_or_else(ended)?;
        self.try_register(&dst, &dst_spelling).ok_or_else(ended)?;
        let mut refs = self.register_all(referrers.to_vec()).ok_or_else(ended)?;
        crate::bench_events::mark("rename_registered", refs.len() as u64);
        refs.remove(&src);
        refs.remove(&dst);
        let root = self.store.graph.root.clone();
        let format = view.config.file_name_format;
        let destination = root.join(dst_spelling.as_str());
        let deadline = std::time::Instant::now() + OPERATION_WAIT;
        let (store, map, rev) = (self.store.clone(), map.clone(), view.rev());
        let planned = view.config.config.clone();
        let shared = Arc::downgrade(&self.driver.shared);
        let admit = move |state: &mut HostState| {
            // Admission in the planner's context (B2); a stale view
            // replans. In-scope scenario: a sync service or external
            // editor changed a page or `config.edn` since the plan.
            let config = store.config().config;
            if store.changes.rev() != rev || !Arc::ptr_eq(&config, &planned) {
                return Wait::Ready(Err(Err(RenameRefusal::Refused)));
            }
            let (disposition, seen, pages, reply) = state.progress.with_host(|host| {
                // The keys the policy is called on: the operation's
                // pages and every held buffer (A-R5: not every key
                // ever registered).
                let spellings: BTreeMap<PageKey, String> = refs
                    .iter()
                    .chain([&src, &dst])
                    .chain(host.pages.keys())
                    .map(|key| (key.clone(), host.fs.spelling(key)))
                    .collect();
                let record = Mutex::new(Rewrites::default());
                let policy = |bytes: &Text, key: &str, moving: bool| {
                    let Some(old) = bytes else { return Ok(None) };
                    let spelling = &spellings[key];
                    let path = root.join(spelling);
                    let rewritten = if moving {
                        rewrite_move(old, &destination, &map, format)
                    } else if let Some(new) =
                        prepared(&PageId::from(spelling.as_str()), old, format)
                    {
                        reuse_rewrite(old, new, &path)
                    } else {
                        rewrite_refs(old, &path, &map, format)
                    };
                    let mut record = record.lock().unwrap();
                    let Ok(new) = rewritten else {
                        record.unwritable = Some(spelling.clone());
                        return Err(());
                    };
                    if new.as_slice() == &old[..] {
                        return Ok(bytes.clone());
                    }
                    let syntax = tine_core::model::Format::from_path(&path);
                    let marked = std::str::from_utf8(old).is_ok_and(|text| {
                        !tine_core::concord_queue::vcs_conflict_markers(text, syntax).is_empty()
                    });
                    if marked {
                        record.skipped.insert(spelling.clone());
                        return Ok(bytes.clone());
                    }
                    if !moving {
                        record.changed.insert(spelling.clone());
                    }
                    Ok(Some(Arc::from(new)))
                };
                let disposition = host.rename_with(&src, &dst, &refs, policy);
                let reply = if disposition == Disposition::Pending {
                    host.register_reply()
                } else {
                    None
                };
                // The final value (R3/R4): the operation's pages, or for
                // a refusal the pages it would have written; empty when
                // a references-only rerun has nothing left to write.
                let pages = match &host.worker {
                    Some(worker) if disposition == Disposition::Pending => worker.pages.clone(),
                    _ => {
                        let mut pages = host
                            .rename_refs(&src, &dst, &refs, &policy)
                            .unwrap_or_default();
                        if !pages.is_empty() || host.pages.get(&src).is_none_or(|p| p.buf.is_some())
                        {
                            pages.extend([src.clone(), dst.clone()]);
                        }
                        pages
                    }
                };
                (disposition, record.into_inner().unwrap(), pages, reply)
            });
            if state.progress.host.lock_request.is_some() {
                // A plan: it runs again under its path locks.
                return Wait::Until(None);
            }
            let slot = reply.map(|id| ReplySlot::new(shared.clone(), id));
            match (disposition, slot) {
                (Disposition::Pending, Some(slot)) => {
                    // OG-RULES Rule 8 (A-K1): the operation's writes are renames.
                    state.book.took(pages, EditKind::RenamePage);
                    Wait::Ready(Ok((seen, slot)))
                }
                (Disposition::Waiting | Disposition::Pending, _) => {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        return Wait::Ready(Err(Err(RenameRefusal::Busy)));
                    }
                    let pause = now + std::time::Duration::from_millis(100);
                    Wait::Until(Some(pause.min(deadline)))
                }
                _ => Wait::Ready(Err(if let Some(page) = seen.unwritable {
                    Err(RenameRefusal::Unwritable(PageId::from(page)))
                } else if pages.is_empty() {
                    Ok((Vec::new(), Vec::new()))
                } else {
                    Err(
                        match unsaved(&state.progress.host, &pages).into_values().next() {
                            Some(page) => RenameRefusal::Unsaved(page),
                            None => RenameRefusal::Refused,
                        },
                    )
                })),
            }
        };
        let (seen, slot) = match self
            .driver
            .shared
            .locked_wait(Deadline::At(deadline), admit)
        {
            Ok(Ok(admitted)) => admitted,
            Ok(Err(done)) => return done,
            // Not admitted: other work held its pages, or a path lock.
            Err(Unanswered::Withdrawn) => return Err(RenameRefusal::Busy),
            // Admission was under way: it may complete.
            Err(Unanswered::Late) => return Err(RenameRefusal::Uncertain),
            Err(Unanswered::Ended) => return Err(RenameRefusal::Refused),
        };
        crate::bench_events::mark("rename_admitted", seen.changed.len() as u64);
        let settled = self.settle(&slot, None, deadline);
        crate::bench_events::mark("rename_settled", 0);
        match settled {
            Settled::Applied => {
                let pages = |set: BTreeSet<String>| set.into_iter().map(PageId::from).collect();
                Ok((pages(seen.changed), pages(seen.skipped)))
            }
            Settled::DraftFailed => Err(RenameRefusal::DraftFailed),
            Settled::Unwritten(page) => Err(RenameRefusal::Unwritten(self.spelling(&page))),
            Settled::Superseded(page) => Err(RenameRefusal::Superseded(self.spelling(&page))),
            Settled::Uncertain => Err(RenameRefusal::Uncertain),
        }
    }

    /// Wait for a caller's admitted operation (Finding B): its answer, then
    /// each answered page's own publication at the answered version or later.
    /// Never success without both; Uncertain at `deadline`, once the driver
    /// ended or the state is poisoned, once the host stopped or relaunched,
    /// or once window `session` is not current (F1: a result never vouches
    /// for another session's pages). The deadline bounds a stall, not the
    /// operation: the answer and each newly published page move it to
    /// `OPERATION_WAIT` from then, so a long operation that keeps publishing
    /// (a rename with a thousand referrers on Windows) is waited for. The
    /// caller waits on the same moving deadline, so a driver stalled in a
    /// disk call answers `Uncertain` too.
    pub(super) fn settle(
        &self,
        slot: &ReplySlot,
        session: Option<u64>,
        deadline: std::time::Instant,
    ) -> Settled {
        let deadline = Arc::new(Mutex::new(deadline));
        let moving = deadline.clone();
        let id = slot.id;
        // (answered, pages witnessed) at the last ask.
        let mut progress = (false, 0);
        let step = move |state: &mut HostState| {
            let host = &state.progress.host;
            let Some(reply) = host.order.slot(id) else {
                return Wait::Ready(Settled::Uncertain);
            };
            if session.is_some_and(|session| session != state.book.session) {
                return Wait::Ready(Settled::Uncertain);
            }
            match &reply.reply {
                Some(OperationReply::DraftFailed) => return Wait::Ready(Settled::DraftFailed),
                Some(OperationReply::Applied(pages)) => {
                    if let Some(page) = &reply.superseded {
                        return Wait::Ready(Settled::Superseded(page.clone()));
                    }
                    let now = (true, reply.witnessed.len());
                    if now != progress {
                        progress = now;
                        let mut at = moving.lock().unwrap();
                        *at = (*at).max(std::time::Instant::now() + OPERATION_WAIT);
                    }
                    // `witnessed` holds answered pages only (`Order::published`).
                    if reply.witnessed.len() == pages.len() {
                        return Wait::Ready(Settled::Applied);
                    }
                    // Per ask, the host's few stuck pages, not every answered
                    // one: an operation's asks grow with its pages (R5).
                    let mut stuck = state.progress.stuck().inspect(|_| {
                        #[cfg(feature = "test-faults")]
                        crate::cost_counters::host_publication_probes(1);
                    });
                    let open = |key: &&PageKey| {
                        pages.contains_key(*key) && !reply.witnessed.contains(*key)
                    };
                    if let Some(page) = stuck.find(open) {
                        return Wait::Ready(Settled::Unwritten(page.clone()));
                    }
                    #[cfg(feature = "test-faults")]
                    crate::cost_counters::host_publication_probes(1);
                }
                None => {}
            }
            let now = std::time::Instant::now();
            let at = *moving.lock().unwrap();
            if !host.alive || host.incarnation != id.incarnation || now >= at {
                return Wait::Ready(Settled::Uncertain);
            }
            Wait::Until(Some(at))
        };
        let settled = self.driver.shared.wait(Deadline::Moving(deadline), step);
        settled.unwrap_or(Settled::Uncertain)
    }

    /// The alias spelling move (§2, Q4): under its reservation the caller
    /// moved `page`'s directory entry to `to`, another spelling of the same
    /// entry. The page keeps its key, buffer, drafts and queued requests;
    /// its I/O, path lock and held index follow `to`.
    pub fn respell(&self, page: &PageId, to: &PageId) {
        let graph = &self.store.graph;
        let lock = graph.page_lock(&graph.root.join(to.as_str()));
        let (store, page, to) = (self.store.clone(), page.clone(), to.clone());
        // Holds change under the writer, which the driver takes first (lock
        // order writer → state). The held index names the key, so it
        // follows the host's spelling table; its installed rows move with
        // it (A-H2).
        let respell = move |state: &mut HostState| {
            let host = &mut state.progress.host;
            let Some(key) = host.key_spelled(page.as_str()) else {
                return;
            };
            host.respell(&key, to.as_str(), lock);
            let graph = &store.graph;
            graph.respelled(&key, &graph.root.join(page.as_str()));
            store.publish_retired();
        };
        // A host that ended holds nothing to respell.
        let _ = self.driver.shared.call_writer(&self.store.writer, respell);
    }

    /// The key naming `page`'s directory entry (§2), by the one identity
    /// rule (B1, [`crate::model::Graph::identify`]): a registered key the
    /// path names, or whose entry it reaches as an alias (Q4's test: the
    /// same file, not listed apart); else a key for the new entry
    /// ([`super::Host::key_for`]: never a live key spelling another entry).
    /// Also whether the host holds that page, and whether the path is proved
    /// to name that key's entry. None when the driver ended.
    pub(super) fn try_identify(&self, page: &PageId) -> Option<(PageKey, PageId, bool, bool)> {
        let graph = &self.store.graph;
        let ended = std::cell::Cell::new(false);
        let registered = |fold: &str| {
            let fold = fold.to_owned();
            let read = move |state: &HostState| state.progress.host.candidates(&fold);
            self.driver.shared.read(read).unwrap_or_else(|_| {
                ended.set(true);
                Vec::new()
            })
        };
        let identity = graph.identify(&graph.root.join(page.as_str()), &registered);
        // The path names the key's entry: its own leaf, Q4's same-file alias,
        // or no registered collision. An unproved collision or a path outside
        // the root does not (step 3b P1: `page_open`'s baseline entry).
        let proved = !matches!(
            identity,
            Identity::Outside | Identity::Unknown { alias: None, .. }
        );
        let named = match identity {
            Identity::Key(key)
            | Identity::Unknown {
                alias: Some(key), ..
            } => Some(key),
            _ => None,
        };
        let disk = self
            .store
            .disk_spelling(page)
            .unwrap_or_else(|| page.clone());
        if ended.get() {
            return None;
        }
        let read = move |state: &HostState| {
            let host = &state.progress.host;
            let (key, spelling) = match named {
                Some(key) => (key.clone(), PageId::from(host.fs.spelling(&key))),
                None => (host.key_for(disk.as_str()), disk),
            };
            let held = host.pages.contains_key(&key);
            (key, spelling, held, proved)
        };
        self.driver.shared.read(read).ok()
    }

    /// Register `key` at `spelling` with its page lock; None when the
    /// driver ended.
    pub(super) fn try_register(&self, key: &str, spelling: &PageId) -> Option<()> {
        let graph = &self.store.graph;
        let lock = graph.page_lock(&graph.root.join(spelling.as_str()));
        let (key, spelling) = (key.to_owned(), spelling.clone());
        let register = move |state: &mut HostState| {
            state
                .progress
                .with_host(|host| host.register(key, spelling.as_str(), lock))
        };
        self.driver.shared.call(register).ok()
    }

    /// `keys`' current spellings, the pages a refusal names (REVIEW-3a5):
    /// a key is opaque, and need not be its page's spelling.
    fn spellings(&self, keys: &BTreeSet<PageKey>) -> BTreeSet<PageId> {
        let keys = keys.clone();
        let read = move |state: &HostState| {
            let fs = &state.progress.host.fs;
            keys.iter()
                .map(|key| PageId::from(fs.spelling(key)))
                .collect()
        };
        self.driver
            .shared
            .read(read)
            .expect("page host driver ended")
    }

    /// A key's current spelling, as the page it names.
    pub(super) fn spelling(&self, key: &str) -> PageId {
        let key = key.to_owned();
        let read = move |state: &HostState| PageId::from(state.progress.host.fs.spelling(&key));
        self.driver
            .shared
            .read(read)
            .expect("page host driver ended")
    }

    /// Wait, holding nothing, until no page in `pages` has unsaved input,
    /// within the writer's `deadline` (N1); a page in conflict or with a
    /// persistent save error refuses the writer, by its spelling. The rename
    /// dependency is classified over the writer's whole set, `reserving`, not
    /// only the pages it waits to see saved (REVIEW-3b-P2b-3): a rename
    /// holding a clean page of the set bounds the wait too.
    pub(super) fn flush(
        &self,
        pages: &BTreeSet<PageKey>,
        reserving: &BTreeSet<PageKey>,
        deadline: std::time::Instant,
    ) -> Result<(), ReserveRefusal> {
        #[cfg(test)]
        crate::store::pause_at_hook(&self.driver.shared.flush_pause);
        let (pages, reserving) = (pages.clone(), reserving.clone());
        self.bounded(deadline, move |state| {
            let progress = &state.progress;
            let host = &progress.host;
            let stuck: BTreeSet<PageId> = pages
                .iter()
                .filter(|key| host.logical(key).conflict() || progress.notice(key).save_error)
                .map(|key| PageId::from(host.fs.spelling(key)))
                .collect();
            if !stuck.is_empty() {
                return Step::Done(Err(ReserveRefusal::Unsaved(stuck)));
            }
            if unsaved(host, &pages).is_empty() {
                return Step::Done(Ok(()));
            }
            Step::Wait {
                renaming: host.renaming(&reserving),
            }
        })?
    }

    /// Stop the host (see `Drop`).
    #[cfg(test)]
    pub(crate) fn stop(self) {}

    /// Begin a switch or restore stop (§6 step 3, §7 step 3) once the
    /// window has consumed every answer up to `consumed_last_id`: admission
    /// closes and the driver saves every dirty page it can at once, before
    /// any draft of it. A failed save puts the page at risk, which drafts it
    /// in a switch (the model's switchReq, needed by no page a save-first
    /// order leaves dirty). False: the window has more to drain.
    pub(crate) fn stop_begin(&self, consumed_last_id: u64, mode: StopMode) -> bool {
        let begin = self.driver.shared.call(move |state| {
            let closed = state
                .progress
                .with_host(|host| host.switch_ready(consumed_last_id));
            let closed = closed == Disposition::Applied;
            if closed {
                state.progress.stopping = Some(Stopping {
                    restore: mode == StopMode::Restore,
                    failed: BTreeSet::new(),
                });
            }
            closed
        });
        begin.expect("page host driver ended")
    }

    /// The stop's barrier (§6 step 4) and its abort condition (step 5).
    pub(crate) fn stop_state(&self) -> StopState {
        let read = |state: &HostState| stop_state(&state.progress, &state.book);
        self.driver
            .shared
            .read(read)
            .expect("page host driver ended")
    }

    /// Stop a host no window owns any more (plan v3 §3, `HostRetirement`;
    /// legal as the model's `windowCrash`, `storage-s3.qnt:588-590`). Unless
    /// a stop already began (the window's own, kept), the window's session
    /// ends, once per abandoned owner: its mail is dropped, nothing more is
    /// admitted, and every admitted request keeps applying with custody.
    /// Once they have all applied, a switch stop begins over that watermark
    /// (a stale request's answer never enters the outbox, so none is owed).
    /// Returns where the stop stands: the caller calls `stop_finish` once
    /// the stop may complete, or `stop_abort` on `Aborted` and asks again
    /// later. While known draft vehicles are cleanup debt (REVIEW-3b-P1
    /// R2), each call first re-probes draft I/O, the Retry no window is
    /// left to press, so a repaired filesystem lets the stop complete.
    pub fn orphan_stop(&self) -> StopState {
        let debt = |state: &HostState| !state.progress.host.vehicle_debt.is_empty();
        if self
            .driver
            .shared
            .read(debt)
            .expect("page host driver ended")
        {
            let _ = self.drafts_retry();
        }
        let stopped = self.driver.shared.call(|state| {
            if state.progress.host.switch_confirmation.is_none() {
                if !state.book.orphaned {
                    state.book.orphaned = true;
                    state.book.session = next_session();
                    state.progress.with_host(|host| host.window_crash());
                }
                let ready = state.progress.with_host(|host| {
                    let last = host.last_admitted;
                    host.switch_ready(last)
                });
                if ready == Disposition::Applied {
                    state.progress.stopping = Some(Stopping {
                        restore: false,
                        failed: BTreeSet::new(),
                    });
                }
            }
            stop_state(&state.progress, &state.book)
        });
        stopped.expect("page host driver ended")
    }

    /// Point page mail at the window that now owns the binding (plan v3 §3,
    /// S2: an adopting window). Mail already collected keeps the session it
    /// was collected under.
    pub fn retarget(&self, mail: impl FnMut(PageMail) + Send + 'static) {
        *self.launch.mail.lock().unwrap() = Box::new(mail);
    }

    /// Abort the stop: admission reopens and the pages keep their state.
    pub fn stop_abort(&self) {
        let abort = |state: &mut HostState| state.progress.with_host(|host| host.switch_abort());
        // A host that ended has no stop left to abort.
        let _ = self.driver.shared.call(abort);
    }

    /// Stop the host if its stop may complete now, then join the driver (§7 step
    /// 4): a delivery in flight finishes first (with admission closed, every
    /// host step runs on the driver and is delivered with it), and the
    /// watcher indexes every page again. The binding holds no host until
    /// `Stopped::relaunch`. Otherwise the host is handed back.
    pub fn stop_finish(self) -> Result<Stopped, Self> {
        let stopped = self.driver.shared.call(|state| {
            stop_state(&state.progress, &state.book) == StopState::Ready
                && state.progress.with_host(|host| host.switch_finish()) == Disposition::Applied
        });
        let stopped = stopped.expect("page host driver ended");
        if stopped {
            let launch = self.launch.clone();
            drop(self);
            Ok(Stopped { launch })
        } else {
            Err(self)
        }
    }

    /// A whole stop for a backup restore or a switch (§6 steps 3–5, §7
    /// steps 3–4): begin it once the window consumed every answer up to
    /// `consumed_last_id`, wait for its barrier, then stop. When it cannot
    /// complete without losing custody, admission reopens and the host
    /// comes back with the affected pages (none: the window had more to
    /// drain, or trash custody cannot be listed), named by their current
    /// spellings; today's restore likewise stops when its flush fails.
    pub fn stop_saved(
        self,
        consumed_last_id: u64,
        mode: StopMode,
    ) -> Result<Stopped, (Box<Self>, BTreeSet<PageId>)> {
        if !self.stop_begin(consumed_last_id, mode) {
            return Err((Box::new(self), BTreeSet::new()));
        }
        let mut host = self;
        loop {
            match host.stop_state() {
                StopState::Waiting => {
                    // Until the barrier moves: asked after each driver step.
                    let moved =
                        |state: &mut HostState| match stop_state(&state.progress, &state.book) {
                            StopState::Waiting => Wait::Until(None),
                            _ => Wait::Ready(()),
                        };
                    let _ = host.driver.shared.wait(Deadline::Never, moved);
                }
                StopState::Ready => match host.stop_finish() {
                    Ok(stopped) => return Ok(stopped),
                    Err(back) => host = back,
                },
                StopState::Aborted(pages) => {
                    let pages = host.spellings(&pages);
                    host.stop_abort();
                    return Err((Box::new(host), pages));
                }
            }
        }
    }
}
