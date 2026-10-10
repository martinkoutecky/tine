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
use super::binding::{Book, Delivery, Indexing, Publication};
use super::progress::{Clock, Progress};
use super::*;
use std::sync::{Condvar, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Where a driver step's results go, called on the driver thread after it
/// has released the state mutex and every path lock: host events in host
/// order (the publication consumer, §5) and the mail the host delivered to
/// the window (the page-mail bridge, §3.3).
pub(super) trait Sink: Send {
    /// `owner` answers, under the state mutex, who publishes a key's index
    /// now. Returns each publication's result, recorded in order (§5).
    fn deliver(
        &mut self,
        delivery: Delivery,
        owner: &dyn Fn(&str) -> Owner,
    ) -> Vec<(Publication, Indexing)>;
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
}

pub(super) struct State<F: HostIo, C: Clock> {
    pub progress: Progress<F, C>,
    /// Bumped by every wake source; the driver sleeps only while it is
    /// unchanged since its plan, so no wakeup is lost.
    wake_seq: u64,
    stopping: bool,
    /// Keys whose disk state the driver owes the host an observation of.
    pub observe: BTreeMap<PageKey, Observation>,
    /// Keys whose path lock another thread held at the driver's last try:
    /// consecutive misses and the retry instant (A4, REVIEW-3a). A path lock
    /// is a physical resource, so its retry runs on the monotonic clock, not
    /// on the host's logical one.
    contended: BTreeMap<PageKey, (u32, Instant)>,
    /// The binding's bookkeeping beside the host (handoffs, notices sent).
    pub book: Book,
    #[cfg(test)]
    pub polls: u64,
}

impl<F: HostIo, C: Clock> State<F, C> {
    fn wake(&mut self) {
        self.wake_seq = self.wake_seq.wrapping_add(1);
    }

    /// One driver step. A key whose path lock another thread holds is not
    /// waited for until its retry: its step waits and other work runs (A4).
    fn step(&mut self) -> Disposition {
        let now = self.progress.clock.now_ms();
        // A miss not retried for a second is history: its step went away.
        let at = Instant::now();
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
    fn contend(&mut self, key: PageKey) {
        let (misses, retry) = self.contended.entry(key).or_insert((0, Instant::now()));
        *misses = misses.saturating_add(1);
        *retry = Instant::now() + Duration::from_millis(10 << (*misses).min(5).saturating_sub(1));
    }

    /// The keys still waiting for their path-lock retry at `at`.
    fn contending(&self, at: Instant) -> impl Iterator<Item = PageKey> + '_ {
        let waiting = self
            .contended
            .iter()
            .filter(move |(_, (_, retry))| *retry > at);
        waiting.map(|(key, _)| key.clone())
    }

    /// The earliest path-lock retry still ahead.
    fn retry(&self) -> Option<Instant> {
        let now = Instant::now();
        self.contended
            .values()
            .map(|(_, retry)| *retry)
            .filter(|r| *r > now)
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
            .find(|(key, o)| o.due <= now && self.observable(key))
            .map(|(key, _)| key.clone());
        if let Some(key) = owed {
            let result = self.progress.with_host(|host| host.observe(&key));
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
        self.progress.poll(0)
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

pub(super) struct Shared<F: HostIo, C: Clock> {
    pub state: Mutex<State<F, C>>,
    condition: Condvar,
}

impl<F: HostIo, C: Clock> Shared<F, C> {
    /// Run `action` under the state mutex and wake the driver: the wake
    /// source for admission, watcher reads, reservations and switches.
    pub fn with_state<R>(&self, action: impl FnOnce(&mut State<F, C>) -> R) -> R {
        self.try_with_state(action)
            .expect("page host state poisoned")
    }

    /// [`Self::with_state`], or None when the state is poisoned: a host
    /// that already panicked under its lock. The lock result is the only
    /// check, so a poison landing just before it is seen, not raced (E90).
    pub fn try_with_state<R>(&self, action: impl FnOnce(&mut State<F, C>) -> R) -> Option<R> {
        let mut state = self.state.lock().ok()?;
        let result = action(&mut state);
        state.wake();
        drop(state);
        self.condition.notify_all();
        Some(result)
    }

    /// Run one host step on a command thread as plan, lock, revalidate
    /// (§1), then wake the driver. None once the driver is stopping.
    pub fn locked_step<R>(&self, step: impl FnMut(&mut State<F, C>) -> R) -> Option<R> {
        let state = self.state.lock().unwrap();
        let (mut state, result, _) = locked(self, state, false, step)?;
        state.wake();
        drop(state);
        self.condition.notify_all();
        Some(result)
    }

    /// Wait on the host condition, under no lock but the state mutex
    /// (a reservation that found its keys busy, §7). The driver notifies
    /// after every step, so a finished job wakes the waiter.
    pub fn wait<'a>(
        &self,
        state: MutexGuard<'a, State<F, C>>,
        timeout: Duration,
    ) -> MutexGuard<'a, State<F, C>> {
        self.condition.wait_timeout(state, timeout).unwrap().0
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
                stopping: false,
                observe: BTreeMap::new(),
                contended: BTreeMap::new(),
                book: Book::default(),
                #[cfg(test)]
                polls: 0,
            }),
            condition: Condvar::new(),
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
        self.shared.with_state(|state| state.stopping = true);
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
            if let Ok(mut state) = self.shared.state.lock() {
                state.stopping = true;
                state.wake();
            }
            self.shared.condition.notify_all();
            let _ = thread.join();
        }
    }
}

/// The state mutex, a step's result, and whether it ran under path locks.
type Locked<'a, F, C, R> = (MutexGuard<'a, State<F, C>>, R, bool);

/// Plan, lock, revalidate (§1), for the driver and for command threads:
/// run `step` holding no path lock; while it names path locks it lacks, take
/// them in canonical spelling order holding nothing else, retake the state
/// mutex and run it again, now the next legal step under those locks. An
/// alias spelling move (§2) may replace a key's lock while this thread
/// waits; the old spelling's lock does not exclude writers of the new one,
/// so everything is released and the step replanned. Returns with the state
/// mutex held, no path lock and `held` empty, plus whether a step ran under
/// path locks; None once the driver is stopping.
pub(super) fn locked<'a, F: HostIo, C: Clock, R>(
    shared: &'a Shared<F, C>,
    mut state: MutexGuard<'a, State<F, C>>,
    driver: bool,
    mut step: impl FnMut(&mut State<F, C>) -> R,
) -> Option<Locked<'a, F, C, R>> {
    state.progress.host.held = Some(BTreeSet::new());
    let mut result = step(&mut state);
    while let Some(keys) = state.progress.host.lock_request.take() {
        let host = &state.progress.host;
        let mut handles: Vec<_> = keys
            .iter()
            .map(|key| (host.fs.spelling(key), host.locks[key].clone(), key.clone()))
            .collect();
        handles.sort_by(|a, b| a.0.cmp(&b.0));
        drop(state);
        // The driver never blocks on a path lock another thread holds (A4):
        // it marks the key and replans, so other pages' work runs.
        let mut guards = Vec::new();
        let mut held_elsewhere = None;
        for (_, lock, key) in &handles {
            if !driver {
                guards.push(lock.lock().unwrap());
                continue;
            }
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
        if state.stopping {
            return None;
        }
        if let Some(key) = held_elsewhere {
            state.contend(key);
            result = step(&mut state);
            continue;
        }
        state.contended.retain(|key, _| !keys.contains(key));
        let host = &state.progress.host;
        let current = keys.iter().all(|key| {
            let lock = &host.locks[key];
            handles.iter().any(|(_, held, _)| Arc::ptr_eq(held, lock))
        });
        if current {
            state.progress.host.held = Some(keys);
            result = step(&mut state);
            state.progress.host.held = Some(BTreeSet::new());
            if state.progress.host.lock_request.is_none() {
                drop(guards);
                return Some((state, result, true));
            }
        }
        // Another key set or lock: release everything and replan.
        drop(guards);
        if !current {
            result = step(&mut state);
        }
    }
    Some((state, result, false))
}

fn run<F: HostIo, C: Clock>(shared: &Shared<F, C>, sink: &mut impl Sink) {
    loop {
        // Every step retakes the state mutex, so admission, watcher reads
        // and reservations interleave with a long run of driver steps.
        let state = shared.state.lock().unwrap();
        if state.stopping {
            return;
        }
        let seq = state.wake_seq;
        let Some((mut state, result, stepped)) = locked(shared, state, true, State::step) else {
            return;
        };
        let delivery = {
            let state = &mut *state;
            state.book.collect(&mut state.progress)
        };
        // A finished job or worker step may unblock a waiting reservation;
        // every step chain ends here.
        shared.condition.notify_all();
        let idle = !stepped && matches!(result, Disposition::Waiting | Disposition::Disabled);
        if !delivery.is_empty() {
            drop(state);
            let owner = |key: &str| {
                let state = shared.state.lock().unwrap();
                state.book.owner(key, &state.progress.host)
            };
            let results = sink.deliver(delivery, &owner);
            state = shared.state.lock().unwrap();
            if !results.is_empty() {
                // Collect again: an eviction or a notice may now be due.
                let now = state.progress.clock.now_ms();
                state.book.record(results, now);
                state.wake();
            }
        }
        if idle {
            // Sleep until woken, or until the earliest timed work is due.
            let (until, retry) = (state.until(), state.retry());
            while state.wake_seq == seq && !state.stopping {
                let now = state.progress.clock.now_ms();
                let timer = until.map(|due| Duration::from_millis(due.saturating_sub(now)));
                let lock = retry.map(|at| at.saturating_duration_since(Instant::now()));
                state = match timer.into_iter().chain(lock).min() {
                    Some(wait) if wait.is_zero() => break,
                    Some(wait) => shared.condition.wait_timeout(state, wait).unwrap().0,
                    None => shared.condition.wait(state).unwrap(),
                };
            }
        }
    }
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
