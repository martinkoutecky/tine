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
use super::binding::{Book, Delivery};
use super::progress::{Clock, Progress};
use super::*;
use std::sync::{Condvar, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

/// Where a driver step's results go, called on the driver thread after it
/// has released the state mutex and every path lock: host events in host
/// order (the publication consumer, §5) and the mail the host delivered to
/// the window (the page-mail bridge, §3.3).
pub(super) trait Sink: Send {
    fn deliver(&mut self, delivery: Delivery);
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
    /// The binding's bookkeeping beside the host (handoffs, notices sent).
    pub book: Book,
    #[cfg(test)]
    pub polls: u64,
}

impl<F: HostIo, C: Clock> State<F, C> {
    fn wake(&mut self) {
        self.wake_seq = self.wake_seq.wrapping_add(1);
    }

    /// One host step: an owed observation first, then progress. Read
    /// failures retry with the save backoff; the third is reported.
    fn step(&mut self) -> Disposition {
        #[cfg(test)]
        {
            self.polls += 1;
        }
        let now = self.progress.clock.now_ms();
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
        !host.busy(key) && !host.allocator_busy()
    }

    /// The earliest timed work, or None to sleep until woken.
    fn until(&self) -> Option<u64> {
        let observe = self
            .observe
            .iter()
            .filter(|(key, _)| self.observable(key))
            .map(|(_, o)| o.due)
            .min();
        self.progress
            .next_deadline()
            .into_iter()
            .chain(observe)
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
        let mut state = self.state.lock().unwrap();
        let result = action(&mut state);
        state.wake();
        drop(state);
        self.condition.notify_all();
        result
    }

    /// Run one host step on a command thread as plan, lock, revalidate
    /// (§1), then wake the driver. None once the driver is stopping.
    pub fn locked_step<R>(&self, step: impl FnMut(&mut State<F, C>) -> R) -> Option<R> {
        let state = self.state.lock().unwrap();
        let (mut state, result, _) = locked(self, state, step)?;
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
            thread.join().expect("page host driver panicked");
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
    mut step: impl FnMut(&mut State<F, C>) -> R,
) -> Option<Locked<'a, F, C, R>> {
    state.progress.host.held = Some(BTreeSet::new());
    let mut result = step(&mut state);
    while let Some(keys) = state.progress.host.lock_request.take() {
        let host = &state.progress.host;
        let mut handles: Vec<_> = keys
            .iter()
            .map(|key| (host.fs.spelling(key), host.locks[key].clone()))
            .collect();
        handles.sort_by(|a, b| a.0.cmp(&b.0));
        drop(state);
        let guards: Vec<_> = handles.iter().map(|(_, l)| l.lock().unwrap()).collect();
        state = shared.state.lock().unwrap();
        if state.stopping {
            return None;
        }
        let host = &state.progress.host;
        let current = keys.iter().all(|key| {
            let lock = &host.locks[key];
            handles.iter().any(|(_, held)| Arc::ptr_eq(held, lock))
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
        let Some((mut state, result, stepped)) = locked(shared, state, State::step) else {
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
            sink.deliver(delivery);
            state = shared.state.lock().unwrap();
        }
        if idle {
            // Sleep until woken, or until the earliest timed work is due.
            let until = state.until();
            while state.wake_seq == seq && !state.stopping {
                let now = state.progress.clock.now_ms();
                state = match until {
                    Some(due) if due <= now => break,
                    Some(due) => {
                        shared
                            .condition
                            .wait_timeout(state, Duration::from_millis(due - now))
                            .unwrap()
                            .0
                    }
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
