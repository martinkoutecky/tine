//! Plants in the page host's own I/O for tests outside this crate
//! (STEP3 Q-P2b-2): the next host started on the calling thread fails or
//! aborts at a phase, as the filesystem or a kill would there. Writes and
//! syncs are counted at the same seam (`cost_counters`).
pub use super::io::Phase;

/// The next host started on this thread fails `phase` `times` times with
/// `kind` (one failure per call), then succeeds.
pub fn fail(phase: Phase, kind: super::production::FaultKind, times: usize) {
    super::production::ATTACH_FAULTS.with(|faults| {
        let mut faults = faults.borrow_mut();
        let queue = faults.entry(phase).or_default();
        queue.extend(std::iter::repeat_n(kind, times));
    });
}

/// The next host started on this thread aborts the process just before
/// `phase`'s call number `n` (0-based): a kill at that boundary.
pub fn abort_before(phase: Phase, n: usize) {
    super::production::ATTACH_ABORT.with(|abort| abort.set(Some((phase, n))));
}
