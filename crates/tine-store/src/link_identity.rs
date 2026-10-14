//! Stable graph identity for external links (GH #181). Browsing reads only;
//! explicit link copy alone creates one 37-byte UUID file through Transaction.
//! A concurrent creator wins via no-clobber publication. Crash before rename
//! leaves no identity; crash after rename leaves the complete reusable identity.
use crate::{Area, Content, Store, StoreError, TxOutcome, Why};
use std::path::Path;

const NAME: &str = "tine-graph-id";

/// Which external-link question to inspect before opening a known graph.
pub enum LinkProbe<'a> {
    /// Read only the graph identity, without inspecting target files.
    Identity,
    /// Rule out graphs whose text cannot contain this block UUID.
    Block {
        /// The requested block UUID.
        id: &'a str,
        /// Previously approved external assets directory, as for Store opening.
        approved_external_assets: Option<&'a Path>,
    },
    /// Rule out graphs whose discovered page names cannot match this name.
    Page {
        /// The requested page name.
        name: &'a str,
        /// Previously approved external assets directory, as for Store opening.
        approved_external_assets: Option<&'a Path>,
    },
}

/// Identity errors remain independent of conservative target discovery.
pub struct LinkInspection {
    /// Existing identity, absence, or the unchanged identity inspection error.
    pub identity: Result<Option<String>, String>,
    /// False only for a definite target miss; true for identity-only or uncertainty.
    pub target_possible: bool,
}

fn decode(bytes: &[u8]) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Graph link identity is not UTF-8")?;
    uuid::Uuid::parse_str(text.trim())
        .map(|id| id.to_string())
        .map_err(|_| "Graph link identity is invalid; existing file was left untouched".into())
}

impl Store {
    /// Conservative, read-only probe for a link into a closed graph. A false
    /// result rules out its target without starting a Store, watcher or graph
    /// parse. Runtime UUID candidates retain full lookup without scanning.
    /// Other block probes stream graph-text bytes case-insensitively; page
    /// probes reuse parser-owned preamble names and journal naming. Positive
    /// or uncertain probes (including I/O/layout failures) require a full
    /// Store lookup. No persistent index or graph writes. Cost O(graph bytes)
    /// for blocks, O(files + preamble bytes) for pages; bounded per file by the
    /// existing parse-input limit and one 64 KiB streaming buffer for blocks.
    /// Identity-only inspection is O(path components + 128 bounded bytes).
    /// Missing identity is None; identity errors do not suppress target probes.
    pub fn read_link_identity_at(root: &Path, probe: LinkProbe<'_>) -> LinkInspection {
        let identity = read_identity(root);
        let (block, page, approved_external_assets) = match probe {
            LinkProbe::Identity => {
                return LinkInspection {
                    identity,
                    target_possible: true,
                }
            }
            LinkProbe::Block {
                id,
                approved_external_assets,
            } => {
                if tine_core::projection::may_be_runtime_block_id(id) {
                    return LinkInspection {
                        identity,
                        target_possible: true,
                    };
                }
                (Some(id), None, approved_external_assets)
            }
            LinkProbe::Page {
                name,
                approved_external_assets,
            } => (None, Some(name), approved_external_assets),
        };
        let target_possible = Self::canonical_root(root)
            .ok()
            .and_then(|root| {
                let graph = crate::model::Graph::open_checked_with_assets_inner(
                    &root,
                    approved_external_assets,
                )
                .ok()?;
                if graph.config_read_problem.is_some() {
                    return None;
                }
                graph.link_target_may_exist(block, page).ok()
            })
            .unwrap_or(true);
        LinkInspection {
            identity,
            target_possible,
        }
    }
}

// Never reconstruct an established identity or write while inspecting it.
fn read_identity(root: &Path) -> Result<Option<String>, String> {
    let root = Store::canonical_root(root).map_err(|e| e.to_string())?;
    Store::inspect(&root).map_err(|e| e.to_string())?;
    let path = root.join("logseq").join(NAME);
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(meta) if !meta.is_file() || meta.len() > 128 => {
            return Err("Invalid graph link identity file".into())
        }
        Ok(_) => {}
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(129)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 128 {
        return Err("Graph link identity is too large".into());
    }
    decode(&bytes).map(Some)
}

impl Store {
    /// Return or lazily create the stable graph UUID on explicit Copy link.
    /// O(128 bytes), plus transaction publication cost on first creation. One
    /// file, 37 bytes independent of page size; no per-edit or transport record.
    /// Disk/layout errors and malformed existing identity are returned unchanged.
    /// Only a clean create collision reads the concurrent winner. Other failed
    /// transactions reject, even when new bytes remain visible on disk.
    pub fn ensure_link_identity(&self) -> Result<String, String> {
        let file = self
            .file_id(Area::Meta, NAME)
            .map_err(|e| format!("{e:?}"))?;
        match self.read(&file, Some(128)) {
            Ok((bytes, _)) => return decode(&bytes),
            Err(StoreError::NotFound) => {}
            Err(e) => return Err(format!("{e:?}")),
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut tx = self.transaction(None);
        tx.create(&file, Content::Bytes(format!("{id}\n").into_bytes()));
        match tx.commit() {
            TxOutcome::Committed { .. } => Ok(id),
            TxOutcome::NotCommitted {
                why: Why::Conflict { disk: Some(_), .. },
                rollback,
                publication_errors,
                ..
            } if rollback.undo_failed.is_empty() && publication_errors.is_empty() => self
                .read(&file, Some(128))
                .map_err(|e| format!("Couldn't read concurrent graph identity: {e:?}"))
                .and_then(|(bytes, _)| decode(&bytes)),
            outcome => Err(format!("Couldn't save graph link identity: {outcome:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn link_inspection_keeps_identity_errors_separate_from_probe_uncertainty() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("logseq")).unwrap();
        std::fs::write(temp.path().join("logseq/tine-graph-id"), "torn").unwrap();
        let identity = Store::read_link_identity_at(temp.path(), LinkProbe::Identity);
        assert!(identity.identity.is_err());
        assert!(identity.target_possible);
        let missing = Store::read_link_identity_at(
            temp.path(),
            LinkProbe::Block {
                id: "11111111-1111-4111-8111-111111111111",
                approved_external_assets: None,
            },
        );
        assert!(missing.identity.is_err());
        assert!(!missing.target_possible);
        std::fs::write(
            temp.path().join("logseq/config.edn"),
            "{:pages-directory \"../outside\"}",
        )
        .unwrap();
        let uncertain = Store::read_link_identity_at(
            temp.path(),
            LinkProbe::Page {
                name: "Missing",
                approved_external_assets: None,
            },
        );
        assert!(uncertain.identity.is_err());
        assert!(uncertain.target_possible);
    }

    #[test]
    fn link_identity_crash_worker() {
        let Ok(root) = std::env::var("TINE_LINK_CRASH_ROOT") else {
            return;
        };
        let (store, _, _) = Store::open(Path::new(&root), Default::default()).unwrap();
        store.inject_fault(crate::FaultPoint::AbortAfterStep(0));
        store.ensure_link_identity().unwrap();
        panic!("identity publication did not reach the crash boundary");
    }
    #[test]
    fn link_identity_kill_reopen_keeps_published_id_and_page_bytes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("pages")).unwrap();
        std::fs::write(temp.path().join("pages/A.md"), b"- no block id yet\n").unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "link_identity::tests::link_identity_crash_worker",
                "--nocapture",
            ])
            .env("TINE_LINK_CRASH_ROOT", temp.path())
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "worker must die after identity publication"
        );
        let published = Store::read_link_identity_at(temp.path(), LinkProbe::Identity)
            .identity
            .unwrap()
            .unwrap();
        let (reopened, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        assert_eq!(reopened.ensure_link_identity().unwrap(), published);
        assert_eq!(
            std::fs::read(temp.path().join("pages/A.md")).unwrap(),
            b"- no block id yet\n"
        );
    }
    #[test]
    fn link_identity_failed_write_can_retry_without_partial_identity() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        store.inject_fault(crate::FaultPoint::MidStepIo);
        assert!(store.ensure_link_identity().is_err());
        assert_eq!(
            Store::read_link_identity_at(temp.path(), LinkProbe::Identity)
                .identity
                .unwrap(),
            None
        );
        let id = store.ensure_link_identity().unwrap();
        assert_eq!(
            Store::read_link_identity_at(temp.path(), LinkProbe::Identity)
                .identity
                .unwrap(),
            Some(id)
        );
    }
    #[test]
    fn link_identity_reports_sync_and_publication_failures() {
        // Directory sync exists only on Unix targets: Windows has no
        // directory flush (`sync_directory_entry` returns Ok there), so the
        // DirectorySyncIo fault is armed only on Unix, as in
        // tests/transaction.rs. Its scenario (EIO/ENOSPC on a directory
        // fsync after publication) cannot arise on Windows.
        let mut points = vec![crate::FaultPoint::PublicationReadIo];
        if cfg!(unix) {
            points.insert(0, crate::FaultPoint::DirectorySyncIo);
        }
        for point in points {
            let temp = tempfile::tempdir().unwrap();
            let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
            store.inject_fault(point);
            assert!(
                store.ensure_link_identity().is_err(),
                "failed publication must not authorize clipboard copy"
            );
            let existing = Store::read_link_identity_at(temp.path(), LinkProbe::Identity)
                .identity
                .unwrap();
            let retried = store.ensure_link_identity().unwrap();
            if let Some(existing) = existing {
                assert_eq!(retried, existing);
            }
        }
    }
    #[test]
    fn link_identity_is_lazy_durable_and_survives_moves() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("graph");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("logseq")).unwrap();
        let page = root.join("pages/A.md");
        std::fs::write(&page, "- unchanged\r\n").unwrap();
        assert_eq!(
            Store::read_link_identity_at(&root, LinkProbe::Identity)
                .identity
                .unwrap(),
            None
        );
        let (store, _, _) = Store::open(&root, Default::default()).unwrap();
        assert!(!root.join("logseq/tine-graph-id").exists());
        let id = store.ensure_link_identity().unwrap();
        assert_eq!(store.ensure_link_identity().unwrap(), id);
        assert_eq!(
            std::fs::read(root.join("logseq/tine-graph-id"))
                .unwrap()
                .len(),
            37
        );
        assert_eq!(std::fs::read_dir(root.join("logseq")).unwrap().count(), 1);
        drop(store);
        let moved = temp.path().join("renamed");
        std::fs::rename(root, &moved).unwrap();
        assert_eq!(
            Store::read_link_identity_at(&moved, LinkProbe::Identity)
                .identity
                .unwrap(),
            Some(id)
        );
        assert_eq!(
            std::fs::read(moved.join("pages/A.md")).unwrap(),
            b"- unchanged\r\n"
        );
    }
    #[test]
    fn malformed_identity_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("logseq")).unwrap();
        let path = temp.path().join("logseq/tine-graph-id");
        std::fs::write(&path, b"torn").unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        assert!(store.ensure_link_identity().is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"torn");
    }
}
