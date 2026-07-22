//! Standalone managed-sync CRDT storage for a Logseq-compatible graph.
//!
//! The module intentionally has no dependency on [`crate::model`]. Callers
//! exchange plain, serializable snapshots and project materialized pages into
//! their own model.

mod graph;
mod snapshot;
mod store;

pub use graph::CrdtGraph;
pub use snapshot::{
    AffectedPage, BlockId, BlockSnapshot, CommitReport, CrdtError, CrdtStatus, ImportReport,
    PageId, PageSelector, PageSnapshot, ProjectionPrecondition,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedSyncStoreState {
    Absent,
    /// Only empty directory scaffolding exists. No device owns activation yet.
    Unclaimed,
    /// A durable genesis claim exists, but genesis publication did not finish.
    Claimed,
    Initialized,
}
