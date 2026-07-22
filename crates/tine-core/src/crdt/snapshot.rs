use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}

uuid_id!(
    /// Immutable UUID of a page in the managed-sync workspace.
    PageId
);
uuid_id!(
    /// Logseq-compatible immutable UUID of a block.
    BlockId
);

/// A complete block projection. `parent` is `None` for a page's top-level
/// blocks; `order` is unique among siblings and starts at zero.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockSnapshot {
    /// Immutable Logseq block UUID.
    pub id: BlockId,
    /// Parent block, or `None` for a top-level block on the page.
    pub parent: Option<BlockId>,
    /// Zero-based position among blocks with the same parent.
    pub order: u32,
    /// Markdown or Org source for the block.
    pub raw: String,
}

/// A complete page projection, independent of Tine's runtime graph model.
/// `kind` and `format` are deliberately strings so future Logseq-compatible
/// values can round-trip without a schema migration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageSnapshot {
    /// Immutable page UUID.
    pub id: PageId,
    /// Mutable graph-relative file path.
    pub path: String,
    /// Mutable page name.
    pub name: String,
    /// Mutable page kind, such as `page` or `journal`.
    pub kind: String,
    /// Mutable source format, such as `markdown` or `org`.
    pub format: String,
    /// Mergeable source before the page's first block. `None` is distinct from
    /// a present but empty `Some(String::new())`.
    pub pre_block: Option<String>,
    /// Flat, parent-linked block tree.
    pub blocks: Vec<BlockSnapshot>,
}

impl PageSnapshot {
    pub(crate) fn validate(&self) -> Result<(), CrdtError> {
        if self.path.is_empty() {
            return Err(CrdtError::InvalidSnapshot("page path is empty".into()));
        }

        let mut blocks = HashMap::with_capacity(self.blocks.len());
        let mut sibling_orders = HashSet::with_capacity(self.blocks.len());
        for block in &self.blocks {
            if blocks.insert(block.id, block).is_some() {
                return Err(CrdtError::DuplicateBlockId(block.id));
            }
            if !sibling_orders.insert((block.parent, block.order)) {
                return Err(CrdtError::InvalidSnapshot(format!(
                    "duplicate sibling order {} under {:?}",
                    block.order, block.parent
                )));
            }
        }

        for block in &self.blocks {
            if block.parent == Some(block.id) {
                return Err(CrdtError::InvalidSnapshot(format!(
                    "block {} is its own parent",
                    block.id
                )));
            }
            if let Some(parent) = block.parent {
                if !blocks.contains_key(&parent) {
                    return Err(CrdtError::InvalidSnapshot(format!(
                        "block {} has missing parent {}",
                        block.id, parent
                    )));
                }
            }

            let mut seen = HashSet::new();
            let mut cursor = block.parent;
            while let Some(parent) = cursor {
                if !seen.insert(parent) {
                    return Err(CrdtError::InvalidSnapshot(format!(
                        "block {} belongs to a parent cycle",
                        block.id
                    )));
                }
                cursor = blocks.get(&parent).and_then(|ancestor| ancestor.parent);
            }
        }

        let mut orders_by_parent: HashMap<Option<BlockId>, Vec<u32>> = HashMap::new();
        for block in &self.blocks {
            orders_by_parent
                .entry(block.parent)
                .or_default()
                .push(block.order);
        }
        for (parent, mut orders) in orders_by_parent {
            orders.sort_unstable();
            if orders
                .iter()
                .enumerate()
                .any(|(expected, actual)| *actual as usize != expected)
            {
                return Err(CrdtError::InvalidSnapshot(format!(
                    "sibling orders under {parent:?} must be contiguous from zero"
                )));
            }
        }

        Ok(())
    }
}

/// Selects a page for deletion by immutable ID or current path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageSelector {
    Id(PageId),
    Path(String),
}

impl From<PageId> for PageSelector {
    fn from(value: PageId) -> Self {
        Self::Id(value)
    }
}

impl From<String> for PageSelector {
    fn from(value: String) -> Self {
        Self::Path(value)
    }
}

impl From<&str> for PageSelector {
    fn from(value: &str) -> Self {
        Self::Path(value.to_owned())
    }
}

impl From<&String> for PageSelector {
    fn from(value: &String) -> Self {
        Self::Path(value.clone())
    }
}

/// One logical page touched by a chunk and the old/new projection paths that
/// may need reconciliation for that page only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffectedPage {
    pub page_id: PageId,
    pub paths: Vec<String>,
}

/// Exact local filesystem state that one explicit operation is allowed to
/// replace while its projection is being completed after a crash.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionPrecondition {
    pub path: String,
    /// `None` distinguishes an absent path from a present empty file.
    pub expected_content: Option<String>,
}

/// Result of a durably published local mutation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitReport {
    /// Whether the requested snapshot produced any CRDT operations.
    pub changed: bool,
    /// SHA-256 content ID of the immutable update chunk; empty when unchanged.
    pub chunk_id: String,
    /// Page IDs whose filesystem projections may need updating.
    pub affected_page_ids: Vec<PageId>,
    /// Old or new page paths whose projections may need updating.
    pub affected_paths: Vec<String>,
    pub affected_pages: Vec<AffectedPage>,
}

/// Result of importing newly delivered immutable chunks.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImportReport {
    /// Number of unique, previously unseen chunks imported.
    pub imported_chunks: usize,
    /// Page IDs named by the imported chunk envelopes.
    pub affected_page_ids: Vec<PageId>,
    /// Page paths named by the imported chunk envelopes.
    pub affected_paths: Vec<String>,
    pub affected_pages: Vec<AffectedPage>,
}

/// Current identity and durability status of an open CRDT workspace.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrdtStatus {
    /// Immutable workspace UUID established by genesis.
    pub workspace_id: Uuid,
    /// UUID of this device.
    pub device_id: Uuid,
    /// Fresh UUID of this process/session and its Loro peer.
    pub session_id: Uuid,
    /// Number of currently materialized pages.
    pub page_count: usize,
    /// Number of unique immutable chunks known locally.
    pub imported_chunks: usize,
    /// Path to `.tine-sync/v1`.
    pub store_root: PathBuf,
    /// Whether writes are blocked because persisted-state recovery failed.
    pub durability_blocked: bool,
}

/// Errors raised by validation, CRDT replay, and immutable storage.
#[derive(Debug)]
pub enum CrdtError {
    Io(std::io::Error),
    Loro(String),
    Serialization(String),
    InvalidSnapshot(String),
    DuplicatePageId(PageId),
    DuplicatePagePath(String),
    DuplicateBlockId(BlockId),
    PageNotFound,
    StoreNotInitialized,
    MultipleGenesis(usize),
    InvalidChunk(String),
    InvalidDocument(String),
    ChecksumMismatch,
    SchemaMismatch { expected: u32, found: u32 },
    WorkspaceMismatch { expected: Uuid, found: Uuid },
    SessionAlreadyExists(Uuid),
    DurabilityBlocked(String),
}

impl fmt::Display for CrdtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Loro(error) => write!(f, "Loro error: {error}"),
            Self::Serialization(error) => write!(f, "serialization error: {error}"),
            Self::InvalidSnapshot(error) => write!(f, "invalid page snapshot: {error}"),
            Self::DuplicatePageId(id) => write!(f, "duplicate page ID {id}"),
            Self::DuplicatePagePath(path) => write!(f, "duplicate page path {path:?}"),
            Self::DuplicateBlockId(id) => write!(f, "duplicate block ID {id}"),
            Self::PageNotFound => f.write_str("page not found"),
            Self::StoreNotInitialized => f.write_str("managed-sync store is not initialized"),
            Self::MultipleGenesis(count) => {
                write!(
                    f,
                    "managed-sync store has {count} genesis chunks; expected exactly one"
                )
            }
            Self::InvalidChunk(error) => write!(f, "invalid managed-sync chunk: {error}"),
            Self::InvalidDocument(error) => write!(f, "invalid CRDT document: {error}"),
            Self::ChecksumMismatch => f.write_str("managed-sync chunk checksum mismatch"),
            Self::SchemaMismatch { expected, found } => {
                write!(f, "schema mismatch: expected {expected}, found {found}")
            }
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::SessionAlreadyExists(id) => {
                write!(f, "session directory for {id} already exists")
            }
            Self::DurabilityBlocked(error) => {
                write!(
                    f,
                    "CRDT writes are blocked after a durability failure: {error}"
                )
            }
        }
    }
}

impl std::error::Error for CrdtError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CrdtError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub(crate) fn validate_pages(pages: &[PageSnapshot]) -> Result<(), CrdtError> {
    let mut page_ids = HashSet::with_capacity(pages.len());
    let mut paths = HashSet::with_capacity(pages.len());
    let mut block_ids = HashSet::new();
    for page in pages {
        page.validate()?;
        if !page_ids.insert(page.id) {
            return Err(CrdtError::DuplicatePageId(page.id));
        }
        if !paths.insert(&page.path) {
            return Err(CrdtError::DuplicatePagePath(page.path.clone()));
        }
        for block in &page.blocks {
            if !block_ids.insert(block.id) {
                return Err(CrdtError::DuplicateBlockId(block.id));
            }
        }
    }
    Ok(())
}
