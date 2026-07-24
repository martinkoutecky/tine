//! Complete, disposable graph-wide materialization behind the SQLite frontier.
//!
//! The types in this module are an adapter boundary, not a second authority.
//! An accepted semantic effect does not currently contain parser-derived names,
//! references, properties, tags, task facets, formatting facets, or searchable
//! text.  Callers must therefore provide those values explicitly from an
//! authoritative post-acceptance snapshot.  The SQLite applier validates the
//! input against the accepted semantic effect and applies it in the same SQL
//! transaction that advances the accepted frontier.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusqlite::{params, Connection, OptionalExtension as _, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    AcceptedBatchEvent, BatchId, BlockId, BlockOwner, ContentDigest, DocumentId,
    LogseqIdentityOrigin, LogseqUuid, ManagedPath, ManagedTextKind, PageId, PageState,
    PolicyGeneratedAnchorReason, SemanticEffect,
};

pub const MAX_MATERIALIZATION_QUERY_ROWS: usize = 10_000;
/// Largest accepted materialization string other than a page preamble.
///
/// This retains the established semantic block-content capacity while keeping
/// individual SQLite/FTS values bounded.
pub const MAX_MATERIALIZATION_FIELD_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MATERIALIZATION_PREAMBLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MATERIALIZATION_FACET_VALUES: usize = 16_384;
pub const MAX_MATERIALIZATION_FACET_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MATERIALIZATION_CHANGE_PAGES: usize = 65_536;
pub const MAX_MATERIALIZATION_CHANGE_BLOCKS: usize = 262_144;
pub const MAX_MATERIALIZATION_CHANGE_FACET_VALUES: usize = 1_048_576;
pub const MAX_MATERIALIZATION_CHANGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_MATERIALIZATION_QUERY_BYTES: usize = 64 * 1024;
pub const MAX_MATERIALIZATION_READ_BYTES: usize = 64 * 1024 * 1024;

const MATERIALIZATION_PAGE_OVERHEAD_BYTES: usize = 96;
const MATERIALIZATION_BLOCK_OVERHEAD_BYTES: usize = 128;
const MATERIALIZATION_REFERENCE_OVERHEAD_BYTES: usize = 48;
const MATERIALIZATION_PROPERTY_OVERHEAD_BYTES: usize = 24;
const MATERIALIZATION_TAG_OVERHEAD_BYTES: usize = 16;
const MATERIALIZATION_STRING_OVERHEAD_BYTES: usize = 16;
const MATERIALIZATION_INPUT_SCHEMA_VERSION: u32 = 2;

pub(crate) const MATERIALIZATION_STAMP_DDL: &str = "CREATE TABLE materialization_stamp (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    acceptance_sequence INTEGER NOT NULL CHECK (acceptance_sequence >= 0),
    frontier_root_digest BLOB NOT NULL CHECK (length(frontier_root_digest) = 32)
) STRICT";
pub(crate) const MATERIALIZATION_BATCHES_DDL: &str = "CREATE TABLE materialization_batches (
    acceptance_sequence INTEGER PRIMARY KEY CHECK (acceptance_sequence > 0),
    batch_id BLOB NOT NULL UNIQUE CHECK (length(batch_id) = 16),
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32)
) STRICT";
pub(crate) const PAGES_DDL: &str = "CREATE TABLE pages (
    page_id BLOB PRIMARY KEY CHECK (length(page_id) = 16),
    home_document_id BLOB NOT NULL CHECK (length(home_document_id) = 16),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 4194304),
    name_key TEXT NOT NULL CHECK (length(CAST(name_key AS BLOB)) BETWEEN 1 AND 4194304),
    path TEXT NOT NULL CHECK (length(CAST(path AS BLOB)) BETWEEN 1 AND 4194304),
    text_kind INTEGER NOT NULL CHECK (text_kind IN (0, 1)),
    preamble TEXT CHECK (preamble IS NULL OR length(CAST(preamble AS BLOB)) <= 16777216),
    searchable_text TEXT NOT NULL CHECK (length(CAST(searchable_text AS BLOB)) <= 4194304)
) STRICT";
pub(crate) const BLOCKS_DDL: &str = "CREATE TABLE blocks (
    block_id BLOB PRIMARY KEY CHECK (length(block_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16)
        REFERENCES pages(page_id) ON DELETE CASCADE,
    home_document_id BLOB NOT NULL CHECK (length(home_document_id) = 16),
    parent_block_id BLOB CHECK (
        parent_block_id IS NULL OR length(parent_block_id) = 16
    ),
    order_key TEXT NOT NULL CHECK (length(CAST(order_key AS BLOB)) BETWEEN 1 AND 4194304),
    content TEXT NOT NULL CHECK (length(CAST(content AS BLOB)) <= 4194304),
    searchable_text TEXT NOT NULL CHECK (length(CAST(searchable_text AS BLOB)) <= 4194304),
    heading_level INTEGER CHECK (
        heading_level IS NULL OR heading_level BETWEEN 1 AND 6
    ),
    collapsed INTEGER NOT NULL CHECK (collapsed IN (0, 1)),
    logseq_uuid BLOB CHECK (logseq_uuid IS NULL OR length(logseq_uuid) = 16),
    logseq_identity_origin INTEGER CHECK (
        logseq_identity_origin IS NULL
        OR logseq_identity_origin BETWEEN 0 AND 4
    ),
    CHECK (
        (logseq_uuid IS NULL AND logseq_identity_origin IS NULL)
        OR (logseq_uuid IS NOT NULL AND logseq_identity_origin IS NOT NULL)
    )
) STRICT";
pub(crate) const REFERENCES_DDL: &str = "CREATE TABLE refs (
    source_type INTEGER NOT NULL CHECK (source_type IN (0, 1)),
    source_id BLOB NOT NULL CHECK (length(source_id) = 16),
    source_page_id BLOB NOT NULL CHECK (length(source_page_id) = 16),
    target_type INTEGER NOT NULL CHECK (target_type IN (0, 1)),
    target_id BLOB NOT NULL CHECK (length(target_id) = 16),
    reference_kind INTEGER NOT NULL CHECK (reference_kind BETWEEN 0 AND 3),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (source_type, source_id, target_type, target_id, reference_kind, ordinal)
) WITHOUT ROWID, STRICT";
pub(crate) const PROPERTIES_DDL: &str = "CREATE TABLE properties (
    owner_type INTEGER NOT NULL CHECK (owner_type IN (0, 1)),
    owner_id BLOB NOT NULL CHECK (length(owner_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 4194304),
    value TEXT NOT NULL CHECK (length(CAST(value AS BLOB)) <= 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (owner_type, owner_id, name, ordinal)
) WITHOUT ROWID, STRICT";
pub(crate) const TAGS_DDL: &str = "CREATE TABLE tags (
    owner_type INTEGER NOT NULL CHECK (owner_type IN (0, 1)),
    owner_id BLOB NOT NULL CHECK (length(owner_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    tag TEXT NOT NULL CHECK (length(CAST(tag AS BLOB)) BETWEEN 1 AND 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (owner_type, owner_id, ordinal)
) WITHOUT ROWID, STRICT";
pub(crate) const TASKS_DDL: &str = "CREATE TABLE tasks (
    block_id BLOB PRIMARY KEY CHECK (length(block_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    marker TEXT NOT NULL CHECK (length(CAST(marker AS BLOB)) BETWEEN 1 AND 4194304),
    priority TEXT CHECK (priority IS NULL OR length(CAST(priority AS BLOB)) <= 4194304),
    scheduled TEXT CHECK (scheduled IS NULL OR length(CAST(scheduled AS BLOB)) <= 4194304),
    deadline TEXT CHECK (deadline IS NULL OR length(CAST(deadline AS BLOB)) <= 4194304)
) STRICT";
pub(crate) const SEARCH_FTS_DDL: &str = "CREATE VIRTUAL TABLE search_fts USING fts5(
    entity_type UNINDEXED,
    entity_id UNINDEXED,
    page_id UNINDEXED,
    text,
    tokenize = 'unicode61 remove_diacritics 0'
)";

pub(crate) const PAGES_NAME_INDEX_DDL: &str = "CREATE INDEX pages_name_idx ON pages(name, page_id)";
pub(crate) const PAGES_NAME_KEY_INDEX_DDL: &str =
    "CREATE INDEX pages_name_key_idx ON pages(name_key, page_id)";
pub(crate) const PAGES_PATH_INDEX_DDL: &str = "CREATE INDEX pages_path_idx ON pages(path, page_id)";
pub(crate) const BLOCKS_PAGE_ORDER_INDEX_DDL: &str =
    "CREATE INDEX blocks_page_order_idx ON blocks(page_id, order_key, block_id)";
pub(crate) const REFERENCES_TARGET_INDEX_DDL: &str = "CREATE INDEX references_target_idx
    ON refs(target_type, target_id, source_page_id, source_type, source_id)";
pub(crate) const REFERENCES_SOURCE_INDEX_DDL: &str = "CREATE INDEX references_source_idx
    ON refs(source_page_id, source_type, source_id)";
pub(crate) const PROPERTIES_LOOKUP_INDEX_DDL: &str = "CREATE INDEX properties_lookup_idx
    ON properties(name, value, page_id, owner_type, owner_id)";
pub(crate) const TAGS_LOOKUP_INDEX_DDL: &str =
    "CREATE INDEX tags_lookup_idx ON tags(tag, page_id, owner_type, owner_id)";
pub(crate) const TASKS_MARKER_INDEX_DDL: &str =
    "CREATE INDEX tasks_marker_idx ON tasks(marker, page_id, block_id)";
pub(crate) const TASKS_DEADLINE_INDEX_DDL: &str =
    "CREATE INDEX tasks_deadline_idx ON tasks(deadline, scheduled, page_id, block_id)";

const MATERIALIZATION_TABLE_COLUMNS: [(&str, &[&str]); 8] = [
    (
        "materialization_stamp",
        &["singleton", "acceptance_sequence", "frontier_root_digest"],
    ),
    (
        "materialization_batches",
        &["acceptance_sequence", "batch_id", "input_digest"],
    ),
    (
        "pages",
        &[
            "page_id",
            "home_document_id",
            "name",
            "name_key",
            "path",
            "text_kind",
            "preamble",
            "searchable_text",
        ],
    ),
    (
        "blocks",
        &[
            "block_id",
            "page_id",
            "home_document_id",
            "parent_block_id",
            "order_key",
            "content",
            "searchable_text",
            "heading_level",
            "collapsed",
            "logseq_uuid",
            "logseq_identity_origin",
        ],
    ),
    (
        "refs",
        &[
            "source_type",
            "source_id",
            "source_page_id",
            "target_type",
            "target_id",
            "reference_kind",
            "ordinal",
        ],
    ),
    (
        "properties",
        &[
            "owner_type",
            "owner_id",
            "page_id",
            "name",
            "value",
            "ordinal",
        ],
    ),
    (
        "tags",
        &["owner_type", "owner_id", "page_id", "tag", "ordinal"],
    ),
    (
        "tasks",
        &[
            "block_id",
            "page_id",
            "marker",
            "priority",
            "scheduled",
            "deadline",
        ],
    ),
];

const MATERIALIZATION_SCHEMA_OBJECTS: [(&str, &str, &str); 18] = [
    ("table", "materialization_stamp", MATERIALIZATION_STAMP_DDL),
    (
        "table",
        "materialization_batches",
        MATERIALIZATION_BATCHES_DDL,
    ),
    ("table", "pages", PAGES_DDL),
    ("table", "blocks", BLOCKS_DDL),
    ("table", "refs", REFERENCES_DDL),
    ("table", "properties", PROPERTIES_DDL),
    ("table", "tags", TAGS_DDL),
    ("table", "tasks", TASKS_DDL),
    ("table", "search_fts", SEARCH_FTS_DDL),
    ("index", "pages_name_idx", PAGES_NAME_INDEX_DDL),
    ("index", "pages_name_key_idx", PAGES_NAME_KEY_INDEX_DDL),
    ("index", "pages_path_idx", PAGES_PATH_INDEX_DDL),
    (
        "index",
        "blocks_page_order_idx",
        BLOCKS_PAGE_ORDER_INDEX_DDL,
    ),
    (
        "index",
        "references_target_idx",
        REFERENCES_TARGET_INDEX_DDL,
    ),
    (
        "index",
        "references_source_idx",
        REFERENCES_SOURCE_INDEX_DDL,
    ),
    (
        "index",
        "properties_lookup_idx",
        PROPERTIES_LOOKUP_INDEX_DDL,
    ),
    ("index", "tags_lookup_idx", TAGS_LOOKUP_INDEX_DDL),
    ("index", "tasks_marker_idx", TASKS_MARKER_INDEX_DDL),
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedEntityId {
    Page(PageId),
    Block(BlockId),
}

impl MaterializedEntityId {
    fn sql_parts(self) -> (i64, [u8; 16]) {
        match self {
            Self::Page(id) => (0, id.as_uuid().into_bytes()),
            Self::Block(id) => (1, id.as_uuid().into_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializedReferenceKind {
    Reference,
    Embed,
    TagReference,
    PropertyReference,
}

impl MaterializedReferenceKind {
    const fn sql_value(self) -> i64 {
        match self {
            Self::Reference => 0,
            Self::Embed => 1,
            Self::TagReference => 2,
            Self::PropertyReference => 3,
        }
    }

    fn from_sql(value: i64) -> Result<Self, MaterializationError> {
        match value {
            0 => Ok(Self::Reference),
            1 => Ok(Self::Embed),
            2 => Ok(Self::TagReference),
            3 => Ok(Self::PropertyReference),
            _ => Err(MaterializationError::Corrupt(format!(
                "unknown reference kind {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedReference {
    pub target: MaterializedEntityId,
    pub kind: MaterializedReferenceKind,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedProperty {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedTask {
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedBlockInput {
    pub block_id: BlockId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
    pub references: Vec<MaterializedReference>,
    pub properties: Vec<MaterializedProperty>,
    pub tags: Vec<String>,
    pub task: Option<MaterializedTask>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedPageInput {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub name: String,
    pub name_key: String,
    pub path: ManagedPath,
    pub kind: ManagedTextKind,
    pub preamble: Option<String>,
    pub searchable_text: String,
    pub references: Vec<MaterializedReference>,
    pub properties: Vec<MaterializedProperty>,
    pub tags: Vec<String>,
    pub blocks: Vec<MaterializedBlockInput>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializationChange {
    schema_version: u32,
    batch_id: BatchId,
    replacements: Vec<MaterializedPageInput>,
    deletions: Vec<PageId>,
}

impl MaterializationChange {
    pub fn new(
        batch_id: BatchId,
        mut replacements: Vec<MaterializedPageInput>,
        mut deletions: Vec<PageId>,
    ) -> Result<Self, MaterializationError> {
        let mut input_budget = MaterializationInputBudget::default();
        input_budget.add_pages(replacements.len())?;
        input_budget.add_pages(deletions.len())?;
        replacements.sort_unstable_by_key(|page| page.page_id);
        deletions.sort_unstable();
        let change = Self {
            schema_version: MATERIALIZATION_INPUT_SCHEMA_VERSION,
            batch_id,
            replacements,
            deletions,
        };
        change.validate_shape()?;
        Ok(change)
    }

    pub const fn batch_id(&self) -> BatchId {
        self.batch_id
    }

    pub fn replacements(&self) -> &[MaterializedPageInput] {
        &self.replacements
    }

    pub fn deletions(&self) -> &[PageId] {
        &self.deletions
    }

    pub fn digest(&self) -> Result<ContentDigest, MaterializationError> {
        self.validate_shape()?;
        let encoded = postcard::to_allocvec(self)
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        if encoded.len() > MAX_MATERIALIZATION_CHANGE_BYTES {
            return Err(resource_limit(
                "materialization change bytes",
                encoded.len(),
                MAX_MATERIALIZATION_CHANGE_BYTES,
            ));
        }
        Ok(ContentDigest::of(&encoded))
    }

    pub(crate) fn validate_for_event(
        &self,
        event: &AcceptedBatchEvent,
    ) -> Result<ContentDigest, MaterializationError> {
        if self.batch_id != event.batch_id() {
            return Err(MaterializationError::BatchMismatch {
                expected: event.batch_id(),
                found: self.batch_id,
            });
        }
        let effect = SemanticEffect::decode(event.semantic_effect())
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        self.validate_against_effect(&effect)?;
        self.digest()
    }

    pub(crate) fn validate_against_stored(
        &self,
        batch_id: BatchId,
        semantic_effect: &[u8],
    ) -> Result<ContentDigest, MaterializationError> {
        if self.batch_id != batch_id {
            return Err(MaterializationError::BatchMismatch {
                expected: batch_id,
                found: self.batch_id,
            });
        }
        let effect = SemanticEffect::decode(semantic_effect)
            .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
        self.validate_against_effect(&effect)?;
        self.digest()
    }

    fn validate_shape(&self) -> Result<(), MaterializationError> {
        if self.schema_version != MATERIALIZATION_INPUT_SCHEMA_VERSION {
            return Err(MaterializationError::InvalidInput(format!(
                "unknown materialization input schema {}",
                self.schema_version
            )));
        }
        if !strictly_sorted_unique_by(&self.replacements, |page| page.page_id)
            || !strictly_sorted_unique_by(&self.deletions, |page_id| *page_id)
        {
            return Err(MaterializationError::InvalidInput(
                "page replacements/deletions are not canonical".into(),
            ));
        }
        let mut input_budget = MaterializationInputBudget::default();
        input_budget.add_pages(self.replacements.len())?;
        input_budget.add_pages(self.deletions.len())?;
        input_budget.add_bytes(
            self.deletions
                .len()
                .checked_mul(32)
                .ok_or_else(|| {
                    resource_limit(
                        "materialization change bytes",
                        usize::MAX,
                        MAX_MATERIALIZATION_CHANGE_BYTES,
                    )
                })?,
        )?;
        let replacement_ids = self
            .replacements
            .iter()
            .map(|page| page.page_id)
            .collect::<BTreeSet<_>>();
        if self
            .deletions
            .iter()
            .any(|page_id| replacement_ids.contains(page_id))
        {
            return Err(MaterializationError::InvalidInput(
                "one page is both replaced and deleted".into(),
            ));
        }
        let mut block_ids = BTreeSet::new();
        for page in &self.replacements {
            validate_page(page, &mut input_budget)?;
            for block in &page.blocks {
                if !block_ids.insert(block.block_id) {
                    return Err(MaterializationError::InvalidInput(format!(
                        "block {} occurs in multiple replacement pages",
                        block.block_id
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_against_effect(&self, effect: &SemanticEffect) -> Result<(), MaterializationError> {
        self.validate_shape()?;
        let replacements = self
            .replacements
            .iter()
            .map(|page| (page.page_id, page))
            .collect::<BTreeMap<_, _>>();
        let deletions = self.deletions.iter().copied().collect::<BTreeSet<_>>();
        let mut affected = BTreeSet::new();
        let mut required_deletions = BTreeSet::new();

        for delta in effect.pages() {
            affected.insert(delta.page_id);
            match delta.after.as_ref() {
                Some(PageState::Live {
                    name,
                    path,
                    home_document_id,
                    kind,
                }) => {
                    let page = replacements.get(&delta.page_id).ok_or_else(|| {
                        MaterializationError::Incomplete(format!(
                            "live page {} has no complete replacement",
                            delta.page_id
                        ))
                    })?;
                    let expected_name_key = name.canonical_key();
                    if page.name.as_str() != name.as_str()
                        || page.name_key.as_str() != expected_name_key.as_str()
                        || &page.path != path
                        || page.home_document_id != *home_document_id
                        || page.kind != *kind
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "page {} replacement differs from accepted name/key/path/kind/home",
                            delta.page_id
                        )));
                    }
                }
                Some(PageState::Tombstone { .. }) => {
                    required_deletions.insert(delta.page_id);
                }
                None => {
                    return Err(MaterializationError::Incomplete(format!(
                        "accepted page {} has no post-state",
                        delta.page_id
                    )));
                }
            }
        }
        for delta in effect.page_preambles() {
            affected.insert(delta.page_id);
            let page = replacements.get(&delta.page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "preamble change for page {} has no replacement",
                    delta.page_id
                ))
            })?;
            let after = delta.after.as_ref().ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "preamble change for page {} has no post-state",
                    delta.page_id
                ))
            })?;
            if page.home_document_id != after.home_document_id || page.preamble != after.preamble {
                return Err(MaterializationError::Contradiction(format!(
                    "page {} replacement differs from accepted preamble",
                    delta.page_id
                )));
            }
        }
        for delta in effect.memberships() {
            affected.insert(delta.page_id);
            if required_deletions.contains(&delta.page_id) {
                continue;
            }
            let page = replacements.get(&delta.page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "membership change for page {} has no replacement",
                    delta.page_id
                ))
            })?;
            match delta.after.as_ref() {
                Some(after) => {
                    let block = page
                        .blocks
                        .iter()
                        .find(|block| block.block_id == delta.block_id)
                        .ok_or_else(|| {
                            MaterializationError::Contradiction(format!(
                                "accepted member {} is absent from page {}",
                                delta.block_id, delta.page_id
                            ))
                        })?;
                    if block.home_document_id != after.home_document_id
                        || block.parent != after.parent
                        || block.order != after.order
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "member {} differs from accepted parent/order/home",
                            delta.block_id
                        )));
                    }
                }
                None if page
                    .blocks
                    .iter()
                    .any(|block| block.block_id == delta.block_id) =>
                {
                    return Err(MaterializationError::Contradiction(format!(
                        "removed member {} remains on page {}",
                        delta.block_id, delta.page_id
                    )));
                }
                None => {}
            }
        }
        for delta in effect.blocks() {
            let owner = delta
                .after
                .as_ref()
                .and_then(block_owner_page)
                .or_else(|| delta.before.as_ref().and_then(block_owner_page));
            let Some(page_id) = owner else {
                continue;
            };
            affected.insert(page_id);
            if required_deletions.contains(&page_id) {
                continue;
            }
            let page = replacements.get(&page_id).ok_or_else(|| {
                MaterializationError::Incomplete(format!(
                    "block change for page {page_id} has no replacement"
                ))
            })?;
            match delta.after.as_ref() {
                Some(after) if matches!(after.owner, BlockOwner::Page(owner) if owner == page_id) =>
                {
                    let block = page
                        .blocks
                        .iter()
                        .find(|block| block.block_id == delta.block_id)
                        .ok_or_else(|| {
                            MaterializationError::Contradiction(format!(
                                "accepted live block {} is absent from page {page_id}",
                                delta.block_id
                            ))
                        })?;
                    if block.home_document_id != after.home_document_id
                        || block.content != after.content
                        || block.logseq_uuid != after.logseq_uuid
                        || block.logseq_identity_origin != after.logseq_identity_origin
                    {
                        return Err(MaterializationError::Contradiction(format!(
                            "block {} replacement differs from accepted state",
                            delta.block_id
                        )));
                    }
                }
                Some(_) | None
                    if page
                        .blocks
                        .iter()
                        .any(|block| block.block_id == delta.block_id) =>
                {
                    return Err(MaterializationError::Contradiction(format!(
                        "non-live block {} remains on page {page_id}",
                        delta.block_id
                    )));
                }
                Some(_) | None => {}
            }
        }

        let supplied = replacements
            .keys()
            .copied()
            .chain(deletions.iter().copied())
            .collect::<BTreeSet<_>>();
        if supplied != affected {
            return Err(MaterializationError::Incomplete(format!(
                "supplied pages {supplied:?} differ from accepted affected pages {affected:?}"
            )));
        }
        if deletions != required_deletions {
            return Err(MaterializationError::Contradiction(format!(
                "supplied deletions {deletions:?} differ from accepted deletions {required_deletions:?}"
            )));
        }
        Ok(())
    }

    /// Preserves page identity metadata when the accepted effect did not
    /// authoritatively replace it. The existing row is only a previously
    /// validated value to preserve; it does not become semantic authority.
    fn validate_preserved_page_metadata(
        &self,
        transaction: &Transaction<'_>,
        effect: &SemanticEffect,
    ) -> Result<(), MaterializationError> {
        let pages_with_live_delta = effect
            .pages()
            .iter()
            .filter(|delta| matches!(delta.after.as_ref(), Some(PageState::Live { .. })))
            .map(|delta| delta.page_id)
            .collect::<BTreeSet<_>>();

        for page in &self.replacements {
            if pages_with_live_delta.contains(&page.page_id) {
                continue;
            }
            let metadata_matches: Option<bool> = transaction
                .query_row(
                    "SELECT home_document_id = ?2
                              AND name = ?3
                              AND name_key = ?4
                              AND path = ?5
                              AND text_kind = ?6
                       FROM pages
                       WHERE page_id = ?1",
                    params![
                        page.page_id.as_uuid().as_bytes().as_slice(),
                        page.home_document_id.as_uuid().as_bytes().as_slice(),
                        &page.name,
                        &page.name_key,
                        page.path.as_str(),
                        text_kind_to_sql(page.kind),
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            match metadata_matches {
                Some(true) => {}
                Some(false) => {
                    return Err(MaterializationError::Contradiction(format!(
                        "page {} replacement changes metadata without an accepted live page delta",
                        page.page_id
                    )));
                }
                None => {
                    return Err(MaterializationError::Incomplete(format!(
                        "page {} replacement lacks prior validated metadata",
                        page.page_id
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct MaterializationInputBudget {
    bytes: usize,
    pages: usize,
    blocks: usize,
    facet_values: usize,
}

impl MaterializationInputBudget {
    fn add_bytes(&mut self, bytes: usize) -> Result<(), MaterializationError> {
        self.bytes = checked_budget_add(
            "materialization change bytes",
            self.bytes,
            bytes,
            MAX_MATERIALIZATION_CHANGE_BYTES,
        )?;
        Ok(())
    }

    fn add_pages(&mut self, pages: usize) -> Result<(), MaterializationError> {
        self.pages = checked_budget_add(
            "materialization change pages",
            self.pages,
            pages,
            MAX_MATERIALIZATION_CHANGE_PAGES,
        )?;
        Ok(())
    }

    fn add_blocks(&mut self, blocks: usize) -> Result<(), MaterializationError> {
        self.blocks = checked_budget_add(
            "materialization change blocks",
            self.blocks,
            blocks,
            MAX_MATERIALIZATION_CHANGE_BLOCKS,
        )?;
        Ok(())
    }

    fn add_facet_values(&mut self, values: usize) -> Result<(), MaterializationError> {
        self.facet_values = checked_budget_add(
            "materialization change facet values",
            self.facet_values,
            values,
            MAX_MATERIALIZATION_CHANGE_FACET_VALUES,
        )?;
        Ok(())
    }

    fn add_field(
        &mut self,
        resource: &'static str,
        value: &str,
        maximum: usize,
    ) -> Result<(), MaterializationError> {
        if value.len() > maximum {
            return Err(resource_limit(resource, value.len(), maximum));
        }
        self.add_bytes(value.len())?;
        self.add_bytes(MATERIALIZATION_STRING_OVERHEAD_BYTES)
    }
}

fn checked_budget_add(
    resource: &'static str,
    current: usize,
    added: usize,
    maximum: usize,
) -> Result<usize, MaterializationError> {
    let found = current
        .checked_add(added)
        .ok_or_else(|| resource_limit(resource, usize::MAX, maximum))?;
    if found > maximum {
        return Err(resource_limit(resource, found, maximum));
    }
    Ok(found)
}

fn resource_limit(resource: &'static str, found: usize, maximum: usize) -> MaterializationError {
    MaterializationError::ResourceLimit {
        resource,
        found,
        maximum,
    }
}

fn block_owner_page(state: &super::BlockState) -> Option<PageId> {
    match state.owner {
        BlockOwner::Page(page_id) => Some(page_id),
        BlockOwner::Tombstone => None,
    }
}

fn validate_page(
    page: &MaterializedPageInput,
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    input_budget.add_bytes(MATERIALIZATION_PAGE_OVERHEAD_BYTES)?;
    input_budget.add_field("page name bytes", &page.name, MAX_MATERIALIZATION_FIELD_BYTES)?;
    input_budget.add_field(
        "page name key bytes",
        &page.name_key,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    input_budget.add_field(
        "page path bytes",
        page.path.as_str(),
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    if let Some(preamble) = &page.preamble {
        input_budget.add_field(
            "page preamble bytes",
            preamble,
            MAX_MATERIALIZATION_PREAMBLE_BYTES,
        )?;
    }
    input_budget.add_field(
        "page searchable text bytes",
        &page.searchable_text,
        MAX_MATERIALIZATION_FIELD_BYTES,
    )?;
    if page.name.is_empty() || page.name_key.is_empty() {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} has an empty name/name key",
            page.page_id
        )));
    }
    validate_references(&page.references, input_budget)?;
    validate_properties(&page.properties, input_budget)?;
    validate_tags(&page.tags, input_budget)?;
    input_budget.add_blocks(page.blocks.len())?;
    let block_ids = page
        .blocks
        .iter()
        .map(|block| block.block_id)
        .collect::<BTreeSet<_>>();
    if block_ids.len() != page.blocks.len() {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} contains duplicate block identities",
            page.page_id
        )));
    }
    if !page
        .blocks
        .windows(2)
        .all(|pair| (&pair[0].order, pair[0].block_id) < (&pair[1].order, pair[1].block_id))
    {
        return Err(MaterializationError::InvalidInput(format!(
            "page {} blocks are not in canonical order",
            page.page_id
        )));
    }
    for block in &page.blocks {
        input_budget.add_bytes(MATERIALIZATION_BLOCK_OVERHEAD_BYTES)?;
        input_budget.add_field(
            "block order bytes",
            &block.order,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "block content bytes",
            &block.content,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "block searchable text bytes",
            &block.searchable_text,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        if block.order.is_empty() {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has an empty order key",
                block.block_id
            )));
        }
        if block
            .heading_level
            .is_some_and(|level| !(1..=6).contains(&level))
        {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has an invalid heading level",
                block.block_id
            )));
        }
        if block.logseq_uuid.is_some() != block.logseq_identity_origin.is_some() {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has incomplete Logseq identity metadata",
                block.block_id
            )));
        }
        if block
            .parent
            .is_some_and(|parent| !block_ids.contains(&parent))
        {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has a parent outside page {}",
                block.block_id, page.page_id
            )));
        }
        validate_references(&block.references, input_budget)?;
        validate_properties(&block.properties, input_budget)?;
        validate_tags(&block.tags, input_budget)?;
        if let Some(task) = &block.task {
            input_budget.add_field(
                "task marker bytes",
                &task.marker,
                MAX_MATERIALIZATION_FIELD_BYTES,
            )?;
            for (resource, value) in [
                ("task priority bytes", task.priority.as_deref()),
                ("task scheduled bytes", task.scheduled.as_deref()),
                ("task deadline bytes", task.deadline.as_deref()),
            ] {
                if let Some(value) = value {
                    input_budget.add_field(resource, value, MAX_MATERIALIZATION_FIELD_BYTES)?;
                }
            }
            if task.marker.is_empty() {
                return Err(MaterializationError::InvalidInput(format!(
                    "block {} has an empty task marker",
                    block.block_id
                )));
            }
        }
    }
    Ok(())
}

fn validate_references(
    references: &[MaterializedReference],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = references
        .len()
        .checked_mul(MATERIALIZATION_REFERENCE_OVERHEAD_BYTES)
        .ok_or_else(|| {
            resource_limit(
                "reference facet bytes",
                usize::MAX,
                MAX_MATERIALIZATION_FACET_BYTES,
            )
        })?;
    validate_facet("reference", references.len(), bytes)?;
    input_budget.add_facet_values(references.len())?;
    input_budget.add_bytes(bytes)
}

fn validate_properties(
    properties: &[MaterializedProperty],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = properties.iter().try_fold(0_usize, |total, property| {
        total
            .checked_add(property.name.len())
            .and_then(|total| total.checked_add(property.value.len()))
            .and_then(|total| total.checked_add(MATERIALIZATION_PROPERTY_OVERHEAD_BYTES))
    });
    let bytes = bytes.ok_or_else(|| {
        resource_limit(
            "property facet bytes",
            usize::MAX,
            MAX_MATERIALIZATION_FACET_BYTES,
        )
    })?;
    validate_facet("property", properties.len(), bytes)?;
    if properties.iter().any(|property| property.name.is_empty()) {
        return Err(MaterializationError::InvalidInput(
            "property names must be non-empty".into(),
        ));
    }
    input_budget.add_facet_values(properties.len())?;
    for property in properties {
        input_budget.add_bytes(MATERIALIZATION_PROPERTY_OVERHEAD_BYTES)?;
        input_budget.add_field(
            "property name bytes",
            &property.name,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
        input_budget.add_field(
            "property value bytes",
            &property.value,
            MAX_MATERIALIZATION_FIELD_BYTES,
        )?;
    }
    Ok(())
}

fn validate_tags(
    tags: &[String],
    input_budget: &mut MaterializationInputBudget,
) -> Result<(), MaterializationError> {
    let bytes = tags.iter().try_fold(0_usize, |total, tag| {
        total
            .checked_add(tag.len())
            .and_then(|total| total.checked_add(MATERIALIZATION_TAG_OVERHEAD_BYTES))
    });
    let bytes = bytes.ok_or_else(|| {
        resource_limit(
            "tag facet bytes",
            usize::MAX,
            MAX_MATERIALIZATION_FACET_BYTES,
        )
    })?;
    validate_facet("tag", tags.len(), bytes)?;
    if tags.iter().any(String::is_empty) {
        return Err(MaterializationError::InvalidInput(
            "tags must be non-empty".into(),
        ));
    }
    input_budget.add_facet_values(tags.len())?;
    for tag in tags {
        input_budget.add_bytes(MATERIALIZATION_TAG_OVERHEAD_BYTES)?;
        input_budget.add_field("tag bytes", tag, MAX_MATERIALIZATION_FIELD_BYTES)?;
    }
    Ok(())
}

fn validate_facet(
    facet: &'static str,
    values: usize,
    bytes: usize,
) -> Result<(), MaterializationError> {
    if values > MAX_MATERIALIZATION_FACET_VALUES {
        return Err(resource_limit(
            match facet {
                "reference" => "reference facet values",
                "property" => "property facet values",
                "tag" => "tag facet values",
                _ => "materialization facet values",
            },
            values,
            MAX_MATERIALIZATION_FACET_VALUES,
        ));
    }
    if bytes > MAX_MATERIALIZATION_FACET_BYTES {
        return Err(resource_limit(
            match facet {
                "reference" => "reference facet bytes",
                "property" => "property facet bytes",
                "tag" => "tag facet bytes",
                _ => "materialization facet bytes",
            },
            bytes,
            MAX_MATERIALIZATION_FACET_BYTES,
        ));
    }
    Ok(())
}

fn strictly_sorted_unique_by<T, K: Ord>(values: &[T], key: impl Fn(&T) -> K) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}

pub(crate) fn initialize_schema(
    connection: &Connection,
    empty_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    connection.execute_batch(&format!(
        "{MATERIALIZATION_STAMP_DDL};
         {MATERIALIZATION_BATCHES_DDL};
         {PAGES_DDL};
         {BLOCKS_DDL};
         {REFERENCES_DDL};
         {PROPERTIES_DDL};
         {TAGS_DDL};
         {TASKS_DDL};
         {SEARCH_FTS_DDL};
         {PAGES_NAME_INDEX_DDL};
         {PAGES_NAME_KEY_INDEX_DDL};
         {PAGES_PATH_INDEX_DDL};
         {BLOCKS_PAGE_ORDER_INDEX_DDL};
         {REFERENCES_TARGET_INDEX_DDL};
         {REFERENCES_SOURCE_INDEX_DDL};
         {PROPERTIES_LOOKUP_INDEX_DDL};
         {TAGS_LOOKUP_INDEX_DDL};
         {TASKS_MARKER_INDEX_DDL};
         {TASKS_DEADLINE_INDEX_DDL};"
    ))?;
    connection.execute(
        "INSERT INTO materialization_stamp (
             singleton, acceptance_sequence, frontier_root_digest
         ) VALUES (1, 0, ?1)",
        params![empty_frontier_digest.as_bytes().as_slice()],
    )?;
    Ok(())
}

pub(crate) fn validate_schema(connection: &Connection) -> Result<(), MaterializationError> {
    for (table, expected) in MATERIALIZATION_TABLE_COLUMNS {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns: Vec<String> = statement
            .query_map([], |row| row.get(1))?
            .collect::<Result<_, _>>()?;
        if columns != expected {
            return Err(MaterializationError::Schema(format!(
                "{table} columns {columns:?} != {expected:?}"
            )));
        }
    }
    for (object_type, name, expected) in MATERIALIZATION_SCHEMA_OBJECTS {
        validate_schema_sql(connection, object_type, name, expected)?;
    }
    validate_schema_sql(
        connection,
        "index",
        "tasks_deadline_idx",
        TASKS_DEADLINE_INDEX_DDL,
    )?;
    let stamp_rows: i64 =
        connection.query_row("SELECT COUNT(*) FROM materialization_stamp", [], |row| {
            row.get(0)
        })?;
    if stamp_rows != 1 {
        return Err(MaterializationError::Corrupt(
            "materialization stamp cardinality is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_schema_sql(
    connection: &Connection,
    object_type: &str,
    name: &str,
    expected: &str,
) -> Result<(), MaterializationError> {
    let found: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
        params![object_type, name],
        |row| row.get(0),
    )?;
    if canonical_sql(&found) != canonical_sql(expected) {
        return Err(MaterializationError::Schema(format!(
            "{object_type} {name} does not match canonical DDL"
        )));
    }
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn ensure_stamp(
    connection: &Connection,
    sequence: u64,
    frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    let (found_sequence, found_digest): (i64, Vec<u8>) = connection.query_row(
        "SELECT acceptance_sequence, frontier_root_digest
         FROM materialization_stamp WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if u64::try_from(found_sequence).ok() != Some(sequence)
        || found_digest.as_slice() != frontier_digest.as_bytes()
    {
        return Err(MaterializationError::Stale {
            materialized: u64::try_from(found_sequence).unwrap_or(0),
            frontier: sequence,
        });
    }
    Ok(())
}

pub(crate) fn recorded_digest(
    connection: &Connection,
    sequence: u64,
) -> Result<Option<ContentDigest>, MaterializationError> {
    let bytes: Option<Vec<u8>> = connection
        .query_row(
            "SELECT input_digest FROM materialization_batches
             WHERE acceptance_sequence = ?1",
            params![i64::try_from(sequence).map_err(|_| {
                MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into())
            })?],
            |row| row.get(0),
        )
        .optional()?;
    bytes.map(decode_digest).transpose()
}

pub(crate) fn apply_change(
    transaction: &Transaction<'_>,
    change: &MaterializationChange,
    semantic_effect: &[u8],
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    let effect = SemanticEffect::decode(semantic_effect)
        .map_err(|error| MaterializationError::InvalidInput(error.to_string()))?;
    change.validate_against_effect(&effect)?;
    change.validate_preserved_page_metadata(transaction, &effect)?;
    // A block can move between two replacement pages. Keep its inbound refs
    // through every cleanup pass, then remove every old owner before inserting
    // any new owner so page-ID sort order cannot collide on the block primary key.
    let retained_blocks = change
        .replacements
        .iter()
        .flat_map(|page| page.blocks.iter().map(|block| block.block_id))
        .collect::<BTreeSet<_>>();
    for page_id in &change.deletions {
        delete_page(transaction, *page_id, true, &retained_blocks)?;
    }
    for page in &change.replacements {
        delete_page(transaction, page.page_id, false, &retained_blocks)?;
    }
    for page in &change.replacements {
        insert_page(transaction, page)?;
    }
    transaction.execute(
        "INSERT INTO materialization_batches (
             acceptance_sequence, batch_id, input_digest
         ) VALUES (?1, ?2, ?3)",
        params![
            i64::try_from(sequence).map_err(|_| {
                MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into())
            })?,
            change.batch_id.as_uuid().as_bytes().as_slice(),
            input_digest.as_bytes().as_slice(),
        ],
    )?;
    transaction.execute(
        "UPDATE materialization_stamp
         SET acceptance_sequence = ?1, frontier_root_digest = ?2
         WHERE singleton = 1",
        params![
            i64::try_from(sequence).map_err(|_| {
                MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into())
            })?,
            post_frontier_digest.as_bytes().as_slice(),
        ],
    )?;
    Ok(())
}

pub(crate) fn reset(
    transaction: &Transaction<'_>,
    empty_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    transaction.execute_batch(
        "DELETE FROM search_fts;
         DELETE FROM tasks;
         DELETE FROM tags;
         DELETE FROM properties;
         DELETE FROM refs;
         DELETE FROM blocks;
         DELETE FROM pages;
         DELETE FROM materialization_batches;",
    )?;
    transaction.execute(
        "UPDATE materialization_stamp
         SET acceptance_sequence = 0, frontier_root_digest = ?1
         WHERE singleton = 1",
        params![empty_frontier_digest.as_bytes().as_slice()],
    )?;
    Ok(())
}

fn delete_page(
    transaction: &Transaction<'_>,
    page_id: PageId,
    remove_incoming_page_references: bool,
    retained_blocks: &BTreeSet<BlockId>,
) -> Result<(), MaterializationError> {
    let page_uuid = page_id.as_uuid();
    let page = page_uuid.as_bytes();
    let old_blocks = {
        let mut statement =
            transaction.prepare("SELECT block_id FROM blocks WHERE page_id = ?1")?;
        let block_ids = statement
            .query_map(params![page.as_slice()], |row| row.get::<_, Vec<u8>>(0))?
            .map(|block_id| {
                block_id
                    .map_err(MaterializationError::from)
                    .and_then(|bytes| decode_block_id(&bytes))
            })
            .collect::<Result<Vec<_>, _>>()?;
        block_ids
    };
    transaction.execute(
        "DELETE FROM search_fts
         WHERE (entity_type = 'page' AND entity_id = lower(hex(?1)))
            OR (entity_type = 'block' AND page_id = lower(hex(?1)))",
        params![page.as_slice()],
    )?;
    transaction.execute(
        "DELETE FROM refs
         WHERE source_page_id = ?1",
        params![page.as_slice()],
    )?;
    if remove_incoming_page_references {
        transaction.execute(
            "DELETE FROM refs WHERE target_type = 0 AND target_id = ?1",
            params![page.as_slice()],
        )?;
    }
    for block_id in old_blocks {
        if !retained_blocks.contains(&block_id) {
            transaction.execute(
                "DELETE FROM refs WHERE target_type = 1 AND target_id = ?1",
                params![block_id.as_uuid().as_bytes().as_slice()],
            )?;
        }
    }
    transaction.execute(
        "DELETE FROM properties WHERE page_id = ?1",
        params![page.as_slice()],
    )?;
    transaction.execute(
        "DELETE FROM tags WHERE page_id = ?1",
        params![page.as_slice()],
    )?;
    transaction.execute(
        "DELETE FROM tasks WHERE page_id = ?1",
        params![page.as_slice()],
    )?;
    transaction.execute(
        "DELETE FROM blocks WHERE page_id = ?1",
        params![page.as_slice()],
    )?;
    transaction.execute(
        "DELETE FROM pages WHERE page_id = ?1",
        params![page.as_slice()],
    )?;
    Ok(())
}

fn insert_page(
    transaction: &Transaction<'_>,
    page: &MaterializedPageInput,
) -> Result<(), MaterializationError> {
    let page_uuid = page.page_id.as_uuid();
    let page_id = page_uuid.as_bytes();
    transaction.execute(
        "INSERT INTO pages (
             page_id, home_document_id, name, name_key, path, text_kind,
             preamble, searchable_text
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            page_id.as_slice(),
            page.home_document_id.as_uuid().as_bytes().as_slice(),
            &page.name,
            &page.name_key,
            page.path.as_str(),
            text_kind_to_sql(page.kind),
            &page.preamble,
            &page.searchable_text,
        ],
    )?;
    insert_fts(
        transaction,
        "page",
        page.page_id.as_uuid(),
        page.page_id,
        &page.searchable_text,
    )?;
    insert_references(
        transaction,
        MaterializedEntityId::Page(page.page_id),
        page.page_id,
        &page.references,
    )?;
    insert_properties(
        transaction,
        MaterializedEntityId::Page(page.page_id),
        page.page_id,
        &page.properties,
    )?;
    insert_tags(
        transaction,
        MaterializedEntityId::Page(page.page_id),
        page.page_id,
        &page.tags,
    )?;
    for block in &page.blocks {
        insert_block(transaction, page.page_id, block)?;
    }
    Ok(())
}

fn insert_block(
    transaction: &Transaction<'_>,
    page_id: PageId,
    block: &MaterializedBlockInput,
) -> Result<(), MaterializationError> {
    let (logseq_uuid, origin) = match (block.logseq_uuid, block.logseq_identity_origin) {
        (Some(uuid), Some(origin)) => (
            Some(uuid.as_uuid().as_bytes().to_vec()),
            Some(identity_origin_to_sql(origin)),
        ),
        (None, None) => (None, None),
        _ => {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has incomplete Logseq identity metadata",
                block.block_id
            )));
        }
    };
    transaction.execute(
        "INSERT INTO blocks (
             block_id, page_id, home_document_id, parent_block_id, order_key,
             content, searchable_text, heading_level, collapsed, logseq_uuid,
             logseq_identity_origin
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            block.block_id.as_uuid().as_bytes().as_slice(),
            page_id.as_uuid().as_bytes().as_slice(),
            block.home_document_id.as_uuid().as_bytes().as_slice(),
            block
                .parent
                .map(|parent| parent.as_uuid().as_bytes().to_vec()),
            &block.order,
            &block.content,
            &block.searchable_text,
            block.heading_level.map(i64::from),
            i64::from(block.collapsed),
            logseq_uuid,
            origin,
        ],
    )?;
    insert_fts(
        transaction,
        "block",
        block.block_id.as_uuid(),
        page_id,
        &block.searchable_text,
    )?;
    let owner = MaterializedEntityId::Block(block.block_id);
    insert_references(transaction, owner, page_id, &block.references)?;
    insert_properties(transaction, owner, page_id, &block.properties)?;
    insert_tags(transaction, owner, page_id, &block.tags)?;
    if let Some(task) = &block.task {
        transaction.execute(
            "INSERT INTO tasks (
                 block_id, page_id, marker, priority, scheduled, deadline
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                block.block_id.as_uuid().as_bytes().as_slice(),
                page_id.as_uuid().as_bytes().as_slice(),
                &task.marker,
                &task.priority,
                &task.scheduled,
                &task.deadline,
            ],
        )?;
    }
    Ok(())
}

fn insert_fts(
    transaction: &Transaction<'_>,
    entity_type: &str,
    entity_id: Uuid,
    page_id: PageId,
    text: &str,
) -> Result<(), MaterializationError> {
    transaction.execute(
        "INSERT INTO search_fts (entity_type, entity_id, page_id, text)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            entity_type,
            entity_id.simple().to_string(),
            page_id.as_uuid().simple().to_string(),
            text,
        ],
    )?;
    Ok(())
}

fn insert_references(
    transaction: &Transaction<'_>,
    source: MaterializedEntityId,
    source_page_id: PageId,
    references: &[MaterializedReference],
) -> Result<(), MaterializationError> {
    let (source_type, source_id) = source.sql_parts();
    for (ordinal, reference) in references.iter().enumerate() {
        let (target_type, target_id) = reference.target.sql_parts();
        transaction.execute(
            "INSERT INTO refs (
                 source_type, source_id, source_page_id, target_type, target_id,
                 reference_kind, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                source_type,
                source_id.as_slice(),
                source_page_id.as_uuid().as_bytes().as_slice(),
                target_type,
                target_id.as_slice(),
                reference.kind.sql_value(),
                i64::try_from(ordinal).map_err(|_| {
                    MaterializationError::InvalidInput("reference ordinal overflowed".into())
                })?,
            ],
        )?;
    }
    Ok(())
}

fn insert_properties(
    transaction: &Transaction<'_>,
    owner: MaterializedEntityId,
    page_id: PageId,
    properties: &[MaterializedProperty],
) -> Result<(), MaterializationError> {
    let (owner_type, owner_id) = owner.sql_parts();
    for (ordinal, property) in properties.iter().enumerate() {
        transaction.execute(
            "INSERT INTO properties (
                 owner_type, owner_id, page_id, name, value, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                owner_type,
                owner_id.as_slice(),
                page_id.as_uuid().as_bytes().as_slice(),
                &property.name,
                &property.value,
                i64::try_from(ordinal).map_err(|_| {
                    MaterializationError::InvalidInput("property ordinal overflowed".into())
                })?,
            ],
        )?;
    }
    Ok(())
}

fn insert_tags(
    transaction: &Transaction<'_>,
    owner: MaterializedEntityId,
    page_id: PageId,
    tags: &[String],
) -> Result<(), MaterializationError> {
    let (owner_type, owner_id) = owner.sql_parts();
    for (ordinal, tag) in tags.iter().enumerate() {
        transaction.execute(
            "INSERT INTO tags (owner_type, owner_id, page_id, tag, ordinal)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                owner_type,
                owner_id.as_slice(),
                page_id.as_uuid().as_bytes().as_slice(),
                tag,
                i64::try_from(ordinal).map_err(|_| {
                    MaterializationError::InvalidInput("tag ordinal overflowed".into())
                })?,
            ],
        )?;
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedPageRow {
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub name: String,
    pub name_key: String,
    pub path: ManagedPath,
    pub kind: ManagedTextKind,
    pub preamble: Option<String>,
    pub searchable_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedBlockRow {
    pub block_id: BlockId,
    pub page_id: PageId,
    pub home_document_id: DocumentId,
    pub parent: Option<BlockId>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<LogseqUuid>,
    pub logseq_identity_origin: Option<LogseqIdentityOrigin>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedReferrerRow {
    pub source: MaterializedEntityId,
    pub source_page_id: PageId,
    pub kind: MaterializedReferenceKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedPropertyRow {
    pub owner: MaterializedEntityId,
    pub page_id: PageId,
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedTagRow {
    pub owner: MaterializedEntityId,
    pub page_id: PageId,
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedTaskRow {
    pub block_id: BlockId,
    pub page_id: PageId,
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterializedSearchHit {
    pub entity: MaterializedEntityId,
    pub page_id: PageId,
    pub text: String,
    pub rank: f64,
}

#[derive(Default)]
struct MaterializationReadBudget {
    bytes: usize,
}

impl MaterializationReadBudget {
    fn add(&mut self, bytes: usize) -> Result<(), MaterializationError> {
        self.bytes = checked_budget_add(
            "materialization read output bytes",
            self.bytes,
            bytes,
            MAX_MATERIALIZATION_READ_BYTES,
        )?;
        Ok(())
    }
}

fn checked_output_bytes<'a>(
    fixed_bytes: usize,
    fields: impl IntoIterator<Item = Option<&'a str>>,
) -> Result<usize, MaterializationError> {
    fields.into_iter().try_fold(fixed_bytes, |total, field| {
        let Some(field) = field else {
            return Ok(total);
        };
        total
            .checked_add(field.len())
            .and_then(|total| total.checked_add(MATERIALIZATION_STRING_OVERHEAD_BYTES))
            .ok_or_else(|| {
                resource_limit(
                    "materialization read output bytes",
                    usize::MAX,
                    MAX_MATERIALIZATION_READ_BYTES,
                )
            })
    })
}

fn page_row_output_bytes(row: &MaterializedPageRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        64,
        [
            Some(row.name.as_str()),
            Some(row.name_key.as_str()),
            Some(row.path.as_str()),
            row.preamble.as_deref(),
            Some(row.searchable_text.as_str()),
        ],
    )
}

fn block_row_output_bytes(row: &MaterializedBlockRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        96,
        [
            Some(row.order.as_str()),
            Some(row.content.as_str()),
            Some(row.searchable_text.as_str()),
        ],
    )
}

fn referrer_row_output_bytes(_: &MaterializedReferrerRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [])
}

fn property_row_output_bytes(
    row: &MaterializedPropertyRow,
) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [Some(row.name.as_str()), Some(row.value.as_str())])
}

fn tag_row_output_bytes(row: &MaterializedTagRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [Some(row.tag.as_str())])
}

fn task_row_output_bytes(row: &MaterializedTaskRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        64,
        [
            Some(row.marker.as_str()),
            row.priority.as_deref(),
            row.scheduled.as_deref(),
            row.deadline.as_deref(),
        ],
    )
}

fn search_hit_output_bytes(row: &MaterializedSearchHit) -> Result<usize, MaterializationError> {
    checked_output_bytes(72, [Some(row.text.as_str())])
}

fn collect_read_rows<T>(
    rows: impl IntoIterator<Item = Result<T, MaterializationError>>,
    row_bytes: impl Fn(&T) -> Result<usize, MaterializationError>,
) -> Result<Vec<T>, MaterializationError> {
    let mut output = Vec::new();
    let mut budget = MaterializationReadBudget::default();
    for row in rows {
        let row = row?;
        budget.add(row_bytes(&row)?)?;
        output.push(row);
    }
    Ok(output)
}

fn checked_query_text(value: &str) -> Result<(), MaterializationError> {
    if value.len() > MAX_MATERIALIZATION_QUERY_BYTES {
        return Err(resource_limit(
            "materialization query bytes",
            value.len(),
            MAX_MATERIALIZATION_QUERY_BYTES,
        ));
    }
    Ok(())
}

/// A bounded, read-only view at the exact accepted frontier captured on open.
pub struct SqliteMaterializedRead<'a> {
    connection: &'a Connection,
    acceptance_sequence: u64,
}

impl<'a> SqliteMaterializedRead<'a> {
    pub(crate) fn new(
        connection: &'a Connection,
        acceptance_sequence: u64,
        frontier_digest: ContentDigest,
    ) -> Result<Self, MaterializationError> {
        ensure_stamp(connection, acceptance_sequence, frontier_digest)?;
        Ok(Self {
            connection,
            acceptance_sequence,
        })
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    pub fn page(
        &self,
        page_id: PageId,
    ) -> Result<Option<MaterializedPageRow>, MaterializationError> {
        let page = self
            .connection
            .query_row(
                "SELECT page_id, home_document_id, name, name_key, path,
                        text_kind, preamble, searchable_text
                 FROM pages WHERE page_id = ?1",
                params![page_id.as_uuid().as_bytes().as_slice()],
                page_row,
            )
            .optional()
            .map_err(MaterializationError::from)?;
        if let Some(row) = &page {
            let mut budget = MaterializationReadBudget::default();
            budget.add(page_row_output_bytes(row)?)?;
        }
        Ok(page)
    }

    pub fn block(
        &self,
        block_id: BlockId,
    ) -> Result<Option<MaterializedBlockRow>, MaterializationError> {
        let block = self
            .connection
            .query_row(
                "SELECT block_id, page_id, home_document_id, parent_block_id,
                        order_key, content, searchable_text, heading_level,
                        collapsed, logseq_uuid, logseq_identity_origin
                 FROM blocks WHERE block_id = ?1",
                params![block_id.as_uuid().as_bytes().as_slice()],
                block_row,
            )
            .optional()
            .map_err(MaterializationError::from)?;
        if let Some(row) = &block {
            let mut budget = MaterializationReadBudget::default();
            budget.add(block_row_output_bytes(row)?)?;
        }
        Ok(block)
    }

    pub fn pages_by_name(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        self.pages_by_text_column("name", name, limit)
    }

    pub fn pages_by_name_key(
        &self,
        name_key: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        self.pages_by_text_column("name_key", name_key, limit)
    }

    pub fn pages_by_path(
        &self,
        path: &ManagedPath,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        self.pages_by_text_column("path", path.as_str(), limit)
    }

    fn pages_by_text_column(
        &self,
        column: &str,
        value: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedPageRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(value)?;
        let sql = format!(
            "SELECT page_id, home_document_id, name, name_key, path,
                    text_kind, preamble, searchable_text
             FROM pages WHERE {column} = ?1 ORDER BY page_id LIMIT ?2"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![value, limit], page_row)?;
        collect_read_rows(rows.map(|row| row.map_err(MaterializationError::from)), page_row_output_bytes)
    }

    pub fn blocks_on_page(
        &self,
        page_id: PageId,
        limit: usize,
    ) -> Result<Vec<MaterializedBlockRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT block_id, page_id, home_document_id, parent_block_id,
                    order_key, content, searchable_text, heading_level,
                    collapsed, logseq_uuid, logseq_identity_origin
             FROM blocks WHERE page_id = ?1
             ORDER BY order_key, block_id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(
                params![page_id.as_uuid().as_bytes().as_slice(), limit],
                block_row,
            )?;
        collect_read_rows(rows.map(|row| row.map_err(MaterializationError::from)), block_row_output_bytes)
    }

    pub fn referrers_to(
        &self,
        target: MaterializedEntityId,
        limit: usize,
    ) -> Result<Vec<MaterializedReferrerRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let (target_type, target_id) = target.sql_parts();
        let mut statement = self.connection.prepare(
            "SELECT source_type, source_id, source_page_id, reference_kind
             FROM refs
             WHERE target_type = ?1 AND target_id = ?2
             ORDER BY source_page_id, source_type, source_id, reference_kind, ordinal
             LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![target_type, target_id.as_slice(), limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })?;
        let rows = rows.map(|row| {
            let (source_type, source_id, source_page_id, kind) = row?;
            Ok(MaterializedReferrerRow {
                source: decode_entity(source_type, &source_id)?,
                source_page_id: decode_page_id(&source_page_id)?,
                kind: MaterializedReferenceKind::from_sql(kind)?,
            })
        });
        collect_read_rows(rows, referrer_row_output_bytes)
    }

    pub fn properties(
        &self,
        owner: MaterializedEntityId,
        limit: usize,
    ) -> Result<Vec<MaterializedPropertyRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let (owner_type, owner_id) = owner.sql_parts();
        let mut statement = self.connection.prepare(
            "SELECT owner_type, owner_id, page_id, name, value
             FROM properties WHERE owner_type = ?1 AND owner_id = ?2
             ORDER BY name, ordinal, value LIMIT ?3",
        )?;
        let rows = property_rows(statement.query_map(
            params![owner_type, owner_id.as_slice(), limit],
            property_tuple,
        )?);
        rows
    }

    pub fn properties_named(
        &self,
        name: &str,
        value: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MaterializedPropertyRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(name)?;
        if let Some(value) = value {
            checked_query_text(value)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match value {
            Some(value) => (
                "SELECT owner_type, owner_id, page_id, name, value
                 FROM properties WHERE name = ?1 AND value = ?2
                 ORDER BY page_id, owner_type, owner_id, ordinal LIMIT ?3",
                vec![
                    rusqlite::types::Value::Text(name.to_owned()),
                    rusqlite::types::Value::Text(value.to_owned()),
                    limit.into(),
                ],
            ),
            None => (
                "SELECT owner_type, owner_id, page_id, name, value
                 FROM properties WHERE name = ?1
                 ORDER BY page_id, owner_type, owner_id, ordinal LIMIT ?2",
                vec![rusqlite::types::Value::Text(name.to_owned()), limit.into()],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = property_rows(statement.query_map(
            rusqlite::params_from_iter(args),
            property_tuple,
        )?);
        rows
    }

    pub fn tags(
        &self,
        tag: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedTagRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(tag)?;
        let mut statement = self.connection.prepare(
            "SELECT owner_type, owner_id, page_id, tag
             FROM tags WHERE tag = ?1
             ORDER BY page_id, owner_type, owner_id, ordinal LIMIT ?2",
        )?;
        let rows = statement.query_map(params![tag, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let rows = rows.map(|row| {
            let (owner_type, owner_id, page_id, tag) = row?;
            Ok(MaterializedTagRow {
                owner: decode_entity(owner_type, &owner_id)?,
                page_id: decode_page_id(&page_id)?,
                tag,
            })
        });
        collect_read_rows(rows, tag_row_output_bytes)
    }

    pub fn tasks(
        &self,
        marker: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MaterializedTaskRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        if let Some(marker) = marker {
            checked_query_text(marker)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match marker {
            Some(marker) => (
                "SELECT block_id, page_id, marker, priority, scheduled, deadline
                 FROM tasks WHERE marker = ?1
                 ORDER BY deadline IS NULL, deadline, scheduled IS NULL, scheduled,
                          page_id, block_id LIMIT ?2",
                vec![
                    rusqlite::types::Value::Text(marker.to_owned()),
                    limit.into(),
                ],
            ),
            None => (
                "SELECT block_id, page_id, marker, priority, scheduled, deadline
                 FROM tasks
                 ORDER BY deadline IS NULL, deadline, scheduled IS NULL, scheduled,
                          page_id, block_id LIMIT ?1",
                vec![limit.into()],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let rows = rows.map(|row| {
            let (block_id, page_id, marker, priority, scheduled, deadline) = row?;
            Ok(MaterializedTaskRow {
                block_id: decode_block_id(&block_id)?,
                page_id: decode_page_id(&page_id)?,
                marker,
                priority,
                scheduled,
                deadline,
            })
        });
        collect_read_rows(rows, task_row_output_bytes)
    }

    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MaterializedSearchHit>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(query)?;
        if query.trim().is_empty() {
            return Err(MaterializationError::InvalidQuery(
                "FTS query must be non-empty".into(),
            ));
        }
        let mut statement = self.connection.prepare(
            "SELECT entity_type, entity_id, page_id, text, bm25(search_fts)
             FROM search_fts WHERE search_fts MATCH ?1
             ORDER BY bm25(search_fts), entity_type, entity_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![query, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?;
        let rows = rows.map(|row| {
            let (entity_type, entity_id, page_id, text, rank) = row?;
            let uuid = Uuid::parse_str(&entity_id)
                .map_err(|error| MaterializationError::Corrupt(error.to_string()))?;
            let entity = match entity_type.as_str() {
                "page" => MaterializedEntityId::Page(PageId::from_uuid(uuid)),
                "block" => MaterializedEntityId::Block(BlockId::from_uuid(uuid)),
                _ => {
                    return Err(MaterializationError::Corrupt(format!(
                        "unknown FTS entity type {entity_type:?}"
                    )));
                }
            };
            Ok(MaterializedSearchHit {
                entity,
                page_id: PageId::from_uuid(
                    Uuid::parse_str(&page_id)
                        .map_err(|error| MaterializationError::Corrupt(error.to_string()))?,
                ),
                text,
                rank,
            })
        });
        collect_read_rows(rows, search_hit_output_bytes)
    }
}

fn page_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MaterializedPageRow> {
    let page_id: Vec<u8> = row.get(0)?;
    let home_document_id: Vec<u8> = row.get(1)?;
    let path: String = row.get(4)?;
    let kind: i64 = row.get(5)?;
    Ok(MaterializedPageRow {
        page_id: decode_page_id_sql(&page_id)?,
        home_document_id: decode_document_id_sql(&home_document_id)?,
        name: row.get(2)?,
        name_key: row.get(3)?,
        path: ManagedPath::parse(path).map_err(sql_decode_error)?,
        kind: text_kind_from_sql(kind).map_err(sql_decode_error)?,
        preamble: row.get(6)?,
        searchable_text: row.get(7)?,
    })
}

fn block_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MaterializedBlockRow> {
    let block_id: Vec<u8> = row.get(0)?;
    let page_id: Vec<u8> = row.get(1)?;
    let home_document_id: Vec<u8> = row.get(2)?;
    let parent: Option<Vec<u8>> = row.get(3)?;
    let heading_level: Option<i64> = row.get(7)?;
    let logseq_uuid: Option<Vec<u8>> = row.get(9)?;
    let origin: Option<i64> = row.get(10)?;
    Ok(MaterializedBlockRow {
        block_id: decode_block_id_sql(&block_id)?,
        page_id: decode_page_id_sql(&page_id)?,
        home_document_id: decode_document_id_sql(&home_document_id)?,
        parent: parent.as_deref().map(decode_block_id_sql).transpose()?,
        order: row.get(4)?,
        content: row.get(5)?,
        searchable_text: row.get(6)?,
        heading_level: heading_level
            .map(|value| u8::try_from(value).map_err(sql_decode_error))
            .transpose()?,
        collapsed: row.get::<_, i64>(8)? != 0,
        logseq_uuid: logseq_uuid
            .as_deref()
            .map(decode_logseq_uuid_sql)
            .transpose()?,
        logseq_identity_origin: origin
            .map(identity_origin_from_sql)
            .transpose()
            .map_err(sql_decode_error)?,
    })
}

fn property_tuple(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(i64, Vec<u8>, Vec<u8>, String, String)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}

fn property_rows(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(i64, Vec<u8>, Vec<u8>, String, String)>,
    >,
) -> Result<Vec<MaterializedPropertyRow>, MaterializationError> {
    let rows = rows.map(|row| {
        let (owner_type, owner_id, page_id, name, value) = row?;
        Ok(MaterializedPropertyRow {
            owner: decode_entity(owner_type, &owner_id)?,
            page_id: decode_page_id(&page_id)?,
            name,
            value,
        })
    });
    collect_read_rows(rows, property_row_output_bytes)
}

fn checked_limit(limit: usize) -> Result<i64, MaterializationError> {
    if limit == 0 || limit > MAX_MATERIALIZATION_QUERY_ROWS {
        return Err(MaterializationError::InvalidQuery(format!(
            "query limit {limit} is outside 1..={MAX_MATERIALIZATION_QUERY_ROWS}"
        )));
    }
    i64::try_from(limit)
        .map_err(|_| MaterializationError::InvalidQuery("query limit overflowed".into()))
}

fn text_kind_to_sql(kind: ManagedTextKind) -> i64 {
    match kind {
        ManagedTextKind::Page => 0,
        ManagedTextKind::Journal => 1,
    }
}

fn text_kind_from_sql(value: i64) -> Result<ManagedTextKind, MaterializationError> {
    match value {
        0 => Ok(ManagedTextKind::Page),
        1 => Ok(ManagedTextKind::Journal),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown managed text kind {value}"
        ))),
    }
}

fn identity_origin_to_sql(origin: LogseqIdentityOrigin) -> i64 {
    match origin {
        LogseqIdentityOrigin::ExternalImported => 0,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockReference,
        } => 1,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockEmbed,
        } => 2,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::Export,
        } => 3,
        LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::CopiedDeepLink,
        } => 4,
    }
}

fn identity_origin_from_sql(value: i64) -> Result<LogseqIdentityOrigin, MaterializationError> {
    match value {
        0 => Ok(LogseqIdentityOrigin::ExternalImported),
        1 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockReference,
        }),
        2 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::BlockEmbed,
        }),
        3 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::Export,
        }),
        4 => Ok(LogseqIdentityOrigin::PolicyGenerated {
            reason: PolicyGeneratedAnchorReason::CopiedDeepLink,
        }),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown Logseq identity origin {value}"
        ))),
    }
}

fn decode_entity(
    entity_type: i64,
    bytes: &[u8],
) -> Result<MaterializedEntityId, MaterializationError> {
    match entity_type {
        0 => Ok(MaterializedEntityId::Page(decode_page_id(bytes)?)),
        1 => Ok(MaterializedEntityId::Block(decode_block_id(bytes)?)),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown entity type {entity_type}"
        ))),
    }
}

fn decode_page_id(bytes: &[u8]) -> Result<PageId, MaterializationError> {
    decode_uuid(bytes).map(PageId::from_uuid)
}

fn decode_block_id(bytes: &[u8]) -> Result<BlockId, MaterializationError> {
    decode_uuid(bytes).map(BlockId::from_uuid)
}

fn decode_digest(bytes: Vec<u8>) -> Result<ContentDigest, MaterializationError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| MaterializationError::Corrupt("invalid digest length".into()))?;
    Ok(ContentDigest::from_bytes(bytes))
}

fn decode_uuid(bytes: &[u8]) -> Result<Uuid, MaterializationError> {
    Uuid::from_slice(bytes).map_err(|error| MaterializationError::Corrupt(error.to_string()))
}

fn decode_page_id_sql(bytes: &[u8]) -> rusqlite::Result<PageId> {
    decode_uuid_sql(bytes).map(PageId::from_uuid)
}

fn decode_block_id_sql(bytes: &[u8]) -> rusqlite::Result<BlockId> {
    decode_uuid_sql(bytes).map(BlockId::from_uuid)
}

fn decode_document_id_sql(bytes: &[u8]) -> rusqlite::Result<DocumentId> {
    decode_uuid_sql(bytes).map(DocumentId::from_uuid)
}

fn decode_logseq_uuid_sql(bytes: &[u8]) -> rusqlite::Result<LogseqUuid> {
    decode_uuid_sql(bytes).map(LogseqUuid::from_uuid)
}

fn decode_uuid_sql(bytes: &[u8]) -> rusqlite::Result<Uuid> {
    Uuid::from_slice(bytes).map_err(sql_decode_error)
}

fn sql_decode_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaterializationError {
    Sqlite(String),
    Schema(String),
    Corrupt(String),
    ResourceLimit {
        resource: &'static str,
        found: usize,
        maximum: usize,
    },
    InvalidInput(String),
    Incomplete(String),
    Contradiction(String),
    BatchMismatch { expected: BatchId, found: BatchId },
    Stale { materialized: u64, frontier: u64 },
    DuplicateCollision(BatchId),
    InvalidQuery(String),
}

impl fmt::Display for MaterializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite materialization error: {error}"),
            Self::Schema(error) => write!(f, "materialization schema mismatch: {error}"),
            Self::Corrupt(error) => write!(f, "corrupt materialization: {error}"),
            Self::ResourceLimit {
                resource,
                found,
                maximum,
            } => write!(
                f,
                "materialization {resource} {found} exceeds limit {maximum}"
            ),
            Self::InvalidInput(error) => write!(f, "invalid materialization input: {error}"),
            Self::Incomplete(error) => write!(f, "incomplete materialization input: {error}"),
            Self::Contradiction(error) => {
                write!(f, "materialization contradicts accepted semantics: {error}")
            }
            Self::BatchMismatch { expected, found } => {
                write!(
                    f,
                    "materialization batch {found} != accepted batch {expected}"
                )
            }
            Self::Stale {
                materialized,
                frontier,
            } => write!(
                f,
                "materialization frontier {materialized} is stale against accepted frontier {frontier}"
            ),
            Self::DuplicateCollision(batch_id) => {
                write!(
                    f,
                    "materialization for batch {batch_id} has different canonical bytes"
                )
            }
            Self::InvalidQuery(error) => write!(f, "invalid materialization query: {error}"),
        }
    }
}

impl std::error::Error for MaterializationError {}

impl From<rusqlite::Error> for MaterializationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_id(value: u128) -> PageId {
        PageId::from_uuid(Uuid::from_u128(value))
    }

    fn document_id(value: u128) -> DocumentId {
        DocumentId::from_uuid(Uuid::from_u128(value))
    }

    fn batch_id(value: u128) -> BatchId {
        BatchId::from_uuid(Uuid::from_u128(value))
    }

    fn page_input(page: PageId, searchable_text: String) -> MaterializedPageInput {
        MaterializedPageInput {
            page_id: page,
            home_document_id: document_id(10_000),
            name: "shared".into(),
            name_key: "shared".into(),
            path: ManagedPath::parse(format!("test/{page}.md")).unwrap(),
            kind: ManagedTextKind::Page,
            preamble: None,
            searchable_text,
            references: Vec::new(),
            properties: Vec::new(),
            tags: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn semantic_effect_for_replacements(pages: &[MaterializedPageInput]) -> Vec<u8> {
        SemanticEffect::new(
            pages
                .iter()
                .map(|page| super::super::PageDelta {
                    page_id: page.page_id,
                    before: None,
                    after: Some(PageState::Live {
                        name: super::super::LogicalPageName::parse(&page.name).unwrap(),
                        path: page.path.clone(),
                        home_document_id: page.home_document_id,
                        kind: page.kind,
                    }),
                })
                .collect(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
        .encode()
        .unwrap()
    }

    fn resource_limit(error: Result<MaterializationChange, MaterializationError>, resource: &str) {
        assert!(matches!(
            error,
            Err(MaterializationError::ResourceLimit {
                resource: found,
                ..
            }) if found == resource
        ));
    }

    #[test]
    fn materialization_input_limits_reject_before_digest_or_sqlite_write() {
        let page = page_id(1);
        let mut oversized_field = page_input(page, String::new());
        oversized_field.name = "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES + 1);
        resource_limit(
            MaterializationChange::new(batch_id(1), vec![oversized_field], Vec::new()),
            "page name bytes",
        );

        let reference = MaterializedReference {
            target: MaterializedEntityId::Page(page_id(2)),
            kind: MaterializedReferenceKind::Reference,
        };
        let mut oversized_facet_count = page_input(page_id(3), String::new());
        oversized_facet_count.references =
            vec![reference; MAX_MATERIALIZATION_FACET_VALUES + 1];
        resource_limit(
            MaterializationChange::new(batch_id(2), vec![oversized_facet_count], Vec::new()),
            "reference facet values",
        );

        let oversized_property = MaterializedProperty {
            name: "n".into(),
            value: "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES),
        };
        let mut oversized_facet_bytes = page_input(page_id(4), String::new());
        oversized_facet_bytes.properties = vec![
            oversized_property;
            MAX_MATERIALIZATION_FACET_BYTES / MAX_MATERIALIZATION_FIELD_BYTES
        ];
        resource_limit(
            MaterializationChange::new(batch_id(3), vec![oversized_facet_bytes], Vec::new()),
            "property facet bytes",
        );

        let too_many_deletions = (0..=MAX_MATERIALIZATION_CHANGE_PAGES)
            .map(|index| page_id(100_000 + index as u128))
            .collect();
        resource_limit(
            MaterializationChange::new(batch_id(4), Vec::new(), too_many_deletions),
            "materialization change pages",
        );

        let oversized_change = (0..=MAX_MATERIALIZATION_CHANGE_BYTES / MAX_MATERIALIZATION_FIELD_BYTES)
            .map(|index| {
                page_input(
                    page_id(200_000 + index as u128),
                    "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES),
                )
            })
            .collect();
        resource_limit(
            MaterializationChange::new(batch_id(5), oversized_change, Vec::new()),
            "materialization change bytes",
        );

        let connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let too_large = "x".repeat(MAX_MATERIALIZATION_FIELD_BYTES + 1);
        assert!(connection
            .execute(
                "INSERT INTO pages (
                     page_id, home_document_id, name, name_key, path, text_kind,
                     preamble, searchable_text
                 ) VALUES (?1, ?2, ?3, 'key', 'test/schema.md', 0, NULL, '')",
                params![
                    page_id(300_000).as_uuid().as_bytes().as_slice(),
                    document_id(300_001).as_uuid().as_bytes().as_slice(),
                    too_large,
                ],
            )
            .is_err());
    }

    #[test]
    fn materialization_input_schema_refuses_prior_and_future_before_sqlite_write() {
        assert_eq!(MATERIALIZATION_INPUT_SCHEMA_VERSION, 2);
        let current = MaterializationChange::new(
            batch_id(500_000),
            vec![page_input(page_id(500_001), "current".into())],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(current.schema_version, MATERIALIZATION_INPUT_SCHEMA_VERSION);

        for schema_version in [
            MATERIALIZATION_INPUT_SCHEMA_VERSION - 1,
            MATERIALIZATION_INPUT_SCHEMA_VERSION + 1,
        ] {
            let mut rejected = current.clone();
            rejected.schema_version = schema_version;
            let encoded = postcard::to_allocvec(&rejected).unwrap();
            let rejected: MaterializationChange = postcard::from_bytes(&encoded).unwrap();
            assert!(matches!(
                rejected.digest(),
                Err(MaterializationError::InvalidInput(message))
                    if message == format!("unknown materialization input schema {schema_version}")
            ));

            let mut connection = Connection::open_in_memory().unwrap();
            let empty_frontier = ContentDigest::of(b"empty");
            initialize_schema(&connection, empty_frontier).unwrap();
            let transaction = connection.transaction().unwrap();
            assert!(matches!(
                apply_change(
                    &transaction,
                    &rejected,
                    b"",
                    1,
                    ContentDigest::of(b"input"),
                    ContentDigest::of(b"next"),
                ),
                Err(MaterializationError::InvalidInput(message))
                    if message == format!("unknown materialization input schema {schema_version}")
            ));
            transaction.commit().unwrap();
            let page_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
                .unwrap();
            let batch_count: i64 = connection
                .query_row("SELECT COUNT(*) FROM materialization_batches", [], |row| row.get(0))
                .unwrap();
            let stamp_sequence: i64 = connection
                .query_row(
                    "SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!((page_count, batch_count, stamp_sequence), (0, 0, 0));
        }
    }

    #[test]
    fn non_page_effect_replacement_without_prior_metadata_fails_closed() {
        let page_id = page_id(600_000);
        let mut page = page_input(page_id, "preamble searchable".into());
        page.preamble = Some("updated preamble".into());
        let change = MaterializationChange::new(batch_id(600_001), vec![page], Vec::new()).unwrap();
        let semantic_effect = SemanticEffect::new_with_page_preambles(
            Vec::new(),
            vec![super::super::PagePreambleDelta {
                page_id,
                home_document_id: document_id(10_000),
                before: Some(super::super::PagePreambleState {
                    page_id,
                    home_document_id: document_id(10_000),
                    preamble: None,
                }),
                after: Some(super::super::PagePreambleState {
                    page_id,
                    home_document_id: document_id(10_000),
                    preamble: Some("updated preamble".into()),
                }),
            }],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
        .encode()
        .unwrap();
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let transaction = connection.transaction().unwrap();
        assert!(matches!(
            apply_change(
                &transaction,
                &change,
                &semantic_effect,
                1,
                change.digest().unwrap(),
                ContentDigest::of(b"next"),
            ),
            Err(MaterializationError::Incomplete(message))
                if message.contains("lacks prior validated metadata")
        ));
        transaction.commit().unwrap();
        let state: (i64, i64, i64) = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM pages),
                     (SELECT COUNT(*) FROM materialization_batches),
                     (SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (0, 0, 0));
    }

    #[test]
    fn materialized_reads_reject_oversized_queries_and_aggregate_output() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, ContentDigest::of(b"empty")).unwrap();
        let searchable_text = format!(
            "needle {}",
            "x".repeat(1024 * 1024 - "needle ".len())
        );
        let pages_per_change = 33;
        let mut final_frontier = ContentDigest::of(b"empty");
        for group in 0..2 {
            let replacements = (0..pages_per_change)
                .map(|index| {
                    page_input(
                        page_id(400_000 + (group * pages_per_change + index) as u128),
                        searchable_text.clone(),
                    )
                })
                .collect();
            let change = MaterializationChange::new(
                batch_id(400_000 + group as u128),
                replacements,
                Vec::new(),
            )
            .unwrap();
            let digest = change.digest().unwrap();
            final_frontier = ContentDigest::of(&[group as u8 + 1]);
            let semantic_effect = semantic_effect_for_replacements(change.replacements());
            let transaction = connection.transaction().unwrap();
            apply_change(
                &transaction,
                &change,
                &semantic_effect,
                group as u64 + 1,
                digest,
                final_frontier,
            )
            .unwrap();
            transaction.commit().unwrap();
        }
        let read = SqliteMaterializedRead::new(&connection, 2, final_frontier).unwrap();
        let oversized_query = "q".repeat(MAX_MATERIALIZATION_QUERY_BYTES + 1);
        assert!(matches!(
            read.search(&oversized_query, 1),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization query bytes",
                ..
            })
        ));
        assert!(matches!(
            read.search("needle", pages_per_change * 2),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization read output bytes",
                ..
            })
        ));
    }
}
