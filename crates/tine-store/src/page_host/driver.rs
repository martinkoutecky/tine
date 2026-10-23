//! The page host's driver thread (STEP3 §1). One per graph binding.
//!
//! Lock order, one rule: store writer → path locks (canonical spelling
//! order) → host state mutex. Nobody waits for a lock while holding a later
//! one, and nobody waits on a condition while holding any lock. A driver
//! step is plan, lock, revalidate: under the state mutex the host names the
//! path locks its next step needs (`Host::lacks_locks`) and returns before
//! any side effect; the driver drops the state mutex, takes those path locks,
//! retakes the state mutex and polls again. That poll recomputes the next
//! legal step from the current state, so an admission, reservation or
//! observation that arrived meanwhile is honoured; a step needing other locks
//! releases everything and replans.
//!
//! The state is private to this file (host-only-writer §3.2): every other
//! thread reaches it through a request on the driver's inbox, run on the
//! driver thread between its steps. A caller's only wait is for its answer,
//! bounded by its deadline even while the driver sits in a disk call. A
//! request that needs the store writer has the driver take it first (lock
//! order); one whose path lock another thread holds is parked and retried
//! (A4), so a caller never waits holding a lock the driver may need.
use super::binding::{Book, Delivery, Indexing, Publication};
use super::progress::{Clock, Progress};
use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Condvar, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Where a driver step's results go, called on the driver thread after it
/// has released the state mutex and every path lock: host events in host
/// order (the publication consumer, §5) and the mail the host delivered to
/// the window (the page-mail bridge, §3.3).
pub(super) trait Sink: Send {
    /// `owner` answers, under the state mutex, who publishes a key's index
    /// now. Returns each publication's result, recorded in order (§5). A
    /// delivery that needs the store writer while another thread holds it
    /// comes back untouched unless `wait`: the driver never waits for a lock
    /// another thread holds while callers wait on it (A4), only at its stop.
    fn deliver(
        &mut self,
        delivery: Delivery,
        owner: &dyn Fn(&str) -> Owner,
        wait: bool,
    ) -> Result<Vec<(Publication, Indexing)>, Delivery>;
}

/// Who publishes a key's index (§5, Q6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    /// The watcher: the host does not hold the key.
    Watcher,
    /// The driver's publication consumer, in host event order.
    Consumer,
    /// A retained writer's reservation: its transaction publishes.
    Reservation,
}

/// A watcher read or a released reservation the driver must observe (§5, §7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Observation {
    pub failures: u32,
    pub due: u64,
    /// The watcher forwarded it: its read bills as an observation's (O5).
    pub forwarded: bool,
}

pub(super) struct State<F: HostIo, C: Clock> {
    pub progress: Progress<F, C>,
    /// Bumped by every wake source; the driver sleeps only while it is
    /// unchanged since its plan, so no wakeup is lost.
    wake_seq: u64,
    /// Keys whose disk state the driver owes the host an observation of.
    pub observe: BTreeMap<PageKey, Observation>,
    /// Keys whose path lock another thread held at the driver's last try:
    /// consecutive misses and the retry instant (A4, REVIEW-3a). A path lock
    /// is a physical resource, so its retry runs on the monotonic clock, not
    /// on the host's logical one.
    contended: BTreeMap<PageKey, (u32, Instant)>,
    /// The instant the last step judged `contended` at: a retry after it
    /// was a held key for that step and must still wake the driver, even
    /// once it has passed by the time the driver plans its sleep.
    stepped: Instant,
    /// The binding's bookkeeping beside the host (handoffs, notices sent).
    pub book: Book,
    #[cfg(test)]
    pub polls: u64,
}

impl<F: HostIo, C: Clock> State<F, C> {
    fn wake(&mut self) {
        self.wake_seq = self.wake_seq.wrapping_add(1);
    }

    /// Tests' quiescence rounds (`PageHost::quiesce_for_tests`).
    #[cfg(feature = "test-faults")]
    pub fn wake_seq(&self) -> u64 {
        self.wake_seq
    }

    /// One driver step. A key whose path lock another thread holds is not
    /// waited for until its retry: its step waits and other work runs (A4).
    fn step(&mut self) -> Disposition {
        let now = self.progress.clock.now_ms();
        // A miss not retried for a second is history: its step went away.
        let at = Instant::now();
        self.stepped = at;
        let stale = at.checked_sub(Duration::from_secs(1));
        self.contended
            .retain(|_, (_, retry)| stale.is_none_or(|stale| *retry > stale));
        self.progress.host.contended = self.contending(at).collect();
        let result = self.step_unblocked(now);
        self.progress.host.contended.clear();
        result
    }

    /// Another thread held `key`'s path lock: retry it after a short
    /// backoff (10 ms doubling to 160 ms), woken by the driver's timer.
    fn contend(&mut self, key: PageKey) -> Instant {
        let (misses, retry) = self.contended.entry(key).or_insert((0, Instant::now()));
        *misses = misses.saturating_add(1);
        *retry = Instant::now() + Duration::from_millis(10 << (*misses).min(5).saturating_sub(1));
        *retry
    }

    /// The keys still waiting for their path-lock retry at `at`.
    fn contending(&self, at: Instant) -> impl Iterator<Item = PageKey> + '_ {
        let waiting = self
            .contended
            .iter()
            .filter(move |(_, (_, retry))| *retry > at);
        waiting.map(|(key, _)| key.clone())
    }

    /// The earliest path-lock retry the last step waited for: one already
    /// past is due now (a zero sleep), never dropped.
    fn retry(&self) -> Option<Instant> {
        let stepped = self.stepped;
        self.contended
            .values()
            .map(|(_, retry)| *retry)
            .filter(|r| *r > stepped)
            .min()
    }

    /// One host step: an owed observation first, then progress. Read
    /// failures retry with the save backoff; the third is reported.
    fn step_unblocked(&mut self, now: u64) -> Disposition {
        #[cfg(test)]
        {
            self.polls += 1;
        }
        let owed = self
            .observe
            .iter()
            .inspect(|_| work(Work::Candidates, 1))
            .find(|(key, o)| o.due <= now && self.observable(key))
            .map(|(key, o)| (key.clone(), o.forwarded));
        if let Some((key, forwarded)) = owed {
            let observe = || self.progress.with_host(|host| host.observe(&key));
            let result = observation(forwarded, observe);
            if self.progress.host.lock_request.is_some() {
                return Disposition::Waiting;
            }
            match result {
                Disposition::Refused => {
                    let o = self.observe.get_mut(&key).unwrap();
                    o.failures = o.failures.saturating_add(1);
                    o.due = now.saturating_add(super::progress::backoff(o.failures));
                    if o.failures >= 3 {
                        self.progress.host.events.push(Event::ObserveError(key));
                    }
                    return Disposition::Applied;
                }
                // Only a path lock is missing; the caller takes it.
                Disposition::Waiting => {}
                _ => {
                    self.observe.remove(&key);
                    self.book.handover.remove(&key);
                    self.book.observe_errors.remove(&key);
                    return Disposition::Applied;
                }
            }
        }
        self.progress.poll()
    }

    /// A busy page is observed after its job, worker step or reservation
    /// ends; until then its observation is blocked and contributes no
    /// deadline (the job's progress or the release wakes the driver).
    fn observable(&self, key: &str) -> bool {
        let host = &self.progress.host;
        let retried = |(_, retry): &(u32, Instant)| *retry <= Instant::now();
        !host.busy(key) && !host.allocator_busy() && self.contended.get(key).is_none_or(retried)
    }

    /// The earliest timed work, or None to sleep until woken. While another
    /// thread holds a path lock a step needed, the work it blocks is due
    /// already: only later times and the lock's `retry` wake the driver, so
    /// it never spins on a held lock (A4).
    fn until(&self) -> Option<u64> {
        let now = self.progress.clock.now_ms();
        let observe = self
            .observe
            .iter()
            .filter(|(key, _)| self.observable(key))
            .map(|(_, o)| o.due)
            .min();
        let held = self.retry().is_some();
        self.progress
            .next_deadline()
            .into_iter()
            .chain(observe)
            .chain(self.book.next_retry(&self.progress.host))
            .filter(|due| !held || *due > now)
            .min()
    }
}

/// What a waiting request's step decided (§3.2).
pub(super) enum Wait<R> {
    /// The answer.
    Ready(R),
    /// Not yet: the driver asks again after its next step, or at this
    /// instant at the latest.
    Until(Option<Instant>),
}

/// Why a caller got no answer from the driver (§3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unanswered {
    /// The deadline passed before the driver started the request, or while
    /// it was asking and that ask did not answer: it never takes effect.
    Withdrawn,
    /// The driver was asking at the deadline and had not finished that ask
    /// within `ASK_GRACE`; what it started keeps going, and the driver stops
    /// asking.
    Late,
    /// The driver ended: nothing will answer.
    Ended,
}

// A request's claim. The driver moves it QUEUED or PARKED → RUNNING for each
// ask and back to PARKED (or DONE once answered); a caller that leaves swaps
// in LEFT, and the previous value says whether the request was still unrun
// between asks (withdrawn: it never takes effect) or mid-ask (late).
const QUEUED: u8 = 0;
const RUNNING: u8 = 1;
const LEFT: u8 = 2;
const PARKED: u8 = 3;
const DONE: u8 = 4;

/// How long a caller past its deadline waits for an ask in flight to finish
/// (M6): one step under the state mutex, normally milliseconds; one inside a
/// disk call that outlasts it is `Late`.
const ASK_GRACE: Duration = Duration::from_secs(1);

/// How long a caller waits for its answer.
#[derive(Clone)]
pub(super) enum Deadline {
    /// As long as the driver runs.
    Never,
    At(Instant),
    /// An instant the request's own step may move later (a long operation
    /// that keeps making progress).
    Moving(Arc<Mutex<Instant>>),
}

impl Deadline {
    fn current(&self) -> Option<Instant> {
        match self {
            Deadline::Never => None,
            Deadline::At(at) => Some(*at),
            Deadline::Moving(at) => Some(*at.lock().unwrap_or_else(|e| e.into_inner())),
        }
    }
}

type Guard<'a, F, C> = MutexGuard<'a, State<F, C>>;
/// None: answered (or a notice). Some(at): ask again after the driver's
/// next step, or at `at` at the latest.
type Again = Option<Option<Instant>>;
type Run<F, C> =
    Box<dyn for<'a> FnMut(&'a Shared<F, C>, Guard<'a, F, C>) -> (Guard<'a, F, C>, Again) + Send>;

/// A request or notice from another thread, run on the driver thread.
struct Command<F: HostIo, C: Clock> {
    /// Taken before the state mutex (lock order writer → state).
    writer: Option<Arc<Mutex<()>>>,
    /// An admission (a request id the host admits in order): it never
    /// overtakes an earlier admission deferred on the writer.
    ordered: bool,
    claim: Arc<AtomicU8>,
    run: Run<F, C>,
}

pub(super) struct Shared<F: HostIo, C: Clock> {
    state: Mutex<State<F, C>>,
    inbox: Mutex<VecDeque<Command<F, C>>>,
    /// Signalled with the inbox mutex: a command arrived or the driver must stop.
    arrived: Condvar,
    stop: AtomicBool,
    /// The driver thread returned or unwound: nothing will answer a caller.
    ended: AtomicBool,
    /// Pauses a retained writer's flush before its first check (tests, N1).
    #[cfg(test)]
    pub flush_pause: Mutex<Option<crate::store::TestPause>>,
    /// Commands deferred on the store writer after the driver's last pass
    /// (tests: an admission's dispatch, observed).
    #[cfg(test)]
    pub deferred: std::sync::atomic::AtomicUsize,
    /// Tests: milliseconds the driver pauses, holding nothing, between an
    /// idle step chain and planning its sleep (a preempted driver thread).
    #[cfg(test)]
    pub plan_pause: std::sync::atomic::AtomicU64,
}

impl<F: HostIo + 'static, C: Clock + 'static> Shared<F, C> {
    /// Queue `command` and wake the driver; false once it ended.
    fn submit(&self, command: Command<F, C>) -> bool {
        let mut inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        if self.ended() {
            return false;
        }
        inbox.push_back(command);
        drop(inbox);
        self.arrived.notify_all();
        true
    }

    /// Send a request and wait for its answer until `deadline`. Past it,
    /// a request between asks is withdrawn and never takes effect; one the
    /// driver is asking gets that ask's outcome, or `Late` past `ASK_GRACE`.
    fn request<R: Send + 'static>(
        &self,
        deadline: Deadline,
        writer: Option<Arc<Mutex<()>>>,
        run: impl FnOnce(SyncSender<R>) -> Run<F, C>,
    ) -> Result<R, Unanswered> {
        self.request_with(deadline, writer, false, run)
    }

    fn request_with<R: Send + 'static>(
        &self,
        deadline: Deadline,
        writer: Option<Arc<Mutex<()>>>,
        ordered: bool,
        run: impl FnOnce(SyncSender<R>) -> Run<F, C>,
    ) -> Result<R, Unanswered> {
        let (reply, answer) = mpsc::sync_channel(1);
        let claim = Arc::new(AtomicU8::new(QUEUED));
        let command = Command {
            ordered,
            writer,
            claim: claim.clone(),
            run: run(reply),
        };
        if !self.submit(command) {
            return Err(Unanswered::Ended);
        }
        loop {
            let Some(at) = deadline.current() else {
                return answer.recv().map_err(|_| Unanswered::Ended);
            };
            match answer.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(answer) => return Ok(answer),
                Err(RecvTimeoutError::Disconnected) => return Err(Unanswered::Ended),
                // The step moved it: keep waiting.
                Err(RecvTimeoutError::Timeout)
                    if deadline.current().is_some_and(|now| now > Instant::now()) => {}
                Err(RecvTimeoutError::Timeout) => {
                    return match claim.swap(LEFT, Ordering::SeqCst) {
                        QUEUED | PARKED => Err(Unanswered::Withdrawn),
                        // Mid-ask: its outcome decides, for up to ASK_GRACE.
                        // An answer is the caller's; a reply dropped
                        // unanswered (the ask did not admit and cannot park
                        // now, or the driver is stopping) took no effect.
                        _ => match answer.recv_timeout(ASK_GRACE) {
                            Ok(answer) => Ok(answer),
                            Err(RecvTimeoutError::Disconnected) => Err(Unanswered::Withdrawn),
                            Err(RecvTimeoutError::Timeout) => Err(Unanswered::Late),
                        },
                    };
                }
            }
        }
    }

    /// Read the host state on the driver, without waking it for work.
    pub fn read<R: Send + 'static>(
        &self,
        read: impl FnOnce(&State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.read_by(Deadline::Never, read)
    }

    /// [`Self::read`], answered by `deadline` or withdrawn.
    pub fn read_by<R: Send + 'static>(
        &self,
        deadline: Deadline,
        read: impl FnOnce(&State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        let mut read = Some(read);
        self.request(deadline, None, |reply| {
            Box::new(move |_, state| {
                if let Some(read) = read.take() {
                    let _ = reply.send(read(&state));
                }
                (state, None)
            })
        })
    }

    /// Run `action` on the driver under the state mutex and wake it: the
    /// wake source for admission, reservations and switches.
    pub fn call<R: Send + 'static>(
        &self,
        action: impl FnOnce(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.call_with(None, false, action)
    }

    /// [`Self::call`] for an admission, a request id the host admits only
    /// in increasing order: it runs after every admission queued before it,
    /// one deferred on the store writer included, so a window's requests
    /// are admitted in the order they arrive. Never issued holding the store
    /// writer: it could wait behind an open that waits for that writer. Only
    /// the window's page commands admit (`src-tauri`
    /// `window_admissions_run_only_from_the_page_commands`).
    pub fn admission<R: Send + 'static>(
        &self,
        action: impl FnOnce(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.call_with(None, true, action)
    }

    /// [`Self::call`] under the store `writer`, which the driver takes
    /// before the state mutex: a caller never waits for the driver holding
    /// the writer its publication step takes.
    pub fn call_writer<R: Send + 'static>(
        &self,
        writer: &Arc<Mutex<()>>,
        action: impl FnOnce(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.call_with(Some(writer.clone()), false, action)
    }

    /// [`Self::admission`] under the store `writer`, which the driver takes
    /// before the state mutex: a caller never waits for the driver holding
    /// the writer its publication step takes.
    pub fn admission_writer<R: Send + 'static>(
        &self,
        writer: &Arc<Mutex<()>>,
        action: impl FnOnce(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.call_with(Some(writer.clone()), true, action)
    }

    fn call_with<R: Send + 'static>(
        &self,
        writer: Option<Arc<Mutex<()>>>,
        ordered: bool,
        action: impl FnOnce(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        let mut action = Some(action);
        self.request_with(Deadline::Never, writer, ordered, |reply| {
            Box::new(move |_, mut state| {
                if let Some(action) = action.take() {
                    let answer = action(&mut state);
                    state.wake();
                    let _ = reply.send(answer);
                }
                (state, None)
            })
        })
    }

    /// A notice: run `action` on the driver and wake it, answering nothing.
    /// Dropped once the driver ended. Never waits, so a watcher or a
    /// `Drop` sends it holding any lock.
    pub fn notify(&self, action: impl FnOnce(&mut State<F, C>) + Send + 'static) {
        let mut action = Some(action);
        self.submit(Command {
            writer: None,
            ordered: false,
            claim: Arc::new(AtomicU8::new(QUEUED)),
            run: Box::new(move |_, mut state| {
                if let Some(action) = action.take() {
                    action(&mut state);
                    state.wake();
                }
                (state, None)
            }),
        });
    }

    /// Wait on the host: the driver runs `step` under the state mutex now
    /// and again after each of its steps, until it is ready, and answers
    /// then (an answer wakes the driver, a wait does not). Past `deadline`
    /// the caller leaves, and the driver stops asking.
    pub fn wait<R: Send + 'static>(
        &self,
        deadline: Deadline,
        mut step: impl FnMut(&mut State<F, C>) -> Wait<R> + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.request(deadline, None, |reply| {
            Box::new(move |_, mut state| match step(&mut state) {
                Wait::Ready(answer) => {
                    state.wake();
                    let _ = reply.send(answer);
                    (state, None)
                }
                Wait::Until(at) => (state, Some(at)),
            })
        })
    }

    /// [`Self::wait`] as plan, lock, revalidate (§1): `step` runs under the
    /// path locks it names. One another thread holds parks the request
    /// until its retry (A4), so the caller waits holding nothing.
    pub fn locked_wait<R: Send + 'static>(
        &self,
        deadline: Deadline,
        mut step: impl FnMut(&mut State<F, C>) -> Wait<R> + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.request(deadline, None, |reply| {
            Box::new(
                move |shared, state| match locked(shared, state, false, &mut step) {
                    Locked::Done(mut state, Wait::Ready(answer), _) => {
                        state.wake();
                        let _ = reply.send(answer);
                        (state, None)
                    }
                    Locked::Done(state, Wait::Until(at), _) => (state, Some(at)),
                    // Asked again at the held key's own retry, even one
                    // already past: never left without a timer.
                    Locked::Held(state, retry) => (state, Some(Some(retry))),
                    // The reply drops unanswered: the caller sees `Ended`.
                    Locked::Stopping(state) => (state, None),
                },
            )
        })
    }

    /// [`Self::locked_wait`] for a step that is always ready.
    pub fn locked_step<R: Send + 'static>(
        &self,
        deadline: Deadline,
        mut step: impl FnMut(&mut State<F, C>) -> R + Send + 'static,
    ) -> Result<R, Unanswered> {
        self.locked_wait(deadline, move |state| Wait::Ready(step(state)))
    }

    /// Tests: the state, read or held directly (holding it stalls the
    /// driver); a poisoned one too, so an unwinding test can clean up.
    #[cfg(test)]
    pub fn state_for_tests(&self) -> Guard<'_, F, C> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Tests: change the state directly, then wake the driver.
    #[cfg(test)]
    pub fn with_state_for_tests<R>(&self, action: impl FnOnce(&mut State<F, C>) -> R) -> R {
        let mut state = self.state.lock().unwrap();
        let result = action(&mut state);
        state.wake();
        drop(state);
        self.notify(|_| {});
        result
    }
}

impl<F: HostIo, C: Clock> Shared<F, C> {
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Ask the driver to stop after its current step; queued commands run first.
    fn halt(&self) {
        let inbox = self.inbox.lock().unwrap_or_else(|e| e.into_inner());
        self.stop.store(true, Ordering::SeqCst);
        drop(inbox);
        self.arrived.notify_all();
    }

    /// The queued commands, and whether a stop was asked before them.
    fn take(&self) -> (VecDeque<Command<F, C>>, bool) {
        let mut inbox = self.inbox.lock().unwrap();
        (std::mem::take(&mut *inbox), self.stopping())
    }

    /// Sleep until a command arrives, a stop is asked, or `wait` passes.
    fn sleep(&self, wait: Option<Duration>) {
        let inbox = self.inbox.lock().unwrap();
        if !inbox.is_empty() || self.stopping() {
            return;
        }
        match wait {
            Some(wait) if wait.is_zero() => {}
            Some(wait) => drop(self.arrived.wait_timeout(inbox, wait).unwrap()),
            None => drop(self.arrived.wait(inbox).unwrap()),
        }
    }
}

/// Marks the driver ended on every exit, an unwind included, and wakes
/// every waiting caller.
struct Ended<'a, F: HostIo, C: Clock>(&'a Shared<F, C>);

impl<F: HostIo, C: Clock> Drop for Ended<'_, F, C> {
    fn drop(&mut self) {
        // Bench events: the host's event vector at close (a poisoned state,
        // after a panic, reports nothing).
        if crate::bench_events::enabled() {
            if let Ok(state) = self.0.state.lock() {
                host_stats(&state.progress.host);
            }
        }
        // Under the inbox mutex, so nothing is queued after: every caller
        // still waiting sees its request dropped unanswered.
        let mut inbox = self.0.inbox.lock().unwrap_or_else(|e| e.into_inner());
        self.0.ended.store(true, Ordering::SeqCst);
        let dropped = std::mem::take(&mut *inbox);
        drop(inbox);
        drop(dropped);
        self.0.arrived.notify_all();
    }
}

pub(super) struct Driver<F: HostIo, C: Clock> {
    pub shared: Arc<Shared<F, C>>,
    thread: Option<JoinHandle<()>>,
}

impl<F, C> Driver<F, C>
where
    F: HostIo + Send + 'static,
    C: Clock + Send + 'static,
{
    pub fn spawn(mut host: Host<F>, clock: C, mut sink: impl Sink + 'static) -> Self {
        host.held = Some(BTreeSet::new());
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                progress: Progress::new(host, clock),
                wake_seq: 0,
                observe: BTreeMap::new(),
                contended: BTreeMap::new(),
                stepped: Instant::now(),
                book: Book::default(),
                #[cfg(test)]
                polls: 0,
            }),
            inbox: Mutex::new(VecDeque::new()),
            arrived: Condvar::new(),
            stop: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            #[cfg(test)]
            flush_pause: Mutex::new(None),
            #[cfg(test)]
            deferred: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            plan_pause: std::sync::atomic::AtomicU64::new(0),
        });
        let thread = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("tine-page-host".into())
                .spawn(move || run(&shared, &mut sink))
                .expect("page host driver thread")
        };
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Stop the loop and join the thread; its last step's results have been
    /// delivered to the sink when this returns (§7 restore step 4).
    pub fn join(&mut self) {
        self.shared.halt();
        if let Some(thread) = self.thread.take() {
            if let Err(panic) = thread.join() {
                if !std::thread::panicking() {
                    std::panic::resume_unwind(panic);
                }
            }
        }
    }
}

impl<F: HostIo, C: Clock> Drop for Driver<F, C> {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.shared.halt();
            let _ = thread.join();
        }
    }
}

/// Run `observe`; when the watcher forwarded the observation, its
/// whole-file reads bill as `observation_reads` (§5.4, O5). Free without
/// `test-faults`.
#[inline]
#[cfg_attr(not(feature = "test-faults"), allow(unused_variables))]
fn observation<R>(forwarded: bool, observe: impl FnOnce() -> R) -> R {
    #[cfg(feature = "test-faults")]
    if forwarded {
        return crate::cost_counters::observing(observe);
    }
    observe()
}

/// How a plan, lock, revalidate run ended (§1).
pub(super) enum Locked<'a, F: HostIo, C: Clock, R> {
    /// A step ran to its end, with the state mutex held, no path lock and
    /// `held` empty; true when it ran under path locks.
    Done(Guard<'a, F, C>, R, bool),
    /// A request's step needs a path lock another thread holds: nothing ran
    /// under it, and the request waits for that key's retry (A4).
    Held(Guard<'a, F, C>, Instant),
    /// The driver is stopping.
    Stopping(Guard<'a, F, C>),
}

/// Plan, lock, revalidate (§1), for the driver's steps and for requests,
/// all on the driver thread: run `step` holding no path lock; while it
/// names path locks it lacks, take them in canonical spelling order holding
/// nothing else, retake the state mutex and run it again, now the next
/// legal step under those locks. An alias spelling move (§2) may replace a
/// key's lock meanwhile; the old spelling's lock does not exclude writers of
/// the new one, so everything is released and the step replanned. The
/// driver never blocks on a path lock another thread holds (A4): its own
/// step marks the key and replans, so other pages' work runs; a request
/// returns `Held` and is asked again at the key's retry.
pub(super) fn locked<'a, F: HostIo, C: Clock, R>(
    shared: &'a Shared<F, C>,
    mut state: Guard<'a, F, C>,
    driver: bool,
    mut step: impl FnMut(&mut State<F, C>) -> R,
) -> Locked<'a, F, C, R> {
    state.progress.host.held = Some(BTreeSet::new());
    let mut result = step(&mut state);
    while let Some(keys) = state.progress.host.lock_request.take() {
        let host = &state.progress.host;
        work(Work::WorkerKeys, keys.len());
        let mut handles: Vec<_> = keys
            .iter()
            .map(|key| (host.fs.spelling(key), host.locks[key].clone(), key.clone()))
            .collect();
        handles.sort_by(|a, b| a.0.cmp(&b.0));
        drop(state);
        let mut guards = Vec::new();
        let mut held_elsewhere = None;
        for (_, lock, key) in &handles {
            match lock.try_lock() {
                Ok(guard) => guards.push(guard),
                Err(std::sync::TryLockError::WouldBlock) => {
                    held_elsewhere = Some(key.clone());
                    break;
                }
                Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
            }
        }
        if held_elsewhere.is_some() {
            guards.clear();
        }
        state = shared.state.lock().unwrap();
        if shared.stopping() {
            return Locked::Stopping(state);
        }
        if let Some(key) = held_elsewhere {
            let retry = state.contend(key);
            if !driver {
                return Locked::Held(state, retry);
            }
            result = step(&mut state);
            continue;
        }
        state.contended.retain(|key, _| !keys.contains(key));
        let host = &state.progress.host;
        // One handle per key: each is current when it is still its own
        // key's lock (M4: linear in the key set).
        let current = handles.iter().all(|(_, held, key)| {
            work(Work::WorkerKeys, 1);
            Arc::ptr_eq(held, &host.locks[key])
        });
        if current {
            state.progress.host.held = Some(keys);
            result = step(&mut state);
            state.progress.host.held = Some(BTreeSet::new());
            if state.progress.host.lock_request.is_none() {
                drop(guards);
                return Locked::Done(state, result, true);
            }
        }
        // Another key set or lock: release everything and replan.
        drop(guards);
        if !current {
            result = step(&mut state);
        }
    }
    Locked::Done(state, result, false)
}

/// What became of a queued command.
enum Dispatched<F: HostIo, C: Clock> {
    /// Answered, a notice, or withdrawn by its caller.
    Done,
    /// It waits on the host, asked again at the instant.
    Waiting(Command<F, C>, Option<Instant>),
    /// Another thread holds the store writer it needs: still queued.
    Deferred(Command<F, C>),
}

/// Start a queued command: run it on the driver (under the store writer
/// first when it asks for it), unless its caller withdrew it. One whose
/// writer another thread holds is deferred, not waited for (A4), unless
/// `wait` (the driver's stop): it stays queued, and its caller may still
/// withdraw it.
fn dispatch<F: HostIo, C: Clock>(
    shared: &Shared<F, C>,
    mut command: Command<F, C>,
    wait: bool,
) -> Dispatched<F, C> {
    let writer = command.writer.clone();
    let _writer = match writer.as_deref() {
        None => None,
        Some(writer) if wait => Some(writer.lock().unwrap()),
        Some(writer) => match writer.try_lock() {
            Ok(guard) => Some(guard),
            Err(std::sync::TryLockError::WouldBlock) => return Dispatched::Deferred(command),
            Err(std::sync::TryLockError::Poisoned(error)) => panic!("{error}"),
        },
    };
    if !claim(&command.claim, QUEUED) {
        return Dispatched::Done;
    }
    let state = shared.state.lock().unwrap();
    let (state, again) = (command.run)(shared, state);
    drop(state);
    debug_assert!(
        again.is_none() || command.writer.is_none(),
        "a writer request waits"
    );
    match park(command, again) {
        Some((command, at)) => Dispatched::Waiting(command, at),
        None => Dispatched::Done,
    }
}

/// Start an ask of a request that is `from` (queued or parked); false when
/// its caller left.
fn claim(claim: &AtomicU8, from: u8) -> bool {
    let ordering = Ordering::SeqCst;
    claim
        .compare_exchange(from, RUNNING, ordering, ordering)
        .is_ok()
}

/// End an ask: a request that waits is parked, unless its caller left
/// meanwhile; one that answered is done.
fn park<F: HostIo, C: Clock>(
    command: Command<F, C>,
    again: Again,
) -> Option<(Command<F, C>, Option<Instant>)> {
    let ordering = Ordering::SeqCst;
    let Some(at) = again else {
        let _ = command
            .claim
            .compare_exchange(RUNNING, DONE, ordering, ordering);
        return None;
    };
    let parked = command
        .claim
        .compare_exchange(RUNNING, PARKED, ordering, ordering);
    parked.is_ok().then_some((command, at))
}

/// Ask every waiting request again (after a step chain, what a condition
/// broadcast did); a request whose caller left is dropped.
fn revisit<'a, F: HostIo, C: Clock>(
    shared: &'a Shared<F, C>,
    mut state: Guard<'a, F, C>,
    waiting: &mut Vec<(Command<F, C>, Option<Instant>)>,
) -> Guard<'a, F, C> {
    let mut kept = Vec::with_capacity(waiting.len());
    for (mut command, _) in waiting.drain(..) {
        if !claim(&command.claim, PARKED) {
            continue;
        }
        let (next, again) = (command.run)(shared, state);
        state = next;
        kept.extend(park(command, again));
    }
    *waiting = kept;
    state
}

/// How often the driver reports its host's event vector (bench events).
const HOST_STATS_EVERY: Duration = Duration::from_secs(5);

/// Bench events: `host`'s event vector, its length and allocation.
fn host_stats<F: HostIo>(host: &Host<F>) {
    crate::bench_events::emit(|| crate::bench_events::Ev::HostStats {
        events_len: host.events.len(),
        events_bytes: host.events.capacity() * std::mem::size_of::<Event>(),
    });
}

/// The first and the longest pause before a store-writer retry: another
/// thread's transaction holds it for a disk write or an index publication.
const WRITER_RETRY: (Duration, Duration) = (Duration::from_millis(1), Duration::from_millis(20));

fn run<F: HostIo, C: Clock>(shared: &Shared<F, C>, sink: &mut impl Sink) {
    let _ended = Ended(shared);
    // The next bench `host_stats` (None: events off).
    let mut stats = crate::bench_events::enabled().then(|| Instant::now() + HOST_STATS_EVERY);
    // Requests waiting on the host, each with the instant to ask it again.
    let mut waiting = Vec::new();
    // Commands and a delivery whose store writer another thread held,
    // retried after `writer_retry`. A pending delivery is the last step's
    // results: no step runs until it is delivered, so host events reach the
    // sink in host order (§5), as when the driver waited for the writer.
    let mut deferred: VecDeque<Command<F, C>> = VecDeque::new();
    let mut pending: Option<Delivery> = None;
    let mut writer_retry = WRITER_RETRY.0;
    loop {
        // Commands first, between steps, so requests and notices interleave
        // with a long run of driver steps; those queued before a stop run,
        // and at the stop the driver waits for the writer they need.
        let (commands, stop) = shared.take();
        // Admissions keep their order: once one is deferred, every later
        // one queues behind it (deferred commands are older, so first).
        let queued = std::mem::take(&mut deferred).into_iter().chain(commands);
        let mut blocked = false;
        for command in queued {
            if blocked && command.ordered {
                deferred.push_back(command);
                continue;
            }
            match dispatch(shared, command, stop) {
                Dispatched::Done => {}
                Dispatched::Waiting(command, at) => waiting.push((command, at)),
                Dispatched::Deferred(command) => {
                    blocked |= command.ordered;
                    deferred.push_back(command);
                }
            }
        }
        #[cfg(test)]
        shared.deferred.store(deferred.len(), Ordering::SeqCst);
        if stop {
            // The last step's results reach the sink before `join` returns.
            if let Some(delivery) = pending.take() {
                deliver(shared, sink, delivery, true);
            }
            return;
        }
        let mut state = shared.state.lock().unwrap();
        let seq = state.wake_seq;
        let (delivery, idle) = match pending.take() {
            // Retried, with no step: a held writer pauses below.
            Some(delivery) => (delivery, false),
            None => {
                let (next, result, stepped) = match locked(shared, state, true, State::step) {
                    Locked::Done(state, result, stepped) => (state, result, stepped),
                    Locked::Held(..) | Locked::Stopping(_) => return,
                };
                state = next;
                // Before the step's events are drained: what the vector holds.
                if stats.is_some_and(|due| due <= Instant::now()) {
                    host_stats(&state.progress.host);
                    stats = Some(Instant::now() + HOST_STATS_EVERY);
                }
                let delivery = {
                    let state = &mut *state;
                    state.book.collect(&mut state.progress)
                };
                let idle =
                    !stepped && matches!(result, Disposition::Waiting | Disposition::Disabled);
                (delivery, idle)
            }
        };
        if !delivery.is_empty() {
            drop(state);
            pending = deliver(shared, sink, delivery, false);
            state = shared.state.lock().unwrap();
        }
        // A finished job or worker step may answer a waiting request; every
        // step chain ends here.
        state = revisit(shared, state, &mut waiting);
        let retry_writer = pending.is_some() || !deferred.is_empty();
        let writer_wait = retry_writer.then_some(writer_retry);
        writer_retry = match retry_writer {
            true => (writer_retry * 2).min(WRITER_RETRY.1),
            false => WRITER_RETRY.0,
        };
        #[cfg(test)]
        {
            let pause = shared.plan_pause.load(Ordering::SeqCst);
            if pause > 0 && idle && state.wake_seq == seq {
                drop(state);
                std::thread::sleep(Duration::from_millis(pause));
                state = shared.state.lock().unwrap();
            }
        }
        if idle && state.wake_seq == seq {
            // Sleep until a command arrives, or until the earliest timed
            // work, waiting request or writer retry is due.
            let now = state.progress.clock.now_ms();
            let timer = state
                .until()
                .map(|due| Duration::from_millis(due.saturating_sub(now)));
            let at = Instant::now();
            let lock = state
                .retry()
                .map(|retry| retry.saturating_duration_since(at));
            let report = stats.map(|due| due.saturating_duration_since(at));
            let asks = waiting.iter().filter_map(|(_, ask)| *ask);
            let asks = asks.map(|ask: Instant| ask.saturating_duration_since(at));
            drop(state);
            shared.sleep(
                timer
                    .into_iter()
                    .chain(lock)
                    .chain(report)
                    .chain(asks)
                    .chain(writer_wait)
                    .min(),
            );
        } else if pending.is_some() {
            // No step runs until the delivery is made; commands still run
            // between tries (`sleep` returns on one).
            drop(state);
            shared.sleep(writer_wait);
        }
    }
}

/// Hand `delivery` to the sink and record its index results; the delivery
/// back when the store writer it needs is held elsewhere and not `wait`.
fn deliver<F: HostIo, C: Clock>(
    shared: &Shared<F, C>,
    sink: &mut impl Sink,
    delivery: Delivery,
    wait: bool,
) -> Option<Delivery> {
    let owner = |key: &str| {
        let state = shared.state.lock().unwrap();
        state.book.owner(key, &state.progress.host)
    };
    let results = match sink.deliver(delivery, &owner, wait) {
        Ok(results) => results,
        Err(delivery) => return Some(delivery),
    };
    if !results.is_empty() {
        // Collect again: an eviction or a notice may now be due.
        let mut state = shared.state.lock().unwrap();
        let now = state.progress.clock.now_ms();
        state.book.record(results, now);
        state.wake();
    }
    None
}

/// Wall-clock milliseconds since the driver started.
pub(super) struct SystemClock(std::time::Instant);

impl SystemClock {
    pub fn new() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}
