//! Regime-neutral disposable graph projection.
//!
//! This database owns only parser-derived graph facts and their indexes. It has
//! no oplog frontier, sync role, authority claim, or managed-storage lifecycle.
//! A Direct Files watcher/parser and a managed accepted-event adapter can feed
//! the same page replacement/delete transaction.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::sqlite_materialization::{
    self, ApplyChangeInstrumentation, MaterializationError, PhysicalGraphProjectionChange,
    SqliteGraphProjectionRead,
};

const PREPARED_STATEMENT_CACHE_STATEMENTS: usize = 64;
const SOURCE_REVISION_MAX_BYTES: usize = 4096;
const SOURCE_REVISIONS_DDL: &str = "CREATE TABLE direct_source_revisions (
    page_id BLOB PRIMARY KEY CHECK (length(page_id) = 16),
    revision TEXT NOT NULL CHECK (length(CAST(revision AS BLOB)) BETWEEN 1 AND 4096),
    FOREIGN KEY (page_id) REFERENCES pages(page_id) ON DELETE CASCADE
) STRICT";

/// Exact application-authority revision for one disposable projection page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicalGraphProjectionSourceRevision {
    pub page_id: [u8; 16],
    pub revision: String,
}

/// Page IDs whose physical facts differ from an application's current source.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhysicalGraphProjectionSourceDelta {
    pub replacements: Vec<[u8; 16]>,
    pub deletions: Vec<[u8; 16]>,
}

/// Connection-owning standalone graph-fact projection.
///
/// The file is a cache. `synchronous=NORMAL` protects SQLite consistency while
/// avoiding authority-grade barriers on every observed file edit; if the cache
/// is missing, stale, or fails validation, the caller rebuilds it from its
/// actual authority.
pub struct PhysicalGraphProjectionDatabase {
    connection: Connection,
}

impl PhysicalGraphProjectionDatabase {
    pub fn open_writable(path: &Path) -> Result<Self, MaterializationError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.set_prepared_statement_cache_capacity(PREPARED_STATEMENT_CACHE_STATEMENTS);
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;",
        )?;
        Ok(Self { connection })
    }

    pub fn open_read_only(path: &Path) -> Result<Self, MaterializationError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { connection })
    }

    pub fn initialize_schema(&self) -> Result<(), MaterializationError> {
        sqlite_materialization::initialize_graph_projection_schema(&self.connection)?;
        self.connection
            .execute_batch(&format!("{SOURCE_REVISIONS_DDL};"))?;
        Ok(())
    }

    pub fn validate_schema(&self) -> Result<(), MaterializationError> {
        sqlite_materialization::validate_graph_projection_schema(&self.connection)?;
        let mut statement = self
            .connection
            .prepare("PRAGMA table_info(direct_source_revisions)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        if columns != ["page_id", "revision"] {
            return Err(MaterializationError::Schema(format!(
                "direct_source_revisions columns {columns:?} != [page_id, revision]"
            )));
        }
        Ok(())
    }

    pub fn quick_check(&self) -> Result<(), MaterializationError> {
        let result: String = self
            .connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        if result != "ok" {
            return Err(MaterializationError::Corrupt(format!(
                "SQLite graph projection quick_check failed: {result}"
            )));
        }
        Ok(())
    }

    pub fn apply(
        &mut self,
        change: &PhysicalGraphProjectionChange,
    ) -> Result<ApplyChangeInstrumentation, MaterializationError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let instrumentation = sqlite_materialization::apply_graph_projection_rows(
            &transaction,
            &change.replacements,
            &change.deletions,
        )?;
        for page in &change.replacements {
            transaction.execute(
                "DELETE FROM direct_source_revisions WHERE page_id = ?1",
                rusqlite::params![page.page_id.as_slice()],
            )?;
        }
        for page_id in &change.deletions {
            transaction.execute(
                "DELETE FROM direct_source_revisions WHERE page_id = ?1",
                rusqlite::params![page_id.as_slice()],
            )?;
        }
        transaction.commit()?;
        Ok(instrumentation)
    }

    /// Apply physical page facts and publish the exact caller-owned source
    /// revisions in the same SQLite transaction.
    pub fn apply_with_source_revisions(
        &mut self,
        change: &PhysicalGraphProjectionChange,
        revisions: &[PhysicalGraphProjectionSourceRevision],
    ) -> Result<ApplyChangeInstrumentation, MaterializationError> {
        let replacement_ids = change
            .replacements
            .iter()
            .map(|page| page.page_id)
            .collect::<BTreeSet<_>>();
        let revision_ids = validated_source_revisions(revisions)?
            .into_keys()
            .collect::<BTreeSet<_>>();
        if replacement_ids != revision_ids {
            return Err(MaterializationError::InvalidInput(
                "source revisions must exactly cover replacement pages".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let instrumentation = sqlite_materialization::apply_graph_projection_rows(
            &transaction,
            &change.replacements,
            &change.deletions,
        )?;
        for page_id in &change.deletions {
            transaction.execute(
                "DELETE FROM direct_source_revisions WHERE page_id = ?1",
                rusqlite::params![page_id.as_slice()],
            )?;
        }
        for revision in revisions {
            transaction.execute(
                "INSERT INTO direct_source_revisions (page_id, revision)
                 VALUES (?1, ?2)
                 ON CONFLICT(page_id) DO UPDATE SET revision = excluded.revision",
                rusqlite::params![revision.page_id.as_slice(), &revision.revision],
            )?;
        }
        transaction.commit()?;
        Ok(instrumentation)
    }

    /// Compare caller authority revisions to the persisted disposable facts.
    /// Missing metadata is stale, never authoritative.
    pub fn source_delta(
        &self,
        current: &[PhysicalGraphProjectionSourceRevision],
    ) -> Result<PhysicalGraphProjectionSourceDelta, MaterializationError> {
        let current = validated_source_revisions(current)?;
        let mut existing = BTreeMap::<[u8; 16], Option<String>>::new();
        let mut statement = self.connection.prepare(
            "SELECT p.page_id, s.revision
             FROM pages AS p
             LEFT JOIN direct_source_revisions AS s ON s.page_id = p.page_id
             ORDER BY p.page_id",
        )?;
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<String>>(1)?))
        })? {
            let (page_id, revision) = row?;
            let page_id: [u8; 16] = page_id.try_into().map_err(|_| {
                MaterializationError::Corrupt("stored page ID is not 16 bytes".into())
            })?;
            existing.insert(page_id, revision);
        }
        let replacements = current
            .iter()
            .filter_map(|(page_id, revision)| {
                (existing.get(page_id).and_then(Option::as_ref) != Some(revision))
                    .then_some(*page_id)
            })
            .collect();
        let deletions = existing
            .keys()
            .filter(|page_id| !current.contains_key(*page_id))
            .copied()
            .collect();
        Ok(PhysicalGraphProjectionSourceDelta {
            replacements,
            deletions,
        })
    }

    pub fn reset(&mut self) -> Result<(), MaterializationError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        sqlite_materialization::reset_graph_projection_rows(&transaction)?;
        transaction.execute("DELETE FROM direct_source_revisions", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn read(&self) -> SqliteGraphProjectionRead<'_> {
        SqliteGraphProjectionRead::new(&self.connection)
    }

    pub fn checkpoint_truncate(&self) -> Result<(), MaterializationError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }
}

fn validated_source_revisions(
    revisions: &[PhysicalGraphProjectionSourceRevision],
) -> Result<BTreeMap<[u8; 16], String>, MaterializationError> {
    let mut validated = BTreeMap::new();
    for revision in revisions {
        if revision.revision.is_empty() || revision.revision.len() > SOURCE_REVISION_MAX_BYTES {
            return Err(MaterializationError::InvalidInput(
                "source revision must contain 1..=4096 bytes".into(),
            ));
        }
        if validated
            .insert(revision.page_id, revision.revision.clone())
            .is_some()
        {
            return Err(MaterializationError::InvalidInput(
                "source revisions contain a duplicate page ID".into(),
            ));
        }
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::sqlite_materialization::{
        PhysicalBlock, PhysicalMaterializationChange, PhysicalPage, PhysicalTask,
    };
    use crate::ContentDigest;

    fn page(page_id: u8, task: &str, content: &str) -> PhysicalPage {
        PhysicalPage {
            page_id: [page_id; 16],
            home_document_id: [page_id; 16],
            name: format!("Page {page_id}"),
            name_key: format!("page {page_id}"),
            path: format!("pages/page-{page_id}.md"),
            text_kind: 0,
            preamble: None,
            searchable_text: content.into(),
            normalized_searchable_text: content.to_lowercase(),
            references: Vec::new(),
            properties: Vec::new(),
            tags: Vec::new(),
            blocks: vec![PhysicalBlock {
                block_id: [page_id.saturating_add(100); 16],
                home_document_id: [page_id; 16],
                parent: None,
                order: "0001".into(),
                content: content.into(),
                searchable_text: content.into(),
                normalized_searchable_text: content.to_lowercase(),
                heading_level: None,
                collapsed: false,
                logseq_uuid: None,
                logseq_identity_origin: None,
                references: Vec::new(),
                properties: Vec::new(),
                tags: Vec::new(),
                task: Some(PhysicalTask {
                    marker: task.into(),
                    priority: Some("A".into()),
                    scheduled: None,
                    deadline: None,
                }),
            }],
        }
    }

    #[test]
    fn standalone_projection_applies_replaces_deletes_and_reads_graph_facts() {
        let path = std::env::temp_dir().join(format!(
            "tine-storage-graph-projection-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let mut database = PhysicalGraphProjectionDatabase::open_writable(&path).unwrap();
        database.initialize_schema().unwrap();
        database.validate_schema().unwrap();

        let managed_tables: i64 = database
            .connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table'
                   AND name IN ('materialization_stamp', 'materialization_batches')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            managed_tables, 0,
            "the standalone graph projection must not grow managed-frontier tables"
        );

        database
            .apply(&PhysicalGraphProjectionChange {
                replacements: vec![page(1, "TODO", "Needle first")],
                deletions: Vec::new(),
            })
            .unwrap();
        assert_eq!(database.read().tasks(Some("TODO"), 10).unwrap().len(), 1);
        assert_eq!(database.read().search("needle", 10).unwrap().len(), 2);

        database
            .apply(&PhysicalGraphProjectionChange {
                replacements: vec![page(1, "DONE", "Needle changed")],
                deletions: Vec::new(),
            })
            .unwrap();
        assert!(database.read().tasks(Some("TODO"), 10).unwrap().is_empty());
        assert_eq!(database.read().tasks(Some("DONE"), 10).unwrap().len(), 1);

        database
            .apply(&PhysicalGraphProjectionChange {
                replacements: Vec::new(),
                deletions: vec![[1; 16]],
            })
            .unwrap();
        assert!(database.read().tasks(None, 10).unwrap().is_empty());
        assert!(database.read().search("needle", 10).unwrap().is_empty());
        database.quick_check().unwrap();
        drop(database);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn standalone_source_revisions_reuse_exact_pages_and_localize_changes() {
        let path = std::env::temp_dir().join(format!(
            "tine-storage-source-revisions-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let mut database = PhysicalGraphProjectionDatabase::open_writable(&path).unwrap();
        database.initialize_schema().unwrap();
        let initial_revisions = vec![
            PhysicalGraphProjectionSourceRevision {
                page_id: [1; 16],
                revision: "rev-1".into(),
            },
            PhysicalGraphProjectionSourceRevision {
                page_id: [2; 16],
                revision: "rev-2".into(),
            },
        ];
        database
            .apply_with_source_revisions(
                &PhysicalGraphProjectionChange {
                    replacements: vec![page(1, "TODO", "first"), page(2, "DONE", "second")],
                    deletions: Vec::new(),
                },
                &initial_revisions,
            )
            .unwrap();
        assert_eq!(
            database.source_delta(&initial_revisions).unwrap(),
            PhysicalGraphProjectionSourceDelta::default()
        );

        drop(database);
        let mut reopened = PhysicalGraphProjectionDatabase::open_writable(&path).unwrap();
        reopened.validate_schema().unwrap();
        let changed = vec![
            PhysicalGraphProjectionSourceRevision {
                page_id: [1; 16],
                revision: "rev-1-new".into(),
            },
            PhysicalGraphProjectionSourceRevision {
                page_id: [3; 16],
                revision: "rev-3".into(),
            },
        ];
        assert_eq!(
            reopened.source_delta(&changed).unwrap(),
            PhysicalGraphProjectionSourceDelta {
                replacements: vec![[1; 16], [3; 16]],
                deletions: vec![[2; 16]],
            }
        );
        reopened
            .apply_with_source_revisions(
                &PhysicalGraphProjectionChange {
                    replacements: vec![page(1, "DONE", "first changed"), page(3, "TODO", "third")],
                    deletions: vec![[2; 16]],
                },
                &changed,
            )
            .unwrap();
        assert_eq!(
            reopened.source_delta(&changed).unwrap(),
            PhysicalGraphProjectionSourceDelta::default()
        );
        reopened.quick_check().unwrap();
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn ordinary_apply_invalidates_source_reuse_for_replaced_pages() {
        let path = std::env::temp_dir().join(format!(
            "tine-storage-source-invalidation-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let mut database = PhysicalGraphProjectionDatabase::open_writable(&path).unwrap();
        database.initialize_schema().unwrap();
        let revision = PhysicalGraphProjectionSourceRevision {
            page_id: [1; 16],
            revision: "exact-source".into(),
        };
        database
            .apply_with_source_revisions(
                &PhysicalGraphProjectionChange {
                    replacements: vec![page(1, "TODO", "first")],
                    deletions: Vec::new(),
                },
                std::slice::from_ref(&revision),
            )
            .unwrap();
        database
            .apply(&PhysicalGraphProjectionChange {
                replacements: vec![page(1, "DONE", "untracked replacement")],
                deletions: Vec::new(),
            })
            .unwrap();
        assert_eq!(
            database
                .source_delta(std::slice::from_ref(&revision))
                .unwrap(),
            PhysicalGraphProjectionSourceDelta {
                replacements: vec![[1; 16]],
                deletions: Vec::new(),
            }
        );
        drop(database);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn standalone_and_managed_adapters_materialize_identical_graph_facts() {
        let path = std::env::temp_dir().join(format!(
            "tine-storage-graph-projection-parity-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let source = page(7, "TODO", "Shared projection needle");

        let mut standalone = PhysicalGraphProjectionDatabase::open_writable(&path).unwrap();
        standalone.initialize_schema().unwrap();
        standalone
            .apply(&PhysicalGraphProjectionChange {
                replacements: vec![source.clone()],
                deletions: Vec::new(),
            })
            .unwrap();

        let managed = Connection::open_in_memory().unwrap();
        let empty = ContentDigest::of(b"empty");
        let frontier = ContentDigest::of(b"frontier-1");
        sqlite_materialization::initialize_schema(&managed, empty).unwrap();
        let transaction = managed.unchecked_transaction().unwrap();
        sqlite_materialization::apply_change(
            &transaction,
            &PhysicalMaterializationChange {
                batch_id: [9; 16],
                replacements: vec![source],
                deletions: Vec::new(),
                pages_with_live_metadata_delta: BTreeSet::from([[7; 16]]),
                reference_catalog: None,
            },
            1,
            ContentDigest::of(b"input"),
            frontier,
            None,
        )
        .unwrap();
        transaction.commit().unwrap();
        let managed_read =
            sqlite_materialization::SqliteMaterializedRead::new(&managed, 1, frontier).unwrap();

        assert_eq!(
            standalone.read().tasks(None, 10).unwrap(),
            managed_read.tasks(None, 10).unwrap()
        );
        assert_eq!(
            standalone.read().search("needle", 10).unwrap(),
            managed_read.search("needle", 10).unwrap()
        );
        assert_eq!(
            standalone.read().pages(None, 10).unwrap(),
            managed_read.pages(None, 10).unwrap()
        );

        drop(standalone);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }
}
