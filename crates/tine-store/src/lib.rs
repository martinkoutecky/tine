//! `tine-store` — the only owner of a Logseq graph root (og batch 1).
//!
//! v0.6.5's `Graph` remains behind the store boundary. `Transaction` owns
//! guarded multi-file writes, and `Store::save` is one transaction step. The
//! store also owns observation and publication; `src-tauri` only turns a
//! subscription into window events.
//! `SHALLOW.txt` lists legacy public items awaiting later batches; the target
//! interface is `og/batches/01-step1-interface.rs`.

#[cfg(any(test, feature = "legacy-fixtures"))]
pub mod config_edit;
pub mod model;
#[cfg(any(test, feature = "legacy-fixtures"))]
pub mod onboarding;
pub mod publish;
pub mod query;
pub mod query_plan;
pub mod restore;
pub mod store;
pub mod transaction;
mod watch;
pub use publish::{PublishFailed, PublishReceipt, SiteWriter};
pub use restore::{RestoreFailed, RestoreFile, RestoreReport};
pub use store::{
    Area, Budget, Cancel, Change, ChangeKind, Closed, ConfigState, Day, FacetPolicy, FileEntry,
    FileId, FileMeta, FileRev, GraphAccessInspection, GraphRev, Inventory, InventoryEntry, Listing,
    LoadError, OpenError, OpenOptions, Origin, PageId, PageRead, QueryDialect, QueryError,
    QueryResult, Resolved, SaveBase, SaveOutcome, SearchRequest, Store, StoreError, Subscription,
    TrashKind, WatchMode, WholeGraph,
};
#[cfg(any(test, feature = "test-faults"))]
pub use transaction::FaultPoint;
pub use transaction::{
    Content, IoError, Refusal, RenameMap, Rollback, StepResult, Transaction, TxOutcome, Why,
};
