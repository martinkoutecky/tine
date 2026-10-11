//! The tests' acknowledged watcher drain (REVIEW-3b-P2b-2 N3): a cost test
//! measures an operation through the watcher's settlement of its own
//! writes, so it needs a boundary after which no notification of those
//! writes is pending or being reconciled. Quiet counters cannot prove that:
//! a watcher blocked on the store writer is as quiet as an idle one.
//!
//! Each drain request writes its own sentinel file in the graph root, named
//! by the request's id, after the writes it covers. Notifications of one
//! watch arrive in order, so when that sentinel's first notification
//! arrives, every earlier one is already pending; the watcher acknowledges
//! the id the file name carries once the cycle that drained them has
//! reconciled (the store writer included). An acknowledgement names the
//! request whose sentinel produced it, never the latest request: a late or
//! duplicate notification of an older sentinel (a write raises several)
//! acknowledges only that older request (REVIEW-3b-P2b-3). A poll-mode
//! watcher acknowledges each full cycle that began after the request. The
//! launch's racy follow-up (§5.4) must have run too.
//!
//! Test support only: `test-faults` is a dev-dependency feature, so the
//! sentinel write never reaches a shipped build, and it writes only test
//! graphs (no persisted format; `test_*.rs` is test code to the og census).
use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::Condvar;

/// A sentinel's file name in the graph root: this prefix and the request id.
const SENTINEL: &str = ".tine-watch-drain-";

#[derive(Default)]
pub(crate) struct Barrier {
    asked: AtomicU64,
    acked: Mutex<u64>,
    done: Condvar,
    /// Tests: (first request of the plant, notifications the callback may
    /// still take); None: all pass.
    #[cfg(test)]
    gate: Mutex<Option<(u64, u64)>>,
    #[cfg(test)]
    opened: Condvar,
}

impl Barrier {
    /// The latest request (a poll cycle begun now answers it).
    pub(super) fn asked(&self) -> u64 {
        self.asked.load(Ordering::SeqCst)
    }

    /// The request a notification's sentinel names, if it is one.
    pub(super) fn sentinel(event: &notify::Result<notify::Event>) -> Option<u64> {
        let event = event.as_ref().ok()?;
        event
            .paths
            .iter()
            .filter_map(|path| path.file_name()?.to_str()?.strip_prefix(SENTINEL))
            .filter_map(|id| id.parse().ok())
            .max()
    }

    /// A cycle that drained request `id` completed.
    pub(super) fn ack(&self, id: u64) {
        let mut acked = self.acked.lock().unwrap();
        *acked = (*acked).max(id);
        self.done.notify_all();
    }

    /// Tests: the callback takes a notification only with a permit, except
    /// a late one of a request made before the gate was set.
    #[cfg(test)]
    pub(super) fn pass_gate(&self, event: &notify::Result<notify::Event>) {
        let mut gate = self.gate.lock().unwrap();
        if let Some((first, _)) = *gate {
            if Self::sentinel(event).is_some_and(|id| id < first) {
                return;
            }
        }
        while matches!(*gate, Some((_, 0))) {
            gate = self.opened.wait(gate).unwrap();
        }
        if let Some((_, permits)) = gate.as_mut() {
            *permits -= 1;
        }
    }
}

impl WatchHandle {
    /// Drain the watcher (module doc): true once every notification of a
    /// write made before the call has been reconciled and no racy follow-up
    /// is due; false at `timeout`.
    pub(crate) fn drained_for_tests(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let core = &self.core;
        let barrier = &core.barrier;
        loop {
            let asked = barrier.asked.fetch_add(1, Ordering::SeqCst) + 1;
            let sentinel = core.graph.root.join(format!("{SENTINEL}{asked}"));
            if fs::write(&sentinel, b"").is_err() {
                return false;
            }
            let _ = self.wake.send(());
            let mut acked = barrier.acked.lock().unwrap();
            while *acked < asked {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    drop(acked);
                    let _ = fs::remove_file(&sentinel);
                    return false;
                }
                acked = barrier.done.wait_timeout(acked, left).unwrap().0;
            }
            drop(acked);
            let _ = fs::remove_file(&sentinel);
            let due = *core.follow_up.lock().unwrap();
            let Some(due) = due else { return true };
            // The follow-up runs once due; then drain again.
            let wait = due.saturating_duration_since(Instant::now());
            if Instant::now() + wait >= deadline {
                return false;
            }
            std::thread::sleep(wait + Duration::from_millis(50));
        }
    }

    /// Tests: let the notify callback take only `permits` more
    /// notifications (None: all), holding the rest in order. Late
    /// notifications of drains requested before the gate was first set pass.
    #[cfg(test)]
    pub(crate) fn gate_notifications(&self, permits: Option<u64>) {
        let barrier = &self.core.barrier;
        let mut gate = barrier.gate.lock().unwrap();
        let first = gate.map_or(barrier.asked() + 1, |(first, _)| first);
        *gate = permits.map(|permits| (first, permits));
        barrier.opened.notify_all();
    }
}
