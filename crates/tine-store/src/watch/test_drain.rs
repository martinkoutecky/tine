//! The tests' acknowledged watcher drain (REVIEW-3b-P2b-2 N3): a cost test
//! measures an operation through the watcher's settlement of its own
//! writes, so it needs a boundary after which no notification of those
//! writes is pending or being reconciled. Quiet counters cannot prove that:
//! a watcher blocked on the store writer is as quiet as an idle one.
//!
//! The drain writes a sentinel file in the graph root after the writes it
//! covers. Notifications of one watch arrive in order, so when the
//! sentinel's arrives, every earlier one is already pending; the watcher
//! acknowledges it once the cycle that drained them has reconciled (the
//! store writer included). A poll-mode watcher acknowledges each full cycle
//! that began after the request. The launch's racy follow-up (§5.4) must
//! have run too.
//!
//! Test support only: `test-faults` is a dev-dependency feature, so the
//! sentinel write never reaches a shipped build, and it writes only test
//! graphs (no persisted format; `test_*.rs` is test code to the og census).
use super::*;
use std::sync::atomic::AtomicU64;
use std::sync::Condvar;

/// The sentinel's file name, in the graph root.
const SENTINEL: &str = ".tine-watch-drain";

#[derive(Default)]
pub(crate) struct Barrier {
    asked: AtomicU64,
    acked: Mutex<u64>,
    done: Condvar,
}

impl Barrier {
    /// The request a notification or cycle seen now answers.
    pub(super) fn asked(&self) -> u64 {
        self.asked.load(Ordering::SeqCst)
    }

    /// Whether a notification is the sentinel's.
    pub(super) fn sentinel(event: &notify::Result<notify::Event>) -> bool {
        event.as_ref().is_ok_and(|event| {
            event
                .paths
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == SENTINEL))
        })
    }

    /// A cycle that drained request `asked` completed.
    pub(super) fn ack(&self, asked: u64) {
        let mut acked = self.acked.lock().unwrap();
        *acked = (*acked).max(asked);
        self.done.notify_all();
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
            if fs::write(core.graph.root.join(SENTINEL), asked.to_string()).is_err() {
                return false;
            }
            let _ = self.wake.send(());
            let mut acked = barrier.acked.lock().unwrap();
            while *acked < asked {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                acked = barrier.done.wait_timeout(acked, left).unwrap().0;
            }
            drop(acked);
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
}
