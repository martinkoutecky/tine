//! tine-core: parsing, serialization, DTOs and pure evaluators for a
//! Logseq-compatible outliner. Pure Rust, no file I/O (og batch 1: graph files
//! belong to `tine-store`), no GUI dependencies — fully unit
//! testable without the Tauri shell.

pub mod config;
pub mod date;
pub mod doc;
pub mod edn;
pub mod guide;
pub mod html_sanitize;
pub mod logbook;
pub mod model;
pub mod org;
pub mod pdf;
pub mod reference_evidence;
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
pub use model::{BlockDto, BlockPreview, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
