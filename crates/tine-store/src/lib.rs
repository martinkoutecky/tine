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
//!
//! Storage unit cost (I-25, measured 2026-09-26): a one-block edit writes one
//! page file through one temporary file, 8 bytes in the 1-block fixture; a
//! 60-block edit writes 539 bytes in the same one-file protocol. Transport is
//! one page DTO. The persisted record is the page file; there is no private
//! per-edit record. The I-13 counter fixture measures identical disk primitive
//! counts in 20-page and 2000-page graphs.
//!
//! Hostile-input contract (I-22): text entering a page, config, EDN parser or
//! renderer is capped at 64 MiB and 512 source nesting levels. Export rendering
//! flattens descendants past 128 outline levels while retaining their text. An oversize
//! page returns [`StoreError::TooLarge`] on direct page read and appears in
//! [`WholeGraph::unreadable_files`]. A too-deep page is also listed unreadable.
//! A normal outline within the bounds round-trips without byte changes.
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
pub use model::{parse_input_depth_within_limit, PARSE_INPUT_MAX_BYTES};
#[cfg(feature = "test-faults")]
pub mod cost_counters;
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
