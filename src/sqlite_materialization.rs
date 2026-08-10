//! Physical SQLite materialization engine.
//!
//! This module owns disposable SQL shape and bounded physical reads. Inputs are
//! lowered and semantically validated by tine-core before they cross this boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusqlite::{
    functions::FunctionFlags, params, types::ValueRef, Connection, OptionalExtension as _,
    Transaction,
};
use sha2::{Digest as _, Sha256};

use crate::ContentDigest;

pub const MAX_MATERIALIZATION_QUERY_ROWS: usize = 10_000;
pub const MAX_MATERIALIZATION_QUERY_BYTES: usize = 64 * 1024;
pub const MAX_MATERIALIZATION_READ_BYTES: usize = 64 * 1024 * 1024;
const MATERIALIZATION_STRING_OVERHEAD_BYTES: usize = 16;

fn checked_budget_add(
    resource: &'static str,
    current: usize,
    additional: usize,
    maximum: usize,
) -> Result<usize, MaterializationError> {
    let found = current.checked_add(additional).unwrap_or(usize::MAX);
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PhysicalEntityId {
    Page([u8; 16]),
    Block([u8; 16]),
}

impl PhysicalEntityId {
    fn sql_parts(self) -> (i64, [u8; 16]) {
        match self {
            Self::Page(id) => (0, id),
            Self::Block(id) => (1, id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalReference {
    pub target: PhysicalEntityId,
    pub kind: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalProperty {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalTask {
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalBlock {
    pub block_id: [u8; 16],
    pub home_document_id: [u8; 16],
    pub parent: Option<[u8; 16]>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<[u8; 16]>,
    pub logseq_identity_origin: Option<i64>,
    pub references: Vec<PhysicalReference>,
    pub properties: Vec<PhysicalProperty>,
    pub tags: Vec<String>,
    pub task: Option<PhysicalTask>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalPage {
    pub page_id: [u8; 16],
    pub home_document_id: [u8; 16],
    pub name: String,
    pub name_key: String,
    pub path: String,
    pub text_kind: i64,
    pub preamble: Option<String>,
    pub searchable_text: String,
    pub references: Vec<PhysicalReference>,
    pub properties: Vec<PhysicalProperty>,
    pub tags: Vec<String>,
    pub blocks: Vec<PhysicalBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PhysicalReferenceTarget {
    PageName {
        raw_name: String,
        normalized_name: String,
        resolved_page_id: Option<[u8; 16]>,
    },
    ExternalUuid {
        raw_claim: [u8; 16],
        resolved_block_id: Option<[u8; 16]>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalReferencePosting {
    pub source_page_id: [u8; 16],
    pub source_entity: PhysicalEntityId,
    pub source_locator: Vec<u8>,
    pub ordinal: u32,
    pub kind: i64,
    pub target: PhysicalReferenceTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalAliasDeclaration {
    pub source_page_id: [u8; 16],
    pub source_entity: PhysicalEntityId,
    pub source_locator: Vec<u8>,
    pub ordinal: u32,
    pub raw_alias: String,
    pub normalized_alias: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalSourceCoverage {
    pub source_page_id: [u8; 16],
    pub source_digest: ContentDigest,
    pub extractor_dependency_stamp_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalReferenceCatalogChange {
    pub prior_catalog_root: Vec<u8>,
    pub prior_catalog_root_digest: ContentDigest,
    pub prior_source_count: u64,
    pub post_catalog_root: Vec<u8>,
    pub post_catalog_root_digest: ContentDigest,
    pub post_source_count: u64,
    pub coverage_digest: ContentDigest,
    pub extractor_dependency_stamp_digest: ContentDigest,
    pub postings: Vec<PhysicalReferencePosting>,
    pub aliases: Vec<PhysicalAliasDeclaration>,
    pub coverage: Vec<PhysicalSourceCoverage>,
    pub removed_sources: Vec<[u8; 16]>,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalAuthenticatedReference {
    pub event_binding_digest: ContentDigest,
    pub prior_frontier_root_digest: ContentDigest,
    pub post_frontier_root_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalMaterializationChange {
    pub batch_id: [u8; 16],
    pub replacements: Vec<PhysicalPage>,
    pub deletions: Vec<[u8; 16]>,
    pub pages_with_live_metadata_delta: BTreeSet<[u8; 16]>,
    pub reference_catalog: Option<PhysicalReferenceCatalogChange>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ApplyChangeInstrumentation {
    pub cleanup_page_attempts: usize,
    pub cleanup_existing_pages: usize,
    pub cleanup_owned_rows: usize,
    pub cleanup_fts_rowids: usize,
    pub reference_coverage_count: Option<u64>,
    pub reference_coverage_inductive_checks: usize,
    pub reference_coverage_full_scans: usize,
}

/// How one apply establishes the post-apply `reference_source_coverage` row
/// count it checks against the authenticated catalog's post source count.
///
/// `FullScan` reads the whole table and is therefore proportional to the graph,
/// not to the change. `FreshInductive` instead starts from a count the caller
/// already proved at the immediately preceding accepted sequence, checks it
/// against the same authenticated catalog's *prior* source count, and moves it
/// by the rows this apply actually replaced and inserted. Both end at the same
/// equality check, so a caller that has no proved prior count -- a fresh open,
/// a rebuild, a gap in the accepted chain -- selects the scan and loses nothing.
#[derive(Clone, Copy)]
enum CoverageValidation {
    FullScan,
    FreshInductive { prior_count: u64 },
}

pub const MATERIALIZATION_STAMP_DDL: &str = "CREATE TABLE materialization_stamp (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    acceptance_sequence INTEGER NOT NULL CHECK (acceptance_sequence >= 0),
    frontier_root_digest BLOB NOT NULL CHECK (length(frontier_root_digest) = 32),
    catalog_root BLOB CHECK (
        catalog_root IS NULL OR length(catalog_root) BETWEEN 1 AND 4096
    ),
    catalog_root_digest BLOB CHECK (
        catalog_root_digest IS NULL OR length(catalog_root_digest) = 32
    ),
    coverage_digest BLOB CHECK (
        coverage_digest IS NULL OR length(coverage_digest) = 32
    ),
    extractor_dependency_stamp_digest BLOB CHECK (
        extractor_dependency_stamp_digest IS NULL
        OR length(extractor_dependency_stamp_digest) = 32
    ),
    CHECK (
        (catalog_root IS NULL AND catalog_root_digest IS NULL
         AND coverage_digest IS NULL AND extractor_dependency_stamp_digest IS NULL)
        OR
        (catalog_root IS NOT NULL AND catalog_root_digest IS NOT NULL
         AND coverage_digest IS NOT NULL AND extractor_dependency_stamp_digest IS NOT NULL)
    )
) WITHOUT ROWID, STRICT";
pub const MATERIALIZATION_BATCHES_DDL: &str = "CREATE TABLE materialization_batches (
    acceptance_sequence INTEGER PRIMARY KEY CHECK (acceptance_sequence > 0),
    batch_id BLOB NOT NULL UNIQUE CHECK (length(batch_id) = 16),
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    event_binding_digest BLOB CHECK (
        event_binding_digest IS NULL OR length(event_binding_digest) = 32
    ),
    prior_frontier_root_digest BLOB CHECK (
        prior_frontier_root_digest IS NULL OR length(prior_frontier_root_digest) = 32
    ),
    post_frontier_root_digest BLOB CHECK (
        post_frontier_root_digest IS NULL OR length(post_frontier_root_digest) = 32
    ),
    prior_catalog_root BLOB CHECK (
        prior_catalog_root IS NULL OR length(prior_catalog_root) BETWEEN 1 AND 4096
    ),
    prior_catalog_root_digest BLOB CHECK (
        prior_catalog_root_digest IS NULL OR length(prior_catalog_root_digest) = 32
    ),
    post_catalog_root BLOB CHECK (
        post_catalog_root IS NULL OR length(post_catalog_root) BETWEEN 1 AND 4096
    ),
    post_catalog_root_digest BLOB CHECK (
        post_catalog_root_digest IS NULL OR length(post_catalog_root_digest) = 32
    ),
    catalog_change BLOB CHECK (
        catalog_change IS NULL OR length(catalog_change) BETWEEN 1 AND 67108864
    ),
    catalog_change_digest BLOB CHECK (
        catalog_change_digest IS NULL OR length(catalog_change_digest) = 32
    ),
    canonical_input_digest BLOB CHECK (
        canonical_input_digest IS NULL OR length(canonical_input_digest) = 32
    ),
    CHECK (
        (event_binding_digest IS NULL AND prior_frontier_root_digest IS NULL
         AND post_frontier_root_digest IS NULL AND prior_catalog_root IS NULL
         AND prior_catalog_root_digest IS NULL AND post_catalog_root IS NULL
         AND post_catalog_root_digest IS NULL AND catalog_change IS NULL
         AND catalog_change_digest IS NULL AND canonical_input_digest IS NULL)
        OR
        (event_binding_digest IS NOT NULL AND prior_frontier_root_digest IS NOT NULL
         AND post_frontier_root_digest IS NOT NULL AND prior_catalog_root IS NOT NULL
         AND prior_catalog_root_digest IS NOT NULL AND post_catalog_root IS NOT NULL
         AND post_catalog_root_digest IS NOT NULL AND catalog_change IS NOT NULL
         AND catalog_change_digest IS NOT NULL AND canonical_input_digest IS NOT NULL)
    )
) WITHOUT ROWID, STRICT";
// Generic page materialization leaves this authority group NULL.  The packet-3
// adapter fills it atomically only from an accepted catalog transition; it
// must never synthesize zero or sentinel authority.
pub const REFERENCE_SOURCE_COVERAGE_DDL: &str = "CREATE TABLE reference_source_coverage (
    source_page_id BLOB PRIMARY KEY CHECK (length(source_page_id) = 16),
    source_digest BLOB NOT NULL CHECK (length(source_digest) = 32),
    extractor_dependency_stamp_digest BLOB NOT NULL CHECK (
        length(extractor_dependency_stamp_digest) = 32
    )
) WITHOUT ROWID, STRICT";
pub const REFERENCE_POSTINGS_DDL: &str = "CREATE TABLE reference_postings (
    source_page_id BLOB NOT NULL CHECK (length(source_page_id) = 16),
    source_entity_type INTEGER NOT NULL CHECK (source_entity_type IN (0, 1)),
    source_entity_id BLOB NOT NULL CHECK (length(source_entity_id) = 16),
    source_locator BLOB NOT NULL CHECK (length(source_locator) BETWEEN 1 AND 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    reference_kind INTEGER NOT NULL CHECK (reference_kind BETWEEN 0 AND 7),
    target_type INTEGER NOT NULL CHECK (target_type IN (0, 1)),
    raw_name TEXT CHECK (
        raw_name IS NULL OR length(CAST(raw_name AS BLOB)) BETWEEN 1 AND 4194304
    ),
    normalized_name TEXT CHECK (
        normalized_name IS NULL OR length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 4194304
    ),
    raw_uuid_claim BLOB CHECK (
        raw_uuid_claim IS NULL OR length(raw_uuid_claim) = 16
    ),
    resolved_page_id BLOB CHECK (
        resolved_page_id IS NULL OR length(resolved_page_id) = 16
    ),
    resolved_block_id BLOB CHECK (
        resolved_block_id IS NULL OR length(resolved_block_id) = 16
    ),
    CHECK (
        (reference_kind BETWEEN 0 AND 5 AND target_type = 0)
        OR
        (reference_kind IN (6, 7) AND target_type = 1)
    ),
    CHECK (
        (target_type = 0 AND raw_name IS NOT NULL AND normalized_name IS NOT NULL
         AND raw_uuid_claim IS NULL AND resolved_block_id IS NULL)
        OR
        (target_type = 1 AND raw_name IS NULL AND normalized_name IS NULL
         AND raw_uuid_claim IS NOT NULL AND resolved_page_id IS NULL)
    ),
    PRIMARY KEY (
        source_page_id, source_entity_type, source_entity_id, source_locator, ordinal
    )
) WITHOUT ROWID, STRICT";
pub const REFERENCE_NAME_BINDINGS_DDL: &str = "CREATE TABLE reference_name_bindings (
    raw_name TEXT NOT NULL CHECK (length(CAST(raw_name AS BLOB)) BETWEEN 1 AND 4194304),
    normalized_name TEXT NOT NULL CHECK (
        length(CAST(normalized_name AS BLOB)) BETWEEN 1 AND 4194304
    ),
    candidate_ordinal INTEGER NOT NULL CHECK (candidate_ordinal >= 0),
    resolved_page_id BLOB CHECK (
        resolved_page_id IS NULL OR length(resolved_page_id) = 16
    ),
    PRIMARY KEY (raw_name, candidate_ordinal)
) WITHOUT ROWID, STRICT";
pub const REFERENCE_UUID_BINDINGS_DDL: &str = "CREATE TABLE reference_uuid_bindings (
    raw_uuid_claim BLOB NOT NULL CHECK (length(raw_uuid_claim) = 16),
    candidate_ordinal INTEGER NOT NULL CHECK (candidate_ordinal >= 0),
    resolved_block_id BLOB CHECK (
        resolved_block_id IS NULL OR length(resolved_block_id) = 16
    ),
    PRIMARY KEY (raw_uuid_claim, candidate_ordinal)
) WITHOUT ROWID, STRICT";
pub const REFERENCE_ALIAS_DECLARATIONS_DDL: &str = "CREATE TABLE reference_alias_declarations (
    source_page_id BLOB NOT NULL CHECK (length(source_page_id) = 16),
    source_entity_type INTEGER NOT NULL CHECK (source_entity_type IN (0, 1)),
    source_entity_id BLOB NOT NULL CHECK (length(source_entity_id) = 16),
    source_locator BLOB NOT NULL CHECK (length(source_locator) BETWEEN 1 AND 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    raw_alias TEXT NOT NULL CHECK (length(CAST(raw_alias AS BLOB)) BETWEEN 1 AND 4194304),
    normalized_alias TEXT NOT NULL CHECK (
        length(CAST(normalized_alias AS BLOB)) BETWEEN 1 AND 4194304
    ),
    PRIMARY KEY (
        source_page_id, source_entity_type, source_entity_id, source_locator, ordinal
    )
) WITHOUT ROWID, STRICT";
/// An alias binding is the resolution itself: which pages a normalized alias
/// currently names, in candidate order.
///
/// It deliberately does NOT record the catalog root it was resolved against.
/// That stamp was written by every path and read by none, and because it sat
/// in the primary key it made two correct builds of the same graph disagree:
/// an incremental drain stamps the root that was current when the alias was
/// last touched, while a rebuild stamps the root it resolved at. Same alias,
/// same ordinal, same page, different provenance -- enough to fail a
/// byte-equality proof for a value nothing consults. The projection's own
/// `materialization_stamp` already records the catalog root the whole database
/// is at, which is the question anyone actually asks.
pub const REFERENCE_ALIAS_BINDINGS_DDL: &str = "CREATE TABLE reference_alias_bindings (
    normalized_alias TEXT NOT NULL CHECK (
        length(CAST(normalized_alias AS BLOB)) BETWEEN 1 AND 4194304
    ),
    candidate_ordinal INTEGER NOT NULL CHECK (candidate_ordinal >= 0),
    resolved_page_id BLOB CHECK (
        resolved_page_id IS NULL OR length(resolved_page_id) = 16
    ),
    PRIMARY KEY (normalized_alias, candidate_ordinal)
) WITHOUT ROWID, STRICT";
pub const PAGES_DDL: &str = "CREATE TABLE pages (
    page_id BLOB PRIMARY KEY CHECK (length(page_id) = 16),
    home_document_id BLOB NOT NULL CHECK (length(home_document_id) = 16),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 4194304),
    name_key TEXT NOT NULL CHECK (length(CAST(name_key AS BLOB)) BETWEEN 1 AND 4194304),
    path TEXT NOT NULL CHECK (length(CAST(path AS BLOB)) BETWEEN 1 AND 4194304),
    text_kind INTEGER NOT NULL CHECK (text_kind IN (0, 1)),
    preamble TEXT CHECK (preamble IS NULL OR length(CAST(preamble AS BLOB)) <= 16777216),
    searchable_text TEXT NOT NULL CHECK (length(CAST(searchable_text AS BLOB)) <= 4194304)
) STRICT";
pub const BLOCKS_DDL: &str = "CREATE TABLE blocks (
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
// Retained temporarily for active v2 reads/writes. The authenticated catalog
// migration-cleanup slice removes this legacy target-ID representation only
// after every call site has moved to the v10 raw-evidence tables below.
pub const REFERENCES_DDL: &str = "CREATE TABLE refs (
    source_type INTEGER NOT NULL CHECK (source_type IN (0, 1)),
    source_id BLOB NOT NULL CHECK (length(source_id) = 16),
    source_page_id BLOB NOT NULL CHECK (length(source_page_id) = 16),
    target_type INTEGER NOT NULL CHECK (target_type IN (0, 1)),
    target_id BLOB NOT NULL CHECK (length(target_id) = 16),
    reference_kind INTEGER NOT NULL CHECK (reference_kind BETWEEN 0 AND 3),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (source_type, source_id, target_type, target_id, reference_kind, ordinal)
) WITHOUT ROWID, STRICT";
pub const PROPERTIES_DDL: &str = "CREATE TABLE properties (
    owner_type INTEGER NOT NULL CHECK (owner_type IN (0, 1)),
    owner_id BLOB NOT NULL CHECK (length(owner_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    name TEXT NOT NULL CHECK (length(CAST(name AS BLOB)) BETWEEN 1 AND 4194304),
    value TEXT NOT NULL CHECK (length(CAST(value AS BLOB)) <= 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (owner_type, owner_id, name, ordinal)
) WITHOUT ROWID, STRICT";
pub const TAGS_DDL: &str = "CREATE TABLE tags (
    owner_type INTEGER NOT NULL CHECK (owner_type IN (0, 1)),
    owner_id BLOB NOT NULL CHECK (length(owner_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    tag TEXT NOT NULL CHECK (length(CAST(tag AS BLOB)) BETWEEN 1 AND 4194304),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (owner_type, owner_id, ordinal)
) WITHOUT ROWID, STRICT";
pub const TASKS_DDL: &str = "CREATE TABLE tasks (
    block_id BLOB PRIMARY KEY CHECK (length(block_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    marker TEXT NOT NULL CHECK (length(CAST(marker AS BLOB)) BETWEEN 1 AND 4194304),
    priority TEXT CHECK (priority IS NULL OR length(CAST(priority AS BLOB)) <= 4194304),
    scheduled TEXT CHECK (scheduled IS NULL OR length(CAST(scheduled AS BLOB)) <= 4194304),
    deadline TEXT CHECK (deadline IS NULL OR length(CAST(deadline AS BLOB)) <= 4194304)
) STRICT";
pub const SEARCH_FTS_DDL: &str = "CREATE VIRTUAL TABLE search_fts USING fts5(
    entity_type UNINDEXED,
    entity_id UNINDEXED,
    page_id UNINDEXED,
    text,
    tokenize = 'unicode61 remove_diacritics 0'
)";
pub const SEARCH_FTS_OWNERS_DDL: &str = "CREATE TABLE search_fts_owners (
    rowid INTEGER PRIMARY KEY,
    entity_type INTEGER NOT NULL CHECK (entity_type IN (0, 1)),
    entity_id BLOB NOT NULL CHECK (length(entity_id) = 16),
    page_id BLOB NOT NULL CHECK (length(page_id) = 16),
    UNIQUE (entity_type, entity_id)
) STRICT";

pub const PAGES_NAME_INDEX_DDL: &str = "CREATE INDEX pages_name_idx ON pages(name, page_id)";
pub const PAGES_NAME_KEY_INDEX_DDL: &str =
    "CREATE INDEX pages_name_key_idx ON pages(name_key, page_id)";
pub const PAGES_PATH_INDEX_DDL: &str = "CREATE INDEX pages_path_idx ON pages(path, page_id)";
pub const BLOCKS_PAGE_ORDER_INDEX_DDL: &str =
    "CREATE INDEX blocks_page_order_idx ON blocks(page_id, order_key, block_id)";
pub const BLOCKS_LOGSEQ_UUID_INDEX_DDL: &str = "CREATE UNIQUE INDEX blocks_logseq_uuid_idx
    ON blocks(logseq_uuid) WHERE logseq_uuid IS NOT NULL";
pub const SEARCH_FTS_OWNERS_PAGE_INDEX_DDL: &str =
    "CREATE INDEX search_fts_owners_page_idx ON search_fts_owners(page_id, rowid)";
pub const REFERENCES_TARGET_INDEX_DDL: &str = "CREATE INDEX references_target_idx
    ON refs(target_type, target_id, source_page_id, source_type, source_id)";
pub const REFERENCES_SOURCE_INDEX_DDL: &str = "CREATE INDEX references_source_idx
    ON refs(source_page_id, source_type, source_id)";
pub const REFERENCE_SOURCE_COVERAGE_SOURCE_INDEX_DDL: &str =
    "CREATE INDEX reference_source_coverage_source_idx ON reference_source_coverage(source_page_id)";
pub const REFERENCE_POSTINGS_SOURCE_INDEX_DDL: &str = "CREATE INDEX reference_postings_source_idx
    ON reference_postings(source_page_id, source_entity_type, source_entity_id, ordinal)";
pub const REFERENCE_POSTINGS_NORMALIZED_NAME_INDEX_DDL: &str =
    "CREATE INDEX reference_postings_normalized_name_idx
    ON reference_postings(normalized_name, source_page_id, source_entity_type, source_entity_id, ordinal)
    WHERE target_type = 0";
pub const REFERENCE_POSTINGS_RAW_UUID_INDEX_DDL: &str = "CREATE INDEX reference_postings_raw_uuid_idx
    ON reference_postings(raw_uuid_claim, source_page_id, source_entity_type, source_entity_id, ordinal)
    WHERE target_type = 1";
pub const REFERENCE_NAME_BINDINGS_RAW_NAME_INDEX_DDL: &str =
    "CREATE INDEX reference_name_bindings_raw_name_idx
    ON reference_name_bindings(raw_name, candidate_ordinal)";
pub const REFERENCE_NAME_BINDINGS_RESOLVED_PAGE_INDEX_DDL: &str =
    "CREATE INDEX reference_name_bindings_resolved_page_idx
    ON reference_name_bindings(resolved_page_id, raw_name, candidate_ordinal)";
pub const REFERENCE_UUID_BINDINGS_RAW_UUID_INDEX_DDL: &str =
    "CREATE INDEX reference_uuid_bindings_raw_uuid_idx
    ON reference_uuid_bindings(raw_uuid_claim, candidate_ordinal)";
pub const REFERENCE_UUID_BINDINGS_RESOLVED_BLOCK_INDEX_DDL: &str =
    "CREATE INDEX reference_uuid_bindings_resolved_block_idx
    ON reference_uuid_bindings(resolved_block_id, raw_uuid_claim, candidate_ordinal)";
pub const REFERENCE_ALIAS_DECLARATIONS_SOURCE_INDEX_DDL: &str =
    "CREATE INDEX reference_alias_declarations_source_idx
    ON reference_alias_declarations(source_page_id, source_entity_type, source_entity_id, ordinal)";
pub const REFERENCE_ALIAS_BINDINGS_NORMALIZED_ALIAS_INDEX_DDL: &str =
    "CREATE INDEX reference_alias_bindings_normalized_alias_idx
    ON reference_alias_bindings(normalized_alias, candidate_ordinal)";
pub const PROPERTIES_LOOKUP_INDEX_DDL: &str = "CREATE INDEX properties_lookup_idx
    ON properties(name, value, page_id, owner_type, owner_id)";
pub const PROPERTIES_PAGE_INDEX_DDL: &str = "CREATE INDEX properties_page_idx
    ON properties(page_id, owner_type, owner_id, name, ordinal)";
pub const TAGS_LOOKUP_INDEX_DDL: &str =
    "CREATE INDEX tags_lookup_idx ON tags(tag, page_id, owner_type, owner_id)";
pub const TAGS_PAGE_INDEX_DDL: &str =
    "CREATE INDEX tags_page_idx ON tags(page_id, owner_type, owner_id, ordinal)";
pub const TASKS_MARKER_INDEX_DDL: &str =
    "CREATE INDEX tasks_marker_idx ON tasks(marker, page_id, block_id)";
pub const TASKS_DEADLINE_INDEX_DDL: &str =
    "CREATE INDEX tasks_deadline_idx ON tasks(deadline, scheduled, page_id, block_id)";
pub const TASKS_PAGE_INDEX_DDL: &str = "CREATE INDEX tasks_page_idx ON tasks(page_id, block_id)";

const MATERIALIZATION_TABLE_COLUMNS: [(&str, &[&str]); 15] = [
    (
        "materialization_stamp",
        &[
            "singleton",
            "acceptance_sequence",
            "frontier_root_digest",
            "catalog_root",
            "catalog_root_digest",
            "coverage_digest",
            "extractor_dependency_stamp_digest",
        ],
    ),
    (
        "materialization_batches",
        &[
            "acceptance_sequence",
            "batch_id",
            "input_digest",
            "event_binding_digest",
            "prior_frontier_root_digest",
            "post_frontier_root_digest",
            "prior_catalog_root",
            "prior_catalog_root_digest",
            "post_catalog_root",
            "post_catalog_root_digest",
            "catalog_change",
            "catalog_change_digest",
            "canonical_input_digest",
        ],
    ),
    (
        "reference_source_coverage",
        &[
            "source_page_id",
            "source_digest",
            "extractor_dependency_stamp_digest",
        ],
    ),
    (
        "reference_postings",
        &[
            "source_page_id",
            "source_entity_type",
            "source_entity_id",
            "source_locator",
            "ordinal",
            "reference_kind",
            "target_type",
            "raw_name",
            "normalized_name",
            "raw_uuid_claim",
            "resolved_page_id",
            "resolved_block_id",
        ],
    ),
    (
        "reference_name_bindings",
        &[
            "raw_name",
            "normalized_name",
            "candidate_ordinal",
            "resolved_page_id",
        ],
    ),
    (
        "reference_uuid_bindings",
        &["raw_uuid_claim", "candidate_ordinal", "resolved_block_id"],
    ),
    (
        "reference_alias_declarations",
        &[
            "source_page_id",
            "source_entity_type",
            "source_entity_id",
            "source_locator",
            "ordinal",
            "raw_alias",
            "normalized_alias",
        ],
    ),
    (
        "reference_alias_bindings",
        &["normalized_alias", "candidate_ordinal", "resolved_page_id"],
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
    (
        "search_fts_owners",
        &["rowid", "entity_type", "entity_id", "page_id"],
    ),
];

const MATERIALIZATION_SCHEMA_OBJECTS: [(&str, &str, &str); 40] = [
    ("table", "materialization_stamp", MATERIALIZATION_STAMP_DDL),
    (
        "table",
        "materialization_batches",
        MATERIALIZATION_BATCHES_DDL,
    ),
    (
        "table",
        "reference_source_coverage",
        REFERENCE_SOURCE_COVERAGE_DDL,
    ),
    ("table", "reference_postings", REFERENCE_POSTINGS_DDL),
    (
        "table",
        "reference_name_bindings",
        REFERENCE_NAME_BINDINGS_DDL,
    ),
    (
        "table",
        "reference_uuid_bindings",
        REFERENCE_UUID_BINDINGS_DDL,
    ),
    (
        "table",
        "reference_alias_declarations",
        REFERENCE_ALIAS_DECLARATIONS_DDL,
    ),
    (
        "table",
        "reference_alias_bindings",
        REFERENCE_ALIAS_BINDINGS_DDL,
    ),
    ("table", "pages", PAGES_DDL),
    ("table", "blocks", BLOCKS_DDL),
    ("table", "refs", REFERENCES_DDL),
    ("table", "properties", PROPERTIES_DDL),
    ("table", "tags", TAGS_DDL),
    ("table", "tasks", TASKS_DDL),
    ("table", "search_fts_owners", SEARCH_FTS_OWNERS_DDL),
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
        "blocks_logseq_uuid_idx",
        BLOCKS_LOGSEQ_UUID_INDEX_DDL,
    ),
    (
        "index",
        "search_fts_owners_page_idx",
        SEARCH_FTS_OWNERS_PAGE_INDEX_DDL,
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
        "reference_source_coverage_source_idx",
        REFERENCE_SOURCE_COVERAGE_SOURCE_INDEX_DDL,
    ),
    (
        "index",
        "reference_postings_source_idx",
        REFERENCE_POSTINGS_SOURCE_INDEX_DDL,
    ),
    (
        "index",
        "reference_postings_normalized_name_idx",
        REFERENCE_POSTINGS_NORMALIZED_NAME_INDEX_DDL,
    ),
    (
        "index",
        "reference_postings_raw_uuid_idx",
        REFERENCE_POSTINGS_RAW_UUID_INDEX_DDL,
    ),
    (
        "index",
        "reference_name_bindings_raw_name_idx",
        REFERENCE_NAME_BINDINGS_RAW_NAME_INDEX_DDL,
    ),
    (
        "index",
        "reference_name_bindings_resolved_page_idx",
        REFERENCE_NAME_BINDINGS_RESOLVED_PAGE_INDEX_DDL,
    ),
    (
        "index",
        "reference_uuid_bindings_raw_uuid_idx",
        REFERENCE_UUID_BINDINGS_RAW_UUID_INDEX_DDL,
    ),
    (
        "index",
        "reference_uuid_bindings_resolved_block_idx",
        REFERENCE_UUID_BINDINGS_RESOLVED_BLOCK_INDEX_DDL,
    ),
    (
        "index",
        "reference_alias_declarations_source_idx",
        REFERENCE_ALIAS_DECLARATIONS_SOURCE_INDEX_DDL,
    ),
    (
        "index",
        "reference_alias_bindings_normalized_alias_idx",
        REFERENCE_ALIAS_BINDINGS_NORMALIZED_ALIAS_INDEX_DDL,
    ),
    (
        "index",
        "properties_lookup_idx",
        PROPERTIES_LOOKUP_INDEX_DDL,
    ),
    ("index", "properties_page_idx", PROPERTIES_PAGE_INDEX_DDL),
    ("index", "tags_lookup_idx", TAGS_LOOKUP_INDEX_DDL),
    ("index", "tags_page_idx", TAGS_PAGE_INDEX_DDL),
    ("index", "tasks_marker_idx", TASKS_MARKER_INDEX_DDL),
    ("index", "tasks_page_idx", TASKS_PAGE_INDEX_DDL),
];

pub fn initialize_schema(
    connection: &Connection,
    empty_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    connection.execute_batch(&format!(
        "{MATERIALIZATION_STAMP_DDL};
         {MATERIALIZATION_BATCHES_DDL};
         {REFERENCE_SOURCE_COVERAGE_DDL};
         {REFERENCE_POSTINGS_DDL};
         {REFERENCE_NAME_BINDINGS_DDL};
         {REFERENCE_UUID_BINDINGS_DDL};
         {REFERENCE_ALIAS_DECLARATIONS_DDL};
         {REFERENCE_ALIAS_BINDINGS_DDL};
         {PAGES_DDL};
         {BLOCKS_DDL};
         {REFERENCES_DDL};
         {PROPERTIES_DDL};
         {TAGS_DDL};
         {TASKS_DDL};
         {SEARCH_FTS_OWNERS_DDL};
         {SEARCH_FTS_DDL};
         {PAGES_NAME_INDEX_DDL};
         {PAGES_NAME_KEY_INDEX_DDL};
         {PAGES_PATH_INDEX_DDL};
         {BLOCKS_PAGE_ORDER_INDEX_DDL};
         {BLOCKS_LOGSEQ_UUID_INDEX_DDL};
         {SEARCH_FTS_OWNERS_PAGE_INDEX_DDL};
         {REFERENCES_TARGET_INDEX_DDL};
         {REFERENCES_SOURCE_INDEX_DDL};
         {REFERENCE_SOURCE_COVERAGE_SOURCE_INDEX_DDL};
         {REFERENCE_POSTINGS_SOURCE_INDEX_DDL};
         {REFERENCE_POSTINGS_NORMALIZED_NAME_INDEX_DDL};
         {REFERENCE_POSTINGS_RAW_UUID_INDEX_DDL};
         {REFERENCE_NAME_BINDINGS_RAW_NAME_INDEX_DDL};
         {REFERENCE_NAME_BINDINGS_RESOLVED_PAGE_INDEX_DDL};
         {REFERENCE_UUID_BINDINGS_RAW_UUID_INDEX_DDL};
         {REFERENCE_UUID_BINDINGS_RESOLVED_BLOCK_INDEX_DDL};
         {REFERENCE_ALIAS_DECLARATIONS_SOURCE_INDEX_DDL};
         {REFERENCE_ALIAS_BINDINGS_NORMALIZED_ALIAS_INDEX_DDL};
         {PROPERTIES_LOOKUP_INDEX_DDL};
         {PROPERTIES_PAGE_INDEX_DDL};
         {TAGS_LOOKUP_INDEX_DDL};
         {TAGS_PAGE_INDEX_DDL};
         {TASKS_MARKER_INDEX_DDL};
         {TASKS_DEADLINE_INDEX_DDL};
         {TASKS_PAGE_INDEX_DDL};"
    ))?;
    connection.execute(
        "INSERT INTO materialization_stamp (
             singleton, acceptance_sequence, frontier_root_digest
         ) VALUES (1, 0, ?1)",
        params![empty_frontier_digest.as_bytes().as_slice()],
    )?;
    Ok(())
}

pub fn validate_schema(connection: &Connection) -> Result<(), MaterializationError> {
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

/// Canonical digest of every materialized row, including the FTS search
/// surface. This is a harness observation only: normal reads stay on their
/// bounded page/query APIs.
///
/// SQLite orders a deterministic scalar BLOB key produced from that exact
/// encoding. That preserves distinctions SQLite's ordinary comparison rules
/// collapse, notably `0.0` and `-0.0`, while allowing the SHA-256 input to be
/// consumed one row at a time.
fn digested_materialization_tables() -> impl Iterator<Item = (&'static str, &'static [&'static str])>
{
    MATERIALIZATION_TABLE_COLUMNS
        .into_iter()
        .chain(std::iter::once((
            "search_fts",
            &["entity_type", "entity_id", "page_id", "text"] as &[&str],
        )))
}

fn update_table_rows(
    hasher: &mut Sha256,
    connection: &Connection,
    table: &str,
    columns: &[&str],
) -> Result<(), MaterializationError> {
    update_len(hasher, table.len());
    hasher.update(table.as_bytes());
    update_len(hasher, columns.len());
    for column in columns {
        update_len(hasher, column.len());
        hasher.update(column.as_bytes());
    }
    let row_count: i64 =
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
    let row_count = usize::try_from(row_count).map_err(|_| {
        MaterializationError::Corrupt(format!("{table} row count is negative or exceeds usize"))
    })?;
    update_len(hasher, row_count);
    let select_columns = columns.join(", ");
    let sql = format!(
        "SELECT {select_columns} FROM {table}
         ORDER BY tine_materialization_canonical_row({select_columns}) COLLATE BINARY"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        update_canonical_row(hasher, row, columns.len())?;
    }
    Ok(())
}

pub fn row_digest(connection: &Connection) -> Result<ContentDigest, MaterializationError> {
    install_canonical_row_key_function(connection)?;
    let mut hasher = Sha256::new();
    hasher.update(b"tine/sqlite-materialization/rows/v2\0");
    for (table, columns) in digested_materialization_tables() {
        update_table_rows(&mut hasher, connection, table, columns)?;
    }
    Ok(ContentDigest::from_bytes(hasher.finalize().into()))
}

/// Columns that carry SQLite's insertion order rather than an authoritative
/// observation. Two independently built databases agree on the mapping such a
/// column expresses, not on the integers SQLite happened to assign; the FTS
/// owner rowid is joined to `search_fts` and proved inside each database.
#[cfg(any(test, feature = "test-support"))]
const CONSTRUCTION_ORDER_COLUMNS: [(&str, &str); 1] = [("search_fts_owners", "rowid")];

/// Per-table complete row observation.
///
/// Differential tests compare two independently built databases table by
/// table, so a divergence names the table it is in and construction-only
/// provenance tables can be excluded deliberately rather than by weakening the
/// whole-database digest.
#[cfg(any(test, feature = "test-support"))]
pub fn row_digests_by_table(
    connection: &Connection,
) -> Result<Vec<(&'static str, ContentDigest)>, MaterializationError> {
    install_canonical_row_key_function(connection)?;
    digested_materialization_tables()
        .map(|(table, columns)| {
            let columns = columns
                .iter()
                .copied()
                .filter(|column| !CONSTRUCTION_ORDER_COLUMNS.contains(&(table, column)))
                .collect::<Vec<_>>();
            let mut hasher = Sha256::new();
            hasher.update(b"tine/sqlite-materialization/table-rows/v1\0");
            update_table_rows(&mut hasher, connection, table, &columns)?;
            Ok((table, ContentDigest::from_bytes(hasher.finalize().into())))
        })
        .collect()
}

fn install_canonical_row_key_function(connection: &Connection) -> Result<(), MaterializationError> {
    connection.create_scalar_function(
        "tine_materialization_canonical_row",
        -1,
        FunctionFlags::SQLITE_DETERMINISTIC,
        |context| {
            let mut bytes = Vec::new();
            encode_len(&mut bytes, context.len());
            for index in 0..context.len() {
                let mut value = Vec::new();
                encode_sqlite_value(&mut value, context.get_raw(index))
                    .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
                encode_len(&mut bytes, value.len());
                bytes.extend_from_slice(&value);
            }
            Ok(bytes)
        },
    )?;
    Ok(())
}

fn update_canonical_row(
    hasher: &mut Sha256,
    row: &rusqlite::Row<'_>,
    column_count: usize,
) -> Result<(), MaterializationError> {
    let mut row_len = 8_usize;
    for index in 0..column_count {
        let value_len = encoded_sqlite_value_len(row.get_ref(index)?)?;
        row_len = row_len
            .checked_add(8)
            .and_then(|len| len.checked_add(value_len))
            .ok_or_else(|| {
                MaterializationError::Corrupt("canonical row length overflowed".into())
            })?;
    }
    update_len(hasher, row_len);
    update_len(hasher, column_count);
    for index in 0..column_count {
        let value = row.get_ref(index)?;
        update_len(hasher, encoded_sqlite_value_len(value)?);
        update_sqlite_value(hasher, value)?;
    }
    Ok(())
}

fn encoded_sqlite_value_len(value: ValueRef<'_>) -> Result<usize, MaterializationError> {
    Ok(match value {
        ValueRef::Null => 1,
        ValueRef::Integer(_) | ValueRef::Real(_) => 9,
        ValueRef::Text(value) | ValueRef::Blob(value) => {
            9usize.checked_add(value.len()).ok_or_else(|| {
                MaterializationError::Corrupt("canonical value length overflowed".into())
            })?
        }
    })
}

fn update_len(hasher: &mut Sha256, len: usize) {
    hasher.update((len as u64).to_be_bytes());
}

fn update_sqlite_value(
    hasher: &mut Sha256,
    value: ValueRef<'_>,
) -> Result<(), MaterializationError> {
    match value {
        ValueRef::Null => hasher.update([0]),
        ValueRef::Integer(value) => {
            hasher.update([1]);
            hasher.update(value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            hasher.update([2]);
            hasher.update(value.to_bits().to_be_bytes());
        }
        ValueRef::Text(value) => {
            std::str::from_utf8(value).map_err(|error| {
                MaterializationError::Corrupt(format!(
                    "materialized TEXT contains invalid UTF-8: {error}"
                ))
            })?;
            hasher.update([3]);
            update_len(hasher, value.len());
            hasher.update(value);
        }
        ValueRef::Blob(value) => {
            hasher.update([4]);
            update_len(hasher, value.len());
            hasher.update(value);
        }
    }
    Ok(())
}

#[cfg(test)]
fn row_digest_legacy(connection: &Connection) -> Result<ContentDigest, MaterializationError> {
    let mut bytes = b"tine/sqlite-materialization/rows/v2\0".to_vec();
    for (table, columns) in MATERIALIZATION_TABLE_COLUMNS
        .into_iter()
        .chain(std::iter::once((
            "search_fts",
            &["entity_type", "entity_id", "page_id", "text"] as &[&str],
        )))
    {
        encode_len(&mut bytes, table.len());
        bytes.extend_from_slice(table.as_bytes());
        encode_len(&mut bytes, columns.len());
        for column in columns {
            encode_len(&mut bytes, column.len());
            bytes.extend_from_slice(column.as_bytes());
        }
        let sql = format!("SELECT {} FROM {table}", columns.join(", "));
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query([])?;
        let mut canonical_rows = Vec::new();
        while let Some(row) = rows.next()? {
            let mut canonical_row = Vec::new();
            encode_len(&mut canonical_row, columns.len());
            for index in 0..columns.len() {
                let mut value = Vec::new();
                encode_sqlite_value(&mut value, row.get_ref(index)?)?;
                encode_len(&mut canonical_row, value.len());
                canonical_row.extend_from_slice(&value);
            }
            canonical_rows.push(canonical_row);
        }
        canonical_rows.sort_unstable();
        encode_len(&mut bytes, canonical_rows.len());
        for row in canonical_rows {
            encode_len(&mut bytes, row.len());
            bytes.extend_from_slice(&row);
        }
    }
    Ok(ContentDigest::of(&bytes))
}

fn encode_len(bytes: &mut Vec<u8>, len: usize) {
    bytes.extend_from_slice(&(len as u64).to_be_bytes());
}

fn encode_sqlite_value(
    bytes: &mut Vec<u8>,
    value: ValueRef<'_>,
) -> Result<(), MaterializationError> {
    match value {
        ValueRef::Null => bytes.push(0),
        ValueRef::Integer(value) => {
            bytes.push(1);
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            bytes.push(2);
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        ValueRef::Text(value) => {
            std::str::from_utf8(value).map_err(|error| {
                MaterializationError::Corrupt(format!(
                    "materialized TEXT contains invalid UTF-8: {error}"
                ))
            })?;
            bytes.push(3);
            encode_len(bytes, value.len());
            bytes.extend_from_slice(value);
        }
        ValueRef::Blob(value) => {
            bytes.push(4);
            encode_len(bytes, value.len());
            bytes.extend_from_slice(value);
        }
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

pub fn ensure_stamp(
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

pub fn recorded_digest(
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

/// One full disposable-candidate proof after all inductive per-part updates
/// and before publication. Ordinary incremental application continues to use
/// the per-transaction full coverage check.
pub fn finalize_fresh_bootstrap(
    connection: &Connection,
    expected_catalog_source_count: u64,
    inductive_coverage_count: u64,
) -> Result<(), MaterializationError> {
    let coverage_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM reference_source_coverage",
        [],
        |row| row.get(0),
    )?;
    let coverage_count = u64::try_from(coverage_count).map_err(|_| {
        MaterializationError::Corrupt("reference source coverage count is negative".into())
    })?;
    if coverage_count != inductive_coverage_count || coverage_count != expected_catalog_source_count
    {
        return Err(MaterializationError::Incomplete(format!(
            "final SQLite reference source coverage {coverage_count} differs from inductive count {inductive_coverage_count} or authenticated catalog count {}",
            expected_catalog_source_count,
        )));
    }

    let owner_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM search_fts_owners", [], |row| {
            row.get(0)
        })?;
    let fts_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM search_fts", [], |row| row.get(0))?;
    let mismatches: i64 = connection.query_row(
        "SELECT COUNT(*)
         FROM search_fts_owners AS owner
         LEFT JOIN search_fts AS fts ON fts.rowid = owner.rowid
         WHERE fts.rowid IS NULL
            OR fts.entity_type != CASE owner.entity_type WHEN 0 THEN 'page' ELSE 'block' END
            OR fts.entity_id != lower(hex(owner.entity_id))
            OR fts.page_id != lower(hex(owner.page_id))",
        [],
        |row| row.get(0),
    )?;
    if owner_count != fts_count || mismatches != 0 {
        return Err(MaterializationError::Corrupt(
            "FTS rows differ from their authoritative owner mapping".into(),
        ));
    }
    Ok(())
}

/// One bounded chunk of terminal bootstrap rows.
///
/// Terminal construction seeds an unpublished candidate whose materialized
/// tables are still empty, so a chunk carries only insertions: there is no
/// prior page row to clean up and no prior coverage row to replace.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhysicalTerminalMaterializationChunk {
    pub pages: Vec<PhysicalPage>,
    pub coverage: Vec<PhysicalSourceCoverage>,
    pub postings: Vec<PhysicalReferencePosting>,
    pub aliases: Vec<PhysicalAliasDeclaration>,
}

/// Construction provenance for one accepted sequence of a terminal build.
///
/// The terminal builder applies no intermediate per-event page or reference
/// DML, so every row it writes carries the digest of the empty change actually
/// applied at that sequence rather than a fabricated per-event digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalTerminalConstructionBatch {
    pub acceptance_sequence: u64,
    pub batch_id: [u8; 16],
    pub input_digest: ContentDigest,
}

/// The single authenticated catalog stamp a terminal build publishes after its
/// complete terminal rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalTerminalCatalogStamp {
    pub acceptance_sequence: u64,
    pub frontier_root_digest: ContentDigest,
    pub catalog_root: Vec<u8>,
    pub catalog_root_digest: ContentDigest,
    pub coverage_digest: ContentDigest,
    pub extractor_dependency_stamp_digest: ContentDigest,
    pub source_count: u64,
}

const TERMINAL_CONSTRUCTION_EMPTY_TABLES: [&str; 15] = [
    "pages",
    "blocks",
    "refs",
    "properties",
    "tags",
    "tasks",
    "search_fts_owners",
    "search_fts",
    "reference_source_coverage",
    "reference_postings",
    "reference_alias_declarations",
    "reference_alias_bindings",
    "reference_name_bindings",
    "reference_uuid_bindings",
    "materialization_batches",
];

/// Refuse terminal construction unless every materialized table is still empty
/// and the stamp has never advanced. A partially materialized candidate must
/// take the ordinary replay path instead.
pub(crate) fn begin_terminal_construction_in_open_candidate(
    transaction: &Connection,
) -> Result<(), MaterializationError> {
    require_open_candidate(transaction)?;
    for table in TERMINAL_CONSTRUCTION_EMPTY_TABLES {
        let count: i64 =
            transaction.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
        if count != 0 {
            return Err(MaterializationError::Contradiction(format!(
                "terminal construction requires an empty candidate but {table} has {count} rows"
            )));
        }
    }
    let stamp_sequence: i64 = transaction.query_row(
        "SELECT acceptance_sequence FROM materialization_stamp WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if stamp_sequence != 0 {
        return Err(MaterializationError::Contradiction(
            "terminal construction requires an unstamped candidate".into(),
        ));
    }
    Ok(())
}

/// Seed one bounded chunk of terminal pages and reference rows.
pub(crate) fn seed_terminal_chunk_in_open_candidate(
    transaction: &Connection,
    chunk: &PhysicalTerminalMaterializationChunk,
) -> Result<(), MaterializationError> {
    require_open_candidate(transaction)?;
    for page in &chunk.pages {
        insert_page(transaction, page)?;
    }
    for facet in &chunk.coverage {
        execute_cached(
            transaction,
            "INSERT INTO reference_source_coverage (
                 source_page_id, source_digest, extractor_dependency_stamp_digest
             ) VALUES (?1, ?2, ?3)",
            params![
                facet.source_page_id.as_slice(),
                facet.source_digest.as_bytes().as_slice(),
                facet
                    .extractor_dependency_stamp_digest
                    .as_bytes()
                    .as_slice(),
            ],
        )?;
    }
    for posting in &chunk.postings {
        insert_reference_posting(transaction, posting)?;
    }
    for alias in &chunk.aliases {
        insert_alias_declaration(transaction, alias)?;
    }
    Ok(())
}

/// Close one terminal build: derive the alias bindings from the complete
/// declarations, write the accepted-prefix construction provenance, and publish
/// the one authenticated catalog stamp.
pub(crate) fn finish_terminal_construction_in_open_candidate(
    transaction: &Connection,
    provenance: &[PhysicalTerminalConstructionBatch],
    stamp: &PhysicalTerminalCatalogStamp,
) -> Result<u64, MaterializationError> {
    require_open_candidate(transaction)?;
    transaction.execute(
        "INSERT INTO reference_alias_bindings (
             normalized_alias, candidate_ordinal, resolved_page_id
         )
         SELECT normalized_alias, candidate_ordinal, source_page_id
         FROM (
             SELECT normalized_alias, source_page_id,
                    ROW_NUMBER() OVER (
                        PARTITION BY normalized_alias ORDER BY source_page_id
                    ) - 1 AS candidate_ordinal
             FROM (
                 SELECT DISTINCT normalized_alias, source_page_id
                 FROM reference_alias_declarations
             )
         )",
        [],
    )?;
    for batch in provenance {
        transaction.execute(
            "INSERT INTO materialization_batches (
                 acceptance_sequence, batch_id, input_digest
             ) VALUES (?1, ?2, ?3)",
            params![
                i64::try_from(batch.acceptance_sequence).map_err(|_| {
                    MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into())
                })?,
                batch.batch_id.as_slice(),
                batch.input_digest.as_bytes().as_slice(),
            ],
        )?;
    }
    let coverage_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM reference_source_coverage",
        [],
        |row| row.get(0),
    )?;
    let coverage_count = u64::try_from(coverage_count).map_err(|_| {
        MaterializationError::Corrupt("reference source coverage count is negative".into())
    })?;
    if coverage_count != stamp.source_count {
        return Err(MaterializationError::Incomplete(format!(
            "terminal SQLite reference source coverage {coverage_count} differs from authenticated catalog source count {}",
            stamp.source_count,
        )));
    }
    transaction.execute(
        "UPDATE materialization_stamp
         SET acceptance_sequence = ?1,
             frontier_root_digest = ?2,
             catalog_root = ?3,
             catalog_root_digest = ?4,
             coverage_digest = ?5,
             extractor_dependency_stamp_digest = ?6
         WHERE singleton = 1",
        params![
            i64::try_from(stamp.acceptance_sequence).map_err(|_| {
                MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into())
            })?,
            stamp.frontier_root_digest.as_bytes().as_slice(),
            &stamp.catalog_root,
            stamp.catalog_root_digest.as_bytes().as_slice(),
            stamp.coverage_digest.as_bytes().as_slice(),
            stamp
                .extractor_dependency_stamp_digest
                .as_bytes()
                .as_slice(),
        ],
    )?;
    Ok(coverage_count)
}

fn insert_reference_posting(
    transaction: &Connection,
    posting: &PhysicalReferencePosting,
) -> Result<(), MaterializationError> {
    let (source_entity_type, source_entity_id) = posting.source_entity.sql_parts();
    let locator = &posting.source_locator;
    let (
        target_type,
        raw_name,
        normalized_name,
        raw_uuid_claim,
        resolved_page_id,
        resolved_block_id,
    ) = match &posting.target {
        PhysicalReferenceTarget::PageName {
            raw_name,
            normalized_name,
            resolved_page_id,
        } => (
            0_i64,
            Some(raw_name.as_str()),
            Some(normalized_name.as_str()),
            None,
            resolved_page_id.map(|id| id.to_vec()),
            None,
        ),
        PhysicalReferenceTarget::ExternalUuid {
            raw_claim,
            resolved_block_id,
        } => (
            1_i64,
            None,
            None,
            Some(raw_claim.to_vec()),
            None,
            resolved_block_id.map(|id| id.to_vec()),
        ),
    };
    execute_cached(
        transaction,
        "INSERT INTO reference_postings (
             source_page_id, source_entity_type, source_entity_id, source_locator,
             ordinal, reference_kind, target_type, raw_name, normalized_name,
             raw_uuid_claim, resolved_page_id, resolved_block_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            posting.source_page_id.as_slice(),
            source_entity_type,
            source_entity_id.as_slice(),
            locator,
            i64::from(posting.ordinal),
            posting.kind,
            target_type,
            raw_name,
            normalized_name,
            raw_uuid_claim,
            resolved_page_id,
            resolved_block_id,
        ],
    )?;
    Ok(())
}

fn insert_alias_declaration(
    transaction: &Connection,
    alias: &PhysicalAliasDeclaration,
) -> Result<(), MaterializationError> {
    let (source_entity_type, source_entity_id) = alias.source_entity.sql_parts();
    let locator = &alias.source_locator;
    execute_cached(
        transaction,
        "INSERT INTO reference_alias_declarations (
             source_page_id, source_entity_type, source_entity_id, source_locator,
             ordinal, raw_alias, normalized_alias
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            alias.source_page_id.as_slice(),
            source_entity_type,
            source_entity_id.as_slice(),
            locator,
            i64::from(alias.ordinal),
            &alias.raw_alias,
            &alias.normalized_alias,
        ],
    )?;
    Ok(())
}

fn require_open_candidate(transaction: &Connection) -> Result<(), MaterializationError> {
    if transaction.is_autocommit() {
        return Err(MaterializationError::InvalidInput(
            "terminal construction requires an active candidate-build transaction".into(),
        ));
    }
    Ok(())
}

fn apply_reference_catalog_change(
    transaction: &Connection,
    input: &PhysicalReferenceCatalogChange,
    coverage_validation: CoverageValidation,
) -> Result<(Vec<u8>, ContentDigest, ContentDigest, ContentDigest, u64), MaterializationError> {
    let post_root_bytes = input.post_catalog_root.clone();
    let post_root_digest = input.post_catalog_root_digest;
    let extractor_stamp_digest = input.extractor_dependency_stamp_digest;
    let coverage_digest = input.coverage_digest;
    let sources = input
        .coverage
        .iter()
        .map(|facet| facet.source_page_id)
        .chain(input.removed_sources.iter().copied())
        .collect::<BTreeSet<_>>();
    let mut altered_aliases = BTreeSet::new();
    let mut prior_alias_candidates = BTreeMap::<String, BTreeSet<[u8; 16]>>::new();
    let mut replaced_coverage_rows = 0_u64;
    for page_id in &sources {
        let existed: i64 = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM reference_source_coverage WHERE source_page_id = ?1
             )",
            params![page_id.as_slice()],
            |row| row.get(0),
        )?;
        replaced_coverage_rows = replaced_coverage_rows.saturating_add(u64::from(existed != 0));
        let mut statement = transaction.prepare(
            "SELECT normalized_alias FROM reference_alias_declarations
             WHERE source_page_id = ?1",
        )?;
        let aliases = statement
            .query_map(params![page_id.as_slice()], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        altered_aliases.extend(aliases);
    }
    altered_aliases.extend(
        input
            .aliases
            .iter()
            .map(|alias| alias.normalized_alias.clone()),
    );
    for alias in &altered_aliases {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT resolved_page_id FROM reference_alias_bindings
             WHERE normalized_alias = ?1 AND resolved_page_id IS NOT NULL",
        )?;
        let candidates = statement
            .query_map(params![alias], |row| row.get::<_, Vec<u8>>(0))?
            .map(|row| {
                row.map_err(MaterializationError::from)
                    .and_then(|bytes| decode_id(&bytes))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        prior_alias_candidates.insert(alias.clone(), candidates);
    }

    for page_id in &sources {
        let id = page_id;
        transaction.execute(
            "DELETE FROM reference_postings WHERE source_page_id = ?1",
            params![id.as_slice()],
        )?;
        transaction.execute(
            "DELETE FROM reference_alias_declarations WHERE source_page_id = ?1",
            params![id.as_slice()],
        )?;
        transaction.execute(
            "DELETE FROM reference_source_coverage WHERE source_page_id = ?1",
            params![id.as_slice()],
        )?;
    }
    for facet in &input.coverage {
        execute_cached(
            transaction,
            "INSERT INTO reference_source_coverage (
                 source_page_id, source_digest, extractor_dependency_stamp_digest
             ) VALUES (?1, ?2, ?3)",
            params![
                facet.source_page_id.as_slice(),
                facet.source_digest.as_bytes().as_slice(),
                facet
                    .extractor_dependency_stamp_digest
                    .as_bytes()
                    .as_slice(),
            ],
        )?;
    }
    for posting in &input.postings {
        insert_reference_posting(transaction, posting)?;
    }
    for alias in &input.aliases {
        insert_alias_declaration(transaction, alias)?;
    }
    for alias in altered_aliases {
        let mut candidates = prior_alias_candidates.remove(&alias).unwrap_or_default();
        candidates.retain(|page_id| !sources.contains(page_id));
        candidates.extend(
            input
                .aliases
                .iter()
                .filter(|declaration| declaration.normalized_alias == alias)
                .map(|declaration| declaration.source_page_id),
        );
        transaction.execute(
            "DELETE FROM reference_alias_bindings WHERE normalized_alias = ?1",
            params![&alias],
        )?;
        for (ordinal, page_id) in candidates.into_iter().enumerate() {
            transaction.execute(
                "INSERT INTO reference_alias_bindings (
                     normalized_alias, candidate_ordinal, resolved_page_id
                 ) VALUES (?1, ?2, ?3)",
                params![
                    &alias,
                    i64::try_from(ordinal).map_err(|_| {
                        MaterializationError::InvalidInput(
                            "reference alias candidate ordinal overflowed".into(),
                        )
                    })?,
                    page_id.as_slice(),
                ],
            )?;
        }
    }
    let coverage_count = match coverage_validation {
        CoverageValidation::FullScan => {
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM reference_source_coverage",
                [],
                |row| row.get(0),
            )?;
            u64::try_from(count).map_err(|_| {
                MaterializationError::Corrupt("reference source coverage count is negative".into())
            })?
        }
        CoverageValidation::FreshInductive { prior_count } => {
            if prior_count != input.prior_source_count {
                return Err(MaterializationError::Incomplete(format!(
                    "inductive SQLite reference source coverage {prior_count} does not match authenticated prior catalog source count {}",
                    input.prior_source_count,
                )));
            }
            prior_count
                .checked_sub(replaced_coverage_rows)
                .and_then(|count| count.checked_add(input.coverage.len() as u64))
                .ok_or_else(|| {
                    MaterializationError::Corrupt(
                        "inductive reference source coverage count overflowed".into(),
                    )
                })?
        }
    };
    if coverage_count != input.post_source_count {
        return Err(MaterializationError::Incomplete(
            format!(
                "SQLite reference source coverage {coverage_count} does not match authenticated catalog source count {}",
                input.post_source_count,
            ),
        ));
    }
    Ok((
        post_root_bytes,
        post_root_digest,
        coverage_digest,
        extractor_stamp_digest,
        coverage_count,
    ))
}

pub fn apply_change(
    transaction: &Transaction<'_>,
    change: &PhysicalMaterializationChange,
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&PhysicalAuthenticatedReference>,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    apply_change_inner(
        transaction,
        change,
        sequence,
        input_digest,
        post_frontier_digest,
        authenticated_reference,
        CoverageValidation::FullScan,
    )
}

pub fn apply_change_fresh_bootstrap(
    transaction: &Transaction<'_>,
    change: &PhysicalMaterializationChange,
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&PhysicalAuthenticatedReference>,
    prior_reference_coverage_count: u64,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    apply_change_inner(
        transaction,
        change,
        sequence,
        input_digest,
        post_frontier_digest,
        authenticated_reference,
        CoverageValidation::FreshInductive {
            prior_count: prior_reference_coverage_count,
        },
    )
}

pub(crate) fn apply_change_in_open_candidate(
    transaction: &Connection,
    change: &PhysicalMaterializationChange,
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&PhysicalAuthenticatedReference>,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    if transaction.is_autocommit() {
        return Err(MaterializationError::InvalidInput(
            "candidate materialization requires an active transaction".into(),
        ));
    }
    apply_change_inner(
        transaction,
        change,
        sequence,
        input_digest,
        post_frontier_digest,
        authenticated_reference,
        CoverageValidation::FullScan,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_change_fresh_bootstrap_in_open_candidate(
    transaction: &Connection,
    change: &PhysicalMaterializationChange,
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&PhysicalAuthenticatedReference>,
    prior_reference_coverage_count: u64,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    if transaction.is_autocommit() {
        return Err(MaterializationError::InvalidInput(
            "candidate materialization requires an active transaction".into(),
        ));
    }
    apply_change_inner(
        transaction,
        change,
        sequence,
        input_digest,
        post_frontier_digest,
        authenticated_reference,
        CoverageValidation::FreshInductive {
            prior_count: prior_reference_coverage_count,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn apply_change_inner(
    transaction: &Connection,
    change: &PhysicalMaterializationChange,
    sequence: u64,
    input_digest: ContentDigest,
    post_frontier_digest: ContentDigest,
    authenticated_reference: Option<&PhysicalAuthenticatedReference>,
    coverage_validation: CoverageValidation,
) -> Result<ApplyChangeInstrumentation, MaterializationError> {
    if change.reference_catalog.is_some() && authenticated_reference.is_none() {
        return Err(MaterializationError::Incomplete(
            "authenticated reference materialization requires accepted event evidence".into(),
        ));
    }
    validate_preserved_page_metadata(transaction, change)?;
    // A block can move between two replacement pages. Keep its inbound refs
    // through every cleanup pass, then remove every old owner before inserting
    // any new owner so page-ID sort order cannot collide on the block primary key.
    let retained_blocks = change
        .replacements
        .iter()
        .flat_map(|page| page.blocks.iter().map(|block| block.block_id))
        .collect::<BTreeSet<_>>();
    let mut instrumentation = ApplyChangeInstrumentation::default();
    for page_id in &change.deletions {
        let cleanup = delete_page(transaction, *page_id, true, &retained_blocks)?;
        instrumentation.cleanup_page_attempts += 1;
        instrumentation.cleanup_existing_pages += cleanup.existing_pages;
        instrumentation.cleanup_owned_rows += cleanup.owned_rows;
        instrumentation.cleanup_fts_rowids += cleanup.fts_rowids;
    }
    for page in &change.replacements {
        let cleanup = delete_page(transaction, page.page_id, false, &retained_blocks)?;
        instrumentation.cleanup_page_attempts += 1;
        instrumentation.cleanup_existing_pages += cleanup.existing_pages;
        instrumentation.cleanup_owned_rows += cleanup.owned_rows;
        instrumentation.cleanup_fts_rowids += cleanup.fts_rowids;
    }
    for page in &change.replacements {
        insert_page(transaction, page)?;
    }
    let reference_values = change
        .reference_catalog
        .as_ref()
        .map(|input| apply_reference_catalog_change(transaction, input, coverage_validation))
        .transpose()?;
    let sequence = i64::try_from(sequence)
        .map_err(|_| MaterializationError::Corrupt("acceptance sequence exceeds SQLite".into()))?;
    if let Some((
        catalog_root,
        catalog_root_digest,
        coverage_digest,
        extractor_stamp_digest,
        coverage_count,
    )) = reference_values
    {
        instrumentation.reference_coverage_count = Some(coverage_count);
        match coverage_validation {
            CoverageValidation::FullScan => instrumentation.reference_coverage_full_scans = 1,
            CoverageValidation::FreshInductive { .. } => {
                instrumentation.reference_coverage_inductive_checks = 1;
            }
        }
        let authenticated = authenticated_reference
            .expect("reference values require authenticated transition evidence");
        let reference_catalog = change.reference_catalog.as_ref().expect("present");
        let catalog_change = reference_catalog.canonical_bytes.clone();
        let catalog_change_digest = ContentDigest::of(&catalog_change);
        transaction.execute(
            "INSERT INTO materialization_batches (
                 acceptance_sequence, batch_id, input_digest, event_binding_digest,
                 prior_frontier_root_digest, post_frontier_root_digest,
                 prior_catalog_root, prior_catalog_root_digest,
                 post_catalog_root, post_catalog_root_digest,
                 catalog_change, catalog_change_digest, canonical_input_digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                sequence,
                change.batch_id.as_slice(),
                input_digest.as_bytes().as_slice(),
                authenticated.event_binding_digest.as_bytes().as_slice(),
                authenticated
                    .prior_frontier_root_digest
                    .as_bytes()
                    .as_slice(),
                authenticated
                    .post_frontier_root_digest
                    .as_bytes()
                    .as_slice(),
                &reference_catalog.prior_catalog_root,
                reference_catalog
                    .prior_catalog_root_digest
                    .as_bytes()
                    .as_slice(),
                catalog_root,
                catalog_root_digest.as_bytes().as_slice(),
                catalog_change,
                catalog_change_digest.as_bytes().as_slice(),
                input_digest.as_bytes().as_slice(),
            ],
        )?;
        transaction.execute(
            "UPDATE materialization_stamp
             SET acceptance_sequence = ?1,
                 frontier_root_digest = ?2,
                 catalog_root = ?3,
                 catalog_root_digest = ?4,
                 coverage_digest = ?5,
                 extractor_dependency_stamp_digest = ?6
             WHERE singleton = 1",
            params![
                sequence,
                post_frontier_digest.as_bytes().as_slice(),
                catalog_root,
                catalog_root_digest.as_bytes().as_slice(),
                coverage_digest.as_bytes().as_slice(),
                extractor_stamp_digest.as_bytes().as_slice(),
            ],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO materialization_batches (
                 acceptance_sequence, batch_id, input_digest
             ) VALUES (?1, ?2, ?3)",
            params![
                sequence,
                change.batch_id.as_slice(),
                input_digest.as_bytes().as_slice(),
            ],
        )?;
        transaction.execute(
            "UPDATE materialization_stamp
             SET acceptance_sequence = ?1,
                 frontier_root_digest = ?2,
                 catalog_root = NULL,
                 catalog_root_digest = NULL,
                 coverage_digest = NULL,
                 extractor_dependency_stamp_digest = NULL
             WHERE singleton = 1",
            params![sequence, post_frontier_digest.as_bytes().as_slice()],
        )?;
    }
    Ok(instrumentation)
}

fn validate_preserved_page_metadata(
    transaction: &Connection,
    change: &PhysicalMaterializationChange,
) -> Result<(), MaterializationError> {
    for page in &change.replacements {
        if change
            .pages_with_live_metadata_delta
            .contains(&page.page_id)
        {
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
                    page.page_id.as_slice(),
                    page.home_document_id.as_slice(),
                    &page.name,
                    &page.name_key,
                    &page.path,
                    page.text_kind,
                ],
                |row| row.get(0),
            )
            .optional()?;
        match metadata_matches {
            Some(true) => {}
            Some(false) => {
                return Err(MaterializationError::Contradiction(format!(
                    "page {} replacement changes metadata without an accepted live page delta",
                    uuid::Uuid::from_bytes(page.page_id)
                )));
            }
            None => {
                return Err(MaterializationError::Incomplete(format!(
                    "page {} replacement lacks prior validated metadata",
                    uuid::Uuid::from_bytes(page.page_id)
                )));
            }
        }
    }
    Ok(())
}

pub fn reset(
    transaction: &Transaction<'_>,
    empty_frontier_digest: ContentDigest,
) -> Result<(), MaterializationError> {
    transaction.execute_batch(
        "DELETE FROM search_fts;
         DELETE FROM search_fts_owners;
         DELETE FROM tasks;
         DELETE FROM tags;
         DELETE FROM properties;
         DELETE FROM refs;
         DELETE FROM reference_alias_bindings;
         DELETE FROM reference_alias_declarations;
         DELETE FROM reference_uuid_bindings;
         DELETE FROM reference_name_bindings;
         DELETE FROM reference_postings;
         DELETE FROM reference_source_coverage;
         DELETE FROM blocks;
         DELETE FROM pages;
         DELETE FROM materialization_batches;",
    )?;
    transaction.execute(
        "UPDATE materialization_stamp
         SET acceptance_sequence = 0,
             frontier_root_digest = ?1,
             catalog_root = NULL,
             catalog_root_digest = NULL,
             coverage_digest = NULL,
             extractor_dependency_stamp_digest = NULL
         WHERE singleton = 1",
        params![empty_frontier_digest.as_bytes().as_slice()],
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PageCleanupInstrumentation {
    existing_pages: usize,
    owned_rows: usize,
    fts_rowids: usize,
}

fn delete_page(
    transaction: &Connection,
    page_id: [u8; 16],
    remove_incoming_page_references: bool,
    retained_blocks: &BTreeSet<[u8; 16]>,
) -> Result<PageCleanupInstrumentation, MaterializationError> {
    let page = &page_id;
    let existing: i64 = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM pages WHERE page_id = ?1)",
        params![page.as_slice()],
        |row| row.get(0),
    )?;
    let mut instrumentation = PageCleanupInstrumentation {
        existing_pages: usize::from(existing != 0),
        ..PageCleanupInstrumentation::default()
    };
    let old_blocks = {
        let mut statement =
            transaction.prepare("SELECT block_id FROM blocks WHERE page_id = ?1")?;
        let block_ids = statement
            .query_map(params![page.as_slice()], |row| row.get::<_, Vec<u8>>(0))?
            .map(|block_id| {
                block_id
                    .map_err(MaterializationError::from)
                    .and_then(|bytes| decode_id(&bytes))
            })
            .collect::<Result<Vec<_>, _>>()?;
        block_ids
    };
    let fts_rowids = {
        let mut statement = transaction
            .prepare("SELECT rowid FROM search_fts_owners WHERE page_id = ?1 ORDER BY rowid")?;
        let rowids = statement
            .query_map(params![page.as_slice()], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rowids
    };
    instrumentation.fts_rowids = fts_rowids.len();
    for rowid in fts_rowids {
        transaction.execute("DELETE FROM search_fts WHERE rowid = ?1", params![rowid])?;
    }
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM search_fts_owners WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM refs
         WHERE source_page_id = ?1",
            params![page.as_slice()],
        )?);
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
                params![block_id.as_slice()],
            )?;
        }
    }
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM properties WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM tags WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM tasks WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM blocks WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    instrumentation.owned_rows = instrumentation
        .owned_rows
        .saturating_add(transaction.execute(
            "DELETE FROM pages WHERE page_id = ?1",
            params![page.as_slice()],
        )?);
    Ok(instrumentation)
}

/// Execute one materialized row insert through the connection's
/// prepared-statement cache.
///
/// A graph-sized build runs the same handful of insert statements once per
/// page, block, and facet, so re-preparing each one per row dominates it. The
/// SQL text, parameters, and owning transaction are unchanged.
fn execute_cached(
    transaction: &Connection,
    sql: &str,
    parameters: &[&dyn rusqlite::ToSql],
) -> Result<usize, MaterializationError> {
    Ok(transaction.prepare_cached(sql)?.execute(parameters)?)
}

fn insert_page(transaction: &Connection, page: &PhysicalPage) -> Result<(), MaterializationError> {
    let page_id = &page.page_id;
    execute_cached(
        transaction,
        "INSERT INTO pages (
             page_id, home_document_id, name, name_key, path, text_kind,
             preamble, searchable_text
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            page_id.as_slice(),
            page.home_document_id.as_slice(),
            &page.name,
            &page.name_key,
            page.path.as_str(),
            page.text_kind,
            &page.preamble,
            &page.searchable_text,
        ],
    )?;
    insert_fts(
        transaction,
        "page",
        page.page_id,
        page.page_id,
        &page.searchable_text,
    )?;
    insert_references(
        transaction,
        PhysicalEntityId::Page(page.page_id),
        page.page_id,
        &page.references,
    )?;
    insert_properties(
        transaction,
        PhysicalEntityId::Page(page.page_id),
        page.page_id,
        &page.properties,
    )?;
    insert_tags(
        transaction,
        PhysicalEntityId::Page(page.page_id),
        page.page_id,
        &page.tags,
    )?;
    for block in &page.blocks {
        insert_block(transaction, page.page_id, block)?;
    }
    Ok(())
}

fn insert_block(
    transaction: &Connection,
    page_id: [u8; 16],
    block: &PhysicalBlock,
) -> Result<(), MaterializationError> {
    let (logseq_uuid, origin) = match (block.logseq_uuid, block.logseq_identity_origin) {
        (Some(uuid), Some(origin)) => (Some(uuid.to_vec()), Some(origin)),
        (None, None) => (None, None),
        _ => {
            return Err(MaterializationError::InvalidInput(format!(
                "block {} has incomplete Logseq identity metadata",
                uuid::Uuid::from_bytes(block.block_id)
            )));
        }
    };
    execute_cached(
        transaction,
        "INSERT INTO blocks (
             block_id, page_id, home_document_id, parent_block_id, order_key,
             content, searchable_text, heading_level, collapsed, logseq_uuid,
             logseq_identity_origin
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            block.block_id.as_slice(),
            page_id.as_slice(),
            block.home_document_id.as_slice(),
            block.parent.map(|parent| parent.to_vec()),
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
        block.block_id,
        page_id,
        &block.searchable_text,
    )?;
    let owner = PhysicalEntityId::Block(block.block_id);
    insert_references(transaction, owner, page_id, &block.references)?;
    insert_properties(transaction, owner, page_id, &block.properties)?;
    insert_tags(transaction, owner, page_id, &block.tags)?;
    if let Some(task) = &block.task {
        execute_cached(
            transaction,
            "INSERT INTO tasks (
                 block_id, page_id, marker, priority, scheduled, deadline
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                block.block_id.as_slice(),
                page_id.as_slice(),
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
    transaction: &Connection,
    entity_type: &str,
    entity_id: [u8; 16],
    page_id: [u8; 16],
    text: &str,
) -> Result<(), MaterializationError> {
    let entity_type_value = match entity_type {
        "page" => 0_i64,
        "block" => 1_i64,
        _ => {
            return Err(MaterializationError::InvalidInput(
                "unknown FTS entity type".into(),
            ));
        }
    };
    execute_cached(
        transaction,
        "INSERT INTO search_fts_owners (entity_type, entity_id, page_id)
         VALUES (?1, ?2, ?3)",
        params![entity_type_value, entity_id.as_slice(), page_id.as_slice(),],
    )?;
    let rowid = transaction.last_insert_rowid();
    execute_cached(
        transaction,
        "INSERT INTO search_fts (rowid, entity_type, entity_id, page_id, text)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            rowid,
            entity_type,
            uuid::Uuid::from_bytes(entity_id).simple().to_string(),
            uuid::Uuid::from_bytes(page_id).simple().to_string(),
            text,
        ],
    )?;
    Ok(())
}

fn insert_references(
    transaction: &Connection,
    source: PhysicalEntityId,
    source_page_id: [u8; 16],
    references: &[PhysicalReference],
) -> Result<(), MaterializationError> {
    let (source_type, source_id) = source.sql_parts();
    for (ordinal, reference) in references.iter().enumerate() {
        let (target_type, target_id) = reference.target.sql_parts();
        execute_cached(
            transaction,
            "INSERT INTO refs (
                 source_type, source_id, source_page_id, target_type, target_id,
                 reference_kind, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                source_type,
                source_id.as_slice(),
                source_page_id.as_slice(),
                target_type,
                target_id.as_slice(),
                reference.kind,
                i64::try_from(ordinal).map_err(|_| {
                    MaterializationError::InvalidInput("reference ordinal overflowed".into())
                })?,
            ],
        )?;
    }
    Ok(())
}

fn insert_properties(
    transaction: &Connection,
    owner: PhysicalEntityId,
    page_id: [u8; 16],
    properties: &[PhysicalProperty],
) -> Result<(), MaterializationError> {
    let (owner_type, owner_id) = owner.sql_parts();
    for (ordinal, property) in properties.iter().enumerate() {
        execute_cached(
            transaction,
            "INSERT INTO properties (
                 owner_type, owner_id, page_id, name, value, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                owner_type,
                owner_id.as_slice(),
                page_id.as_slice(),
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
    transaction: &Connection,
    owner: PhysicalEntityId,
    page_id: [u8; 16],
    tags: &[String],
) -> Result<(), MaterializationError> {
    let (owner_type, owner_id) = owner.sql_parts();
    for (ordinal, tag) in tags.iter().enumerate() {
        execute_cached(
            transaction,
            "INSERT INTO tags (owner_type, owner_id, page_id, tag, ordinal)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                owner_type,
                owner_id.as_slice(),
                page_id.as_slice(),
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
pub struct PhysicalPageRow {
    pub page_id: [u8; 16],
    pub home_document_id: [u8; 16],
    pub name: String,
    pub name_key: String,
    pub path: String,
    pub text_kind: i64,
    pub preamble: Option<String>,
    pub searchable_text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalPageInventoryRow {
    pub page_id: [u8; 16],
    pub name: String,
    pub path: String,
    pub text_kind: i64,
}

/// Lightweight page row for navigation/autocomplete.  It deliberately omits
/// searchable body text so a title lookup never retains graph-sized content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalNavigationPageRow {
    pub page_id: [u8; 16],
    pub name: String,
    pub name_key: String,
    pub path: String,
    pub text_kind: i64,
    pub preamble: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalNavigationAliasRow {
    pub source_page_id: [u8; 16],
    pub owner_name: String,
    pub owner_path: String,
    pub normalized_alias: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalNavigationReferenceNameRow {
    pub source_page_id: [u8; 16],
    pub owner_path: String,
    pub raw_name: String,
    pub normalized_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalBlockRow {
    pub block_id: [u8; 16],
    pub page_id: [u8; 16],
    pub home_document_id: [u8; 16],
    pub parent: Option<[u8; 16]>,
    pub order: String,
    pub content: String,
    pub searchable_text: String,
    pub heading_level: Option<u8>,
    pub collapsed: bool,
    pub logseq_uuid: Option<[u8; 16]>,
    pub logseq_identity_origin: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalReferrerRow {
    pub source: PhysicalEntityId,
    pub source_page_id: [u8; 16],
    pub kind: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalBlockReferenceCountRow {
    pub raw_uuid_claim: [u8; 16],
    pub distinct_source_blocks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalBlockReferrerCandidateRow {
    pub source_page_id: [u8; 16],
    pub source_block_id: [u8; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalPageReferrerCandidateRow {
    pub source_page_id: [u8; 16],
    pub source: PhysicalEntityId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalPropertyRow {
    pub owner: PhysicalEntityId,
    pub page_id: [u8; 16],
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalTagRow {
    pub owner: PhysicalEntityId,
    pub page_id: [u8; 16],
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalTaskRow {
    pub block_id: [u8; 16],
    pub page_id: [u8; 16],
    pub marker: String,
    pub priority: Option<String>,
    pub scheduled: Option<String>,
    pub deadline: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicalSearchHit {
    pub entity: PhysicalEntityId,
    pub page_id: [u8; 16],
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

fn page_row_output_bytes(row: &PhysicalPageRow) -> Result<usize, MaterializationError> {
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

fn page_inventory_row_output_bytes(
    row: &PhysicalPageInventoryRow,
) -> Result<usize, MaterializationError> {
    checked_output_bytes(32, [Some(row.name.as_str()), Some(row.path.as_str())])
}

fn navigation_page_row_output_bytes(
    row: &PhysicalNavigationPageRow,
) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        32,
        [
            Some(row.name.as_str()),
            Some(row.name_key.as_str()),
            Some(row.path.as_str()),
            row.preamble.as_deref(),
        ],
    )
}

fn navigation_alias_row_output_bytes(
    row: &PhysicalNavigationAliasRow,
) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        16,
        [
            Some(row.owner_name.as_str()),
            Some(row.owner_path.as_str()),
            Some(row.normalized_alias.as_str()),
        ],
    )
}

fn navigation_reference_name_row_output_bytes(
    row: &PhysicalNavigationReferenceNameRow,
) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        0,
        [
            Some(row.owner_path.as_str()),
            Some(row.raw_name.as_str()),
            Some(row.normalized_name.as_str()),
        ],
    )
}

fn block_row_output_bytes(row: &PhysicalBlockRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(
        96,
        [
            Some(row.order.as_str()),
            Some(row.content.as_str()),
            Some(row.searchable_text.as_str()),
        ],
    )
}

fn referrer_row_output_bytes(_: &PhysicalReferrerRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [])
}

fn property_row_output_bytes(row: &PhysicalPropertyRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [Some(row.name.as_str()), Some(row.value.as_str())])
}

fn tag_row_output_bytes(row: &PhysicalTagRow) -> Result<usize, MaterializationError> {
    checked_output_bytes(64, [Some(row.tag.as_str())])
}

fn task_row_output_bytes(row: &PhysicalTaskRow) -> Result<usize, MaterializationError> {
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

fn search_hit_output_bytes(row: &PhysicalSearchHit) -> Result<usize, MaterializationError> {
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

fn allow_any_page_header(_path: &str, _kind: i64) -> Result<(), MaterializationError> {
    Ok(())
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

    /// Construct a read view from a test-owned connection.
    ///
    /// Production callers must obtain views from `PhysicalSqliteDatabase` so
    /// the storage crate retains ownership of the live connection.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn from_connection_for_test(
        connection: &'a Connection,
        acceptance_sequence: u64,
        frontier_digest: ContentDigest,
    ) -> Result<Self, MaterializationError> {
        Self::new(connection, acceptance_sequence, frontier_digest)
    }

    pub const fn acceptance_sequence(&self) -> u64 {
        self.acceptance_sequence
    }

    pub fn page(&self, page_id: [u8; 16]) -> Result<Option<PhysicalPageRow>, MaterializationError> {
        self.page_with_header_validation(page_id, allow_any_page_header)
    }

    pub fn page_with_header_validation(
        &self,
        page_id: [u8; 16],
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Option<PhysicalPageRow>, MaterializationError> {
        let page = self
            .connection
            .query_row(
                "SELECT page_id, home_document_id, name, name_key, path,
                        text_kind, preamble, searchable_text
                 FROM pages WHERE page_id = ?1",
                params![page_id.as_slice()],
                |row| page_row_with_header_validation(row, &mut validate_header),
            )
            .optional()
            .map_err(MaterializationError::from)?;
        let page = page.transpose()?;
        if let Some(row) = &page {
            let mut budget = MaterializationReadBudget::default();
            budget.add(page_row_output_bytes(row)?)?;
        }
        Ok(page)
    }

    pub fn block(
        &self,
        block_id: [u8; 16],
    ) -> Result<Option<PhysicalBlockRow>, MaterializationError> {
        let block = self
            .connection
            .query_row(
                "SELECT block_id, page_id, home_document_id, parent_block_id,
                        order_key, content, searchable_text, heading_level,
                        collapsed, logseq_uuid, logseq_identity_origin
                 FROM blocks WHERE block_id = ?1",
                params![block_id.as_slice()],
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

    /// Point-locate the one accepted block that owns a public Logseq UUID.
    /// The partial unique index makes ambiguity a construction error rather
    /// than a read-time choice; semantic callers must still verify the exact
    /// parser-owned page before exposing the candidate.
    pub fn block_by_logseq_uuid(
        &self,
        logseq_uuid: [u8; 16],
    ) -> Result<Option<PhysicalBlockRow>, MaterializationError> {
        let block = self
            .connection
            .query_row(
                "SELECT block_id, page_id, home_document_id, parent_block_id,
                        order_key, content, searchable_text, heading_level,
                        collapsed, logseq_uuid, logseq_identity_origin
                 FROM blocks WHERE logseq_uuid = ?1",
                params![logseq_uuid.as_slice()],
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
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_name_with_header_validation(name, limit, allow_any_page_header)
    }

    pub fn pages_by_name_with_header_validation(
        &self,
        name: &str,
        limit: usize,
        validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_text_column_with_header_validation("name", name, limit, validate_header)
    }

    pub fn pages_by_name_key(
        &self,
        name_key: &str,
        limit: usize,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_name_key_with_header_validation(name_key, limit, allow_any_page_header)
    }

    pub fn pages_by_name_key_with_header_validation(
        &self,
        name_key: &str,
        limit: usize,
        validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_text_column_with_header_validation(
            "name_key",
            name_key,
            limit,
            validate_header,
        )
    }

    /// Exact OG-compatible logical-name lookup scoped by managed text kind.
    /// Callers use a limit of two to distinguish one owner from ambiguity
    /// without scanning or retaining an unbounded duplicate set.
    pub fn pages_by_name_key_and_kind(
        &self,
        name_key: &str,
        kind: i64,
        limit: usize,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_name_key_and_kind_with_header_validation(
            name_key,
            kind,
            limit,
            allow_any_page_header,
        )
    }

    pub fn pages_by_name_key_and_kind_with_header_validation(
        &self,
        name_key: &str,
        kind: i64,
        limit: usize,
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(name_key)?;
        let mut statement = self.connection.prepare(
            "SELECT page_id, home_document_id, name, name_key, path,
                    text_kind, preamble, searchable_text
             FROM pages
             WHERE name_key = ?1 AND text_kind = ?2
             ORDER BY page_id LIMIT ?3",
        )?;
        let rows = statement.query_map(params![name_key, kind, limit], |row| {
            page_row_with_header_validation(row, &mut validate_header)
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from).and_then(|row| row)),
            page_row_output_bytes,
        )
    }

    pub fn pages_by_path(
        &self,
        path: &String,
        limit: usize,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_path_with_header_validation(path, limit, allow_any_page_header)
    }

    pub fn pages_by_path_with_header_validation(
        &self,
        path: &String,
        limit: usize,
        validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_by_text_column_with_header_validation(
            "path",
            path.as_str(),
            limit,
            validate_header,
        )
    }

    /// Bounded stable page listing for application-facing exact queries. This
    /// only reads the stamped materialization captured on construction; it is
    /// intentionally not a filesystem or graph-tree enumeration.
    pub fn pages(
        &self,
        kind: Option<i64>,
        limit: usize,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        self.pages_with_header_validation(kind, limit, allow_any_page_header)
    }

    pub fn pages_with_header_validation(
        &self,
        kind: Option<i64>,
        limit: usize,
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match kind {
            Some(kind) => (
                "SELECT page_id, home_document_id, name, name_key, path,
                        text_kind, preamble, searchable_text
                 FROM pages WHERE text_kind = ?1 ORDER BY path, page_id LIMIT ?2",
                vec![kind.into(), limit.into()],
            ),
            None => (
                "SELECT page_id, home_document_id, name, name_key, path,
                        text_kind, preamble, searchable_text
                 FROM pages ORDER BY path, page_id LIMIT ?1",
                vec![limit.into()],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            page_row_with_header_validation(row, &mut validate_header)
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from).and_then(|row| row)),
            page_row_output_bytes,
        )
    }

    /// Stable bounded page-inventory pagination. The cursor is the final
    /// `(path, page_id)` returned by the preceding call.
    pub fn page_inventory_after_with_header_validation(
        &self,
        after_path: Option<&str>,
        after_page_id: Option<&[u8; 16]>,
        kind: Option<i64>,
        limit: usize,
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageInventoryRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        if after_path.is_some() != after_page_id.is_some() {
            return Err(MaterializationError::InvalidQuery(
                "page inventory cursor requires both path and page ID".into(),
            ));
        }
        if let Some(path) = after_path {
            checked_query_text(path)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) =
            match (after_path, after_page_id, kind) {
                (None, None, None) => (
                    "SELECT page_id, name, path, text_kind
                     FROM pages ORDER BY path, page_id LIMIT ?1",
                    vec![limit.into()],
                ),
                (None, None, Some(kind)) => (
                    "SELECT page_id, name, path, text_kind
                     FROM pages WHERE text_kind = ?1
                     ORDER BY path, page_id LIMIT ?2",
                    vec![kind.into(), limit.into()],
                ),
                (Some(path), Some(page_id), None) => (
                    "SELECT page_id, name, path, text_kind
                     FROM pages
                     WHERE path > ?1 OR (path = ?1 AND page_id > ?2)
                     ORDER BY path, page_id LIMIT ?3",
                    vec![
                        path.to_owned().into(),
                        page_id.to_vec().into(),
                        limit.into(),
                    ],
                ),
                (Some(path), Some(page_id), Some(kind)) => (
                    "SELECT page_id, name, path, text_kind
                     FROM pages
                     WHERE text_kind = ?1
                       AND (path > ?2 OR (path = ?2 AND page_id > ?3))
                     ORDER BY path, page_id LIMIT ?4",
                    vec![
                        kind.into(),
                        path.to_owned().into(),
                        page_id.to_vec().into(),
                        limit.into(),
                    ],
                ),
                _ => unreachable!("cursor presence was validated above"),
            };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            page_inventory_row_with_header_validation(row, &mut validate_header)
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from).and_then(|row| row)),
            page_inventory_row_output_bytes,
        )
    }

    /// Stable pagination over the small page fields needed by navigation.
    /// Body/search text is deliberately excluded.
    pub fn navigation_pages_after_with_header_validation(
        &self,
        after_path: Option<&str>,
        after_page_id: Option<&[u8; 16]>,
        limit: usize,
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalNavigationPageRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        if after_path.is_some() != after_page_id.is_some() {
            return Err(MaterializationError::InvalidQuery(
                "navigation page cursor requires both path and page ID".into(),
            ));
        }
        if let Some(path) = after_path {
            checked_query_text(path)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match (after_path, after_page_id) {
            (None, None) => (
                "SELECT page_id, name, name_key, path, text_kind, preamble
                     FROM pages ORDER BY path, page_id LIMIT ?1",
                vec![limit.into()],
            ),
            (Some(path), Some(page_id)) => (
                "SELECT page_id, name, name_key, path, text_kind, preamble
                     FROM pages
                     WHERE path > ?1 OR (path = ?1 AND page_id > ?2)
                     ORDER BY path, page_id LIMIT ?3",
                vec![
                    path.to_owned().into(),
                    page_id.to_vec().into(),
                    limit.into(),
                ],
            ),
            _ => unreachable!("cursor presence was validated above"),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            navigation_page_row_with_header_validation(row, &mut validate_header)
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from).and_then(|row| row)),
            navigation_page_row_output_bytes,
        )
    }

    /// Stable, deduplicated alias declarations joined to their owning page.
    /// The cursor is the final `(owner_path, normalized_alias, source_page_id)`.
    pub fn navigation_aliases_after(
        &self,
        after: Option<(&str, &str, &[u8; 16])>,
        limit: usize,
    ) -> Result<Vec<PhysicalNavigationAliasRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        if let Some((path, alias, _)) = after {
            checked_query_text(path)?;
            checked_query_text(alias)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match after {
            None => (
                "SELECT DISTINCT d.source_page_id, p.name, p.path, d.normalized_alias
                 FROM reference_alias_declarations d
                 JOIN pages p ON p.page_id = d.source_page_id
                 ORDER BY p.path, d.normalized_alias, d.source_page_id LIMIT ?1",
                vec![limit.into()],
            ),
            Some((path, alias, page_id)) => (
                "SELECT DISTINCT d.source_page_id, p.name, p.path, d.normalized_alias
                 FROM reference_alias_declarations d
                 JOIN pages p ON p.page_id = d.source_page_id
                 WHERE p.path > ?1
                    OR (p.path = ?1 AND d.normalized_alias > ?2)
                    OR (p.path = ?1 AND d.normalized_alias = ?2 AND d.source_page_id > ?3)
                 ORDER BY p.path, d.normalized_alias, d.source_page_id LIMIT ?4",
                vec![
                    path.to_owned().into(),
                    alias.to_owned().into(),
                    page_id.to_vec().into(),
                    limit.into(),
                ],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            let page_id: Vec<u8> = row.get(0)?;
            Ok(PhysicalNavigationAliasRow {
                source_page_id: decode_id_sql(&page_id)?,
                owner_name: row.get(1)?,
                owner_path: row.get(2)?,
                normalized_alias: row.get(3)?,
            })
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from)),
            navigation_alias_row_output_bytes,
        )
    }

    /// Stable distinct page-reference spellings. Property-key pseudo pages are
    /// excluded because the legacy navigation surface never advertised them.
    pub fn navigation_reference_names_after(
        &self,
        after: Option<(&str, &str, &str, &[u8; 16])>,
        limit: usize,
    ) -> Result<Vec<PhysicalNavigationReferenceNameRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        if let Some((path, raw, normalized, _)) = after {
            checked_query_text(path)?;
            checked_query_text(raw)?;
            checked_query_text(normalized)?;
        }
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match after {
            None => (
                "SELECT DISTINCT r.source_page_id, p.path, r.raw_name, r.normalized_name
                 FROM reference_postings r JOIN pages p ON p.page_id = r.source_page_id
                 WHERE r.target_type = 0 AND r.reference_kind <= 4
                 ORDER BY p.path, r.raw_name, r.normalized_name, r.source_page_id LIMIT ?1",
                vec![limit.into()],
            ),
            Some((path, raw, normalized, page_id)) => (
                "SELECT DISTINCT r.source_page_id, p.path, r.raw_name, r.normalized_name
                 FROM reference_postings r JOIN pages p ON p.page_id = r.source_page_id
                 WHERE r.target_type = 0 AND r.reference_kind <= 4
                   AND (p.path > ?1
                     OR (p.path = ?1 AND r.raw_name > ?2)
                     OR (p.path = ?1 AND r.raw_name = ?2 AND r.normalized_name > ?3)
                     OR (p.path = ?1 AND r.raw_name = ?2 AND r.normalized_name = ?3
                         AND r.source_page_id > ?4))
                 ORDER BY p.path, r.raw_name, r.normalized_name, r.source_page_id LIMIT ?5",
                vec![
                    path.to_owned().into(),
                    raw.to_owned().into(),
                    normalized.to_owned().into(),
                    page_id.to_vec().into(),
                    limit.into(),
                ],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            let page_id: Vec<u8> = row.get(0)?;
            Ok(PhysicalNavigationReferenceNameRow {
                source_page_id: decode_id_sql(&page_id)?,
                owner_path: row.get(1)?,
                raw_name: row.get(2)?,
                normalized_name: row.get(3)?,
            })
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from)),
            navigation_reference_name_row_output_bytes,
        )
    }

    fn pages_by_text_column_with_header_validation(
        &self,
        column: &str,
        value: &str,
        limit: usize,
        mut validate_header: impl FnMut(&str, i64) -> Result<(), MaterializationError>,
    ) -> Result<Vec<PhysicalPageRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(value)?;
        let sql = format!(
            "SELECT page_id, home_document_id, name, name_key, path,
                    text_kind, preamble, searchable_text
             FROM pages WHERE {column} = ?1 ORDER BY page_id LIMIT ?2"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![value, limit], |row| {
            page_row_with_header_validation(row, &mut validate_header)
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from).and_then(|row| row)),
            page_row_output_bytes,
        )
    }

    pub fn blocks_on_page(
        &self,
        page_id: [u8; 16],
        limit: usize,
    ) -> Result<Vec<PhysicalBlockRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT block_id, page_id, home_document_id, parent_block_id,
                    order_key, content, searchable_text, heading_level,
                    collapsed, logseq_uuid, logseq_identity_origin
             FROM blocks WHERE page_id = ?1
             ORDER BY order_key, block_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![page_id.as_slice(), limit], block_row)?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from)),
            block_row_output_bytes,
        )
    }

    pub fn referrers_to(
        &self,
        target: PhysicalEntityId,
        limit: usize,
    ) -> Result<Vec<PhysicalReferrerRow>, MaterializationError> {
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
            Ok(PhysicalReferrerRow {
                source: decode_entity(source_type, &source_id)?,
                source_page_id: decode_id(&source_page_id)?,
                kind,
            })
        });
        collect_read_rows(rows, referrer_row_output_bytes)
    }

    /// Aggregate raw UUID postings by distinct source block. Raw claims are
    /// used deliberately: a dangling `((uuid))` still drives a badge if a
    /// matching block later appears.
    pub fn block_reference_counts_after(
        &self,
        after: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<PhysicalBlockReferenceCountRow>, MaterializationError> {
        self.block_reference_counts_query(None, after, limit)
    }

    pub fn block_reference_counts_for_source_page_after(
        &self,
        source_page_id: [u8; 16],
        after: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<PhysicalBlockReferenceCountRow>, MaterializationError> {
        self.block_reference_counts_query(Some(source_page_id), after, limit)
    }

    fn block_reference_counts_query(
        &self,
        source_page_id: Option<[u8; 16]>,
        after: Option<[u8; 16]>,
        limit: usize,
    ) -> Result<Vec<PhysicalBlockReferenceCountRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match (source_page_id, after) {
            (None, None) => (
                "SELECT raw_uuid_claim, COUNT(DISTINCT source_entity_id)
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                 GROUP BY raw_uuid_claim ORDER BY raw_uuid_claim LIMIT ?1",
                vec![limit.into()],
            ),
            (None, Some(after)) => (
                "SELECT raw_uuid_claim, COUNT(DISTINCT source_entity_id)
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                   AND raw_uuid_claim > ?1
                 GROUP BY raw_uuid_claim ORDER BY raw_uuid_claim LIMIT ?2",
                vec![after.to_vec().into(), limit.into()],
            ),
            (Some(page_id), None) => (
                "SELECT raw_uuid_claim, COUNT(DISTINCT source_entity_id)
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                   AND source_page_id = ?1
                 GROUP BY raw_uuid_claim ORDER BY raw_uuid_claim LIMIT ?2",
                vec![page_id.to_vec().into(), limit.into()],
            ),
            (Some(page_id), Some(after)) => (
                "SELECT raw_uuid_claim, COUNT(DISTINCT source_entity_id)
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                   AND source_page_id = ?1 AND raw_uuid_claim > ?2
                 GROUP BY raw_uuid_claim ORDER BY raw_uuid_claim LIMIT ?3",
                vec![page_id.to_vec().into(), after.to_vec().into(), limit.into()],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            let uuid: Vec<u8> = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok(PhysicalBlockReferenceCountRow {
                raw_uuid_claim: decode_id_sql(&uuid)?,
                distinct_source_blocks: u64::try_from(count).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Integer,
                        "negative block-reference count".into(),
                    )
                })?,
            })
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from)),
            |_| Ok(32),
        )
    }

    pub fn block_referrer_candidates_after(
        &self,
        raw_uuid_claim: [u8; 16],
        after: Option<([u8; 16], [u8; 16])>,
        limit: usize,
    ) -> Result<Vec<PhysicalBlockReferrerCandidateRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match after {
            None => (
                "SELECT DISTINCT source_page_id, source_entity_id
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                   AND raw_uuid_claim = ?1
                 ORDER BY source_page_id, source_entity_id LIMIT ?2",
                vec![raw_uuid_claim.to_vec().into(), limit.into()],
            ),
            Some((page_id, block_id)) => (
                "SELECT DISTINCT source_page_id, source_entity_id
                 FROM reference_postings
                 WHERE target_type = 1 AND source_entity_type = 1
                   AND raw_uuid_claim = ?1
                   AND (source_page_id > ?2
                     OR (source_page_id = ?2 AND source_entity_id > ?3))
                 ORDER BY source_page_id, source_entity_id LIMIT ?4",
                vec![
                    raw_uuid_claim.to_vec().into(),
                    page_id.to_vec().into(),
                    block_id.to_vec().into(),
                    limit.into(),
                ],
            ),
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            let page_id: Vec<u8> = row.get(0)?;
            let block_id: Vec<u8> = row.get(1)?;
            Ok(PhysicalBlockReferrerCandidateRow {
                source_page_id: decode_id_sql(&page_id)?,
                source_block_id: decode_id_sql(&block_id)?,
            })
        })?;
        collect_read_rows(
            rows.map(|row| row.map_err(MaterializationError::from)),
            |_| Ok(32),
        )
    }

    /// Stable source candidates for one normalized explicit page-reference
    /// target. Property-key pseudo pages are not backlinks. Duplicate syntax
    /// occurrences collapse to one source entity; the parser-owned application
    /// page verifies exact membership before exposure.
    pub fn page_referrer_candidates_after(
        &self,
        normalized_name: &str,
        after: Option<([u8; 16], PhysicalEntityId)>,
        limit: usize,
    ) -> Result<Vec<PhysicalPageReferrerCandidateRow>, MaterializationError> {
        let limit = checked_limit(limit)?;
        checked_query_text(normalized_name)?;
        let (sql, args): (&str, Vec<rusqlite::types::Value>) = match after {
            None => (
                "SELECT DISTINCT source_page_id, source_entity_type, source_entity_id
                 FROM reference_postings
                 WHERE target_type = 0 AND reference_kind <= 4
                   AND normalized_name = ?1
                 ORDER BY source_page_id, source_entity_type, source_entity_id LIMIT ?2",
                vec![normalized_name.to_owned().into(), limit.into()],
            ),
            Some((page_id, source)) => {
                let (source_type, source_id) = source.sql_parts();
                (
                    "SELECT DISTINCT source_page_id, source_entity_type, source_entity_id
                     FROM reference_postings
                     WHERE target_type = 0 AND reference_kind <= 4
                       AND normalized_name = ?1
                       AND (source_page_id > ?2
                         OR (source_page_id = ?2 AND source_entity_type > ?3)
                         OR (source_page_id = ?2 AND source_entity_type = ?3
                             AND source_entity_id > ?4))
                     ORDER BY source_page_id, source_entity_type, source_entity_id LIMIT ?5",
                    vec![
                        normalized_name.to_owned().into(),
                        page_id.to_vec().into(),
                        source_type.into(),
                        source_id.to_vec().into(),
                        limit.into(),
                    ],
                )
            }
        };
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(args), |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        let rows = rows.map(
            |row| -> Result<PhysicalPageReferrerCandidateRow, MaterializationError> {
                let (page_id, source_type, source_id) = row.map_err(MaterializationError::from)?;
                Ok(PhysicalPageReferrerCandidateRow {
                    source_page_id: decode_id(&page_id)?,
                    source: decode_entity(source_type, &source_id)?,
                })
            },
        );
        collect_read_rows(rows, |_| Ok(32))
    }

    pub fn properties(
        &self,
        owner: PhysicalEntityId,
        limit: usize,
    ) -> Result<Vec<PhysicalPropertyRow>, MaterializationError> {
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
    ) -> Result<Vec<PhysicalPropertyRow>, MaterializationError> {
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
        let rows =
            property_rows(statement.query_map(rusqlite::params_from_iter(args), property_tuple)?);
        rows
    }

    pub fn tags(
        &self,
        tag: &str,
        limit: usize,
    ) -> Result<Vec<PhysicalTagRow>, MaterializationError> {
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
            Ok(PhysicalTagRow {
                owner: decode_entity(owner_type, &owner_id)?,
                page_id: decode_id(&page_id)?,
                tag,
            })
        });
        collect_read_rows(rows, tag_row_output_bytes)
    }

    pub fn tasks(
        &self,
        marker: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PhysicalTaskRow>, MaterializationError> {
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
            Ok(PhysicalTaskRow {
                block_id: decode_id(&block_id)?,
                page_id: decode_id(&page_id)?,
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
    ) -> Result<Vec<PhysicalSearchHit>, MaterializationError> {
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
            let uuid = uuid::Uuid::parse_str(&entity_id)
                .map_err(|error| MaterializationError::Corrupt(error.to_string()))?
                .into_bytes();
            let entity = match entity_type.as_str() {
                "page" => PhysicalEntityId::Page(uuid),
                "block" => PhysicalEntityId::Block(uuid),
                _ => {
                    return Err(MaterializationError::Corrupt(format!(
                        "unknown FTS entity type {entity_type:?}"
                    )));
                }
            };
            Ok(PhysicalSearchHit {
                entity,
                page_id: uuid::Uuid::parse_str(&page_id)
                    .map_err(|error| MaterializationError::Corrupt(error.to_string()))?
                    .into_bytes(),
                text,
                rank,
            })
        });
        collect_read_rows(rows, search_hit_output_bytes)
    }
}

fn page_row_with_header_validation(
    row: &rusqlite::Row<'_>,
    validate_header: &mut impl FnMut(&str, i64) -> Result<(), MaterializationError>,
) -> rusqlite::Result<Result<PhysicalPageRow, MaterializationError>> {
    let page_id: Vec<u8> = row.get(0)?;
    let home_document_id: Vec<u8> = row.get(1)?;
    let path: String = row.get(4)?;
    let kind: i64 = row.get(5)?;
    if let Err(error) = validate_header(path.as_str(), kind) {
        return Ok(Err(error));
    }
    Ok(Ok(PhysicalPageRow {
        page_id: decode_id_sql(&page_id)?,
        home_document_id: decode_id_sql(&home_document_id)?,
        name: row.get(2)?,
        name_key: row.get(3)?,
        path,
        text_kind: kind,
        preamble: row.get(6)?,
        searchable_text: row.get(7)?,
    }))
}

fn page_inventory_row_with_header_validation(
    row: &rusqlite::Row<'_>,
    validate_header: &mut impl FnMut(&str, i64) -> Result<(), MaterializationError>,
) -> rusqlite::Result<Result<PhysicalPageInventoryRow, MaterializationError>> {
    let page_id: Vec<u8> = row.get(0)?;
    let path: String = row.get(2)?;
    let kind: i64 = row.get(3)?;
    if let Err(error) = validate_header(path.as_str(), kind) {
        return Ok(Err(error));
    }
    Ok(Ok(PhysicalPageInventoryRow {
        page_id: decode_id_sql(&page_id)?,
        name: row.get(1)?,
        path,
        text_kind: kind,
    }))
}

fn navigation_page_row_with_header_validation(
    row: &rusqlite::Row<'_>,
    validate_header: &mut impl FnMut(&str, i64) -> Result<(), MaterializationError>,
) -> rusqlite::Result<Result<PhysicalNavigationPageRow, MaterializationError>> {
    let page_id: Vec<u8> = row.get(0)?;
    let path: String = row.get(3)?;
    let kind: i64 = row.get(4)?;
    if let Err(error) = validate_header(path.as_str(), kind) {
        return Ok(Err(error));
    }
    Ok(Ok(PhysicalNavigationPageRow {
        page_id: decode_id_sql(&page_id)?,
        name: row.get(1)?,
        name_key: row.get(2)?,
        path,
        text_kind: kind,
        preamble: row.get(5)?,
    }))
}

fn block_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PhysicalBlockRow> {
    let block_id: Vec<u8> = row.get(0)?;
    let page_id: Vec<u8> = row.get(1)?;
    let home_document_id: Vec<u8> = row.get(2)?;
    let parent: Option<Vec<u8>> = row.get(3)?;
    let heading_level: Option<i64> = row.get(7)?;
    let logseq_uuid: Option<Vec<u8>> = row.get(9)?;
    let origin: Option<i64> = row.get(10)?;
    Ok(PhysicalBlockRow {
        block_id: decode_id_sql(&block_id)?,
        page_id: decode_id_sql(&page_id)?,
        home_document_id: decode_id_sql(&home_document_id)?,
        parent: parent.as_deref().map(decode_id_sql).transpose()?,
        order: row.get(4)?,
        content: row.get(5)?,
        searchable_text: row.get(6)?,
        heading_level: heading_level
            .map(|value| u8::try_from(value).map_err(sql_decode_error))
            .transpose()?,
        collapsed: row.get::<_, i64>(8)? != 0,
        logseq_uuid: logseq_uuid.as_deref().map(decode_id_sql).transpose()?,
        logseq_identity_origin: origin,
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
) -> Result<Vec<PhysicalPropertyRow>, MaterializationError> {
    let rows = rows.map(|row| {
        let (owner_type, owner_id, page_id, name, value) = row?;
        Ok(PhysicalPropertyRow {
            owner: decode_entity(owner_type, &owner_id)?,
            page_id: decode_id(&page_id)?,
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

fn decode_entity(entity_type: i64, bytes: &[u8]) -> Result<PhysicalEntityId, MaterializationError> {
    match entity_type {
        0 => Ok(PhysicalEntityId::Page(decode_id(bytes)?)),
        1 => Ok(PhysicalEntityId::Block(decode_id(bytes)?)),
        _ => Err(MaterializationError::Corrupt(format!(
            "unknown entity type {entity_type}"
        ))),
    }
}

fn decode_id_sql(bytes: &[u8]) -> rusqlite::Result<[u8; 16]> {
    bytes.try_into().map_err(sql_decode_error)
}

fn sql_decode_error(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error))
}

fn decode_digest(bytes: Vec<u8>) -> Result<ContentDigest, MaterializationError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| MaterializationError::Corrupt("invalid digest length".into()))?;
    Ok(ContentDigest::from_bytes(bytes))
}

fn decode_id(bytes: &[u8]) -> Result<[u8; 16], MaterializationError> {
    bytes
        .try_into()
        .map_err(|_| MaterializationError::Corrupt("invalid UUID length".into()))
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
    Stale {
        materialized: u64,
        frontier: u64,
    },
    InvalidQuery(String),
}

impl fmt::Display for MaterializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "SQLite materialization error: {error}"),
            Self::Schema(error) => write!(f, "materialization schema mismatch: {error}"),
            Self::Corrupt(error) => write!(f, "corrupt materialization: {error}"),
            Self::ResourceLimit { resource, found, maximum } => write!(f, "materialization {resource} {found} exceeds limit {maximum}"),
            Self::InvalidInput(error) => write!(f, "invalid materialization input: {error}"),
            Self::Incomplete(error) => write!(f, "incomplete materialization input: {error}"),
            Self::Contradiction(error) => write!(f, "materialization contradicts accepted semantics: {error}"),
            Self::Stale { materialized, frontier } => write!(f, "materialization frontier {materialized} is stale against accepted frontier {frontier}"),
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

    fn id(value: u128) -> [u8; 16] {
        value.to_be_bytes()
    }

    fn digest(label: &[u8]) -> ContentDigest {
        ContentDigest::of(label)
    }

    fn page(value: u128, text: &str) -> PhysicalPage {
        let page_id = id(value);
        let block_id = id(value + 0x1000);
        PhysicalPage {
            page_id,
            home_document_id: id(value + 0x2000),
            name: format!("Page {value}"),
            name_key: format!("page {value}"),
            path: format!("pages/{value}.md"),
            text_kind: 0,
            preamble: Some("preamble".into()),
            searchable_text: text.into(),
            references: Vec::new(),
            properties: vec![PhysicalProperty {
                name: "category".into(),
                value: "test".into(),
            }],
            tags: vec!["storage".into()],
            blocks: vec![PhysicalBlock {
                block_id,
                home_document_id: id(value + 0x2000),
                parent: None,
                order: "a".into(),
                content: "block content".into(),
                searchable_text: format!("{text} block"),
                heading_level: None,
                collapsed: false,
                logseq_uuid: None,
                logseq_identity_origin: None,
                references: Vec::new(),
                properties: vec![PhysicalProperty {
                    name: "block-property".into(),
                    value: "value".into(),
                }],
                tags: vec!["block-tag".into()],
                task: Some(PhysicalTask {
                    marker: "TODO".into(),
                    priority: Some("A".into()),
                    scheduled: None,
                    deadline: None,
                }),
            }],
        }
    }

    fn change(
        batch: u128,
        replacements: Vec<PhysicalPage>,
        deletions: Vec<[u8; 16]>,
    ) -> PhysicalMaterializationChange {
        PhysicalMaterializationChange {
            batch_id: id(batch),
            pages_with_live_metadata_delta: replacements.iter().map(|page| page.page_id).collect(),
            replacements,
            deletions,
            reference_catalog: None,
        }
    }

    fn apply_and_commit(
        connection: &mut Connection,
        change: &PhysicalMaterializationChange,
        sequence: u64,
        frontier: ContentDigest,
    ) -> ApplyChangeInstrumentation {
        let transaction = connection.transaction().unwrap();
        let stats = apply_change(
            &transaction,
            change,
            sequence,
            digest(format!("input-{sequence}").as_bytes()),
            frontier,
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
        stats
    }

    fn assert_streaming_digest_matches_legacy(connection: &Connection) {
        assert_eq!(
            row_digest(connection).unwrap(),
            row_digest_legacy(connection).unwrap()
        );
    }

    #[test]
    fn streaming_row_digest_matches_legacy_across_materialized_surfaces() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();

        let mut first = page(0x101, "\u{200b}\u{00e9} searchable\0 boundary");
        first.name = "\u{00c5}ngstr\u{00f6}m \u{1f600}".into();
        first.name_key = "\u{00e5}ngstr\u{00f6}m \u{1f600}".into();
        first.path = "pages/\u{00c5}ngstr\u{00f6}m.md".into();
        first.preamble = Some("\u{0} preamble \u{1f642}".into());
        first.references = vec![PhysicalReference {
            target: PhysicalEntityId::Page(id(0x202)),
            kind: 3,
        }];
        first.properties = vec![PhysicalProperty {
            name: "\u{00fc}nicode".into(),
            value: "\u{0}value\u{1f680}".into(),
        }];
        first.tags = vec!["\u{00e9}tiquette".into()];
        first.blocks[0].references = vec![PhysicalReference {
            target: PhysicalEntityId::Block(id(0x1202)),
            kind: 2,
        }];
        first.blocks[0].properties = vec![PhysicalProperty {
            name: "edge".into(),
            value: "\u{0}\u{1f9ea}".into(),
        }];
        first.blocks[0].tags = vec!["\u{1f3f7}\u{fe0f}".into()];
        first.blocks[0].task = Some(PhysicalTask {
            marker: "TODO".into(),
            priority: Some("A".into()),
            scheduled: Some("2026-08-02".into()),
            deadline: Some("2026-08-03".into()),
        });

        let mut second = page(0x202, "replacement target \u{00e9}");
        second.name = "z\u{0}".into();
        second.name_key = "z\u{0}".into();
        second.path = "pages/z.md".into();
        apply_and_commit(
            &mut connection,
            &change(0x301, vec![second.clone(), first.clone()], Vec::new()),
            1,
            digest(b"frontier-1"),
        );

        connection
            .execute(
                "INSERT INTO reference_source_coverage (
                     source_page_id, source_digest, extractor_dependency_stamp_digest
                 ) VALUES (?1, ?2, ?3)",
                params![
                    first.page_id.as_slice(),
                    digest(b"source").as_bytes().as_slice(),
                    digest(b"extractor").as_bytes().as_slice(),
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO reference_postings (
                     source_page_id, source_entity_type, source_entity_id, source_locator,
                     ordinal, reference_kind, target_type, raw_name, normalized_name,
                     raw_uuid_claim, resolved_page_id, resolved_block_id
                 ) VALUES (?1, 0, ?1, ?2, 0, 0, 0, ?3, ?4, NULL, ?5, NULL)",
                params![
                    first.page_id.as_slice(),
                    [0_u8, 0xff].as_slice(),
                    "Alias \u{00c5}",
                    "alias \u{00e5}",
                    second.page_id.as_slice(),
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO reference_name_bindings (
                     raw_name, normalized_name, candidate_ordinal, resolved_page_id
                 ) VALUES (?1, ?2, 0, ?3)",
                params![
                    "Alias \u{00c5}",
                    "alias \u{00e5}",
                    second.page_id.as_slice()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO reference_uuid_bindings (
                     raw_uuid_claim, candidate_ordinal, resolved_block_id
                 ) VALUES (?1, 0, ?2)",
                params![id(0x444).as_slice(), first.blocks[0].block_id.as_slice()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO reference_alias_declarations (
                     source_page_id, source_entity_type, source_entity_id, source_locator,
                     ordinal, raw_alias, normalized_alias
                 ) VALUES (?1, 0, ?1, ?2, 0, ?3, ?4)",
                params![
                    first.page_id.as_slice(),
                    [0xff_u8, 0].as_slice(),
                    "\u{00c5}lias \u{0}",
                    "\u{00e5}lias \u{0}",
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO reference_alias_bindings (
                     normalized_alias, candidate_ordinal, resolved_page_id
                 ) VALUES (?1, 0, ?2)",
                params!["\u{00e5}lias \u{0}", first.page_id.as_slice()],
            )
            .unwrap();
        assert_streaming_digest_matches_legacy(&connection);

        first.searchable_text = "replacement \u{1f680}".into();
        first.blocks[0].content = "replacement block \u{0}".into();
        first.blocks[0].searchable_text = "replacement block \u{1f680}".into();
        first.properties[0].value = "replaced".into();
        first.tags = vec!["replaced".into()];
        apply_and_commit(
            &mut connection,
            &change(0x302, vec![first], vec![second.page_id]),
            2,
            digest(b"frontier-2"),
        );
        assert_streaming_digest_matches_legacy(&connection);
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM search_fts", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn canonical_row_sql_order_matches_legacy_bytes_for_all_sqlite_value_types() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE digest_value_types (value);
                 INSERT INTO digest_value_types (value) VALUES
                    (NULL), (0), (-1), (9223372036854775807),
                    (-9223372036854775808), (0.0), (-0.0),
                    (''), ('\u{00e9}'), (X''), (X'00FF');",
            )
            .unwrap();
        install_canonical_row_key_function(&connection).unwrap();

        let mut legacy = Vec::new();
        let mut statement = connection
            .prepare("SELECT value FROM digest_value_types")
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            let mut bytes = Vec::new();
            encode_len(&mut bytes, 1);
            let mut value = Vec::new();
            encode_sqlite_value(&mut value, row.get_ref(0).unwrap()).unwrap();
            encode_len(&mut bytes, value.len());
            bytes.extend_from_slice(&value);
            legacy.push(bytes);
        }
        legacy.sort_unstable();
        assert!(legacy
            .iter()
            .any(|row| row.ends_with(&[2, 0, 0, 0, 0, 0, 0, 0, 0])));
        assert!(legacy
            .iter()
            .any(|row| row.ends_with(&[2, 0x80, 0, 0, 0, 0, 0, 0, 0])));

        let mut statement = connection
            .prepare(
                "SELECT tine_materialization_canonical_row(value)
                 FROM digest_value_types
                 ORDER BY tine_materialization_canonical_row(value) COLLATE BINARY",
            )
            .unwrap();
        let ordered = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(ordered, legacy);
    }

    #[test]
    fn synthetic_apply_reads_typed_rows_and_exact_frontier() {
        let mut connection = Connection::open_in_memory().unwrap();
        let empty = digest(b"empty");
        let frontier = digest(b"frontier-1");
        initialize_schema(&connection, empty).unwrap();
        let mut page = page(1, "alpha searchable");
        let logseq_uuid = id(0xfeed);
        page.blocks[0].logseq_uuid = Some(logseq_uuid);
        page.blocks[0].logseq_identity_origin = Some(0);
        let block_id = page.blocks[0].block_id;
        let stats = apply_and_commit(
            &mut connection,
            &change(10, vec![page.clone()], Vec::new()),
            1,
            frontier,
        );

        assert_eq!(stats.cleanup_page_attempts, 1);
        assert_eq!(stats.cleanup_existing_pages, 0);
        ensure_stamp(&connection, 1, frontier).unwrap();
        assert_eq!(
            recorded_digest(&connection, 1).unwrap(),
            Some(digest(b"input-1"))
        );

        let read = SqliteMaterializedRead::new(&connection, 1, frontier).unwrap();
        assert_eq!(read.page(page.page_id).unwrap().unwrap().name, page.name);
        assert_eq!(
            read.block(block_id).unwrap().unwrap().content,
            "block content"
        );
        assert_eq!(
            read.block_by_logseq_uuid(logseq_uuid)
                .unwrap()
                .unwrap()
                .block_id,
            block_id
        );
        assert_eq!(
            read.properties(PhysicalEntityId::Page(page.page_id), 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(read.tags("storage", 10).unwrap().len(), 1);
        assert_eq!(read.tasks(Some("TODO"), 10).unwrap().len(), 1);
        let hits = read.search("alpha", 10).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.page_id == page.page_id));
    }

    #[test]
    fn logseq_uuid_index_rejects_duplicate_owners_atomically() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let claimed = id(0xbeef);
        let mut first = page(101, "first");
        first.blocks[0].logseq_uuid = Some(claimed);
        first.blocks[0].logseq_identity_origin = Some(0);
        let mut second = page(102, "second");
        second.blocks[0].logseq_uuid = Some(claimed);
        second.blocks[0].logseq_identity_origin = Some(0);
        let transaction = connection.transaction().unwrap();
        assert!(apply_change(
            &transaction,
            &change(0x1234, vec![first, second], Vec::new()),
            1,
            digest(b"input"),
            digest(b"frontier"),
            None,
        )
        .is_err());
        transaction.rollback().unwrap();
        let read = SqliteMaterializedRead::new(&connection, 0, digest(b"empty")).unwrap();
        assert!(read.block_by_logseq_uuid(claimed).unwrap().is_none());
    }

    #[test]
    fn raw_block_reference_queries_count_distinct_sources_and_page_cursors() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let first = page(201, "first");
        let second = page(202, "second");
        let first_page = first.page_id;
        let first_block = first.blocks[0].block_id;
        let second_page = second.page_id;
        let second_block = second.blocks[0].block_id;
        apply_and_commit(
            &mut connection,
            &change(0x6789, vec![first, second], Vec::new()),
            1,
            digest(b"frontier"),
        );
        let target = id(0xaaaa);
        let other = id(0xbbbb);
        for (page_id, block_id, locator, ordinal, claim) in [
            (first_page, first_block, b"first-a".as_slice(), 0, target),
            (first_page, first_block, b"first-b".as_slice(), 1, target),
            (second_page, second_block, b"second-a".as_slice(), 0, target),
            (second_page, second_block, b"second-b".as_slice(), 1, other),
        ] {
            connection
                .execute(
                    "INSERT INTO reference_postings (
                         source_page_id, source_entity_type, source_entity_id,
                         source_locator, ordinal, reference_kind, target_type,
                         raw_uuid_claim
                     ) VALUES (?1, 1, ?2, ?3, ?4, 6, 1, ?5)",
                    params![
                        page_id.as_slice(),
                        block_id.as_slice(),
                        locator,
                        ordinal,
                        claim.as_slice(),
                    ],
                )
                .unwrap();
        }
        let read = SqliteMaterializedRead::new(&connection, 1, digest(b"frontier")).unwrap();
        assert_eq!(
            read.block_reference_counts_after(None, 10).unwrap(),
            vec![
                PhysicalBlockReferenceCountRow {
                    raw_uuid_claim: target,
                    distinct_source_blocks: 2,
                },
                PhysicalBlockReferenceCountRow {
                    raw_uuid_claim: other,
                    distinct_source_blocks: 1,
                },
            ]
        );
        assert_eq!(
            read.block_reference_counts_for_source_page_after(first_page, None, 10)
                .unwrap(),
            vec![PhysicalBlockReferenceCountRow {
                raw_uuid_claim: target,
                distinct_source_blocks: 1,
            }]
        );
        let first_candidate = read
            .block_referrer_candidates_after(target, None, 1)
            .unwrap();
        assert_eq!(first_candidate.len(), 1);
        let cursor = (
            first_candidate[0].source_page_id,
            first_candidate[0].source_block_id,
        );
        assert_eq!(
            read.block_referrer_candidates_after(target, Some(cursor), 10)
                .unwrap(),
            vec![PhysicalBlockReferrerCandidateRow {
                source_page_id: second_page,
                source_block_id: second_block,
            }]
        );

        for (page_id, source_type, source_id, locator, ordinal, kind) in [
            (first_page, 0, first_page, b"page-alias".as_slice(), 0, 4),
            (first_page, 1, first_block, b"block-link-a".as_slice(), 0, 0),
            (first_page, 1, first_block, b"block-link-b".as_slice(), 1, 1),
            (
                second_page,
                1,
                second_block,
                b"block-embed".as_slice(),
                0,
                2,
            ),
            (
                second_page,
                1,
                second_block,
                b"property-key".as_slice(),
                1,
                5,
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO reference_postings (
                         source_page_id, source_entity_type, source_entity_id,
                         source_locator, ordinal, reference_kind, target_type,
                         raw_name, normalized_name
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 'Target', 'target')",
                    params![
                        page_id.as_slice(),
                        source_type,
                        source_id.as_slice(),
                        locator,
                        ordinal,
                        kind,
                    ],
                )
                .unwrap();
        }
        let first_page_candidates = read
            .page_referrer_candidates_after("target", None, 2)
            .unwrap();
        assert_eq!(
            first_page_candidates,
            vec![
                PhysicalPageReferrerCandidateRow {
                    source_page_id: first_page,
                    source: PhysicalEntityId::Page(first_page),
                },
                PhysicalPageReferrerCandidateRow {
                    source_page_id: first_page,
                    source: PhysicalEntityId::Block(first_block),
                },
            ]
        );
        assert_eq!(
            read.page_referrer_candidates_after(
                "target",
                Some((first_page, PhysicalEntityId::Block(first_block))),
                10,
            )
            .unwrap(),
            vec![PhysicalPageReferrerCandidateRow {
                source_page_id: second_page,
                source: PhysicalEntityId::Block(second_block),
            }]
        );
    }

    #[test]
    fn replacement_cleanup_removes_owned_rows_and_fts() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let old = page(2, "obsolete-token");
        let old_block = old.blocks[0].block_id;
        apply_and_commit(
            &mut connection,
            &change(20, vec![old.clone()], Vec::new()),
            1,
            digest(b"frontier-1"),
        );

        let mut replacement = page(2, "current-token");
        replacement.blocks.clear();
        replacement.properties.clear();
        replacement.tags.clear();
        let stats = apply_and_commit(
            &mut connection,
            &change(21, vec![replacement.clone()], Vec::new()),
            2,
            digest(b"frontier-2"),
        );
        assert_eq!(stats.cleanup_existing_pages, 1);
        assert!(stats.cleanup_owned_rows >= 5);
        assert_eq!(stats.cleanup_fts_rowids, 2);

        let read = SqliteMaterializedRead::new(&connection, 2, digest(b"frontier-2")).unwrap();
        assert!(read.block(old_block).unwrap().is_none());
        assert!(read.search("obsolete", 10).unwrap().is_empty());
        assert_eq!(read.search("current", 10).unwrap().len(), 1);
        assert!(read.tags("storage", 10).unwrap().is_empty());
        assert!(read.tasks(None, 10).unwrap().is_empty());
    }

    #[test]
    fn physical_apply_and_stamp_roll_back_together() {
        let mut connection = Connection::open_in_memory().unwrap();
        let empty = digest(b"empty");
        initialize_schema(&connection, empty).unwrap();
        {
            let transaction = connection.transaction().unwrap();
            apply_change(
                &transaction,
                &change(30, vec![page(3, "rollback-token")], Vec::new()),
                1,
                digest(b"input"),
                digest(b"frontier"),
                None,
            )
            .unwrap();
        }
        ensure_stamp(&connection, 0, empty).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn catalog_rows_and_stamp_are_one_physical_transition() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let mut page = page(4, "catalog");
        page.blocks.clear();
        let page_id = page.page_id;
        let post_root_digest = digest(b"post-root");
        let mut physical = change(40, vec![page], Vec::new());
        physical.reference_catalog = Some(PhysicalReferenceCatalogChange {
            prior_catalog_root: vec![1],
            prior_catalog_root_digest: digest(b"prior-root"),
            prior_source_count: 0,
            post_catalog_root: vec![2],
            post_catalog_root_digest: post_root_digest,
            post_source_count: 1,
            coverage_digest: digest(b"coverage"),
            extractor_dependency_stamp_digest: digest(b"extractor"),
            postings: vec![PhysicalReferencePosting {
                source_page_id: page_id,
                source_entity: PhysicalEntityId::Page(page_id),
                source_locator: vec![1, 2, 3],
                ordinal: 0,
                kind: 0,
                target: PhysicalReferenceTarget::PageName {
                    raw_name: "Target".into(),
                    normalized_name: "target".into(),
                    resolved_page_id: None,
                },
            }],
            aliases: vec![PhysicalAliasDeclaration {
                source_page_id: page_id,
                source_entity: PhysicalEntityId::Page(page_id),
                source_locator: vec![4, 5],
                ordinal: 0,
                raw_alias: "Alias".into(),
                normalized_alias: "alias".into(),
            }],
            coverage: vec![PhysicalSourceCoverage {
                source_page_id: page_id,
                source_digest: digest(b"source"),
                extractor_dependency_stamp_digest: digest(b"source-extractor"),
            }],
            removed_sources: Vec::new(),
            canonical_bytes: vec![9, 8, 7],
        });
        let authenticated = PhysicalAuthenticatedReference {
            event_binding_digest: digest(b"event"),
            prior_frontier_root_digest: digest(b"frontier-0"),
            post_frontier_root_digest: digest(b"frontier-1"),
        };
        let transaction = connection.transaction().unwrap();
        let stats = apply_change(
            &transaction,
            &physical,
            1,
            digest(b"input"),
            digest(b"frontier-1"),
            Some(&authenticated),
        )
        .unwrap();
        transaction.commit().unwrap();
        assert_eq!(stats.reference_coverage_count, Some(1));
        let stamp: (Vec<u8>, Vec<u8>) = connection
            .query_row(
                "SELECT catalog_root, catalog_root_digest FROM materialization_stamp",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stamp.0, vec![2]);
        assert_eq!(stamp.1.as_slice(), post_root_digest.as_bytes());
        let postings: i64 = connection
            .query_row("SELECT COUNT(*) FROM reference_postings", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(postings, 1);
    }

    #[test]
    fn bounded_reads_reject_query_and_aggregate_overflow() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let oversized_query = "q".repeat(MAX_MATERIALIZATION_QUERY_BYTES + 1);
        let read = SqliteMaterializedRead::new(&connection, 0, digest(b"empty")).unwrap();
        assert!(matches!(
            read.search(&oversized_query, 1),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization query bytes",
                ..
            })
        ));
        drop(read);

        let text = "x".repeat(4 * 1024 * 1024);
        let pages = (0..17)
            .map(|offset| {
                let mut page = page(0x100 + offset, &text);
                page.blocks.clear();
                page
            })
            .collect::<Vec<_>>();
        apply_and_commit(
            &mut connection,
            &change(50, pages, Vec::new()),
            1,
            digest(b"frontier"),
        );
        let read = SqliteMaterializedRead::new(&connection, 1, digest(b"frontier")).unwrap();
        assert!(matches!(
            read.pages(None, 17),
            Err(MaterializationError::ResourceLimit {
                resource: "materialization read output bytes",
                ..
            })
        ));
    }

    #[test]
    fn schema_validation_refuses_canonical_sql_tampering() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        validate_schema(&connection).unwrap();
        connection
            .execute_batch(
                "DROP INDEX tags_page_idx;
                 CREATE INDEX tags_page_idx ON tags(tag, page_id);",
            )
            .unwrap();
        assert!(matches!(
            validate_schema(&connection),
            Err(MaterializationError::Schema(_))
        ));
    }

    #[test]
    fn schema_constraints_reject_cross_kind_reference_postings() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection, digest(b"empty")).unwrap();
        let result = connection.execute(
            "INSERT INTO reference_postings (
                 source_page_id, source_entity_type, source_entity_id, source_locator,
                 ordinal, reference_kind, target_type, raw_name, normalized_name,
                 raw_uuid_claim, resolved_page_id, resolved_block_id
             ) VALUES (?1, 0, ?1, ?2, 0, 6, 0, 'target', 'target', NULL, NULL, NULL)",
            params![id(1).as_slice(), [1_u8].as_slice()],
        );
        assert!(result.is_err());
    }
}
