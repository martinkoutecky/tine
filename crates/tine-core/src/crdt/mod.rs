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
    PageId, PageSelector, PageSnapshot,
};
