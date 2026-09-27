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
//! based on. Creates use a no-clobber rename; replacements use an ordinary
//! rename after the final revision guard. An external process can replace a
//! file between that comparison and the rename. Keep unsaved
//! edits on every refusal. [`Store::subscribe`] delivers published changes in
//! order to one consumer without replay; displacement has a typed end reason. A held
//! [`WholeGraph`] view never waits on later writers, while acquiring the first
//! view can wait for the initial parse. [`Store::close`] stops observation and releases
//! callers waiting for load. Writes, restore, publication, and graph acquisition
//! can block without a timeout; run them off a UI thread.
//! [`FileId`] and [`PageId`] are re-exports of the same types in
//! `tine_core::model`, not separate store-specific identities.
//!
//! Storage unit cost (I-25, measured 2026-09-26): a one-block edit writes one
//! page file through one temporary file, 8 bytes in the 1-block fixture; a
//! 60-block edit writes 539 bytes in the same one-file protocol. Transport is
//! one full page DTO per edit, proportional to that page's blocks and text.
//! The persisted record is the page file; there is no private
//! per-edit record. Written bytes scale with the whole serialized page, not
//! only the changed block. The I-13 counter fixture measures identical write
//! primitive counts in 20-page and 2000-page graphs. An existing-page edit
//! does not enumerate directories; creating a page can walk O(P) file-list
//! metadata for twin checks and update O(P) name-index entries. A held view
//! can also make an edit copy O(P) in-memory page pointers.
//!
//! Hostile-input contract (I-22): page and config inputs are capped at 64 MiB.
//! Pages admit at most 512 outline/list levels, 512 nested closed callouts,
//! 1,024 parsed-tree levels, and 512 matched inline delimiter levels within one
//! source line; exactly 512 outline levels are allowed. Org headlines also
//! stop at level 512, including empty and tab-separated headings. Long runs
//! of Org quote markers are refused before projection. Unmatched punctuation
//! across lines does not accumulate depth. The EDN parser has its own 128-level
//! value bound. The `tine-graph-features` export renderer flattens descendants
//! past 128 outline levels while retaining their text. An oversize
//! page returns [`StoreError::TooLarge`] on direct page read and appears in
//! [`WholeGraph::unreadable_files`]. A too-deep page is also listed unreadable,
//! and direct `page()` returns [`StoreError::Undecodable`] for its parse-validation refusal.
//! A normal outline within the bounds round-trips without byte changes.
#![deny(missing_docs)]

#[cfg(test)]
mod derived_cache_fuzz_tests;
pub mod file_kind;
#[cfg(test)]
mod gh221_malformed_html_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod issue137_investigation_tests;
#[cfg(test)]
mod legacy_graph_writer_guard_tests;
pub mod model;
pub use file_kind::{is_asset_sidecar, is_graph_text};
pub use model::{parse_input_depth_within_limit, PARSE_INPUT_MAX_BYTES};
mod atomic_file;
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
    Area, Budget, Cancel, Change, ChangeKind, ConfigState, Day, FacetPolicy, FileEntry, FileId,
    FileMeta, FileRev, GraphAccessInspection, GraphRev, Inventory, InventoryEntry, Listing,
    LoadError, OpenError, OpenOptions, Origin, PageId, PageRead, QueryDialect, QueryError,
    QueryResult, Resolved, SaveBase, SaveOutcome, SearchRequest, Store, StoreError, Subscription,
    SubscriptionEnd, TrashKind, WatchMode, WholeGraph,
};
#[cfg(any(test, feature = "test-faults"))]
pub use transaction::FaultPoint;
pub use transaction::{
    Content, IoError, Refusal, RenameMap, Rollback, StepResult, Transaction, TxOutcome, Why,
};
