//! `tine-store` — the only owner of a Logseq graph root (og batch 1).
//!
//! v0.6.5's `Graph` remains behind the store boundary. `Transaction` owns
//! guarded multi-file writes, and `Store::save` is one transaction step.
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
pub mod store;
pub mod transaction;
pub use publish::{PublishFailed, PublishReceipt, SiteWriter};
pub use store::{
    Area, Budget, Cancel, ConfigState, Day, FacetPolicy, FileEntry, FileId, FileMeta, FileRev,
    GraphAccessInspection, GraphRev, Inventory, InventoryEntry, Listing, LoadError, OpenError,
    OpenOptions, PageId, PageRead, QueryDialect, QueryError, QueryResult, Resolved, SaveBase,
    SaveOutcome, SearchRequest, Store, StoreError, TrashKind, WholeGraph,
};
#[cfg(any(test, feature = "test-faults"))]
pub use transaction::FaultPoint;
pub use transaction::{
    Content, IoError, Refusal, RenameMap, Rollback, StepResult, Transaction, TxOutcome, Why,
};
