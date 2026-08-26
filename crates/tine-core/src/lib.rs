//! tine-core: parsing, serialization, and the graph model for a
//! Logseq-compatible outliner. Pure Rust, no GUI dependencies — fully unit
//! testable without the Tauri shell.

pub mod concord_ledger;
pub mod concord_queue;
pub mod config;
/// Test-only: every `Config` field either reaches `GraphMeta` (Rust and TS) or
/// says why it does not.
#[cfg(test)]
mod config_projection_parity;
pub mod date;
mod direct_projection;
pub mod doc;
pub mod edn;
pub mod fast_commit;
mod filesystem_durability;

pub mod graph_name_folding;
pub mod graph_text_scope;
pub mod html_sanitize;
pub mod logbook;
pub mod managed_storage_journey;
pub mod model;
pub mod onboarding;
pub mod oplog;
pub mod org;
mod outline;
pub mod pdf;
pub mod publish;
pub mod query;
pub mod query_plan;
mod reference_evidence;
pub mod refs;
pub mod render;
pub mod search_query;
pub mod sync_diff;
pub mod perf_count;
pub mod sync_runtime;
#[cfg(test)]
mod test_support;
pub mod text_merge;

/// Re-export the lsdoc parser so the Tauri shell can name its AST types
/// (`tine_core::lsdoc::ast::Block`) without depending on lsdoc directly.
pub use lsdoc;

pub use config::{Config, Workflow};
pub use date::JournalDate;
pub use doc::{DocBlock, Document};
pub use graph_text_scope::{
    GraphTextScope, GraphTextScopeBinding, GraphTextScopeBindingError,
    GRAPH_TEXT_SCOPE_BINDING_SCHEMA_VERSION, GRAPH_TEXT_SCOPE_VERSION,
};
pub use model::{
    ActivationIntent, BlockDto, BlockPreview, ConflictOverride, ConflictPresentation,
    EditorActivation, EditorActivationHandle, Graph, GraphMeta, LiveSaveConflictCapture, PageDto,
    PageEntry, PageKind, RefGroup, ReferencedPageNames,
};
