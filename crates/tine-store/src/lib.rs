//! `tine-store` — the only owner of a Logseq graph root (og batch 1).
//!
//! Step 2 (mechanical move): v0.6.5's `Graph` and every module that reads or
//! writes graph files now live here; `tine-core` is pure. The public surface is
//! still v0.6.5's; `SHALLOW.txt` lists every public item that is not on the
//! target surface (`og/batches/01-step1-interface.rs`), and may only shrink.

pub mod config_edit;
pub mod model;
pub mod onboarding;
pub mod publish;
pub mod query;
pub mod query_plan;
pub mod store;
pub use store::{
    Area, Budget, Cancel, Day, FacetPolicy, FileId, FileRev, GraphRev, Inventory, InventoryEntry,
    LoadError, PageId, PageRead, QueryDialect, QueryError, QueryResult, Resolved, SearchRequest,
    Store, StoreError, TrashKind, WholeGraph,
};
