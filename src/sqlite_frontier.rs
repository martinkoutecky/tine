//! Physical authenticated accepted-prefix storage for the disposable SQLite projection.
//!
//! This module deliberately knows only fixed-width identifiers, digests, counters, and
//! canonical bytes supplied by the domain owner. It owns the SQLite representation and the
//! single transaction that advances accepted history, authenticated indexes, materialized rows,
//! and the terminal frontier row.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;

use rusqlite::{params, Connection, OptionalExtension as _, Transaction, TransactionBehavior};

use crate::sqlite_materialization::{
    self, ApplyChangeInstrumentation, MaterializationError, PhysicalAuthenticatedReference,
    PhysicalMaterializationChange,
};
use crate::ContentDigest;

pub const SQLITE_APPLICATION_ID: u32 = 0x5449_4e45;
pub const SQLITE_SCHEMA_VERSION: u32 = 12;
const MAX_AUTHENTICATED_MAP_DEPTH: usize = 256;

pub const META_DDL: &str = "CREATE TABLE meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    workspace_id BLOB NOT NULL CHECK (length(workspace_id) = 16),
    lineage_digest BLOB NOT NULL CHECK (length(lineage_digest) = 32),
    oplog_protocol_version INTEGER NOT NULL,
    operation_schema_version INTEGER NOT NULL,
    object_envelope_schema_version INTEGER NOT NULL,
    manifest_encoding_version INTEGER NOT NULL,
    managed_entity_set_version INTEGER NOT NULL
) STRICT";
pub const FRONTIER_DDL: &str = "CREATE TABLE frontier (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    frontier_root BLOB NOT NULL,
    frontier_root_digest BLOB NOT NULL CHECK (length(frontier_root_digest) = 32),
    applied_batch_count INTEGER NOT NULL CHECK (applied_batch_count >= 0)
) STRICT";
pub const FRONTIER_DOCUMENTS_DDL: &str = "CREATE TABLE frontier_documents (
    document_id BLOB PRIMARY KEY CHECK (length(document_id) = 16),
    dependencies BLOB NOT NULL,
    dependencies_digest BLOB NOT NULL CHECK (length(dependencies_digest) = 32),
    left_document_id BLOB,
    left_digest BLOB,
    right_document_id BLOB,
    right_digest BLOB,
    node_digest BLOB NOT NULL CHECK (length(node_digest) = 32),
    CHECK ((left_document_id IS NULL AND left_digest IS NULL)
        OR (length(left_document_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_document_id IS NULL AND right_digest IS NULL)
        OR (length(right_document_id) = 16 AND length(right_digest) = 32))
) STRICT";
pub const CAUSAL_CLOCK_NODES_DDL: &str = "CREATE TABLE causal_clock_nodes (
    node_digest BLOB PRIMARY KEY CHECK (length(node_digest) = 32),
    peer_id BLOB NOT NULL CHECK (length(peer_id) = 16),
    counter INTEGER NOT NULL CHECK (counter > 0),
    value_digest BLOB NOT NULL CHECK (length(value_digest) = 32),
    left_peer_id BLOB,
    left_digest BLOB,
    right_peer_id BLOB,
    right_digest BLOB,
    CHECK ((left_peer_id IS NULL AND left_digest IS NULL)
        OR (length(left_peer_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_peer_id IS NULL AND right_digest IS NULL)
        OR (length(right_peer_id) = 16 AND length(right_digest) = 32))
) STRICT";
pub const ACCEPTED_BATCH_NODES_DDL: &str = "CREATE TABLE accepted_batch_nodes (
    node_digest BLOB PRIMARY KEY CHECK (length(node_digest) = 32),
    batch_id BLOB NOT NULL CHECK (length(batch_id) = 16),
    value_digest BLOB NOT NULL CHECK (length(value_digest) = 32),
    left_batch_id BLOB,
    left_digest BLOB,
    right_batch_id BLOB,
    right_digest BLOB,
    CHECK ((left_batch_id IS NULL AND left_digest IS NULL)
        OR (length(left_batch_id) = 16 AND length(left_digest) = 32)),
    CHECK ((right_batch_id IS NULL AND right_digest IS NULL)
        OR (length(right_batch_id) = 16 AND length(right_digest) = 32))
) STRICT";
pub const APPLIED_BATCHES_DDL: &str = "CREATE TABLE applied_batches (
    sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
    batch_id BLOB NOT NULL CHECK (length(batch_id) = 16),
    manifest_digest BLOB NOT NULL CHECK (length(manifest_digest) = 32),
    semantic_effect BLOB NOT NULL,
    semantic_effect_digest BLOB NOT NULL CHECK (length(semantic_effect_digest) = 32),
    dependency_frontier BLOB NOT NULL,
    dependency_frontier_digest BLOB NOT NULL
        CHECK (length(dependency_frontier_digest) = 32),
    prior_frontier_root BLOB NOT NULL,
    prior_frontier_root_digest BLOB NOT NULL
        CHECK (length(prior_frontier_root_digest) = 32),
    post_frontier_root BLOB NOT NULL,
    post_frontier_root_digest BLOB NOT NULL
        CHECK (length(post_frontier_root_digest) = 32),
    affected_documents BLOB NOT NULL,
    affected_documents_digest BLOB NOT NULL
        CHECK (length(affected_documents_digest) = 32),
    causal_dependency_heads BLOB NOT NULL,
    causal_peer_id BLOB NOT NULL CHECK (length(causal_peer_id) = 16),
    causal_counter INTEGER NOT NULL CHECK (causal_counter > 0),
    causal_clock_root_key BLOB NOT NULL CHECK (length(causal_clock_root_key) = 16),
    causal_clock_root_digest BLOB NOT NULL CHECK (length(causal_clock_root_digest) = 32),
    acceptance_sequence INTEGER NOT NULL CHECK (acceptance_sequence > 0),
    retained_bytes INTEGER NOT NULL CHECK (retained_bytes >= 0)
) STRICT";
pub const BATCH_ID_INDEX_DDL: &str =
    "CREATE UNIQUE INDEX applied_batches_batch_id_uq ON applied_batches(batch_id)";
pub const ACCEPTANCE_SEQUENCE_INDEX_DDL: &str = "CREATE UNIQUE INDEX \
    applied_batches_acceptance_sequence_uq ON applied_batches(acceptance_sequence)";

const EXPECTED_TABLES: [&str; 27] = [
    "accepted_batch_nodes",
    "applied_batches",
    "blocks",
    "causal_clock_nodes",
    "frontier",
    "frontier_documents",
    "materialization_batches",
    "materialization_stamp",
    "meta",
    "pages",
    "properties",
    "reference_alias_bindings",
    "reference_alias_declarations",
    "reference_name_bindings",
    "reference_postings",
    "reference_source_coverage",
    "reference_uuid_bindings",
    "refs",
    "search_fts",
    "search_fts_config",
    "search_fts_content",
    "search_fts_data",
    "search_fts_docsize",
    "search_fts_idx",
    "search_fts_owners",
    "tags",
    "tasks",
];
const EXPECTED_INDEXES: [&str; 26] = [
    "applied_batches_acceptance_sequence_uq",
    "applied_batches_batch_id_uq",
    "blocks_page_order_idx",
    "pages_name_idx",
    "pages_name_key_idx",
    "pages_path_idx",
    "properties_lookup_idx",
    "properties_page_idx",
    "reference_alias_bindings_normalized_alias_idx",
    "reference_alias_declarations_source_idx",
    "reference_name_bindings_raw_name_idx",
    "reference_name_bindings_resolved_page_idx",
    "reference_postings_normalized_name_idx",
    "reference_postings_raw_uuid_idx",
    "reference_postings_source_idx",
    "reference_source_coverage_source_idx",
    "reference_uuid_bindings_raw_uuid_idx",
    "reference_uuid_bindings_resolved_block_idx",
    "references_source_idx",
    "references_target_idx",
    "search_fts_owners_page_idx",
    "tags_lookup_idx",
    "tags_page_idx",
    "tasks_deadline_idx",
    "tasks_marker_idx",
    "tasks_page_idx",
];

const META_COLUMNS: &[&str] = &[
    "singleton",
    "workspace_id",
    "lineage_digest",
    "oplog_protocol_version",
    "operation_schema_version",
    "object_envelope_schema_version",
    "manifest_encoding_version",
    "managed_entity_set_version",
];
const FRONTIER_COLUMNS: &[&str] = &[
    "singleton",
    "frontier_root",
    "frontier_root_digest",
    "applied_batch_count",
];
const FRONTIER_DOCUMENT_COLUMNS: &[&str] = &[
    "document_id",
    "dependencies",
    "dependencies_digest",
    "left_document_id",
    "left_digest",
    "right_document_id",
    "right_digest",
    "node_digest",
];
const CAUSAL_CLOCK_NODE_COLUMNS: &[&str] = &[
    "node_digest",
    "peer_id",
    "counter",
    "value_digest",
    "left_peer_id",
    "left_digest",
    "right_peer_id",
    "right_digest",
];
const ACCEPTED_BATCH_NODE_COLUMNS: &[&str] = &[
    "node_digest",
    "batch_id",
    "value_digest",
    "left_batch_id",
    "left_digest",
    "right_batch_id",
    "right_digest",
];
const APPLIED_BATCH_COLUMNS: &[&str] = &[
    "sequence",
    "batch_id",
    "manifest_digest",
    "semantic_effect",
    "semantic_effect_digest",
    "dependency_frontier",
    "dependency_frontier_digest",
    "prior_frontier_root",
    "prior_frontier_root_digest",
    "post_frontier_root",
    "post_frontier_root_digest",
    "affected_documents",
    "affected_documents_digest",
    "causal_dependency_heads",
    "causal_peer_id",
    "causal_counter",
    "causal_clock_root_key",
    "causal_clock_root_digest",
    "acceptance_sequence",
    "retained_bytes",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalClaim {
    pub workspace_id: [u8; 16],
    pub lineage_digest: ContentDigest,
    pub oplog_protocol_version: u32,
    pub operation_schema_version: u32,
    pub object_envelope_schema_version: u32,
    pub manifest_encoding_version: u32,
    pub managed_entity_set_version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalFrontierRoot {
    pub canonical_bytes: Vec<u8>,
    pub acceptance_sequence: u64,
    pub document_count: u64,
    pub document_map_root_key: Option<[u8; 16]>,
    pub document_map_root_digest: ContentDigest,
    pub batch_map_root_key: Option<[u8; 16]>,
    pub batch_map_root_digest: ContentDigest,
    pub state_digest: ContentDigest,
}

impl PhysicalFrontierRoot {
    pub fn digest(&self) -> ContentDigest {
        ContentDigest::of(&self.canonical_bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalFrontierDocument {
    pub document_id: [u8; 16],
    pub canonical_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalAcceptedBatch {
    pub batch_id: [u8; 16],
    pub manifest_digest: ContentDigest,
    pub event_binding_digest: ContentDigest,
    pub semantic_effect: Vec<u8>,
    pub semantic_effect_digest: ContentDigest,
    pub dependency_frontier: Vec<u8>,
    pub prior_frontier_root: PhysicalFrontierRoot,
    pub post_frontier_root: PhysicalFrontierRoot,
    pub affected_documents: Vec<PhysicalFrontierDocument>,
    pub affected_documents_bytes: Vec<u8>,
    pub causal_dependency_heads: Vec<[u8; 16]>,
    pub causal_dependency_heads_bytes: Vec<u8>,
    pub causal_peer_id: [u8; 16],
    pub causal_counter: u64,
    pub acceptance_sequence: u64,
    pub retained_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ApplyFault {
    #[default]
    None,
    ReturnAfterInsert,
    ReturnAfterMaterialization,
    AbortAfterInsert,
}

#[derive(Clone, Debug)]
pub struct PhysicalApplyRequest {
    pub batch: PhysicalAcceptedBatch,
    pub materialization: Option<PhysicalMaterializationChange>,
    pub materialization_input_digest: Option<ContentDigest>,
    pub authenticated_reference: Option<PhysicalAuthenticatedReference>,
    pub prior_reference_coverage_count: Option<u64>,
    pub fault: ApplyFault,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyDisposition {
    Applied,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyResult {
    pub disposition: ApplyDisposition,
    pub materialization: ApplyChangeInstrumentation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightDisposition {
    New,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredFrontier {
    pub canonical_bytes: Vec<u8>,
    pub digest: ContentDigest,
    pub applied_batch_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredBatch {
    pub sequence: i64,
    pub batch_id: [u8; 16],
    pub manifest_digest: Vec<u8>,
    pub semantic_effect: Vec<u8>,
    pub semantic_effect_digest: Vec<u8>,
    pub dependency_frontier: Vec<u8>,
    pub dependency_frontier_digest: Vec<u8>,
    pub prior_frontier_root: Vec<u8>,
    pub prior_frontier_root_digest: Vec<u8>,
    pub post_frontier_root: Vec<u8>,
    pub post_frontier_root_digest: Vec<u8>,
    pub affected_documents: Vec<u8>,
    pub affected_documents_digest: Vec<u8>,
    pub causal_dependency_heads: Vec<u8>,
    pub causal_peer_id: Vec<u8>,
    pub causal_counter: i64,
    pub causal_clock_root_key: Vec<u8>,
    pub causal_clock_root_digest: Vec<u8>,
    pub acceptance_sequence: i64,
    pub retained_bytes: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrontierError {
    Sqlite(String),
    Materialization(MaterializationError),
    Schema(String),
    ClaimBytes {
        field: &'static str,
        expected: Vec<u8>,
        found: Vec<u8>,
    },
    ClaimVersion {
        field: &'static str,
        expected: i64,
        found: i64,
    },
    Corrupt(String),
    InvalidInput(String),
    MissingDependency([u8; 16]),
    AcceptanceOrder {
        expected: u64,
        found: u64,
    },
    FrontierRegression,
    BatchCollision([u8; 16]),
    MaterializationCollision([u8; 16]),
    InjectedFailure,
}

impl fmt::Display for FrontierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite frontier error: {error}"),
            Self::Materialization(error) => error.fmt(formatter),
            Self::Schema(error) => write!(formatter, "SQLite frontier schema mismatch: {error}"),
            Self::ClaimBytes { field, .. } => write!(formatter, "SQLite claim {field} mismatch"),
            Self::ClaimVersion {
                field,
                expected,
                found,
            } => {
                write!(
                    formatter,
                    "SQLite claim {field} mismatch: expected {expected}, found {found}"
                )
            }
            Self::Corrupt(error) => write!(formatter, "corrupt SQLite frontier: {error}"),
            Self::InvalidInput(error) => {
                write!(formatter, "invalid physical frontier input: {error}")
            }
            Self::MissingDependency(id) => write!(formatter, "missing dependency {}", HexId(id)),
            Self::AcceptanceOrder { expected, found } => {
                write!(
                    formatter,
                    "acceptance sequence {found} cannot apply before {expected}"
                )
            }
            Self::FrontierRegression => formatter.write_str("accepted frontier regression"),
            Self::BatchCollision(id) => write!(formatter, "batch {} collides", HexId(id)),
            Self::MaterializationCollision(id) => {
                write!(
                    formatter,
                    "materialization for batch {} collides",
                    HexId(id)
                )
            }
            Self::InjectedFailure => formatter.write_str("injected SQLite transaction failure"),
        }
    }
}

impl std::error::Error for FrontierError {}

impl From<rusqlite::Error> for FrontierError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value.to_string())
    }
}

impl From<MaterializationError> for FrontierError {
    fn from(value: MaterializationError) -> Self {
        Self::Materialization(value)
    }
}

struct HexId<'a>(&'a [u8; 16]);
impl fmt::Display for HexId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub fn initialize_schema(
    connection: &Connection,
    claim: PhysicalClaim,
    empty_frontier: &[u8],
) -> Result<(), FrontierError> {
    connection.execute_batch(&format!(
        "PRAGMA application_id = {SQLITE_APPLICATION_ID};
         PRAGMA user_version = {SQLITE_SCHEMA_VERSION};
         {META_DDL}; {FRONTIER_DDL}; {FRONTIER_DOCUMENTS_DDL};
         {CAUSAL_CLOCK_NODES_DDL}; {ACCEPTED_BATCH_NODES_DDL};
         {APPLIED_BATCHES_DDL}; {BATCH_ID_INDEX_DDL}; {ACCEPTANCE_SEQUENCE_INDEX_DDL};"
    ))?;
    let empty_digest = ContentDigest::of(empty_frontier);
    sqlite_materialization::initialize_schema(connection, empty_digest)?;
    connection.execute(
        "INSERT INTO meta (
             singleton, workspace_id, lineage_digest, oplog_protocol_version,
             operation_schema_version, object_envelope_schema_version,
             manifest_encoding_version, managed_entity_set_version
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            claim.workspace_id.as_slice(),
            claim.lineage_digest.as_bytes().as_slice(),
            i64::from(claim.oplog_protocol_version),
            i64::from(claim.operation_schema_version),
            i64::from(claim.object_envelope_schema_version),
            i64::from(claim.manifest_encoding_version),
            i64::from(claim.managed_entity_set_version),
        ],
    )?;
    connection.execute(
        "INSERT INTO frontier (
             singleton, frontier_root, frontier_root_digest, applied_batch_count
         ) VALUES (1, ?1, ?2, 0)",
        params![empty_frontier, empty_digest.as_bytes().as_slice()],
    )?;
    validate_schema_and_claim(connection, claim)
}

pub fn validate_schema_and_claim(
    connection: &Connection,
    claim: PhysicalClaim,
) -> Result<(), FrontierError> {
    let application_id: u32 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    if application_id != SQLITE_APPLICATION_ID {
        return Err(FrontierError::Schema(format!(
            "application_id {application_id:#x} != {SQLITE_APPLICATION_ID:#x}"
        )));
    }
    let user_version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if user_version != SQLITE_SCHEMA_VERSION {
        return Err(FrontierError::Schema(format!(
            "user_version {user_version} != {SQLITE_SCHEMA_VERSION}"
        )));
    }
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(FrontierError::Schema(format!(
            "journal_mode {journal_mode:?} is not WAL"
        )));
    }
    let tables = schema_names(connection, "table")?;
    let expected_tables = EXPECTED_TABLES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    if tables != expected_tables {
        return Err(FrontierError::Schema(format!(
            "unexpected P2.1 tables: {tables:?}"
        )));
    }
    let indexes = schema_names(connection, "index")?;
    let expected_indexes = EXPECTED_INDEXES
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    if indexes != expected_indexes {
        return Err(FrontierError::Schema(format!(
            "unexpected P2.1 indexes: {indexes:?}"
        )));
    }
    for (table, columns) in [
        ("meta", META_COLUMNS),
        ("frontier", FRONTIER_COLUMNS),
        ("frontier_documents", FRONTIER_DOCUMENT_COLUMNS),
        ("causal_clock_nodes", CAUSAL_CLOCK_NODE_COLUMNS),
        ("accepted_batch_nodes", ACCEPTED_BATCH_NODE_COLUMNS),
        ("applied_batches", APPLIED_BATCH_COLUMNS),
    ] {
        validate_table_columns(connection, table, columns)?;
    }
    for (kind, name, ddl) in [
        ("table", "meta", META_DDL),
        ("table", "frontier", FRONTIER_DDL),
        ("table", "frontier_documents", FRONTIER_DOCUMENTS_DDL),
        ("table", "causal_clock_nodes", CAUSAL_CLOCK_NODES_DDL),
        ("table", "accepted_batch_nodes", ACCEPTED_BATCH_NODES_DDL),
        ("table", "applied_batches", APPLIED_BATCHES_DDL),
        ("index", "applied_batches_batch_id_uq", BATCH_ID_INDEX_DDL),
        (
            "index",
            "applied_batches_acceptance_sequence_uq",
            ACCEPTANCE_SEQUENCE_INDEX_DDL,
        ),
    ] {
        validate_schema_sql(connection, kind, name, ddl)?;
    }
    sqlite_materialization::validate_schema(connection)?;
    let stored: (Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64) = connection.query_row(
        "SELECT workspace_id, lineage_digest, oplog_protocol_version,
                operation_schema_version, object_envelope_schema_version,
                manifest_encoding_version, managed_entity_set_version
         FROM meta WHERE singleton = 1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    compare_claim_bytes("workspace_id", claim.workspace_id.as_slice(), &stored.0)?;
    compare_claim_bytes("lineage_digest", claim.lineage_digest.as_bytes(), &stored.1)?;
    for (field, found, expected) in [
        (
            "oplog_protocol_version",
            stored.2,
            i64::from(claim.oplog_protocol_version),
        ),
        (
            "operation_schema_version",
            stored.3,
            i64::from(claim.operation_schema_version),
        ),
        (
            "object_envelope_schema_version",
            stored.4,
            i64::from(claim.object_envelope_schema_version),
        ),
        (
            "manifest_encoding_version",
            stored.5,
            i64::from(claim.manifest_encoding_version),
        ),
        (
            "managed_entity_set_version",
            stored.6,
            i64::from(claim.managed_entity_set_version),
        ),
    ] {
        if found != expected {
            return Err(FrontierError::ClaimVersion {
                field,
                expected,
                found,
            });
        }
    }
    let counts: (i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM meta), (SELECT COUNT(*) FROM frontier)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if counts != (1, 1) {
        return Err(FrontierError::Corrupt(
            "meta/frontier singleton cardinality is invalid".into(),
        ));
    }
    Ok(())
}

fn schema_names(connection: &Connection, kind: &str) -> Result<BTreeSet<String>, FrontierError> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = ?1 AND name NOT LIKE 'sqlite_%'")?;
    let names = statement
        .query_map([kind], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(names)
}

fn compare_claim_bytes(
    field: &'static str,
    expected: &[u8],
    found: &[u8],
) -> Result<(), FrontierError> {
    if found != expected {
        return Err(FrontierError::ClaimBytes {
            field,
            expected: expected.to_vec(),
            found: found.to_vec(),
        });
    }
    Ok(())
}

fn validate_table_columns(
    connection: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), FrontierError> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns: Vec<String> = statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()?;
    if columns != expected {
        return Err(FrontierError::Schema(format!(
            "{table} columns {columns:?} != {expected:?}"
        )));
    }
    Ok(())
}

fn validate_schema_sql(
    connection: &Connection,
    kind: &str,
    name: &str,
    expected: &str,
) -> Result<(), FrontierError> {
    let found: String = connection.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
        params![kind, name],
        |row| row.get(0),
    )?;
    if canonical_sql(&found) != canonical_sql(expected) {
        return Err(FrontierError::Schema(format!(
            "{kind} {name} does not match canonical DDL"
        )));
    }
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn read_frontier(connection: &Connection) -> Result<StoredFrontier, FrontierError> {
    let (canonical_bytes, digest, count): (Vec<u8>, Vec<u8>, i64) = connection.query_row(
        "SELECT frontier_root, frontier_root_digest, applied_batch_count
         FROM frontier WHERE singleton = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let found = decode_digest(&digest)?;
    let expected = ContentDigest::of(&canonical_bytes);
    if found != expected {
        return Err(FrontierError::Corrupt(
            "frontier-root digest does not match frontier-root bytes".into(),
        ));
    }
    let applied_batch_count = u64::try_from(count)
        .map_err(|_| FrontierError::Corrupt("applied batch count is negative".into()))?;
    Ok(StoredFrontier {
        canonical_bytes,
        digest: found,
        applied_batch_count,
    })
}

pub fn load_batch(
    connection: &Connection,
    batch_id: [u8; 16],
) -> Result<Option<StoredBatch>, FrontierError> {
    connection
        .query_row(
            "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier, dependency_frontier_digest,
                prior_frontier_root, prior_frontier_root_digest, post_frontier_root,
                post_frontier_root_digest, affected_documents, affected_documents_digest,
                causal_dependency_heads, causal_peer_id, causal_counter, causal_clock_root_key,
                causal_clock_root_digest, acceptance_sequence, retained_bytes
         FROM applied_batches WHERE batch_id = ?1",
            [batch_id.as_slice()],
            stored_batch_from_row,
        )
        .optional()
        .map_err(Into::into)
}

pub fn load_batch_at_sequence(
    connection: &Connection,
    sequence: i64,
) -> Result<Option<StoredBatch>, FrontierError> {
    connection
        .query_row(
            "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier, dependency_frontier_digest,
                prior_frontier_root, prior_frontier_root_digest, post_frontier_root,
                post_frontier_root_digest, affected_documents, affected_documents_digest,
                causal_dependency_heads, causal_peer_id, causal_counter, causal_clock_root_key,
                causal_clock_root_digest, acceptance_sequence, retained_bytes
         FROM applied_batches WHERE sequence = ?1",
            [sequence],
            stored_batch_from_row,
        )
        .optional()
        .map_err(Into::into)
}

pub fn load_all_batches(connection: &Connection) -> Result<Vec<StoredBatch>, FrontierError> {
    let mut statement = connection.prepare(
        "SELECT sequence, batch_id, manifest_digest, semantic_effect,
                semantic_effect_digest, dependency_frontier, dependency_frontier_digest,
                prior_frontier_root, prior_frontier_root_digest, post_frontier_root,
                post_frontier_root_digest, affected_documents, affected_documents_digest,
                causal_dependency_heads, causal_peer_id, causal_counter, causal_clock_root_key,
                causal_clock_root_digest, acceptance_sequence, retained_bytes
         FROM applied_batches ORDER BY sequence",
    )?;
    let batches = statement
        .query_map([], stored_batch_from_row)?
        .collect::<Result<_, _>>()?;
    Ok(batches)
}

pub fn diagnostic_row_counts(connection: &Connection) -> Result<(u64, u64), FrontierError> {
    let (batches, documents): (i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM applied_batches),
                (SELECT COUNT(*) FROM frontier_documents)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((
        u64::try_from(batches)
            .map_err(|_| FrontierError::Corrupt("accepted row count is negative".into()))?,
        u64::try_from(documents).map_err(|_| {
            FrontierError::Corrupt("frontier document row count is negative".into())
        })?,
    ))
}

pub fn semantic_projection_digest(connection: &Connection) -> Result<ContentDigest, FrontierError> {
    let mut statement = connection.prepare(
        "SELECT batch_id, manifest_digest, semantic_effect, semantic_effect_digest,
                dependency_frontier FROM applied_batches ORDER BY batch_id",
    )?;
    let mut rows = statement.query([])?;
    let mut bytes = b"tine/sqlite-frontier/semantic-projection/v1\0".to_vec();
    while let Some(row) = rows.next()? {
        for index in 0..5 {
            let value: Vec<u8> = row.get(index)?;
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(&value);
        }
    }
    let frontier = read_frontier(connection)?.canonical_bytes;
    bytes.extend_from_slice(&(frontier.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&frontier);
    Ok(ContentDigest::of(&bytes))
}

pub fn stored_semantic_effects(connection: &Connection) -> Result<Vec<Vec<u8>>, FrontierError> {
    let mut statement =
        connection.prepare("SELECT semantic_effect FROM applied_batches ORDER BY sequence")?;
    let effects = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(effects)
}

fn stored_batch_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredBatch> {
    let id: Vec<u8> = row.get(1)?;
    let batch_id = id.as_slice().try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            16,
            rusqlite::types::Type::Blob,
            "invalid batch ID length".into(),
        )
    })?;
    Ok(StoredBatch {
        sequence: row.get(0)?,
        batch_id,
        manifest_digest: row.get(2)?,
        semantic_effect: row.get(3)?,
        semantic_effect_digest: row.get(4)?,
        dependency_frontier: row.get(5)?,
        dependency_frontier_digest: row.get(6)?,
        prior_frontier_root: row.get(7)?,
        prior_frontier_root_digest: row.get(8)?,
        post_frontier_root: row.get(9)?,
        post_frontier_root_digest: row.get(10)?,
        affected_documents: row.get(11)?,
        affected_documents_digest: row.get(12)?,
        causal_dependency_heads: row.get(13)?,
        causal_peer_id: row.get(14)?,
        causal_counter: row.get(15)?,
        causal_clock_root_key: row.get(16)?,
        causal_clock_root_digest: row.get(17)?,
        acceptance_sequence: row.get(18)?,
        retained_bytes: row.get(19)?,
    })
}

fn decode_digest(bytes: &[u8]) -> Result<ContentDigest, FrontierError> {
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| FrontierError::Corrupt("invalid digest length".into()))?;
    Ok(ContentDigest::from_bytes(bytes))
}

fn decode_id(bytes: &[u8], what: &str) -> Result<[u8; 16], FrontierError> {
    bytes
        .try_into()
        .map_err(|_| FrontierError::Corrupt(format!("invalid {what} length")))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct MapLink {
    key: [u8; 16],
    digest: ContentDigest,
}

fn authenticated_map_priority(key: [u8; 16]) -> ContentDigest {
    let mut bytes = b"tine/oplog/authenticated-map/v1/priority\0".to_vec();
    bytes.extend_from_slice(&key);
    ContentDigest::of(&bytes)
}

fn authenticated_map_priority_order(left: [u8; 16], right: [u8; 16]) -> Ordering {
    authenticated_map_priority(left)
        .as_bytes()
        .cmp(authenticated_map_priority(right).as_bytes())
        .then_with(|| left.cmp(&right))
}

fn authenticated_map_node_digest(
    key: [u8; 16],
    value_digest: ContentDigest,
    left: Option<([u8; 16], ContentDigest)>,
    right: Option<([u8; 16], ContentDigest)>,
) -> ContentDigest {
    let mut bytes = b"tine/oplog/authenticated-map/v1/node\0".to_vec();
    bytes.extend_from_slice(&key);
    bytes.extend_from_slice(value_digest.as_bytes());
    for child in [left, right] {
        match child {
            Some((child_key, digest)) => {
                bytes.push(1);
                bytes.extend_from_slice(&child_key);
                bytes.extend_from_slice(digest.as_bytes());
            }
            None => bytes.push(0),
        }
    }
    ContentDigest::of(&bytes)
}

fn causal_clock_counter_digest(peer: [u8; 16], counter: u64) -> ContentDigest {
    let mut bytes = b"tine/oplog/causal-clock-entry/v1\0".to_vec();
    bytes.extend_from_slice(&peer);
    bytes.extend_from_slice(&counter.to_be_bytes());
    ContentDigest::of(&bytes)
}

fn accepted_causal_record_digest(
    batch: &PhysicalAcceptedBatch,
    clock_root: &MapLink,
) -> ContentDigest {
    let mut bytes = b"tine/oplog/accepted-causal-record/v1\0".to_vec();
    bytes.extend_from_slice(&batch.batch_id);
    bytes.extend_from_slice(batch.manifest_digest.as_bytes());
    bytes.extend_from_slice(batch.event_binding_digest.as_bytes());
    bytes.extend_from_slice(&batch.causal_peer_id);
    bytes.extend_from_slice(&batch.causal_counter.to_be_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&clock_root.key);
    bytes.extend_from_slice(clock_root.digest.as_bytes());
    ContentDigest::of(&bytes)
}

fn valid_map_children(key: [u8; 16], left: Option<&MapLink>, right: Option<&MapLink>) -> bool {
    left.is_none_or(|child| {
        child.key < key && authenticated_map_priority_order(key, child.key).is_lt()
    }) && right.is_none_or(|child| {
        child.key > key && authenticated_map_priority_order(key, child.key).is_lt()
    })
}

fn decode_map_link(
    key: Option<Vec<u8>>,
    digest: Option<Vec<u8>>,
) -> Result<Option<MapLink>, FrontierError> {
    match (key, digest) {
        (None, None) => Ok(None),
        (Some(key), Some(digest)) => Ok(Some(MapLink {
            key: decode_id(&key, "authenticated map key")?,
            digest: decode_digest(&digest)?,
        })),
        _ => Err(FrontierError::Corrupt(
            "authenticated map child is incomplete".into(),
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClockNode {
    peer: [u8; 16],
    counter: u64,
    left: Option<MapLink>,
    right: Option<MapLink>,
}

#[derive(Clone, Debug)]
struct BatchMapNode {
    batch_id: [u8; 16],
    value_digest: ContentDigest,
    left: Option<MapLink>,
    right: Option<MapLink>,
}

fn ensure_depth(depth: usize, operation: &str) -> Result<(), FrontierError> {
    if depth > MAX_AUTHENTICATED_MAP_DEPTH {
        return Err(FrontierError::Corrupt(format!(
            "{operation} exceeds its bounded depth"
        )));
    }
    Ok(())
}

fn load_clock_node(
    connection: &Connection,
    expected: &MapLink,
) -> Result<ClockNode, FrontierError> {
    let stored = connection
        .query_row(
            "SELECT peer_id, counter, value_digest, left_peer_id, left_digest,
                right_peer_id, right_digest FROM causal_clock_nodes WHERE node_digest = ?1",
            [expected.digest.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                    row.get::<_, Option<Vec<u8>>>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            FrontierError::Corrupt(format!(
                "authenticated causal clock node {} is missing",
                expected.digest
            ))
        })?;
    let peer = decode_id(&stored.0, "causal peer ID")?;
    let counter = u64::try_from(stored.1)
        .map_err(|_| FrontierError::Corrupt("causal clock counter is invalid".into()))?;
    let value_digest = decode_digest(&stored.2)?;
    let left = decode_map_link(stored.3, stored.4)?;
    let right = decode_map_link(stored.5, stored.6)?;
    let node = ClockNode {
        peer,
        counter,
        left,
        right,
    };
    let computed = authenticated_map_node_digest(
        peer,
        value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    if expected.key != peer
        || counter == 0
        || value_digest != causal_clock_counter_digest(peer, counter)
        || !valid_map_children(peer, node.left.as_ref(), node.right.as_ref())
        || computed != expected.digest
    {
        return Err(FrontierError::Corrupt(
            "authenticated causal clock node is misbound".into(),
        ));
    }
    Ok(node)
}

fn load_batch_map_node(
    connection: &Connection,
    expected: &MapLink,
) -> Result<BatchMapNode, FrontierError> {
    let stored = connection
        .query_row(
            "SELECT batch_id, value_digest, left_batch_id, left_digest,
                right_batch_id, right_digest FROM accepted_batch_nodes WHERE node_digest = ?1",
            [expected.digest.as_bytes().as_slice()],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<Vec<u8>>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            FrontierError::Corrupt(format!(
                "authenticated accepted-batch node {} is missing",
                expected.digest
            ))
        })?;
    let batch_id = decode_id(&stored.0, "batch ID")?;
    let node = BatchMapNode {
        batch_id,
        value_digest: decode_digest(&stored.1)?,
        left: decode_map_link(stored.2, stored.3)?,
        right: decode_map_link(stored.4, stored.5)?,
    };
    let computed = authenticated_map_node_digest(
        batch_id,
        node.value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    if expected.key != batch_id
        || !valid_map_children(batch_id, node.left.as_ref(), node.right.as_ref())
        || computed != expected.digest
    {
        return Err(FrontierError::Corrupt(
            "authenticated accepted-batch node is misbound".into(),
        ));
    }
    Ok(node)
}

fn batch_map_value(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    batch_id: [u8; 16],
) -> Result<Option<ContentDigest>, FrontierError> {
    let Some(key) = root.batch_map_root_key else {
        if root.acceptance_sequence == 0 {
            return Ok(None);
        }
        return Err(FrontierError::Corrupt(
            "nonempty frontier has no authenticated batch-map root".into(),
        ));
    };
    let mut current = Some(MapLink {
        key,
        digest: root.batch_map_root_digest,
    });
    let mut depth = 0;
    while let Some(link) = current {
        ensure_depth(depth, "accepted batch lookup")?;
        let node = load_batch_map_node(connection, &link)?;
        match batch_id.cmp(&node.batch_id) {
            Ordering::Equal => return Ok(Some(node.value_digest)),
            Ordering::Less => current = node.left,
            Ordering::Greater => current = node.right,
        }
        depth += 1;
    }
    Ok(None)
}

fn validate_stored_batch_physical(record: &StoredBatch) -> Result<(), FrontierError> {
    if record.sequence <= 0
        || record.acceptance_sequence != record.sequence
        || record.retained_bytes < 0
        || record.causal_counter <= 0
    {
        return Err(FrontierError::Corrupt(
            "stored accepted sequence or retained-byte count is invalid".into(),
        ));
    }
    for (label, bytes, digest) in [
        (
            "dependency-frontier",
            record.dependency_frontier.as_slice(),
            record.dependency_frontier_digest.as_slice(),
        ),
        (
            "prior-frontier-root",
            record.prior_frontier_root.as_slice(),
            record.prior_frontier_root_digest.as_slice(),
        ),
        (
            "post-frontier-root",
            record.post_frontier_root.as_slice(),
            record.post_frontier_root_digest.as_slice(),
        ),
        (
            "affected-documents",
            record.affected_documents.as_slice(),
            record.affected_documents_digest.as_slice(),
        ),
    ] {
        if digest != ContentDigest::of(bytes).as_bytes() {
            return Err(FrontierError::Corrupt(format!(
                "stored batch {} {label} digest mismatch",
                HexId(&record.batch_id)
            )));
        }
    }
    decode_digest(&record.manifest_digest)?;
    if decode_digest(&record.semantic_effect_digest)? != ContentDigest::of(&record.semantic_effect)
    {
        return Err(FrontierError::Corrupt(format!(
            "stored batch {} semantic-effect digest mismatch",
            HexId(&record.batch_id)
        )));
    }
    decode_id(&record.causal_peer_id, "causal peer ID")?;
    decode_id(&record.causal_clock_root_key, "causal clock root key")?;
    decode_digest(&record.causal_clock_root_digest)?;
    Ok(())
}

fn authenticated_batch_record(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    batch_id: [u8; 16],
    expected_value: Option<ContentDigest>,
) -> Result<Option<StoredBatch>, FrontierError> {
    let Some(value) = batch_map_value(connection, root, batch_id)? else {
        return Ok(None);
    };
    if expected_value.is_some_and(|expected| expected != value) {
        return Err(FrontierError::Corrupt(format!(
            "accepted batch {} differs from its authenticated causal record",
            HexId(&batch_id)
        )));
    }
    let record = load_batch(connection, batch_id)?.ok_or_else(|| {
        FrontierError::Corrupt(format!(
            "authenticated accepted batch {} is missing its exact record",
            HexId(&batch_id)
        ))
    })?;
    validate_stored_batch_physical(&record)?;
    let peer = decode_id(&record.causal_peer_id, "causal peer ID")?;
    let clock_root = MapLink {
        key: decode_id(&record.causal_clock_root_key, "causal clock root key")?,
        digest: decode_digest(&record.causal_clock_root_digest)?,
    };
    if causal_clock_lookup(connection, Some(clock_root), peer)?
        != Some(record.causal_counter as u64)
    {
        return Err(FrontierError::Corrupt(format!(
            "accepted batch {} causal dot is absent from its authenticated clock",
            HexId(&batch_id)
        )));
    }
    Ok(Some(record))
}

pub fn contains_batch(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    batch_id: [u8; 16],
) -> Result<bool, FrontierError> {
    authenticated_batch_record(connection, root, batch_id, None).map(|record| record.is_some())
}

pub fn authenticate_batch(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    batch_id: [u8; 16],
    causal_record_digest: ContentDigest,
) -> Result<bool, FrontierError> {
    authenticated_batch_record(connection, root, batch_id, Some(causal_record_digest))
        .map(|record| record.is_some())
}

fn causal_clock_lookup(
    connection: &Connection,
    mut current: Option<MapLink>,
    peer: [u8; 16],
) -> Result<Option<u64>, FrontierError> {
    let mut depth = 0;
    while let Some(link) = current {
        ensure_depth(depth, "causal clock lookup")?;
        let node = load_clock_node(connection, &link)?;
        match peer.cmp(&node.peer) {
            Ordering::Equal => return Ok(Some(node.counter)),
            Ordering::Less => current = node.left,
            Ordering::Greater => current = node.right,
        }
        depth += 1;
    }
    Ok(None)
}

pub fn batch_descends_from(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    descendant: [u8; 16],
    ancestor: [u8; 16],
) -> Result<bool, FrontierError> {
    let descendant =
        authenticated_batch_record(connection, root, descendant, None)?.ok_or_else(|| {
            FrontierError::Corrupt(
                "descendant batch is absent from the authenticated accepted map".into(),
            )
        })?;
    let Some(ancestor) = authenticated_batch_record(connection, root, ancestor, None)? else {
        return Ok(false);
    };
    let peer = decode_id(&ancestor.causal_peer_id, "causal peer ID")?;
    let counter = u64::try_from(ancestor.causal_counter)
        .map_err(|_| FrontierError::Corrupt("stored causal counter is invalid".into()))?;
    let clock = MapLink {
        key: decode_id(&descendant.causal_clock_root_key, "causal clock root key")?,
        digest: decode_digest(&descendant.causal_clock_root_digest)?,
    };
    Ok(causal_clock_lookup(connection, Some(clock), peer)?.is_some_and(|found| found >= counter))
}

#[derive(Clone)]
struct FrontierMapNode {
    document_id: [u8; 16],
    encoded: Vec<u8>,
    value_digest: ContentDigest,
    left: Option<MapLink>,
    right: Option<MapLink>,
    node_digest: ContentDigest,
}

impl FrontierMapNode {
    fn recompute_digest(&self) -> ContentDigest {
        authenticated_map_node_digest(
            self.document_id,
            self.value_digest,
            self.left.as_ref().map(|child| (child.key, child.digest)),
            self.right.as_ref().map(|child| (child.key, child.digest)),
        )
    }

    fn as_link(&self) -> MapLink {
        MapLink {
            key: self.document_id,
            digest: self.node_digest,
        }
    }
}

fn load_frontier_map_node(
    connection: &Connection,
    document_id: [u8; 16],
    expected_digest: Option<ContentDigest>,
) -> Result<Option<FrontierMapNode>, FrontierError> {
    type StoredRow = (
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Vec<u8>,
    );
    let found: Option<StoredRow> = connection
        .query_row(
            "SELECT dependencies, dependencies_digest, left_document_id, left_digest,
                right_document_id, right_digest, node_digest
         FROM frontier_documents WHERE document_id = ?1",
            [document_id.as_slice()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    let Some((encoded, value_digest, left_id, left_digest, right_id, right_digest, node_digest)) =
        found
    else {
        return Ok(None);
    };
    let value_digest = decode_digest(&value_digest)?;
    if value_digest != ContentDigest::of(&encoded) {
        return Err(FrontierError::Corrupt(format!(
            "frontier document {} digest mismatch",
            HexId(&document_id)
        )));
    }
    let left = decode_map_link(left_id, left_digest)?;
    let right = decode_map_link(right_id, right_digest)?;
    if left.as_ref().is_some_and(|child| child.key >= document_id)
        || right.as_ref().is_some_and(|child| child.key <= document_id)
    {
        return Err(FrontierError::Corrupt(
            "frontier map child ordering is invalid".into(),
        ));
    }
    let mut node = FrontierMapNode {
        document_id,
        encoded,
        value_digest,
        left,
        right,
        node_digest: decode_digest(&node_digest)?,
    };
    let computed = node.recompute_digest();
    if node.node_digest != computed || expected_digest.is_some_and(|expected| expected != computed)
    {
        return Err(FrontierError::Corrupt(format!(
            "frontier document {} is not authenticated by its map root",
            HexId(&document_id)
        )));
    }
    node.node_digest = computed;
    Ok(Some(node))
}

pub fn frontier_document(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
    document_id: [u8; 16],
) -> Result<Option<Vec<u8>>, FrontierError> {
    let mut current = match root.document_map_root_key {
        Some(key) => Some(MapLink {
            key,
            digest: root.document_map_root_digest,
        }),
        None => {
            if root.document_count != 0 {
                return Err(FrontierError::Corrupt(
                    "empty frontier map root is malformed".into(),
                ));
            }
            None
        }
    };
    let mut depth = 0;
    while let Some(link) = current {
        ensure_depth(depth, "frontier map lookup")?;
        let node =
            load_frontier_map_node(connection, link.key, Some(link.digest))?.ok_or_else(|| {
                FrontierError::Corrupt(format!(
                    "authenticated frontier node {} is missing",
                    HexId(&link.key)
                ))
            })?;
        match document_id.cmp(&node.document_id) {
            Ordering::Equal => return Ok(Some(node.encoded)),
            Ordering::Less => current = node.left,
            Ordering::Greater => current = node.right,
        }
        depth += 1;
    }
    Ok(None)
}

pub fn read_frontier_documents(
    connection: &Connection,
    root: &PhysicalFrontierRoot,
) -> Result<Vec<PhysicalFrontierDocument>, FrontierError> {
    let mut pending = root
        .document_map_root_key
        .map(|key| MapLink {
            key,
            digest: root.document_map_root_digest,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut documents = Vec::with_capacity(
        usize::try_from(root.document_count)
            .unwrap_or(1_000_000)
            .min(1_000_000),
    );
    while let Some(link) = pending.pop() {
        let node =
            load_frontier_map_node(connection, link.key, Some(link.digest))?.ok_or_else(|| {
                FrontierError::Corrupt(format!(
                    "authenticated frontier node {} is missing",
                    HexId(&link.key)
                ))
            })?;
        if let Some(right) = node.right.clone() {
            pending.push(right);
        }
        documents.push(PhysicalFrontierDocument {
            document_id: node.document_id,
            canonical_bytes: node.encoded,
        });
        if let Some(left) = node.left {
            pending.push(left);
        }
    }
    documents.sort_unstable_by_key(|document| document.document_id);
    if documents.len() as u64 != root.document_count {
        return Err(FrontierError::Corrupt(
            "authenticated frontier document count is stale".into(),
        ));
    }
    Ok(documents)
}

fn store_frontier_map_node(
    transaction: &Connection,
    node: &FrontierMapNode,
) -> Result<(), FrontierError> {
    if node.node_digest != node.recompute_digest() {
        return Err(FrontierError::Corrupt(
            "frontier map node digest is stale".into(),
        ));
    }
    transaction.execute(
        "INSERT INTO frontier_documents (
             document_id, dependencies, dependencies_digest, left_document_id, left_digest,
             right_document_id, right_digest, node_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(document_id) DO UPDATE SET
             dependencies = excluded.dependencies,
             dependencies_digest = excluded.dependencies_digest,
             left_document_id = excluded.left_document_id,
             left_digest = excluded.left_digest,
             right_document_id = excluded.right_document_id,
             right_digest = excluded.right_digest,
             node_digest = excluded.node_digest",
        params![
            node.document_id.as_slice(),
            &node.encoded,
            node.value_digest.as_bytes().as_slice(),
            node.left.as_ref().map(|child| child.key.as_slice()),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.right.as_ref().map(|child| child.key.as_slice()),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.node_digest.as_bytes().as_slice(),
        ],
    )?;
    Ok(())
}

fn upsert_frontier_map(
    transaction: &Connection,
    root: Option<MapLink>,
    document: &PhysicalFrontierDocument,
    depth: usize,
) -> Result<(MapLink, bool), FrontierError> {
    ensure_depth(depth, "frontier map update")?;
    let Some(root) = root else {
        let value_digest = ContentDigest::of(&document.canonical_bytes);
        let mut node = FrontierMapNode {
            document_id: document.document_id,
            encoded: document.canonical_bytes.clone(),
            value_digest,
            left: None,
            right: None,
            node_digest: ContentDigest::of(b"tine/oplog/authenticated-map/v1/empty"),
        };
        node.node_digest = node.recompute_digest();
        store_frontier_map_node(transaction, &node)?;
        return Ok((node.as_link(), true));
    };
    let mut node =
        load_frontier_map_node(transaction, root.key, Some(root.digest))?.ok_or_else(|| {
            FrontierError::Corrupt(format!(
                "authenticated frontier node {} is missing",
                HexId(&root.key)
            ))
        })?;
    let inserted;
    match document.document_id.cmp(&node.document_id) {
        Ordering::Equal => {
            node.encoded = document.canonical_bytes.clone();
            node.value_digest = ContentDigest::of(&node.encoded);
            inserted = false;
        }
        Ordering::Less => {
            let (left, was_inserted) =
                upsert_frontier_map(transaction, node.left.take(), document, depth + 1)?;
            node.left = Some(left);
            inserted = was_inserted;
            if node.left.as_ref().is_some_and(|left| {
                authenticated_map_priority_order(left.key, node.document_id).is_lt()
            }) {
                return Ok((rotate_frontier_right(transaction, node)?, inserted));
            }
        }
        Ordering::Greater => {
            let (right, was_inserted) =
                upsert_frontier_map(transaction, node.right.take(), document, depth + 1)?;
            node.right = Some(right);
            inserted = was_inserted;
            if node.right.as_ref().is_some_and(|right| {
                authenticated_map_priority_order(right.key, node.document_id).is_lt()
            }) {
                return Ok((rotate_frontier_left(transaction, node)?, inserted));
            }
        }
    }
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    Ok((node.as_link(), inserted))
}

fn rotate_frontier_right(
    transaction: &Connection,
    mut node: FrontierMapNode,
) -> Result<MapLink, FrontierError> {
    let left = node.left.take().ok_or_else(|| {
        FrontierError::Corrupt("frontier map right rotation has no left child".into())
    })?;
    let mut left_node = load_frontier_map_node(transaction, left.key, Some(left.digest))?
        .ok_or_else(|| FrontierError::Corrupt("frontier map rotation child is missing".into()))?;
    node.left = left_node.right.take();
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    left_node.right = Some(node.as_link());
    left_node.node_digest = left_node.recompute_digest();
    store_frontier_map_node(transaction, &left_node)?;
    Ok(left_node.as_link())
}

fn rotate_frontier_left(
    transaction: &Connection,
    mut node: FrontierMapNode,
) -> Result<MapLink, FrontierError> {
    let right = node.right.take().ok_or_else(|| {
        FrontierError::Corrupt("frontier map left rotation has no right child".into())
    })?;
    let mut right_node = load_frontier_map_node(transaction, right.key, Some(right.digest))?
        .ok_or_else(|| FrontierError::Corrupt("frontier map rotation child is missing".into()))?;
    node.right = right_node.left.take();
    node.node_digest = node.recompute_digest();
    store_frontier_map_node(transaction, &node)?;
    right_node.left = Some(node.as_link());
    right_node.node_digest = right_node.recompute_digest();
    store_frontier_map_node(transaction, &right_node)?;
    Ok(right_node.as_link())
}

fn write_clock_node(connection: &Connection, node: &ClockNode) -> Result<MapLink, FrontierError> {
    let value_digest = causal_clock_counter_digest(node.peer, node.counter);
    let digest = authenticated_map_node_digest(
        node.peer,
        value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    let link = MapLink {
        key: node.peer,
        digest,
    };
    connection.execute(
        "INSERT OR IGNORE INTO causal_clock_nodes (
             node_digest, peer_id, counter, value_digest, left_peer_id, left_digest,
             right_peer_id, right_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            digest.as_bytes().as_slice(),
            node.peer.as_slice(),
            i64::try_from(node.counter)
                .map_err(|_| FrontierError::Corrupt("causal counter exceeds SQLite".into()))?,
            value_digest.as_bytes().as_slice(),
            node.left.as_ref().map(|child| child.key.as_slice()),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.right.as_ref().map(|child| child.key.as_slice()),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
        ],
    )?;
    let _ = load_clock_node(connection, &link)?;
    Ok(link)
}

fn upsert_causal_clock(
    connection: &Connection,
    root: Option<MapLink>,
    peer: [u8; 16],
    counter: u64,
    depth: usize,
) -> Result<MapLink, FrontierError> {
    ensure_depth(depth, "causal clock update")?;
    let Some(root) = root else {
        return write_clock_node(
            connection,
            &ClockNode {
                peer,
                counter,
                left: None,
                right: None,
            },
        );
    };
    let mut node = load_clock_node(connection, &root)?;
    match peer.cmp(&node.peer) {
        Ordering::Equal => {
            node.counter = node.counter.max(counter);
            write_clock_node(connection, &node)
        }
        Ordering::Less => {
            node.left = Some(upsert_causal_clock(
                connection,
                node.left.take(),
                peer,
                counter,
                depth + 1,
            )?);
            if node
                .left
                .as_ref()
                .is_some_and(|left| authenticated_map_priority_order(left.key, node.peer).is_lt())
            {
                rotate_clock_right(connection, node)
            } else {
                write_clock_node(connection, &node)
            }
        }
        Ordering::Greater => {
            node.right = Some(upsert_causal_clock(
                connection,
                node.right.take(),
                peer,
                counter,
                depth + 1,
            )?);
            if node
                .right
                .as_ref()
                .is_some_and(|right| authenticated_map_priority_order(right.key, node.peer).is_lt())
            {
                rotate_clock_left(connection, node)
            } else {
                write_clock_node(connection, &node)
            }
        }
    }
}

fn rotate_clock_right(
    connection: &Connection,
    mut node: ClockNode,
) -> Result<MapLink, FrontierError> {
    let left = node
        .left
        .take()
        .ok_or_else(|| FrontierError::Corrupt("causal clock rotation has no left child".into()))?;
    let mut left_node = load_clock_node(connection, &left)?;
    node.left = left_node.right.take();
    left_node.right = Some(write_clock_node(connection, &node)?);
    write_clock_node(connection, &left_node)
}

fn rotate_clock_left(
    connection: &Connection,
    mut node: ClockNode,
) -> Result<MapLink, FrontierError> {
    let right = node
        .right
        .take()
        .ok_or_else(|| FrontierError::Corrupt("causal clock rotation has no right child".into()))?;
    let mut right_node = load_clock_node(connection, &right)?;
    node.right = right_node.left.take();
    right_node.left = Some(write_clock_node(connection, &node)?);
    write_clock_node(connection, &right_node)
}

type ClockSplit = (Option<MapLink>, Option<u64>, Option<MapLink>);

fn union_clocks(
    connection: &Connection,
    left: Option<MapLink>,
    right: Option<MapLink>,
    depth: usize,
) -> Result<Option<MapLink>, FrontierError> {
    ensure_depth(depth, "causal clock union")?;
    let (left_link, right_link) = match (left, right) {
        (None, right) => return Ok(right),
        (left, None) => return Ok(left),
        (Some(left), Some(right)) => (left, right),
    };
    if left_link == right_link {
        return Ok(Some(left_link));
    }
    if left_link.key == right_link.key {
        let left_node = load_clock_node(connection, &left_link)?;
        let right_node = load_clock_node(connection, &right_link)?;
        return Ok(Some(write_clock_node(
            connection,
            &ClockNode {
                peer: left_node.peer,
                counter: left_node.counter.max(right_node.counter),
                left: union_clocks(connection, left_node.left, right_node.left, depth + 1)?,
                right: union_clocks(connection, left_node.right, right_node.right, depth + 1)?,
            },
        )?));
    }
    if authenticated_map_priority_order(left_link.key, right_link.key).is_lt() {
        let left_node = load_clock_node(connection, &left_link)?;
        let (less, counter, greater) =
            split_clock(connection, Some(right_link), left_link.key, depth + 1)?;
        Ok(Some(write_clock_node(
            connection,
            &ClockNode {
                peer: left_node.peer,
                counter: left_node.counter.max(counter.unwrap_or(0)),
                left: union_clocks(connection, left_node.left, less, depth + 1)?,
                right: union_clocks(connection, left_node.right, greater, depth + 1)?,
            },
        )?))
    } else {
        let right_node = load_clock_node(connection, &right_link)?;
        let (less, counter, greater) =
            split_clock(connection, Some(left_link), right_link.key, depth + 1)?;
        Ok(Some(write_clock_node(
            connection,
            &ClockNode {
                peer: right_node.peer,
                counter: right_node.counter.max(counter.unwrap_or(0)),
                left: union_clocks(connection, less, right_node.left, depth + 1)?,
                right: union_clocks(connection, greater, right_node.right, depth + 1)?,
            },
        )?))
    }
}

fn split_clock(
    connection: &Connection,
    root: Option<MapLink>,
    key: [u8; 16],
    depth: usize,
) -> Result<ClockSplit, FrontierError> {
    ensure_depth(depth, "causal clock split")?;
    let Some(link) = root else {
        return Ok((None, None, None));
    };
    let node = load_clock_node(connection, &link)?;
    match key.cmp(&link.key) {
        Ordering::Equal => Ok((node.left, Some(node.counter), node.right)),
        Ordering::Less => {
            let (less, counter, greater_left) = split_clock(connection, node.left, key, depth + 1)?;
            let greater = write_clock_node(
                connection,
                &ClockNode {
                    left: greater_left,
                    ..node
                },
            )?;
            Ok((less, counter, Some(greater)))
        }
        Ordering::Greater => {
            let (less_right, counter, greater) =
                split_clock(connection, node.right, key, depth + 1)?;
            let less = write_clock_node(
                connection,
                &ClockNode {
                    right: less_right,
                    ..node
                },
            )?;
            Ok((Some(less), counter, greater))
        }
    }
}

fn derive_causal_clock_root(
    transaction: &Connection,
    root: &PhysicalFrontierRoot,
    batch: &PhysicalAcceptedBatch,
) -> Result<MapLink, FrontierError> {
    let mut clock = None;
    for parent in &batch.causal_dependency_heads {
        let record = authenticated_batch_record(transaction, root, *parent, None)?
            .ok_or(FrontierError::MissingDependency(*parent))?;
        clock = union_clocks(
            transaction,
            clock,
            Some(MapLink {
                key: decode_id(&record.causal_clock_root_key, "causal clock root key")?,
                digest: decode_digest(&record.causal_clock_root_digest)?,
            }),
            0,
        )?;
    }
    let expected = causal_clock_lookup(transaction, clock.clone(), batch.causal_peer_id)?
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| FrontierError::Corrupt("causal counter overflowed".into()))?;
    if batch.causal_counter != expected {
        return Err(FrontierError::InvalidInput(format!(
            "accepted batch {} causal counter {} does not follow {}",
            HexId(&batch.batch_id),
            batch.causal_counter,
            expected.saturating_sub(1)
        )));
    }
    upsert_causal_clock(
        transaction,
        clock,
        batch.causal_peer_id,
        batch.causal_counter,
        0,
    )
}

fn write_batch_map_node(
    connection: &Connection,
    node: &BatchMapNode,
) -> Result<MapLink, FrontierError> {
    let digest = authenticated_map_node_digest(
        node.batch_id,
        node.value_digest,
        node.left.as_ref().map(|child| (child.key, child.digest)),
        node.right.as_ref().map(|child| (child.key, child.digest)),
    );
    let link = MapLink {
        key: node.batch_id,
        digest,
    };
    connection.execute(
        "INSERT OR IGNORE INTO accepted_batch_nodes (
             node_digest, batch_id, value_digest, left_batch_id, left_digest,
             right_batch_id, right_digest
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            digest.as_bytes().as_slice(),
            node.batch_id.as_slice(),
            node.value_digest.as_bytes().as_slice(),
            node.left.as_ref().map(|child| child.key.as_slice()),
            node.left
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
            node.right.as_ref().map(|child| child.key.as_slice()),
            node.right
                .as_ref()
                .map(|child| child.digest.as_bytes().as_slice()),
        ],
    )?;
    let _ = load_batch_map_node(connection, &link)?;
    Ok(link)
}

fn upsert_batch_map(
    connection: &Connection,
    root: Option<MapLink>,
    batch_id: [u8; 16],
    value_digest: ContentDigest,
    depth: usize,
) -> Result<MapLink, FrontierError> {
    ensure_depth(depth, "accepted batch map update")?;
    let Some(root) = root else {
        return write_batch_map_node(
            connection,
            &BatchMapNode {
                batch_id,
                value_digest,
                left: None,
                right: None,
            },
        );
    };
    let mut node = load_batch_map_node(connection, &root)?;
    match batch_id.cmp(&node.batch_id) {
        Ordering::Equal => {
            node.value_digest = value_digest;
            write_batch_map_node(connection, &node)
        }
        Ordering::Less => {
            node.left = Some(upsert_batch_map(
                connection,
                node.left.take(),
                batch_id,
                value_digest,
                depth + 1,
            )?);
            if node.left.as_ref().is_some_and(|left| {
                authenticated_map_priority_order(left.key, node.batch_id).is_lt()
            }) {
                rotate_batch_right(connection, node)
            } else {
                write_batch_map_node(connection, &node)
            }
        }
        Ordering::Greater => {
            node.right = Some(upsert_batch_map(
                connection,
                node.right.take(),
                batch_id,
                value_digest,
                depth + 1,
            )?);
            if node.right.as_ref().is_some_and(|right| {
                authenticated_map_priority_order(right.key, node.batch_id).is_lt()
            }) {
                rotate_batch_left(connection, node)
            } else {
                write_batch_map_node(connection, &node)
            }
        }
    }
}

fn rotate_batch_right(
    connection: &Connection,
    mut node: BatchMapNode,
) -> Result<MapLink, FrontierError> {
    let left = node.left.take().ok_or_else(|| {
        FrontierError::Corrupt("accepted batch-map rotation has no left child".into())
    })?;
    let mut left_node = load_batch_map_node(connection, &left)?;
    node.left = left_node.right.take();
    left_node.right = Some(write_batch_map_node(connection, &node)?);
    write_batch_map_node(connection, &left_node)
}

fn rotate_batch_left(
    connection: &Connection,
    mut node: BatchMapNode,
) -> Result<MapLink, FrontierError> {
    let right = node.right.take().ok_or_else(|| {
        FrontierError::Corrupt("accepted batch-map rotation has no right child".into())
    })?;
    let mut right_node = load_batch_map_node(connection, &right)?;
    node.right = right_node.left.take();
    right_node.left = Some(write_batch_map_node(connection, &node)?);
    write_batch_map_node(connection, &right_node)
}

fn stored_matches_request(
    record: &StoredBatch,
    batch: &PhysicalAcceptedBatch,
) -> Result<bool, FrontierError> {
    validate_stored_batch_physical(record)?;
    Ok(record.batch_id == batch.batch_id
        && record.manifest_digest.as_slice() == batch.manifest_digest.as_bytes()
        && record.semantic_effect == batch.semantic_effect
        && record.semantic_effect_digest.as_slice() == batch.semantic_effect_digest.as_bytes()
        && record.dependency_frontier == batch.dependency_frontier
        && record.prior_frontier_root == batch.prior_frontier_root.canonical_bytes
        && record.post_frontier_root == batch.post_frontier_root.canonical_bytes
        && record.affected_documents == batch.affected_documents_bytes
        && record.causal_dependency_heads == batch.causal_dependency_heads_bytes
        && record.causal_peer_id.as_slice() == batch.causal_peer_id
        && u64::try_from(record.causal_counter).ok() == Some(batch.causal_counter)
        && u64::try_from(record.acceptance_sequence).ok() == Some(batch.acceptance_sequence)
        && u64::try_from(record.retained_bytes).ok() == Some(batch.retained_bytes))
}

fn insert_event(
    transaction: &Connection,
    batch: &PhysicalAcceptedBatch,
    clock_root: &MapLink,
) -> Result<(), FrontierError> {
    let sequence = i64::try_from(batch.acceptance_sequence)
        .map_err(|_| FrontierError::InvalidInput("acceptance sequence exceeds SQLite".into()))?;
    let causal_counter = i64::try_from(batch.causal_counter)
        .map_err(|_| FrontierError::InvalidInput("causal counter exceeds SQLite".into()))?;
    let retained_bytes = i64::try_from(batch.retained_bytes)
        .map_err(|_| FrontierError::InvalidInput("retained-byte count exceeds SQLite".into()))?;
    transaction.execute(
        "INSERT INTO applied_batches (
             sequence, batch_id, manifest_digest, semantic_effect, semantic_effect_digest,
             dependency_frontier, dependency_frontier_digest, prior_frontier_root,
             prior_frontier_root_digest, post_frontier_root, post_frontier_root_digest,
             affected_documents, affected_documents_digest, causal_dependency_heads,
             causal_peer_id, causal_counter, causal_clock_root_key, causal_clock_root_digest,
             acceptance_sequence, retained_bytes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                   ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        params![
            sequence,
            batch.batch_id.as_slice(),
            batch.manifest_digest.as_bytes().as_slice(),
            &batch.semantic_effect,
            batch.semantic_effect_digest.as_bytes().as_slice(),
            &batch.dependency_frontier,
            ContentDigest::of(&batch.dependency_frontier)
                .as_bytes()
                .as_slice(),
            &batch.prior_frontier_root.canonical_bytes,
            batch.prior_frontier_root.digest().as_bytes().as_slice(),
            &batch.post_frontier_root.canonical_bytes,
            batch.post_frontier_root.digest().as_bytes().as_slice(),
            &batch.affected_documents_bytes,
            ContentDigest::of(&batch.affected_documents_bytes)
                .as_bytes()
                .as_slice(),
            &batch.causal_dependency_heads_bytes,
            batch.causal_peer_id.as_slice(),
            causal_counter,
            clock_root.key.as_slice(),
            clock_root.digest.as_bytes().as_slice(),
            sequence,
            retained_bytes,
        ],
    )?;
    Ok(())
}

fn validate_root_shape(root: &PhysicalFrontierRoot) -> Result<(), FrontierError> {
    let empty_digest = ContentDigest::of(b"tine/oplog/authenticated-map/v1/empty");
    if (root.document_count == 0) != root.document_map_root_key.is_none()
        || (root.document_map_root_key.is_none() && root.document_map_root_digest != empty_digest)
        || (root.acceptance_sequence == 0) != root.batch_map_root_key.is_none()
        || (root.batch_map_root_key.is_none() && root.batch_map_root_digest != empty_digest)
    {
        return Err(FrontierError::InvalidInput(
            "physical frontier authenticated-map shape is inconsistent".into(),
        ));
    }
    Ok(())
}

fn validate_request_shape(request: &PhysicalApplyRequest) -> Result<(), FrontierError> {
    let batch = &request.batch;
    validate_root_shape(&batch.prior_frontier_root)?;
    validate_root_shape(&batch.post_frontier_root)?;
    if batch.acceptance_sequence == 0 || batch.causal_counter == 0 {
        return Err(FrontierError::InvalidInput(
            "physical accepted sequence and causal counter must be positive".into(),
        ));
    }
    if batch
        .affected_documents
        .windows(2)
        .any(|pair| pair[0].document_id >= pair[1].document_id)
        || batch
            .causal_dependency_heads
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(FrontierError::InvalidInput(
            "physical affected documents or dependencies are not sorted unique".into(),
        ));
    }
    if request.materialization.is_some() != request.materialization_input_digest.is_some() {
        return Err(FrontierError::InvalidInput(
            "physical materialization and its digest must be supplied together".into(),
        ));
    }
    if request
        .materialization
        .as_ref()
        .is_some_and(|change| change.batch_id != batch.batch_id)
    {
        return Err(FrontierError::InvalidInput(
            "physical materialization is bound to another batch".into(),
        ));
    }
    Ok(())
}

pub fn preflight(
    connection: &Connection,
    current_root: &PhysicalFrontierRoot,
    request: &PhysicalApplyRequest,
) -> Result<PreflightDisposition, FrontierError> {
    validate_request_shape(request)?;
    let batch = &request.batch;
    let stored_frontier = read_frontier(connection)?;
    if stored_frontier.canonical_bytes != current_root.canonical_bytes
        || stored_frontier.digest != current_root.digest()
        || stored_frontier.applied_batch_count != current_root.acceptance_sequence
    {
        return Err(FrontierError::Corrupt(
            "physical frontier row differs from the validated current root".into(),
        ));
    }
    if let Some(existing) = load_batch(connection, batch.batch_id)? {
        let clock_root = MapLink {
            key: decode_id(&existing.causal_clock_root_key, "causal clock root key")?,
            digest: decode_digest(&existing.causal_clock_root_digest)?,
        };
        if authenticated_batch_record(connection, current_root, batch.batch_id, None)?.is_none() {
            return Err(FrontierError::Corrupt(format!(
                "stored batch {} is absent from the authenticated accepted map",
                HexId(&batch.batch_id)
            )));
        }
        if stored_matches_request(&existing, batch)? {
            if batch_map_value(connection, current_root, batch.batch_id)?
                != Some(accepted_causal_record_digest(batch, &clock_root))
            {
                return Err(FrontierError::Corrupt(format!(
                    "accepted batch {} differs from its authenticated causal record",
                    HexId(&batch.batch_id)
                )));
            }
            if current_root.acceptance_sequence >= batch.acceptance_sequence
                && current_root.state_digest != batch.prior_frontier_root.state_digest
            {
                return Ok(PreflightDisposition::Duplicate);
            }
            return Err(FrontierError::FrontierRegression);
        }
        return Err(FrontierError::BatchCollision(batch.batch_id));
    }
    for dependency in &batch.causal_dependency_heads {
        if authenticated_batch_record(connection, current_root, *dependency, None)?.is_none() {
            return Err(FrontierError::MissingDependency(*dependency));
        }
    }
    let expected = current_root
        .acceptance_sequence
        .checked_add(1)
        .ok_or_else(|| FrontierError::Corrupt("applied batch sequence overflowed".into()))?;
    if batch.acceptance_sequence != expected {
        return Err(FrontierError::AcceptanceOrder {
            expected,
            found: batch.acceptance_sequence,
        });
    }
    if current_root != &batch.prior_frontier_root
        || batch.post_frontier_root.acceptance_sequence != batch.acceptance_sequence
    {
        return Err(FrontierError::FrontierRegression);
    }
    if request.materialization.is_some() {
        sqlite_materialization::ensure_stamp(
            connection,
            current_root.acceptance_sequence,
            current_root.digest(),
        )?;
    }
    Ok(PreflightDisposition::New)
}

pub fn apply(
    connection: &mut Connection,
    current_root: &PhysicalFrontierRoot,
    request: &PhysicalApplyRequest,
) -> Result<ApplyResult, FrontierError> {
    apply_with_transaction_policy(connection, current_root, request, false)
}

/// Apply one transition inside a candidate-build transaction already owned by
/// the caller. This is deliberately separate from [`apply`]: ordinary live
/// applies continue to own and durably commit exactly one transaction each.
pub fn apply_candidate(
    connection: &mut Connection,
    current_root: &PhysicalFrontierRoot,
    request: &PhysicalApplyRequest,
) -> Result<ApplyResult, FrontierError> {
    apply_with_transaction_policy(connection, current_root, request, true)
}

fn apply_with_transaction_policy(
    connection: &mut Connection,
    current_root: &PhysicalFrontierRoot,
    request: &PhysicalApplyRequest,
    candidate_build: bool,
) -> Result<ApplyResult, FrontierError> {
    validate_request_shape(request)?;
    let batch = &request.batch;
    let stored_frontier = read_frontier(connection)?;
    if stored_frontier.canonical_bytes != current_root.canonical_bytes
        || stored_frontier.digest != current_root.digest()
        || stored_frontier.applied_batch_count != current_root.acceptance_sequence
    {
        return Err(FrontierError::Corrupt(
            "physical frontier row differs from the validated current root".into(),
        ));
    }
    if let Some(existing) = load_batch(connection, batch.batch_id)? {
        let existing_clock_root = MapLink {
            key: decode_id(&existing.causal_clock_root_key, "causal clock root key")?,
            digest: decode_digest(&existing.causal_clock_root_digest)?,
        };
        let authenticated =
            authenticated_batch_record(connection, current_root, batch.batch_id, None)?;
        if authenticated.is_none() {
            return Err(FrontierError::Corrupt(format!(
                "stored batch {} is absent from the authenticated accepted map",
                HexId(&batch.batch_id)
            )));
        }
        if stored_matches_request(&existing, batch)? {
            if batch_map_value(connection, current_root, batch.batch_id)?
                != Some(accepted_causal_record_digest(batch, &existing_clock_root))
            {
                return Err(FrontierError::Corrupt(format!(
                    "accepted batch {} differs from its authenticated causal record",
                    HexId(&batch.batch_id)
                )));
            }
            if current_root.acceptance_sequence >= batch.acceptance_sequence
                && current_root.state_digest != batch.prior_frontier_root.state_digest
            {
                if let Some(input_digest) = request.materialization_input_digest {
                    sqlite_materialization::ensure_stamp(
                        connection,
                        batch.acceptance_sequence,
                        batch.post_frontier_root.digest(),
                    )?;
                    if sqlite_materialization::recorded_digest(
                        connection,
                        batch.acceptance_sequence,
                    )? != Some(input_digest)
                    {
                        return Err(FrontierError::MaterializationCollision(batch.batch_id));
                    }
                }
                return Ok(ApplyResult {
                    disposition: ApplyDisposition::Duplicate,
                    materialization: ApplyChangeInstrumentation::default(),
                });
            }
            return Err(FrontierError::FrontierRegression);
        }
        return Err(FrontierError::BatchCollision(batch.batch_id));
    }

    for dependency in &batch.causal_dependency_heads {
        if authenticated_batch_record(connection, current_root, *dependency, None)?.is_none() {
            return Err(FrontierError::MissingDependency(*dependency));
        }
    }
    let expected_sequence = current_root
        .acceptance_sequence
        .checked_add(1)
        .ok_or_else(|| FrontierError::Corrupt("applied batch sequence overflowed".into()))?;
    if batch.acceptance_sequence != expected_sequence {
        return Err(FrontierError::AcceptanceOrder {
            expected: expected_sequence,
            found: batch.acceptance_sequence,
        });
    }
    if current_root != &batch.prior_frontier_root
        || batch.post_frontier_root.acceptance_sequence != batch.acceptance_sequence
    {
        return Err(FrontierError::FrontierRegression);
    }
    if request.materialization.is_some() {
        sqlite_materialization::ensure_stamp(
            connection,
            current_root.acceptance_sequence,
            current_root.digest(),
        )?;
    }

    if candidate_build {
        if connection.is_autocommit() {
            return Err(FrontierError::InvalidInput(
                "candidate apply requires an active candidate-build transaction".into(),
            ));
        }
        return apply_in_open_transaction(
            OpenApplyTransaction::Candidate(connection),
            current_root,
            request,
            &stored_frontier,
        );
    }
    if !connection.is_autocommit() {
        return Err(FrontierError::InvalidInput(
            "ordinary apply cannot join a caller-owned transaction".into(),
        ));
    }
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let result = apply_in_open_transaction(
        OpenApplyTransaction::Ordinary(&transaction),
        current_root,
        request,
        &stored_frontier,
    )?;
    transaction.commit()?;
    Ok(result)
}

enum OpenApplyTransaction<'transaction, 'connection> {
    Ordinary(&'transaction Transaction<'connection>),
    Candidate(&'transaction Connection),
}

impl OpenApplyTransaction<'_, '_> {
    fn connection(&self) -> &Connection {
        match self {
            Self::Ordinary(transaction) => transaction,
            Self::Candidate(connection) => connection,
        }
    }
}

fn apply_in_open_transaction(
    transaction: OpenApplyTransaction<'_, '_>,
    current_root: &PhysicalFrontierRoot,
    request: &PhysicalApplyRequest,
    stored_frontier: &StoredFrontier,
) -> Result<ApplyResult, FrontierError> {
    let connection = transaction.connection();
    let batch = &request.batch;
    // Re-read after BEGIN IMMEDIATE so every physical premise is protected by the write
    // transaction even if this API is used without Tine's higher-level single-writer lease.
    let transaction_frontier = read_frontier(connection)?;
    if &transaction_frontier != stored_frontier {
        return Err(FrontierError::Corrupt(
            "physical frontier changed while beginning apply".into(),
        ));
    }
    let clock_root = derive_causal_clock_root(connection, current_root, batch)?;
    let causal_record_digest = accepted_causal_record_digest(batch, &clock_root);
    let post_batch_root = upsert_batch_map(
        connection,
        current_root.batch_map_root_key.map(|key| MapLink {
            key,
            digest: current_root.batch_map_root_digest,
        }),
        batch.batch_id,
        causal_record_digest,
        0,
    )?;
    if batch.post_frontier_root.batch_map_root_key != Some(post_batch_root.key)
        || batch.post_frontier_root.batch_map_root_digest != post_batch_root.digest
    {
        return Err(FrontierError::FrontierRegression);
    }
    insert_event(connection, batch, &clock_root)?;
    match request.fault {
        ApplyFault::ReturnAfterInsert => return Err(FrontierError::InjectedFailure),
        ApplyFault::AbortAfterInsert => std::process::abort(),
        _ => {}
    }

    let mut document_root = current_root.document_map_root_key.map(|key| MapLink {
        key,
        digest: current_root.document_map_root_digest,
    });
    let mut new_documents = 0_u64;
    for document in &batch.affected_documents {
        let (root, inserted) = upsert_frontier_map(connection, document_root, document, 0)?;
        document_root = Some(root);
        new_documents = new_documents.saturating_add(u64::from(inserted));
    }
    let (document_key, document_digest) = document_root
        .map(|root| (Some(root.key), root.digest))
        .unwrap_or((None, current_root.document_map_root_digest));
    if batch.post_frontier_root.document_count
        != current_root.document_count.saturating_add(new_documents)
        || batch.post_frontier_root.document_map_root_key != document_key
        || batch.post_frontier_root.document_map_root_digest != document_digest
    {
        return Err(FrontierError::FrontierRegression);
    }

    let mut materialization_stats = ApplyChangeInstrumentation::default();
    if let (Some(change), Some(input_digest)) = (
        request.materialization.as_ref(),
        request.materialization_input_digest,
    ) {
        materialization_stats = match (&transaction, request.prior_reference_coverage_count) {
            (OpenApplyTransaction::Ordinary(transaction), Some(prior_count)) => {
                sqlite_materialization::apply_change_fresh_bootstrap(
                    transaction,
                    change,
                    batch.acceptance_sequence,
                    input_digest,
                    batch.post_frontier_root.digest(),
                    request.authenticated_reference.as_ref(),
                    prior_count,
                )?
            }
            (OpenApplyTransaction::Ordinary(transaction), None) => {
                sqlite_materialization::apply_change(
                    transaction,
                    change,
                    batch.acceptance_sequence,
                    input_digest,
                    batch.post_frontier_root.digest(),
                    request.authenticated_reference.as_ref(),
                )?
            }
            (OpenApplyTransaction::Candidate(transaction), Some(prior_count)) => {
                sqlite_materialization::apply_change_fresh_bootstrap_in_open_candidate(
                    transaction,
                    change,
                    batch.acceptance_sequence,
                    input_digest,
                    batch.post_frontier_root.digest(),
                    request.authenticated_reference.as_ref(),
                    prior_count,
                )?
            }
            (OpenApplyTransaction::Candidate(transaction), None) => {
                sqlite_materialization::apply_change_in_open_candidate(
                    transaction,
                    change,
                    batch.acceptance_sequence,
                    input_digest,
                    batch.post_frontier_root.digest(),
                    request.authenticated_reference.as_ref(),
                )?
            }
        };
    }
    if matches!(request.fault, ApplyFault::ReturnAfterMaterialization) {
        return Err(FrontierError::InjectedFailure);
    }
    connection.execute(
        "UPDATE frontier SET frontier_root = ?1, frontier_root_digest = ?2,
                             applied_batch_count = ?3 WHERE singleton = 1",
        params![
            &batch.post_frontier_root.canonical_bytes,
            batch.post_frontier_root.digest().as_bytes().as_slice(),
            i64::try_from(batch.acceptance_sequence).map_err(|_| FrontierError::Corrupt(
                "applied batch sequence exceeds SQLite".into()
            ))?,
        ],
    )?;
    Ok(ApplyResult {
        disposition: ApplyDisposition::Applied,
        materialization: materialization_stats,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    use crate::sqlite::{
        PhysicalEntityId, PhysicalReferenceCatalogChange, PhysicalReferencePosting,
        PhysicalReferenceTarget, PhysicalSourceCoverage, PhysicalSqliteDatabase,
    };

    use super::*;

    static NEXT_DATABASE: AtomicU64 = AtomicU64::new(1);

    struct TestDatabase {
        path: PathBuf,
    }

    impl TestDatabase {
        fn new() -> Self {
            let serial = NEXT_DATABASE.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tine-storage-frontier-{}-{serial}.sqlite3",
                std::process::id()
            ));
            Self { path }
        }

        fn create() -> (Self, Connection) {
            let database = Self::new();
            let connection = Connection::open(&database.path).unwrap();
            connection
                .execute_batch(
                    "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA trusted_schema = OFF;",
                )
                .unwrap();
            (database, connection)
        }

        fn reopen(&self) -> Connection {
            let connection = Connection::open(&self.path).unwrap();
            connection
                .execute_batch(
                    "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA foreign_keys = ON;
                 PRAGMA trusted_schema = OFF;",
                )
                .unwrap();
            connection
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(format!("{}-wal", self.path.display()));
            let _ = std::fs::remove_file(format!("{}-shm", self.path.display()));
        }
    }

    fn id(value: u128) -> [u8; 16] {
        value.to_be_bytes()
    }

    fn map_root(entries: &[([u8; 16], ContentDigest)]) -> (Option<[u8; 16]>, ContentDigest) {
        if entries.is_empty() {
            return (
                None,
                ContentDigest::of(b"tine/oplog/authenticated-map/v1/empty"),
            );
        }
        let root_index = (0..entries.len())
            .min_by(|left, right| {
                authenticated_map_priority_order(entries[*left].0, entries[*right].0)
            })
            .unwrap();
        let (key, value) = entries[root_index];
        let (left_key, left_digest) = map_root(&entries[..root_index]);
        let (right_key, right_digest) = map_root(&entries[root_index + 1..]);
        let digest = authenticated_map_node_digest(
            key,
            value,
            left_key.map(|child| (child, left_digest)),
            right_key.map(|child| (child, right_digest)),
        );
        (Some(key), digest)
    }

    fn root(
        sequence: u64,
        documents: &[PhysicalFrontierDocument],
        batches: &[([u8; 16], ContentDigest)],
    ) -> PhysicalFrontierRoot {
        let document_entries = documents
            .iter()
            .map(|document| {
                (
                    document.document_id,
                    ContentDigest::of(&document.canonical_bytes),
                )
            })
            .collect::<Vec<_>>();
        let (document_map_root_key, document_map_root_digest) = map_root(&document_entries);
        let (batch_map_root_key, batch_map_root_digest) = map_root(batches);
        let mut canonical_bytes = b"synthetic physical frontier\0".to_vec();
        canonical_bytes.extend_from_slice(&sequence.to_be_bytes());
        canonical_bytes.extend_from_slice(document_map_root_digest.as_bytes());
        canonical_bytes.extend_from_slice(batch_map_root_digest.as_bytes());
        PhysicalFrontierRoot {
            canonical_bytes,
            acceptance_sequence: sequence,
            document_count: documents.len() as u64,
            document_map_root_key,
            document_map_root_digest,
            batch_map_root_key,
            batch_map_root_digest,
            state_digest: ContentDigest::of(&sequence.to_be_bytes()),
        }
    }

    fn claim() -> PhysicalClaim {
        PhysicalClaim {
            workspace_id: id(1),
            lineage_digest: ContentDigest::of(b"lineage"),
            oplog_protocol_version: 7,
            operation_schema_version: 8,
            object_envelope_schema_version: 9,
            manifest_encoding_version: 10,
            managed_entity_set_version: 11,
        }
    }

    fn materialization(batch_id: [u8; 16]) -> PhysicalMaterializationChange {
        let page_id = batch_id;
        PhysicalMaterializationChange {
            batch_id,
            replacements: vec![sqlite_materialization::PhysicalPage {
                page_id,
                home_document_id: id(50),
                name: format!("page-{}", u128::from_be_bytes(batch_id)),
                name_key: format!("page-{}", u128::from_be_bytes(batch_id)),
                path: format!("pages/{}.md", u128::from_be_bytes(batch_id)),
                text_kind: 0,
                preamble: None,
                searchable_text: "frontier materialization".into(),
                references: Vec::new(),
                properties: Vec::new(),
                tags: Vec::new(),
                blocks: Vec::new(),
            }],
            deletions: Vec::new(),
            pages_with_live_metadata_delta: BTreeSet::from([page_id]),
            reference_catalog: None,
        }
    }

    fn request(
        prior: &PhysicalFrontierRoot,
        sequence: u64,
        dependency: Option<[u8; 16]>,
        documents: Vec<PhysicalFrontierDocument>,
        prior_batch_entries: &[([u8; 16], ContentDigest)],
        materialized: bool,
    ) -> (PhysicalApplyRequest, Vec<([u8; 16], ContentDigest)>) {
        let batch_id = id(100 + sequence as u128);
        let peer = id(900);
        let dependencies = dependency.into_iter().collect::<Vec<_>>();
        let mut batch = PhysicalAcceptedBatch {
            batch_id,
            manifest_digest: ContentDigest::of(&[b'm', sequence as u8]),
            event_binding_digest: ContentDigest::of(&[b'e', sequence as u8]),
            semantic_effect: vec![b's', sequence as u8],
            semantic_effect_digest: ContentDigest::of(&[b's', sequence as u8]),
            dependency_frontier: vec![b'd', sequence as u8],
            prior_frontier_root: prior.clone(),
            post_frontier_root: prior.clone(),
            affected_documents: documents.clone(),
            affected_documents_bytes: documents
                .iter()
                .flat_map(|document| {
                    document
                        .document_id
                        .into_iter()
                        .chain(document.canonical_bytes.iter().copied())
                })
                .collect(),
            causal_dependency_heads: dependencies.clone(),
            causal_dependency_heads_bytes: dependencies.iter().flatten().copied().collect(),
            causal_peer_id: peer,
            causal_counter: sequence,
            acceptance_sequence: sequence,
            retained_bytes: sequence * 10,
        };
        let clock_value = causal_clock_counter_digest(peer, sequence);
        let clock_root = MapLink {
            key: peer,
            digest: authenticated_map_node_digest(peer, clock_value, None, None),
        };
        let record_digest = accepted_causal_record_digest(&batch, &clock_root);
        let mut batch_entries = prior_batch_entries.to_vec();
        batch_entries.push((batch_id, record_digest));
        batch_entries.sort_unstable_by_key(|entry| entry.0);
        batch.post_frontier_root = root(sequence, &documents, &batch_entries);
        let materialization = materialized.then(|| materialization(batch_id));
        let materialization_input_digest =
            materialized.then(|| ContentDigest::of(&[b'i', sequence as u8]));
        (
            PhysicalApplyRequest {
                batch,
                materialization,
                materialization_input_digest,
                authenticated_reference: None,
                prior_reference_coverage_count: None,
                fault: ApplyFault::None,
            },
            batch_entries,
        )
    }

    fn initialized() -> (TestDatabase, Connection, PhysicalFrontierRoot) {
        let (database, connection) = TestDatabase::create();
        let empty = root(0, &[], &[]);
        initialize_schema(&connection, claim(), &empty.canonical_bytes).unwrap();
        (database, connection, empty)
    }

    fn initialized_facade() -> (TestDatabase, PhysicalSqliteDatabase, PhysicalFrontierRoot) {
        let database = TestDatabase::new();
        let physical = PhysicalSqliteDatabase::open_writable(&database.path).unwrap();
        let empty = root(0, &[], &[]);
        physical
            .initialize_schema(claim(), &empty.canonical_bytes)
            .unwrap();
        (database, physical, empty)
    }

    fn catalog_root(sources: &[[u8; 16]]) -> Vec<u8> {
        let mut bytes = b"synthetic fresh bootstrap catalog\0".to_vec();
        for source in sources {
            bytes.extend_from_slice(source);
        }
        bytes
    }

    fn reference_target_name(source_page_id: [u8; 16]) -> String {
        format!("bootstrap-target-{}", u128::from_be_bytes(source_page_id))
    }

    fn configure_fresh_bootstrap_reference(
        request: &mut PhysicalApplyRequest,
        prior_sources: &[[u8; 16]],
    ) -> [u8; 16] {
        let source_page_id = request.batch.batch_id;
        let mut post_sources = prior_sources.to_vec();
        post_sources.push(source_page_id);
        let prior_catalog_root = catalog_root(prior_sources);
        let post_catalog_root = catalog_root(&post_sources);
        let extractor_dependency_stamp_digest = ContentDigest::of(b"bootstrap extractor");
        let source_digest = ContentDigest::of(&source_page_id);
        let target_name = reference_target_name(source_page_id);
        request.materialization.as_mut().unwrap().reference_catalog =
            Some(PhysicalReferenceCatalogChange {
                prior_catalog_root: prior_catalog_root.clone(),
                prior_catalog_root_digest: ContentDigest::of(&prior_catalog_root),
                prior_source_count: prior_sources.len() as u64,
                post_catalog_root: post_catalog_root.clone(),
                post_catalog_root_digest: ContentDigest::of(&post_catalog_root),
                post_source_count: post_sources.len() as u64,
                coverage_digest: ContentDigest::of(&post_catalog_root),
                extractor_dependency_stamp_digest,
                postings: vec![PhysicalReferencePosting {
                    source_page_id,
                    source_entity: PhysicalEntityId::Page(source_page_id),
                    source_locator: vec![request.batch.acceptance_sequence as u8],
                    ordinal: 0,
                    kind: 0,
                    target: PhysicalReferenceTarget::PageName {
                        raw_name: target_name.clone(),
                        normalized_name: target_name,
                        resolved_page_id: None,
                    },
                }],
                aliases: Vec::new(),
                coverage: vec![PhysicalSourceCoverage {
                    source_page_id,
                    source_digest,
                    extractor_dependency_stamp_digest,
                }],
                removed_sources: Vec::new(),
                canonical_bytes: post_catalog_root,
            });
        request.authenticated_reference = Some(PhysicalAuthenticatedReference {
            event_binding_digest: request.batch.event_binding_digest,
            prior_frontier_root_digest: request.batch.prior_frontier_root.digest(),
            post_frontier_root_digest: request.batch.post_frontier_root.digest(),
        });
        request.prior_reference_coverage_count = Some(prior_sources.len() as u64);
        source_page_id
    }

    struct FreshBootstrapScenario {
        first_apply: ApplyResult,
        second_apply: ApplyResult,
        first_batch_id: [u8; 16],
        second_batch_id: [u8; 16],
        first_source_page_id: [u8; 16],
        second_source_page_id: [u8; 16],
        final_root: PhysicalFrontierRoot,
        batch_entries: Vec<([u8; 16], ContentDigest)>,
    }

    fn apply_two_part_fresh_bootstrap(
        physical: &mut PhysicalSqliteDatabase,
        empty: &PhysicalFrontierRoot,
    ) -> FreshBootstrapScenario {
        apply_two_part_fresh_bootstrap_with(physical, empty, false)
    }

    fn apply_two_part_fresh_bootstrap_with(
        physical: &mut PhysicalSqliteDatabase,
        empty: &PhysicalFrontierRoot,
        candidate: bool,
    ) -> FreshBootstrapScenario {
        let document = PhysicalFrontierDocument {
            document_id: id(50),
            canonical_bytes: b"fresh-bootstrap-document".to_vec(),
        };
        let (mut first, first_entries) = request(empty, 1, None, vec![document.clone()], &[], true);
        let first_source_page_id = configure_fresh_bootstrap_reference(&mut first, &[]);
        let first_batch_id = first.batch.batch_id;
        let first_apply = if candidate {
            physical.apply_candidate(empty, &first)
        } else {
            physical.apply(empty, &first)
        }
        .unwrap();
        let first_root = first.batch.post_frontier_root.clone();

        let (mut second, batch_entries) = request(
            &first_root,
            2,
            Some(first_batch_id),
            vec![document],
            &first_entries,
            true,
        );
        let second_source_page_id =
            configure_fresh_bootstrap_reference(&mut second, &[first_source_page_id]);
        let second_batch_id = second.batch.batch_id;
        let second_apply = if candidate {
            physical.apply_candidate(&first_root, &second)
        } else {
            physical.apply(&first_root, &second)
        }
        .unwrap();

        FreshBootstrapScenario {
            first_apply,
            second_apply,
            first_batch_id,
            second_batch_id,
            first_source_page_id,
            second_source_page_id,
            final_root: second.batch.post_frontier_root,
            batch_entries,
        }
    }

    #[test]
    fn ordered_apply_duplicate_collision_and_missing_dependency_are_physical() {
        let (_database, mut connection, empty) = initialized();
        let document = PhysicalFrontierDocument {
            document_id: id(50),
            canonical_bytes: b"document-v1".to_vec(),
        };
        let (first, entries) = request(&empty, 1, None, vec![document.clone()], &[], true);
        let first_result = apply(&mut connection, &empty, &first).unwrap();
        assert_eq!(first_result.disposition, ApplyDisposition::Applied);
        let first_root = first.batch.post_frontier_root.clone();
        assert_eq!(
            apply(&mut connection, &first_root, &first)
                .unwrap()
                .disposition,
            ApplyDisposition::Duplicate
        );

        let mut collision = first.clone();
        collision.batch.manifest_digest = ContentDigest::of(b"colliding manifest");
        assert!(matches!(
            preflight(&connection, &first_root, &collision),
            Err(FrontierError::BatchCollision(id)) if id == first.batch.batch_id
        ));

        let (missing, _) = request(
            &empty,
            1,
            Some(id(9999)),
            vec![document.clone()],
            &[],
            false,
        );
        let (_other_database, other_connection, other_empty) = initialized();
        assert!(matches!(
            preflight(&other_connection, &other_empty, &missing),
            Err(FrontierError::MissingDependency(found)) if found == id(9999)
        ));

        let document_v2 = PhysicalFrontierDocument {
            document_id: id(50),
            canonical_bytes: b"document-v2".to_vec(),
        };
        let (second, _) = request(
            &first_root,
            2,
            Some(first.batch.batch_id),
            vec![document_v2],
            &entries,
            true,
        );
        assert_eq!(
            apply(&mut connection, &first_root, &second)
                .unwrap()
                .disposition,
            ApplyDisposition::Applied
        );
        assert_eq!(
            read_frontier(&connection).unwrap().canonical_bytes,
            second.batch.post_frontier_root.canonical_bytes
        );
    }

    #[test]
    fn materialization_and_terminal_frontier_roll_back_together() {
        let (_database, mut connection, empty) = initialized();
        let document = PhysicalFrontierDocument {
            document_id: id(60),
            canonical_bytes: b"rollback-doc".to_vec(),
        };
        let (mut change, _) = request(&empty, 1, None, vec![document], &[], true);
        change.fault = ApplyFault::ReturnAfterMaterialization;
        assert_eq!(
            apply(&mut connection, &empty, &change),
            Err(FrontierError::InjectedFailure)
        );
        assert_eq!(
            read_frontier(&connection).unwrap().canonical_bytes,
            empty.canonical_bytes
        );
        assert!(load_batch(&connection, change.batch.batch_id)
            .unwrap()
            .is_none());
        let page_rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))
            .unwrap();
        let search_rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM search_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!((page_rows, search_rows), (0, 0));
        assert_eq!(
            sqlite_materialization::recorded_digest(&connection, 1).unwrap(),
            None
        );
        sqlite_materialization::ensure_stamp(&connection, 0, empty.digest()).unwrap();
    }

    #[test]
    fn reopen_authenticates_roots_and_rejects_tampered_nodes() {
        let (database, mut connection, empty) = initialized();
        let document = PhysicalFrontierDocument {
            document_id: id(70),
            canonical_bytes: b"reopen-doc".to_vec(),
        };
        let (change, _) = request(&empty, 1, None, vec![document], &[], true);
        apply(&mut connection, &empty, &change).unwrap();
        let post = change.batch.post_frontier_root.clone();
        drop(connection);

        let connection = database.reopen();
        validate_schema_and_claim(&connection, claim()).unwrap();
        assert!(contains_batch(&connection, &post, change.batch.batch_id).unwrap());
        connection
            .execute(
                "UPDATE accepted_batch_nodes SET value_digest = ?1 WHERE batch_id = ?2",
                params![
                    ContentDigest::of(b"tampered").as_bytes().as_slice(),
                    change.batch.batch_id.as_slice()
                ],
            )
            .unwrap();
        assert!(matches!(
            contains_batch(&connection, &post, change.batch.batch_id),
            Err(FrontierError::Corrupt(_))
        ));
    }

    #[test]
    fn terminal_frontier_and_materialization_stamp_are_exactly_equal() {
        let (_database, mut connection, empty) = initialized();
        let document = PhysicalFrontierDocument {
            document_id: id(80),
            canonical_bytes: b"terminal-doc".to_vec(),
        };
        let (change, _) = request(&empty, 1, None, vec![document], &[], true);
        let input_digest = change.materialization_input_digest.unwrap();
        apply(&mut connection, &empty, &change).unwrap();
        let stored = read_frontier(&connection).unwrap();
        assert_eq!(
            stored.canonical_bytes,
            change.batch.post_frontier_root.canonical_bytes
        );
        assert_eq!(stored.digest, change.batch.post_frontier_root.digest());
        assert_eq!(stored.applied_batch_count, change.batch.acceptance_sequence);
        sqlite_materialization::ensure_stamp(
            &connection,
            change.batch.acceptance_sequence,
            change.batch.post_frontier_root.digest(),
        )
        .unwrap();
        assert_eq!(
            sqlite_materialization::recorded_digest(&connection, change.batch.acceptance_sequence)
                .unwrap(),
            Some(input_digest)
        );
    }

    #[test]
    fn fresh_bootstrap_inductively_covers_authenticated_reference_catalog_transitions() {
        let (_database, mut physical, empty) = initialized_facade();
        let scenario = apply_two_part_fresh_bootstrap(&mut physical, &empty);

        assert_eq!(
            scenario
                .first_apply
                .materialization
                .reference_coverage_count,
            Some(1)
        );
        assert_eq!(
            scenario
                .second_apply
                .materialization
                .reference_coverage_count,
            Some(2)
        );
        assert_eq!(
            scenario
                .first_apply
                .materialization
                .reference_coverage_inductive_checks,
            1
        );
        assert_eq!(
            scenario
                .second_apply
                .materialization
                .reference_coverage_inductive_checks,
            1
        );
        assert_eq!(
            scenario
                .first_apply
                .materialization
                .reference_coverage_full_scans,
            0
        );
        assert_eq!(
            scenario
                .second_apply
                .materialization
                .reference_coverage_full_scans,
            0
        );
        assert_eq!(physical.reference_source_coverage_count().unwrap(), 2);
        assert_eq!(
            physical
                .reference_page_candidates_for_name(
                    &reference_target_name(scenario.first_source_page_id),
                    2,
                )
                .unwrap(),
            BTreeSet::from([scenario.first_source_page_id])
        );
        assert_eq!(
            physical
                .reference_page_candidates_for_name(
                    &reference_target_name(scenario.second_source_page_id),
                    2,
                )
                .unwrap(),
            BTreeSet::from([scenario.second_source_page_id])
        );
    }

    #[test]
    fn candidate_batch_matches_ordinary_apply_and_keeps_live_commit_durability() {
        let (_ordinary_path, mut ordinary, empty) = initialized_facade();
        let ordinary_scenario = apply_two_part_fresh_bootstrap(&mut ordinary, &empty);
        let ordinary_semantic = ordinary.semantic_projection_digest().unwrap();
        let ordinary_rows = ordinary.materialized_row_digest().unwrap();
        let ordinary_writes = ordinary.write_instrumentation();
        assert_eq!(ordinary_writes.ordinary_transactions, 2);
        assert_eq!(ordinary_writes.ordinary_durability_barriers, 2);
        assert_eq!(ordinary_writes.candidate_transactions, 0);
        assert_eq!(ordinary_writes.candidate_durability_barriers, 0);

        let (_candidate_path, mut candidate, candidate_empty) = initialized_facade();
        candidate.begin_candidate_build().unwrap();
        let candidate_scenario =
            apply_two_part_fresh_bootstrap_with(&mut candidate, &candidate_empty, true);
        assert_eq!(
            candidate.read_frontier().unwrap().canonical_bytes,
            candidate_scenario.final_root.canonical_bytes
        );
        candidate.finish_candidate_build().unwrap();

        assert_eq!(candidate_scenario.final_root, ordinary_scenario.final_root);
        assert_eq!(
            candidate.semantic_projection_digest().unwrap(),
            ordinary_semantic
        );
        assert_eq!(candidate.materialized_row_digest().unwrap(), ordinary_rows);
        let candidate_writes = candidate.write_instrumentation();
        assert_eq!(candidate_writes.ordinary_transactions, 0);
        assert_eq!(candidate_writes.ordinary_durability_barriers, 0);
        assert_eq!(candidate_writes.candidate_transactions, 1);
        assert_eq!(candidate_writes.candidate_durability_barriers, 1);
    }

    #[test]
    fn failed_unfinished_candidate_transaction_is_not_reopenable_as_accepted_state() {
        let (database, mut candidate, empty) = initialized_facade();
        candidate.begin_candidate_build().unwrap();
        let document = PhysicalFrontierDocument {
            document_id: id(50),
            canonical_bytes: b"fresh-bootstrap-document".to_vec(),
        };
        let (mut first, first_entries) =
            request(&empty, 1, None, vec![document.clone()], &[], true);
        let first_source = configure_fresh_bootstrap_reference(&mut first, &[]);
        candidate.apply_candidate(&empty, &first).unwrap();
        let first_root = first.batch.post_frontier_root.clone();

        let (mut second, _) = request(
            &first_root,
            2,
            Some(first.batch.batch_id),
            vec![document],
            &first_entries,
            true,
        );
        configure_fresh_bootstrap_reference(&mut second, &[first_source]);
        second.fault = ApplyFault::ReturnAfterMaterialization;
        assert_eq!(
            candidate.apply_candidate(&first_root, &second),
            Err(FrontierError::InjectedFailure)
        );
        drop(candidate);

        let reopened = PhysicalSqliteDatabase::open_writable(&database.path).unwrap();
        assert_eq!(
            reopened.read_frontier().unwrap().canonical_bytes,
            empty.canonical_bytes
        );
        assert!(reopened.load_all_batches().unwrap().is_empty());
        assert_eq!(reopened.reference_source_coverage_count().unwrap(), 0);
        assert!(reopened.stored_semantic_effects().unwrap().is_empty());
    }

    #[test]
    fn fresh_bootstrap_finalization_retains_only_reachable_accepted_physical_state() {
        let (_database, mut physical, empty) = initialized_facade();
        let scenario = apply_two_part_fresh_bootstrap(&mut physical, &empty);
        let document = PhysicalFrontierDocument {
            document_id: id(50),
            canonical_bytes: b"fresh-bootstrap-document".to_vec(),
        };
        let (mut staged, _) = request(
            &scenario.final_root,
            3,
            Some(scenario.second_batch_id),
            vec![document],
            &scenario.batch_entries,
            true,
        );
        let staged_source_page_id = configure_fresh_bootstrap_reference(
            &mut staged,
            &[
                scenario.first_source_page_id,
                scenario.second_source_page_id,
            ],
        );
        staged.fault = ApplyFault::ReturnAfterMaterialization;
        assert_eq!(
            physical.apply(&scenario.final_root, &staged),
            Err(FrontierError::InjectedFailure)
        );

        physical.finalize_fresh_bootstrap(2, 2).unwrap();
        assert_eq!(
            physical.read_frontier().unwrap(),
            StoredFrontier {
                canonical_bytes: scenario.final_root.canonical_bytes.clone(),
                digest: scenario.final_root.digest(),
                applied_batch_count: 2,
            }
        );
        assert_eq!(
            physical
                .load_all_batches()
                .unwrap()
                .into_iter()
                .map(|batch| batch.batch_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([scenario.first_batch_id, scenario.second_batch_id])
        );
        assert!(physical
            .load_batch(staged.batch.batch_id)
            .unwrap()
            .is_none());
        assert_eq!(physical.reference_source_coverage_count().unwrap(), 2);
        assert!(physical
            .reference_page_candidates_for_name(&reference_target_name(staged_source_page_id), 2)
            .unwrap()
            .is_empty());

        let read = physical
            .materialized_read(
                scenario.final_root.acceptance_sequence,
                scenario.final_root.digest(),
            )
            .unwrap();
        assert_eq!(
            read.pages(None, 3)
                .unwrap()
                .into_iter()
                .map(|page| page.page_id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                scenario.first_source_page_id,
                scenario.second_source_page_id
            ])
        );
        assert!(read.page(staged_source_page_id).unwrap().is_none());
    }

    #[test]
    fn finalized_fresh_bootstrap_reopens_through_public_storage_facade() {
        let (database, mut physical, empty) = initialized_facade();
        let scenario = apply_two_part_fresh_bootstrap(&mut physical, &empty);
        physical.finalize_fresh_bootstrap(2, 2).unwrap();

        let expected_frontier = physical.read_frontier().unwrap();
        let expected_batches = physical.load_all_batches().unwrap();
        let expected_coverage_count = physical.reference_source_coverage_count().unwrap();
        let expected_first_candidates = physical
            .reference_page_candidates_for_name(
                &reference_target_name(scenario.first_source_page_id),
                2,
            )
            .unwrap();
        let expected_second_candidates = physical
            .reference_page_candidates_for_name(
                &reference_target_name(scenario.second_source_page_id),
                2,
            )
            .unwrap();
        let expected_pages = physical
            .materialized_read(
                scenario.final_root.acceptance_sequence,
                scenario.final_root.digest(),
            )
            .unwrap()
            .pages(None, 3)
            .unwrap();
        physical.checkpoint_truncate().unwrap();
        drop(physical);

        let reopened = PhysicalSqliteDatabase::open_read_only(&database.path).unwrap();
        reopened.validate_schema_and_claim(claim()).unwrap();
        assert_eq!(reopened.read_frontier().unwrap(), expected_frontier);
        assert_eq!(reopened.load_all_batches().unwrap(), expected_batches);
        assert_eq!(
            reopened.reference_source_coverage_count().unwrap(),
            expected_coverage_count
        );
        assert_eq!(
            reopened
                .reference_page_candidates_for_name(
                    &reference_target_name(scenario.first_source_page_id),
                    2,
                )
                .unwrap(),
            expected_first_candidates
        );
        assert_eq!(
            reopened
                .reference_page_candidates_for_name(
                    &reference_target_name(scenario.second_source_page_id),
                    2,
                )
                .unwrap(),
            expected_second_candidates
        );
        assert_eq!(
            reopened
                .materialized_read(
                    scenario.final_root.acceptance_sequence,
                    scenario.final_root.digest(),
                )
                .unwrap()
                .pages(None, 3)
                .unwrap(),
            expected_pages
        );
    }
}
