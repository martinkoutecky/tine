//! Open, read, observe, and safely change a Logseq graph rooted on disk.
//!
//! [`Store::open`] returns after listing graph files and starts parsing in the
//! background. [`Store::whole_graph`] waits for that first parse and returns a
//! stable view for graph-wide queries. [`Store::page`] reads one page;
//! [`Store::read`] and [`Store::open_read`] provide raw file data. These calls
//! are synchronous and should run off a UI thread.
//!
//! Use [`Store::save`] for one guarded page edit, or [`Transaction`] for a set
//! of guarded file changes. A [`FileRev`] identifies the bytes an edit was
//! based on. Guards compare current disk bytes, but an external process can
//! replace a file between that comparison and the final rename. Keep unsaved
//! edits on every refusal. [`Store::subscribe`] delivers published changes in
//! order to one consumer. [`Store::close`] stops observation and releases
//! callers waiting for load. Writes, restore, publication, and graph acquisition
//! can block without a timeout; run them off a UI thread.
//! [`FileId`] and [`PageId`] are re-exports of the same types in
//! `tine_core::model`, not separate store-specific identities.
#![deny(missing_docs)]

#[cfg(test)]
mod derived_cache_fuzz_tests;
#[cfg(test)]
mod gh221_malformed_html_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod issue137_investigation_tests;
#[cfg(test)]
mod legacy_graph_writer_guard_tests;
pub mod model;
mod no_replace;
#[cfg(test)]
mod production_index_guard_tests;
pub mod publish;
pub mod query;
pub mod query_plan;
pub mod restore;
#[cfg(test)]
mod search_edit_tests;
pub mod store;
#[cfg(test)]
mod test_config_client;
#[cfg(test)]
mod test_fixture_io;
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
