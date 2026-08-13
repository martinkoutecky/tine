//! Regime-neutral disposable graph projection.
//!
//! This database owns only parser-derived graph facts and their indexes. It has
//! no oplog frontier, sync role, authority claim, or managed-storage lifecycle.
//! A Direct Files watcher/parser and a managed accepted-event adapter can feed
//! the same page replacement/delete transaction.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use crate::sqlite_materialization::{
    self, ApplyChangeInstrumentation, MaterializationError, PhysicalGraphProjectionChange,
    SqliteGraphProjectionRead,
};

const PREPARED_STATEMENT_CACHE_STATEMENTS: usize = 64;

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
        sqlite_materialization::initialize_graph_projection_schema(&self.connection)
    }

    pub fn validate_schema(&self) -> Result<(), MaterializationError> {
        sqlite_materialization::validate_graph_projection_schema(&self.connection)
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
        transaction.commit()?;
        Ok(instrumentation)
    }

    pub fn reset(&mut self) -> Result<(), MaterializationError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        sqlite_materialization::reset_graph_projection_rows(&transaction)?;
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
