//! tine-core: parsing, serialization, and the graph model for a
//! Logseq-compatible outliner. Pure Rust, no GUI dependencies — fully unit
//! testable without the Tauri shell.

pub mod config;
pub mod date;
pub mod doc;
pub mod edn;
pub mod html_sanitize;
pub mod logbook;
pub mod model;
pub mod onboarding;
pub mod org;
pub mod pdf;
pub mod publish;
pub mod query;
pub mod query_plan;
pub mod refs;
pub mod render;
pub mod search_query;
pub mod sync_diff;

/// Re-export the lsdoc parser so the Tauri shell can name its AST types
/// (`tine_core::lsdoc::ast::Block`) without depending on lsdoc directly.
pub use lsdoc;

pub use config::{Config, Workflow};
pub use date::JournalDate;
pub use doc::{DocBlock, Document};
pub use model::{BlockDto, Graph, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
