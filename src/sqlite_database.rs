//! Owned SQLite connection lifecycle and typed physical forwarding.
//!
//! This wrapper is deliberately the only production owner of the live
//! `rusqlite::Connection`. Callers supply and interpret physical DTOs; policy,
//! authority, and domain decoding stay above this layer.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

#[cfg(any(test, feature = "test-support"))]
use rusqlite::TransactionBehavior;
use rusqlite::{params, Connection, OpenFlags};

use crate::sqlite_frontier::{
    self, ApplyResult, FrontierError, PhysicalApplyRequest, PhysicalClaim,
    PhysicalFrontierDocument, PhysicalFrontierRoot, PreflightDisposition, StoredBatch,
    StoredFrontier,
};
use crate::sqlite_materialization::{self, SqliteMaterializedRead};
#[cfg(any(test, feature = "test-support"))]
use crate::sqlite_materialization::{
    ApplyChangeInstrumentation, PhysicalAuthenticatedReference, PhysicalMaterializationChange,
};
use crate::ContentDigest;

/// Prepared-statement cache size for the writable connection.
const PREPARED_STATEMENT_CACHE_STATEMENTS: usize = 64;

/// Physical fields from the reference-catalog materialization stamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalReferenceCatalogStamp {
    pub acceptance_sequence: i64,
    pub frontier_root_digest: Vec<u8>,
    pub catalog_root: Option<Vec<u8>>,
    pub catalog_root_digest: Option<Vec<u8>>,
    pub coverage_digest: Option<Vec<u8>>,
    pub extractor_dependency_stamp_digest: Option<Vec<u8>>,
}

/// Storage-owned live SQLite database.
///
/// There is intentionally no production accessor for the underlying
/// connection. Every operation crossing this boundary is named for the
/// physical data or lifecycle action it performs.
pub struct PhysicalSqliteDatabase {
    connection: Connection,
    write_instrumentation: PhysicalWriteInstrumentation,
    candidate_build_active: bool,
}

/// Physical commit accounting for accepted-event application. Schema setup,
/// checkpoint publication, and directory publication are intentionally outside
/// these apply-path counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PhysicalWriteInstrumentation {
    pub ordinary_transactions: u64,
    pub ordinary_durability_barriers: u64,
    pub candidate_transactions: u64,
    pub candidate_durability_barriers: u64,
}

impl PhysicalSqliteDatabase {
    pub fn open_writable(path: &Path) -> Result<Self, FrontierError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        // The materialized row writers reuse a small fixed set of statements
        // once per page, block, and facet. Keep every one of them resident so a
        // graph-sized build does not re-prepare the same SQL per row.
        connection.set_prepared_statement_cache_capacity(PREPARED_STATEMENT_CACHE_STATEMENTS);
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;",
        )?;
        Ok(Self {
            connection,
            write_instrumentation: PhysicalWriteInstrumentation::default(),
            candidate_build_active: false,
        })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, FrontierError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        // Same 5s budget the writable connection gets. WAL lets readers run
        // alongside a writer, but not through a checkpoint or the writer's
        // exclusive moments — without a busy timeout a reader that lands on one
        // returns SQLITE_BUSY immediately instead of waiting the moment out, and
        // a read path fails for a reason that would have cleared itself in
        // milliseconds.
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self {
            connection,
            write_instrumentation: PhysicalWriteInstrumentation::default(),
            candidate_build_active: false,
        })
    }

    pub fn initialize_schema(
        &self,
        claim: PhysicalClaim,
        empty_frontier: &[u8],
    ) -> Result<(), FrontierError> {
        sqlite_frontier::initialize_schema(&self.connection, claim, empty_frontier)
    }

    pub fn validate_schema_and_claim(&self, claim: PhysicalClaim) -> Result<(), FrontierError> {
        sqlite_frontier::validate_schema_and_claim(&self.connection, claim)
    }

    pub fn quick_check(&self) -> Result<(), FrontierError> {
        let result: String = self
            .connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if result != "ok" {
            return Err(FrontierError::Corrupt(format!(
                "SQLite quick_check failed: {result}"
            )));
        }
        Ok(())
    }

    pub fn checkpoint_truncate(&self) -> Result<(), FrontierError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }

    pub fn checkpoint_truncate_and_disable_wal(&self) -> Result<(), FrontierError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")?;
        Ok(())
    }

    pub fn read_frontier(&self) -> Result<StoredFrontier, FrontierError> {
        sqlite_frontier::read_frontier(&self.connection)
    }

    pub fn load_batch(&self, batch_id: [u8; 16]) -> Result<Option<StoredBatch>, FrontierError> {
        sqlite_frontier::load_batch(&self.connection, batch_id)
    }

    pub fn load_batch_at_sequence(
        &self,
        sequence: i64,
    ) -> Result<Option<StoredBatch>, FrontierError> {
        sqlite_frontier::load_batch_at_sequence(&self.connection, sequence)
    }

    pub fn load_all_batches(&self) -> Result<Vec<StoredBatch>, FrontierError> {
        sqlite_frontier::load_all_batches(&self.connection)
    }

    pub fn diagnostic_row_counts(&self) -> Result<(u64, u64), FrontierError> {
        sqlite_frontier::diagnostic_row_counts(&self.connection)
    }

    pub fn semantic_projection_digest(&self) -> Result<ContentDigest, FrontierError> {
        sqlite_frontier::semantic_projection_digest(&self.connection)
    }

    pub fn stored_semantic_effects(&self) -> Result<Vec<Vec<u8>>, FrontierError> {
        sqlite_frontier::stored_semantic_effects(&self.connection)
    }

    pub fn contains_batch(
        &self,
        root: &PhysicalFrontierRoot,
        batch_id: [u8; 16],
    ) -> Result<bool, FrontierError> {
        sqlite_frontier::contains_batch(&self.connection, root, batch_id)
    }

    pub fn authenticate_batch(
        &self,
        root: &PhysicalFrontierRoot,
        batch_id: [u8; 16],
        causal_record_digest: ContentDigest,
    ) -> Result<bool, FrontierError> {
        sqlite_frontier::authenticate_batch(&self.connection, root, batch_id, causal_record_digest)
    }

    pub fn batch_descends_from(
        &self,
        root: &PhysicalFrontierRoot,
        descendant: [u8; 16],
        ancestor: [u8; 16],
    ) -> Result<bool, FrontierError> {
        sqlite_frontier::batch_descends_from(&self.connection, root, descendant, ancestor)
    }

    pub fn frontier_document(
        &self,
        root: &PhysicalFrontierRoot,
        document_id: [u8; 16],
    ) -> Result<Option<Vec<u8>>, FrontierError> {
        sqlite_frontier::frontier_document(&self.connection, root, document_id)
    }

    pub fn read_frontier_documents(
        &self,
        root: &PhysicalFrontierRoot,
    ) -> Result<Vec<PhysicalFrontierDocument>, FrontierError> {
        sqlite_frontier::read_frontier_documents(&self.connection, root)
    }

    pub fn preflight(
        &self,
        current_root: &PhysicalFrontierRoot,
        request: &PhysicalApplyRequest,
    ) -> Result<PreflightDisposition, FrontierError> {
        sqlite_frontier::preflight(&self.connection, current_root, request)
    }

    pub fn apply(
        &mut self,
        current_root: &PhysicalFrontierRoot,
        request: &PhysicalApplyRequest,
    ) -> Result<ApplyResult, FrontierError> {
        let result = sqlite_frontier::apply(&mut self.connection, current_root, request)?;
        if matches!(
            result.disposition,
            sqlite_frontier::ApplyDisposition::Applied
        ) {
            self.write_instrumentation.ordinary_transactions = self
                .write_instrumentation
                .ordinary_transactions
                .saturating_add(1);
            self.write_instrumentation.ordinary_durability_barriers = self
                .write_instrumentation
                .ordinary_durability_barriers
                .saturating_add(1);
        }
        Ok(result)
    }

    /// Begin one unpublished disposable-candidate transaction. Only the
    /// candidate apply method below can join it; ordinary live apply refuses a
    /// caller-owned transaction.
    pub fn begin_candidate_build(&mut self) -> Result<(), FrontierError> {
        if self.candidate_build_active || !self.connection.is_autocommit() {
            return Err(FrontierError::InvalidInput(
                "candidate build transaction is already active".into(),
            ));
        }
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        self.candidate_build_active = true;
        self.write_instrumentation.candidate_transactions = self
            .write_instrumentation
            .candidate_transactions
            .saturating_add(1);
        Ok(())
    }

    /// Replay one already-validated transition into the active unpublished
    /// candidate without committing it independently.
    pub fn apply_candidate(
        &mut self,
        current_root: &PhysicalFrontierRoot,
        request: &PhysicalApplyRequest,
    ) -> Result<ApplyResult, FrontierError> {
        if !self.candidate_build_active {
            return Err(FrontierError::InvalidInput(
                "candidate apply has no active candidate-build transaction".into(),
            ));
        }
        sqlite_frontier::apply_candidate(&mut self.connection, current_root, request)
    }

    /// Retain one authenticated history transition for a fresh terminal
    /// candidate without incrementally rewriting its document frontier.
    pub fn apply_terminal_prefix_candidate(
        &mut self,
        current_root: &PhysicalFrontierRoot,
        request: &PhysicalApplyRequest,
    ) -> Result<ApplyResult, FrontierError> {
        self.require_candidate_build()?;
        sqlite_frontier::apply_terminal_prefix_candidate(
            &mut self.connection,
            current_root,
            request,
        )
    }

    /// Persist and authenticate the terminal document frontier once after all
    /// accepted-prefix history has been retained.
    pub fn seed_terminal_frontier_documents(
        &mut self,
        expected_root: &PhysicalFrontierRoot,
        documents: &[PhysicalFrontierDocument],
    ) -> Result<(), FrontierError> {
        self.require_candidate_build()?;
        sqlite_frontier::seed_terminal_frontier_documents_candidate(
            &self.connection,
            expected_root,
            documents,
        )
    }

    /// Commit the fully proved candidate once. Under this connection's
    /// `synchronous=FULL` contract, the commit is the candidate apply-path
    /// durability barrier; WAL checkpoint and atomic publication remain later
    /// explicit boundaries.
    pub fn finish_candidate_build(&mut self) -> Result<(), FrontierError> {
        if !self.candidate_build_active || self.connection.is_autocommit() {
            return Err(FrontierError::InvalidInput(
                "candidate build transaction is not active".into(),
            ));
        }
        self.connection.execute_batch("COMMIT")?;
        self.candidate_build_active = false;
        self.write_instrumentation.candidate_durability_barriers = self
            .write_instrumentation
            .candidate_durability_barriers
            .saturating_add(1);
        Ok(())
    }

    /// Refuse terminal bootstrap construction unless the active candidate's
    /// materialized tables are still completely empty.
    pub fn begin_terminal_bootstrap_construction(&mut self) -> Result<(), FrontierError> {
        self.require_candidate_build()?;
        sqlite_materialization::begin_terminal_construction_in_open_candidate(&self.connection)
            .map_err(Into::into)
    }

    /// Seed one bounded chunk of terminal bootstrap rows into the active
    /// candidate-build transaction.
    pub fn seed_terminal_bootstrap_chunk(
        &mut self,
        chunk: &sqlite_materialization::PhysicalTerminalMaterializationChunk,
    ) -> Result<(), FrontierError> {
        self.require_candidate_build()?;
        sqlite_materialization::seed_terminal_chunk_in_open_candidate(&self.connection, chunk)
            .map_err(Into::into)
    }

    /// Close the terminal seed with its accepted-prefix construction provenance
    /// and the one authenticated catalog stamp.
    pub fn finish_terminal_bootstrap_construction(
        &mut self,
        provenance: &[sqlite_materialization::PhysicalTerminalConstructionBatch],
        stamp: &sqlite_materialization::PhysicalTerminalCatalogStamp,
    ) -> Result<u64, FrontierError> {
        self.require_candidate_build()?;
        sqlite_materialization::finish_terminal_construction_in_open_candidate(
            &self.connection,
            provenance,
            stamp,
        )
        .map_err(Into::into)
    }

    /// Close a terminal seed as ordinary frontier-stamped disposable graph
    /// projection state, without a second reference-catalog authority.
    pub fn finish_terminal_graph_projection_construction(
        &mut self,
        provenance: &[sqlite_materialization::PhysicalTerminalConstructionBatch],
        stamp: sqlite_materialization::PhysicalTerminalProjectionStamp,
    ) -> Result<(), FrontierError> {
        self.require_candidate_build()?;
        sqlite_materialization::finish_terminal_graph_projection_in_open_candidate(
            &self.connection,
            provenance,
            stamp,
        )
        .map_err(Into::into)
    }

    fn require_candidate_build(&self) -> Result<(), FrontierError> {
        if !self.candidate_build_active {
            return Err(FrontierError::InvalidInput(
                "terminal bootstrap construction has no active candidate-build transaction".into(),
            ));
        }
        Ok(())
    }

    pub fn write_instrumentation(&self) -> PhysicalWriteInstrumentation {
        self.write_instrumentation
    }

    pub fn ensure_materialization_stamp(
        &self,
        sequence: u64,
        frontier_digest: ContentDigest,
    ) -> Result<(), FrontierError> {
        sqlite_materialization::ensure_stamp(&self.connection, sequence, frontier_digest)
            .map_err(Into::into)
    }

    pub fn finalize_fresh_bootstrap(
        &self,
        expected_catalog_source_count: u64,
        inductive_coverage_count: u64,
    ) -> Result<(), FrontierError> {
        sqlite_materialization::finalize_fresh_bootstrap(
            &self.connection,
            expected_catalog_source_count,
            inductive_coverage_count,
        )
        .map_err(Into::into)
    }

    pub fn materialized_row_digest(&self) -> Result<ContentDigest, FrontierError> {
        sqlite_materialization::row_digest(&self.connection).map_err(Into::into)
    }

    /// Complete per-table row observation for differential comparison of two
    /// independently built databases.
    #[cfg(any(test, feature = "test-support"))]
    pub fn materialized_row_digests_by_table(
        &self,
    ) -> Result<Vec<(&'static str, ContentDigest)>, FrontierError> {
        sqlite_materialization::row_digests_by_table(&self.connection).map_err(Into::into)
    }

    pub fn materialized_read(
        &self,
        acceptance_sequence: u64,
        frontier_digest: ContentDigest,
    ) -> Result<SqliteMaterializedRead<'_>, FrontierError> {
        SqliteMaterializedRead::new(&self.connection, acceptance_sequence, frontier_digest)
            .map_err(Into::into)
    }

    pub fn reference_catalog_stamp(&self) -> Result<PhysicalReferenceCatalogStamp, FrontierError> {
        self.connection
            .query_row(
                "SELECT acceptance_sequence, frontier_root_digest, catalog_root,
                        catalog_root_digest, coverage_digest,
                        extractor_dependency_stamp_digest
                 FROM materialization_stamp WHERE singleton = 1",
                [],
                |row| {
                    Ok(PhysicalReferenceCatalogStamp {
                        acceptance_sequence: row.get(0)?,
                        frontier_root_digest: row.get(1)?,
                        catalog_root: row.get(2)?,
                        catalog_root_digest: row.get(3)?,
                        coverage_digest: row.get(4)?,
                        extractor_dependency_stamp_digest: row.get(5)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn reference_source_coverage_count(&self) -> Result<i64, FrontierError> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM reference_source_coverage",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn reference_page_candidates_for_name(
        &self,
        normalized_name: &str,
        limit: i64,
    ) -> Result<BTreeSet<[u8; 16]>, FrontierError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT source_page_id FROM reference_postings
             WHERE normalized_name = ?1 AND reference_kind != 4
             ORDER BY source_page_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![normalized_name, limit], |row| row.get(0))?;
        collect_ids(rows)
    }

    pub fn reference_page_candidates_for_logseq_uuid(
        &self,
        logseq_uuid: [u8; 16],
        limit: i64,
    ) -> Result<BTreeSet<[u8; 16]>, FrontierError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT source_page_id FROM reference_postings
             WHERE target_type = 1 AND raw_uuid_claim = ?1
             ORDER BY source_page_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![logseq_uuid.as_slice(), limit], |row| row.get(0))?;
        collect_ids(rows)
    }

    pub fn reference_page_candidates_for_alias(
        &self,
        normalized_alias: &str,
        limit: i64,
    ) -> Result<BTreeSet<[u8; 16]>, FrontierError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT resolved_page_id FROM reference_alias_bindings
             WHERE normalized_alias = ?1 AND resolved_page_id IS NOT NULL
             ORDER BY resolved_page_id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![normalized_alias, limit], |row| row.get(0))?;
        collect_ids(rows)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn authority_rejection_snapshot_for_test(
        &self,
    ) -> Result<(i64, i64, i64, i64), FrontierError> {
        self.connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM applied_batches),
                    (SELECT COUNT(*) FROM pages),
                    (SELECT COUNT(*) FROM blocks),
                    (SELECT acceptance_sequence
                     FROM materialization_stamp WHERE singleton = 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(Into::into)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn total_changes_for_test(&self) -> i64 {
        self.connection.total_changes() as i64
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn disable_wal_autocheckpoint_for_test(&self) -> Result<(), FrontierError> {
        self.connection
            .pragma_update(None, "wal_autocheckpoint", 0)?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_materialization_for_test(
        &mut self,
        empty_frontier_digest: ContentDigest,
    ) -> Result<(), FrontierError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        sqlite_materialization::reset(&transaction, empty_frontier_digest)?;
        transaction.commit()?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn apply_materialization_for_test(
        &mut self,
        change: &PhysicalMaterializationChange,
        sequence: u64,
        input_digest: ContentDigest,
        post_frontier_digest: ContentDigest,
        authenticated_reference: Option<&PhysicalAuthenticatedReference>,
    ) -> Result<ApplyChangeInstrumentation, FrontierError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let instrumentation = sqlite_materialization::apply_change(
            &transaction,
            change,
            sequence,
            input_digest,
            post_frontier_digest,
            authenticated_reference,
        )?;
        transaction.commit()?;
        Ok(instrumentation)
    }

    /// Execute SQL whose purpose is to simulate physical damage in tests.
    /// This is never compiled into storage without explicit test support.
    #[cfg(any(test, feature = "test-support"))]
    pub fn execute_corrupting_sql_for_test(&self, sql: &str) -> Result<(), FrontierError> {
        self.connection.execute_batch(sql)?;
        Ok(())
    }

    /// Execute one parameterized statement whose purpose is to simulate
    /// physical damage in tests, without exposing the connection itself.
    #[cfg(any(test, feature = "test-support"))]
    pub fn execute_corrupting_statement_for_test(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
    ) -> Result<(), FrontierError> {
        self.connection.execute(sql, parameters)?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_corrupt_user_version_for_test(&self, version: u32) -> Result<(), FrontierError> {
        self.connection
            .pragma_update(None, "user_version", version)?;
        Ok(())
    }
}

fn collect_ids(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Vec<u8>>>,
) -> Result<BTreeSet<[u8; 16]>, FrontierError> {
    rows.map(|row| {
        let bytes = row?;
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| FrontierError::Corrupt("reference page ID has invalid length".into()))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::sqlite_fileset::SqliteFileSet;

    static NEXT_DATABASE: AtomicU64 = AtomicU64::new(1);

    struct TestDatabasePath(PathBuf);

    impl TestDatabasePath {
        fn new(label: &str) -> Self {
            let serial = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tine-storage-sqlite-database-{label}-{}-{serial}.sqlite",
                std::process::id()
            ));
            Self(path)
        }

        fn as_path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDatabasePath {
        fn drop(&mut self) {
            let _ = SqliteFileSet::new(&self.0).remove();
        }
    }

    fn claim(seed: u8) -> PhysicalClaim {
        PhysicalClaim {
            workspace_id: [seed; 16],
            lineage_digest: ContentDigest::of(&[seed, 1]),
            oplog_protocol_version: 2,
            operation_schema_version: 3,
            object_envelope_schema_version: 4,
            manifest_encoding_version: 5,
            managed_entity_set_version: 6,
        }
    }

    #[test]
    fn fresh_open_initializes_schema_and_typed_reads() {
        let path = TestDatabasePath::new("fresh");
        let database = PhysicalSqliteDatabase::open_writable(path.as_path()).unwrap();
        let empty_frontier = b"canonical empty frontier";
        database
            .initialize_schema(claim(7), empty_frontier)
            .unwrap();
        database.validate_schema_and_claim(claim(7)).unwrap();

        let frontier = database.read_frontier().unwrap();
        assert_eq!(frontier.canonical_bytes, empty_frontier);
        assert_eq!(frontier.applied_batch_count, 0);
        assert!(database.load_all_batches().unwrap().is_empty());
        assert!(database.stored_semantic_effects().unwrap().is_empty());
        assert_eq!(database.diagnostic_row_counts().unwrap(), (0, 0));
        assert_eq!(database.reference_source_coverage_count().unwrap(), 0);
        assert!(database
            .reference_page_candidates_for_name("absent", 2)
            .unwrap()
            .is_empty());
        assert_eq!(
            database
                .materialized_read(0, ContentDigest::of(empty_frontier))
                .unwrap()
                .acceptance_sequence(),
            0
        );
    }

    #[test]
    fn reopen_accepts_only_the_expected_claim_and_schema() {
        let path = TestDatabasePath::new("reopen");
        let database = PhysicalSqliteDatabase::open_writable(path.as_path()).unwrap();
        database.initialize_schema(claim(11), b"empty").unwrap();
        drop(database);

        let reopened = PhysicalSqliteDatabase::open_read_only(path.as_path()).unwrap();
        reopened.validate_schema_and_claim(claim(11)).unwrap();
        assert!(matches!(
            reopened.validate_schema_and_claim(claim(12)),
            Err(FrontierError::ClaimBytes {
                field: "workspace_id",
                ..
            })
        ));
        drop(reopened);

        let writable = PhysicalSqliteDatabase::open_writable(path.as_path()).unwrap();
        writable
            .execute_corrupting_sql_for_test("PRAGMA user_version = 999")
            .unwrap();
        assert!(matches!(
            writable.validate_schema_and_claim(claim(11)),
            Err(FrontierError::Schema(_))
        ));
    }

    #[test]
    fn malformed_file_fails_closed_on_integrity_check() {
        let path = TestDatabasePath::new("corrupt");
        fs::write(path.as_path(), b"not a SQLite database").unwrap();
        let database = PhysicalSqliteDatabase::open_read_only(path.as_path()).unwrap();
        assert!(database.quick_check().is_err());
    }

    #[test]
    fn truncate_checkpoint_drains_wal_and_candidate_checkpoint_disables_it() {
        let path = TestDatabasePath::new("checkpoint");
        let database = PhysicalSqliteDatabase::open_writable(path.as_path()).unwrap();
        database.initialize_schema(claim(17), b"empty").unwrap();
        database
            .execute_corrupting_sql_for_test(
                "UPDATE meta SET managed_entity_set_version = managed_entity_set_version",
            )
            .unwrap();
        let wal_path = SqliteFileSet::new(path.as_path()).wal_path().to_path_buf();
        assert!(fs::metadata(&wal_path).unwrap().len() > 0);

        database.checkpoint_truncate().unwrap();
        assert_eq!(fs::metadata(&wal_path).unwrap().len(), 0);
        database.checkpoint_truncate_and_disable_wal().unwrap();
        drop(database);

        let reopened = Connection::open(path.as_path()).unwrap();
        let journal_mode: String = reopened
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert!(journal_mode.eq_ignore_ascii_case("delete"));
    }
}
