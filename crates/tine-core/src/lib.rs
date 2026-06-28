//! tine-core: parsing, serialization, and the graph model for a
//! Logseq-compatible outliner. Pure Rust, no GUI dependencies — fully unit
//! testable without the Tauri shell.

pub mod config;
pub mod date;
pub mod doc;
pub mod edn;
pub mod model;
pub mod onboarding;
pub mod org;
pub mod pdf;
pub mod publish;
pub mod query;
pub mod refs;

pub use config::{Config, Workflow};
pub use date::JournalDate;
pub use doc::{DocBlock, Document};
pub use model::{BlockDto, Graph, GraphMeta, PageDto, PageEntry, PageKind, RefGroup};
