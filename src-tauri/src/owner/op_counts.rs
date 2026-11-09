//! The primitives an owner write spends, counted per graph (PC-3): turns
//! taken in the graph's line, graph views, name lookups, host flushes, page
//! reads and durable writes. Counted in test builds only; elsewhere [`count`]
//! is empty. Keyed by graph root, as the turn line is, so tests on different
//! graphs never see each other's counts. The store's `cost_counters` are one
//! process-wide set and `seen::last_changed`'s are per thread (a write runs on
//! its request thread, which a test cannot read), so neither fits here.

use std::path::Path;

/// One counted primitive.
#[derive(Clone, Copy)]
pub(super) enum Op {
    /// A place taken in the graph's line (`write::PageTurns::wait_turn`), the
    /// page's own turn or the whole graph's.
    Turn,
    /// A view of the whole graph (`verbs::whole_graph`).
    View,
    /// One name lookup in a view (`WholeGraph::resolve`) made to find the page
    /// a write lands in.
    Lookup,
    /// A flush of the host's pending input for the page.
    Flush,
    /// A read of the page through the host.
    Read,
    /// A guarded save handed to the host (one temp file, fsync, rename).
    Write,
}

/// Count one `op` against the graph at `root` (a slot's `root_key`).
#[inline]
pub(super) fn count(root: &Path, op: Op) {
    #[cfg(test)]
    tally::bump(root, op);
    #[cfg(not(test))]
    let _ = (root, op);
}

#[cfg(test)]
pub(crate) use tally::{take_counts, OpCounts};

// A test module, so the source scans of production call paths
// (`host_slot_guard_tests`) leave it out.
#[cfg(test)]
mod tally {
    use super::Op;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{LazyLock, Mutex};

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct OpCounts {
        pub(crate) turns: u64,
        pub(crate) views: u64,
        pub(crate) lookups: u64,
        pub(crate) flushes: u64,
        pub(crate) reads: u64,
        pub(crate) writes: u64,
    }

    static COUNTS: LazyLock<Mutex<HashMap<PathBuf, OpCounts>>> = LazyLock::new(Default::default);

    pub(super) fn bump(root: &Path, op: Op) {
        let mut counts = COUNTS.lock().unwrap_or_else(|e| e.into_inner());
        let c = counts.entry(root.to_path_buf()).or_default();
        *match op {
            Op::Turn => &mut c.turns,
            Op::View => &mut c.views,
            Op::Lookup => &mut c.lookups,
            Op::Flush => &mut c.flushes,
            Op::Read => &mut c.reads,
            Op::Write => &mut c.writes,
        } += 1;
    }

    /// The counts for the graph at `root` since the last call, which zeroes them.
    pub(crate) fn take_counts(root: &Path) -> OpCounts {
        COUNTS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(root)
            .unwrap_or_default()
    }
}
