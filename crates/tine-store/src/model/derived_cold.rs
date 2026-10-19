//! The cold-move ledger (BqbeQ): each `cache_gen` move made while no cache
//! is installed, with the paths whose rows it moved. A build that read the
//! files before such moves re-derives just those paths instead of being
//! discarded (`launch_patch.rs`). A move that names no paths is never
//! recorded, nor is any move made elsewhere (a discarded cache, an install):
//! the gap it leaves in the ledger makes the build decline, as every move
//! did before. A child of `derived.rs`, so the derived state stays private.

use super::*;
use std::collections::BTreeMap;

/// Moves kept before the ledger starts over (a store whose cache is never
/// installed, after a failed load); a forgotten move only costs a decline.
const KEPT_MOVES: usize = 4096;

/// Per generation a recorded move left, the paths it moved.
#[derive(Default)]
pub(super) struct ColdMoves(BTreeMap<u64, Vec<PathBuf>>);

impl Graph {
    /// Move `cache_gen` for `paths`, recorded when `cold` (no cache is
    /// installed: the caller holds the cache lock or reads it as `None`).
    /// Returns the generation before the move.
    pub(super) fn move_generation(&self, cold: bool, paths: &[&Path]) -> u64 {
        let mut ledger = self.derived.cold_moves.lock().unwrap();
        let before = self.cache_gen.fetch_add(1, Ordering::Release);
        if cold && !paths.is_empty() {
            if ledger.0.len() >= KEPT_MOVES {
                ledger.0.clear();
            }
            let paths = paths.iter().map(|path| path.to_path_buf()).collect();
            ledger.0.insert(before, paths);
        }
        before
    }

    /// The current generation and every path the moves since `from` named;
    /// None when one of those moves is not in the ledger.
    pub(crate) fn cold_moves_since(&self, from: u64) -> Option<(u64, BTreeSet<PathBuf>)> {
        let ledger = self.derived.cold_moves.lock().unwrap();
        let now = self.cache_gen.load(Ordering::Acquire);
        let mut paths = BTreeSet::new();
        for gen in from..now {
            paths.extend(ledger.0.get(&gen)?.iter().cloned());
        }
        Some((now, paths))
    }

    /// A cache was installed: the moves before it concern no build.
    pub(super) fn forget_cold_moves(&self) {
        self.derived.cold_moves.lock().unwrap().0.clear();
    }
}
