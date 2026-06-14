//! logseq-core: parsing, serialization, and the graph model for a
//! Logseq-compatible outliner. Pure Rust, no GUI dependencies — fully unit
//! testable without the Tauri shell.

pub mod config;
pub mod date;
pub mod doc;
pub mod model;

pub use config::{Config, Workflow};
pub use date::JournalDate;
pub use doc::{DocBlock, Document};
pub use model::{BlockDto, Graph, GraphMeta, PageDto, PageEntry, PageKind};
