//! Guarded multi-file graph writes. A transaction queues steps without disk
//! I/O; commit checks them, applies them in order, and attempts undo on a
//! failed apply. Changed final bytes publish before return, with external
//! bytes from undo in a separate `Origin::External` change. A config-file
//! write reloads effective settings before publication of the final state;
//! undo or a failed apply reconciles them
//! against the final disk file.
//! Transactions are not crash atomic and cannot exclude external processes.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use tine_core::doc::Document;
use tine_core::model::PageDto;

use crate::model::{
    atomic_copy_file_new, atomic_copy_new, atomic_write, atomic_write_new, atomic_write_with_check,
    move_file_noreplace, trash_stamp, Withdrawal,
};
use crate::store::{
    Area, ChangeKind, FileId, FileRev, GraphRev, Origin, PageId, SaveBase, Store, StoreError,
};

/// Content staged for a new file. Streams are bounded while being copied.
pub enum Content {
    /// Bytes held by the caller.
    Bytes(Vec<u8>),
    /// Read from an open file during commit. Exceeding `max_bytes` returns
    /// `Why::Failed` with an invalid-data I/O cause; bytes are not truncated.
    Stream {
        /// Open source file.
        source: File,
        /// Maximum accepted byte count.
        max_bytes: u64,
    },
}

/// Old page or tag name to new name, compared using normalized references.
/// Matching trims surrounding space, removes one boundary slash, then uses
/// Unicode lowercase plus NFC: a `Foo` entry
/// does not rewrite `Foo/Child`. Destination spelling is written as supplied;
/// bare tags use brackets for multiword names.
/// Rewrites `[[page]]`, bare and bracketed tags, supported Org page links,
/// embeds containing those references, and bare `tags::` values. Other
/// reference-bearing property values can be found by `explicit_referrers`
/// without being rewritten. Code spans, `alias::`, `title::`, and query
/// arguments are not rewritten.
#[derive(Clone, Debug, Default)]
pub struct RenameMap(pub Vec<(String, String)>);

/// I/O error with a stable kind for handling and a message for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoError {
    /// Platform-independent I/O error category.
    pub kind: io::ErrorKind,
    /// Human-readable cause.
    pub message: String,
}

impl From<io::Error> for IoError {
    fn from(error: io::Error) -> Self {
        Self {
            kind: error.kind(),
            message: error.to_string(),
        }
    }
}

/// Result of an individual committed transaction step.
#[derive(Debug)]
pub enum StepResult {
    /// File was created or replaced.
    Written {
        /// Written file.
        file: FileId,
        /// Revision of its new bytes.
        rev: FileRev,
    },
    /// Save or reference rewrite found the same bytes on disk after its
    /// revision guard matched. A stale base still returns `Why::Conflict`.
    Unchanged {
        /// Unchanged file.
        file: FileId,
        /// Current disk revision.
        rev: FileRev,
    },
    /// Source was moved into graph trash.
    Trashed {
        /// Original file identity.
        file: FileId,
        /// New identity in the trash area.
        trashed: FileId,
    },
    /// File was moved to a new identity. If rewritten bytes required retiring
    /// the source into trash, that trash id is not returned here.
    Moved {
        /// Destination identity.
        to: FileId,
        /// Revision at the destination.
        rev: FileRev,
    },
}

/// A transaction step rejected without authorization to overwrite user data.
#[derive(Debug)]
pub enum Refusal {
    /// Source page cannot be round-tripped safely; reason is for display.
    ReadOnly(String),
    /// Invalid or unsafe destination; reason is for display.
    InvalidTarget(String),
    /// Another page file claims the same name or journal day.
    Twin {
        /// Existing claimant.
        existing: PageId,
    },
    /// Page bytes are not UTF-8.
    Undecodable,
    /// A transaction named the same file more than once. Editing a page and
    /// moving that same file require separate transactions.
    RepeatedFile(FileId),
    /// The store was closed before commit.
    Closed,
}

/// Reason a transaction did not commit.
#[derive(Debug)]
pub enum Why {
    /// An expected revision did not match the disk state. `disk: None` means
    /// the file disappeared; do not recreate it without a new user decision.
    Conflict {
        /// File whose guard failed.
        file: FileId,
        /// Current revision, or `None` if absent.
        disk: Option<FileRev>,
    },
    /// A step was refused by policy or validation.
    Refused(Refusal),
    /// An I/O operation failed.
    Failed(IoError),
}

/// Disk differences left after an unsuccessful transaction's undo.
#[derive(Debug, Default)]
pub struct Rollback {
    /// External changes preserved during undo. `Some(id)` names where changed
    /// bytes were moved into the trash recovery area; `None` means they remain
    /// live. On a changed live file, undo stages it in conflict trash, compares
    /// it with the transaction's bytes, and tries a no-replace move back. It
    /// remains in recovery only if a new live winner took the name. Check
    /// `undo_failed` separately to learn whether old bytes were restored.
    /// Pre-transaction bytes are held in memory during commit; undo writes
    /// them back when possible, or preserves a copy in conflict recovery
    /// when a different live file wins the name. If an external file remains
    /// live, those prior bytes are preserved separately in conflict recovery;
    /// `kept_external` names the external file, not that prior-byte copy.
    pub kept_external: Vec<(FileId, Option<FileId>)>,
    /// Files undo could not restore, with their errors.
    pub undo_failed: Vec<(FileId, IoError)>,
}

/// A committed set of steps or a refusal with its undo result.
#[derive(Debug)]
pub enum TxOutcome {
    /// All steps completed. A changed transaction normally publishes a view
    /// of its final state. After a failed initial load, its writes stand but
    /// publication waits for successful `scan_refresh()` recovery.
    Committed {
        /// Results in input order.
        steps: Vec<StepResult>,
        /// Generation publishing the disk state, or current one if unchanged.
        /// During a failed initial load this is the unchanged current revision:
        /// no view covers the write until recovery's first view does.
        graph_rev: GraphRev,
    },
    /// The transaction did not commit. Preflight checks every step before any
    /// write; an apply failure attempts undo of prior steps and the failed
    /// step, which may already have written. Inspect the final disk state and
    /// `rollback`, including for `step`. Changed final disk bytes publish after
    /// a successful initial load. Store-written bytes use `Origin::Own`;
    /// concurrent external bytes surviving undo use `Origin::External`.
    /// A watcher echo is not guaranteed. After a failed initial load, publication waits
    /// for successful `scan_refresh()` recovery.
    NotCommitted {
        /// Zero-based index of the failed step.
        step: usize,
        /// Conflict, refusal, or I/O failure.
        why: Why,
        /// Differences undo could not remove; empty on preflight failure.
        rollback: Rollback,
        /// Last generation published for the final disk state. If both own and
        /// external changes survive undo, the external publication comes last.
        /// During a failed initial load this is the unchanged current revision:
        /// no view covers a changed result until recovery's first view does.
        graph_rev: GraphRev,
    },
}

#[cfg(any(test, feature = "test-faults"))]
/// Deterministic one-shot failure hooks for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaultPoint {
    /// Simulate a changed file at the second revision guard.
    Stage2Mismatch,
    /// Simulate a changed file at the indexed step's second guard.
    Stage2MismatchAt(usize),
    /// Simulate an external write after the replacement temp file is synced.
    AfterTempSync,
    /// Simulate a changed but valid sidecar at the second guard.
    Stage2ValidSidecar,
    /// Simulate an external config edit at the second guard.
    Stage2ConfigExternal,
    /// Simulate a no-replace destination collision.
    NoReplaceCollision,
    /// Simulate an I/O error after a step starts.
    MidStepIo,
    /// Simulate an I/O error at the indexed step.
    MidStepIoAt(usize),
    /// Simulate an external write while undoing a live file.
    UndoLiveWrite,
    /// Simulate a twin appearing after publication.
    TwinAfterPublish,
    /// Abort the process immediately after the indexed step has reached disk.
    AbortAfterStep(usize),
}

#[cfg(not(any(test, feature = "test-faults")))]
#[allow(dead_code)] // The production fault check is a no-op; test builds read these indexes.
pub(crate) enum FaultPoint {
    Stage2Mismatch,
    Stage2MismatchAt(usize),
    AfterTempSync,
    Stage2ValidSidecar,
    Stage2ConfigExternal,
    NoReplaceCollision,
    MidStepIo,
    MidStepIoAt(usize),
    UndoLiveWrite,
    TwinAfterPublish,
    AbortAfterStep(usize),
}

#[cfg(any(test, feature = "test-faults"))]
impl Store {
    /// Arm one deterministic, one-shot fault in this store instance.
    pub fn inject_fault(&self, point: FaultPoint) {
        self.faults.lock().unwrap().insert(point);
    }
}

#[cfg(any(test, feature = "test-faults"))]
fn fault(store: &Store, point: FaultPoint) -> bool {
    store.faults.lock().unwrap().remove(&point)
}

#[cfg(not(any(test, feature = "test-faults")))]
fn fault(_store: &Store, _point: FaultPoint) -> bool {
    false
}

enum Step {
    Save {
        id: PageId,
        base: SaveBase,
        doc: PageDto,
    },
    Create {
        file: FileId,
        content: Content,
    },
    Unique {
        area: Area,
        stem: String,
        ext: String,
        content: Content,
    },
    Replace {
        file: FileId,
        expected: FileRev,
        bytes: Vec<u8>,
    },
    Rewrite {
        id: PageId,
        expected: FileRev,
        renames: RenameMap,
    },
    Move {
        file: FileId,
        expected: FileRev,
        to: FileId,
        renames: Option<RenameMap>,
    },
    Trash {
        file: FileId,
        expected: FileRev,
    },
}

struct Prepared {
    src: FileId,
    dst: Option<FileId>,
    old: Option<Vec<u8>>,
    new: Option<Vec<u8>>,
    saved_page: Option<Document>,
}

enum Expected {
    Bytes(Vec<u8>),
    File(PathBuf),
}

enum UndoKind {
    Replace,
    Create,
    Move,
    Rename,
    Trash,
}

struct Undo {
    kind: UndoKind,
    src: FileId,
    dst: Option<FileId>,
    trash: Option<FileId>,
    old: Option<Vec<u8>>,
    new: Option<Expected>,
    created: bool,
    moved: bool,
}

/// Builder for guarded file changes, applied by [`Self::commit`]. Dropping
/// without commit performs no disk I/O.
pub struct Transaction<'a> {
    store: &'a Store,
    steps: Vec<Step>,
}

impl Store {
    /// Begin a transaction. Commit serializes writes through this Store; it
    /// does not lock other Store instances or external processes.
    pub fn transaction(&self) -> Transaction<'_> {
        Transaction {
            store: self,
            steps: Vec::new(),
        }
    }
}

impl<'a> Transaction<'a> {
    /// Queue a guarded page save. `CreateNew` requires absence; `Existing`
    /// compares the current raw-byte revision. No disk I/O until commit.
    /// Commit cost includes page bytes and O(P) graph metadata on publication.
    pub fn save_page(&mut self, id: &PageId, base: SaveBase, doc: &PageDto) -> &mut Self {
        self.steps.push(Step::Save {
            id: id.clone(),
            base,
            doc: doc.clone(),
        });
        self
    }

    /// Queue a no-replace file creation. Page text must be UTF-8 and cannot
    /// claim a name or journal day already held by another file in the store's
    /// current file-list index (built before open returns and updated by later
    /// observations). This takes
    /// raw content, unlike `save_page`'s structured `PageDto` serialization.
    /// Queueing bytes copies O(input bytes); commit writes and syncs them and
    /// can spend O(P) on publication metadata.
    /// It checks target safety and UTF-8 for page files, but does not run the
    /// Org round-trip editability check because no prior page is rewritten.
    pub fn create(&mut self, file: &FileId, content: Content) -> &mut Self {
        self.steps.push(Step::Create {
            file: file.clone(),
            content,
        });
        self
    }

    /// Queue a no-replace creation of `stem` + `ext`, trying `stem_1`,
    /// `stem_2`, and so on on collision. `stem` is one path component; `ext`
    /// includes its leading dot, or is empty. An invalid name is refused.
    /// `Area::Trash` is refused. The chosen id appears in the step result.
    /// Cost O(bytes + collisions). A page target uses the same raw UTF-8 and
    /// target-safety checks as `create`, not `PageDto` serialization.
    pub fn create_unique(
        &mut self,
        area: Area,
        stem: &str,
        ext: &str,
        content: Content,
    ) -> &mut Self {
        self.steps.push(Step::Unique {
            area,
            stem: stem.into(),
            ext: ext.into(),
            content,
        });
        self
    }

    /// Queue replacement of a non-page file guarded by `expected`. Use
    /// `Store::file_id(Area::Meta, "config.edn")` to replace graph config.
    /// If its final bytes change, commit reloads effective config before the
    /// publication; a clean rollback leaves effective config at the baseline.
    /// A config replacement can reparse O(P + B) page and block data in
    /// addition to reading the new config.
    // Reload invalidates parsed graph caches before publication.
    /// Page text must instead use [`Self::save_page`]; passing a page target
    /// is refused as `Refusal::InvalidTarget`.
    pub fn replace(&mut self, file: &FileId, expected: FileRev, bytes: Vec<u8>) -> &mut Self {
        self.steps.push(Step::Replace {
            file: file.clone(),
            expected,
            bytes,
        });
        self
    }

    /// Queue a guarded page-reference rewrite. Obtain each referrer's
    /// `FileRev` with `Store::page` or `Store::read` after locating it in a
    /// graph view. Recheck revisions if the view may be stale; each referrer
    /// needs its own file read. Unchanged output reports [`StepResult::Unchanged`];
    /// unsafe Org edits are refused as read-only. There is no partial rename
    /// mode: omit that referrer from the queued steps if leaving its old link
    /// is acceptable. This rewrites file content,
    /// not an unsaved editor buffer. Commit reads, parses, rewrites and writes
    /// each named referrer, O(its text bytes), plus publication metadata.
    pub fn rewrite_refs(
        &mut self,
        id: &PageId,
        expected: FileRev,
        renames: &RenameMap,
    ) -> &mut Self {
        self.steps.push(Step::Rewrite {
            id: id.clone(),
            expected,
            renames: renames.clone(),
        });
        self
    }

    /// Queue a guarded no-replace move. Optional reference rewrites affect
    /// the destination's references, not its `title::`, aliases, or namespace
    /// children. If bytes change, the old source moves to trash; if
    /// bytes stay equal, the source is renamed directly without a trash copy.
    /// After a move, `resolve` follows the destination filename. A retained
    /// `title::` can still affect display text, but `Change::page` and
    /// `PageDto.name` use the destination's decoded filename claim.
    /// Updating `title::` requires a separate page save and is not atomic with
    /// this move. Referrers created after the source view are not included in
    /// queued rewrites; query referrers again after commit if that matters.
    /// Twin claims are refused. A read-only Org source may move without a
    /// rewrite; asking to rewrite its bytes invokes the round-trip check.
    /// Moves between pages and journals change the file's area identity and
    /// still require a guarded source and free destination. Moving into graph
    /// trash is refused; use [`Self::trash`] for that operation. Commit hashes
    /// source bytes and may rewrite/sync the destination, plus O(P) metadata
    /// for publication.
    pub fn move_file(
        &mut self,
        file: &FileId,
        expected: FileRev,
        to: &FileId,
        renames: Option<&RenameMap>,
    ) -> &mut Self {
        self.steps.push(Step::Move {
            file: file.clone(),
            expected,
            to: to.clone(),
            renames: renames.cloned(),
        });
        self
    }

    /// Queue a guarded move into graph trash. The new id is returned in the
    /// step result, and changed source bytes are preserved on guard failure.
    /// This acts on one `FileId`; for a twinned name the caller decides which
    /// claimant or claimants to remove. A duplicate-day journal is typed as
    /// `TrashKind::Journal`; a sync-conflict-named page or journal copy is
    /// typed as `TrashKind::Conflict`. Trashing an Org file does not serialize its content, so its page-edit
    /// read-only flag does not bar this move. Commit hashes the source bytes,
    /// moves the file, and can spend O(P) on publication metadata.
    pub fn trash(&mut self, file: &FileId, expected: FileRev) -> &mut Self {
        self.steps.push(Step::Trash {
            file: file.clone(),
            expected,
        });
        self
    }

    fn path(&self, file: &FileId) -> Result<PathBuf, Why> {
        let path = self
            .store
            .path_for_os_handoff(file, false)
            .map_err(|error| match error {
                StoreError::InvalidTarget(message) => Why::Refused(Refusal::InvalidTarget(message)),
                StoreError::Io(error)
                    if file.as_str().starts_with("logseq/.tine-trash/")
                        && error.kind() == io::ErrorKind::NotADirectory =>
                {
                    let target = self.store.graph.root.join(file.as_str());
                    failed_trash_dir(error, target.parent().unwrap_or(&target))
                }
                StoreError::Io(error) => Why::Failed(error.into()),
                other => Why::Refused(Refusal::InvalidTarget(format!("{other:?}"))),
            })?;
        let checked = if file.as_str().starts_with("assets/") {
            self.store.graph.ensure_asset_write_target(&path)
        } else {
            self.store.graph.ensure_write_target(&path)
        };
        checked.map_err(|error| Why::Refused(Refusal::InvalidTarget(error.to_string())))?;
        Ok(path)
    }

    fn page(&self, file: &FileId) -> bool {
        self.store.as_page(file).is_some()
    }

    /// `moving_from`: a move's source. It claims the same name or day as the
    /// destination only because it is the file being moved, so it is no twin.
    fn twin(&self, file: &FileId, moving_from: Option<&FileId>) -> Result<(), Why> {
        let Some(id) = self.store.as_page(file) else {
            return Ok(());
        };
        let path = self.path(file)?;
        let Some(entry) = self.store.graph.entry_for_path(&path) else {
            return Ok(());
        };
        if let Some(existing) = self.store.graph.find_entry(&entry.name, entry.kind) {
            let is_source = match moving_from {
                Some(source) => existing.path == self.path(source)?,
                None => false,
            };
            if existing.path != path && !is_source {
                return Err(Why::Refused(Refusal::Twin {
                    existing: PageId::from(self.store.graph.rel_path(&existing.path)),
                }));
            }
        }
        let _ = id;
        Ok(())
    }

    fn disk_twin(&self, file: &FileId) -> Result<Option<FileId>, Why> {
        if !self.page(file) {
            return Ok(None);
        }
        let path = self.path(file)?;
        let alt = match path.extension().and_then(|value| value.to_str()) {
            Some("md") => path.with_extension("org"),
            Some("org") => path.with_extension("md"),
            _ => return Ok(None),
        };
        match fs::symlink_metadata(&alt) {
            Ok(_) => Ok(Some(FileId::from(self.store.graph.rel_path(&alt)))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    fn stage(&self, file: &FileId, expected: &FileRev) -> Result<Vec<u8>, Why> {
        let path = self.path(file)?;
        match fs::read(path) {
            Ok(bytes) if FileRev::from_bytes(&bytes) == *expected => Ok(bytes),
            Ok(bytes) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(FileRev::from_bytes(&bytes)),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Why::Conflict {
                file: file.clone(),
                disk: None,
            }),
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    fn absent(&self, file: &FileId) -> Result<(), Why> {
        let path = self.path(file)?;
        match fs::read(&path) {
            Ok(bytes) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(FileRev::from_bytes(&bytes)),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Why::Failed(directory_read_error(error, &path).into())),
        }
    }

    fn preflight(&self, step: &Step) -> Result<Prepared, Why> {
        let unsupported_config_step = match step {
            Step::Trash { file, .. } => file.as_str() == "logseq/config.edn",
            Step::Unique {
                area, stem, ext, ..
            } => *area == Area::Meta && stem == "config" && ext == "edn",
            Step::Move { file, to, .. } => {
                file.as_str() == "logseq/config.edn" || to.as_str() == "logseq/config.edn"
            }
            _ => false,
        };
        if unsupported_config_step {
            return Err(Why::Refused(Refusal::InvalidTarget(
                "config.edn requires live config publication (B7)".into(),
            )));
        }
        match step {
            Step::Save { id, base, doc } => {
                let file = id.file();
                if !self.page(&file) {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                if matches!(base, SaveBase::CreateNew) {
                    if let Some(existing) = self.disk_twin(&file)? {
                        return Err(Why::Refused(Refusal::Twin {
                            existing: PageId::from(existing.as_str()),
                        }));
                    }
                    self.absent(&file)?;
                    self.twin(&file, None)?;
                }
                let old = match base {
                    SaveBase::Existing(rev) => Some(self.stage(&file, rev)?),
                    SaveBase::CreateNew => None,
                };
                let path = self.path(&file)?;
                let text = match old.as_deref() {
                    Some(bytes) => Some(std::str::from_utf8(bytes).map_err(|error| {
                        failed(io::Error::new(io::ErrorKind::InvalidData, error))
                    })?),
                    None => None,
                };
                let (new, saved_page) = self
                    .store
                    .graph
                    .prepare_page_bytes(doc, &path, text)
                    .map_err(|error| {
                        if error.kind() == io::ErrorKind::PermissionDenied {
                            Why::Refused(Refusal::ReadOnly(error.to_string()))
                        } else {
                            Why::Failed(error.into())
                        }
                    })?;
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old,
                    new: Some(new),
                    saved_page: Some(saved_page),
                })
            }
            Step::Create { file, content } => {
                if file.as_str().starts_with("logseq/.tine-trash/")
                    || file.as_str().starts_with("logseq/.tine-")
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                self.path(file)?;
                self.absent(file)?;
                self.twin(file, None)?;
                if self.page(file)
                    && matches!(content, Content::Bytes(bytes) if std::str::from_utf8(bytes).is_err())
                {
                    return Err(Why::Refused(Refusal::Undecodable));
                }
                if let Content::Stream { source, max_bytes } = content {
                    validate_stream(source, *max_bytes, self.page(file))?;
                }
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old: None,
                    new: None,
                    saved_page: None,
                })
            }
            Step::Unique {
                area,
                stem,
                ext,
                content,
            } => {
                let first = format!("{stem}{ext}");
                if first.is_empty()
                    || first == "."
                    || first == ".."
                    || first.contains('/')
                    || first.contains('\\')
                    || (!ext.is_empty() && !ext.starts_with('.'))
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(format!("{stem}{ext}"))));
                }
                let file = self
                    .store
                    .file_id(*area, &first)
                    .map_err(|_| Why::Refused(Refusal::InvalidTarget(first.clone())))?;
                if *area == Area::Trash
                    || (*area == Area::Meta && file.as_str().contains("/.tine-"))
                {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                self.path(&file)?;
                if let Content::Stream { source, max_bytes } = content {
                    validate_stream(source, *max_bytes, self.page(&file))?;
                }
                if self.page(&file)
                    && matches!(content, Content::Bytes(bytes) if std::str::from_utf8(bytes).is_err())
                {
                    return Err(Why::Refused(Refusal::Undecodable));
                }
                for index in 0usize.. {
                    let rel = if index == 0 {
                        first.clone()
                    } else {
                        format!("{stem}_{index}{ext}")
                    };
                    let candidate = self
                        .store
                        .file_id(*area, &rel)
                        .map_err(|_| Why::Refused(Refusal::InvalidTarget(rel)))?;
                    match self.absent(&candidate) {
                        Ok(()) => {
                            if self.fixed_step_names().contains(&candidate) {
                                return Err(Why::Refused(Refusal::RepeatedFile(candidate)));
                            }
                            break;
                        }
                        Err(Why::Conflict { .. }) => continue,
                        Err(error) => return Err(error),
                    }
                }
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old: None,
                    new: None,
                    saved_page: None,
                })
            }
            Step::Replace {
                file,
                expected,
                bytes,
            } => {
                if self.page(file) || file.as_str().starts_with("logseq/.tine-") {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(file, expected)?;
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old: Some(old),
                    new: Some(bytes.clone()),
                    saved_page: None,
                })
            }
            Step::Rewrite {
                id,
                expected,
                renames,
            } => {
                let file = id.file();
                if !self.page(&file) {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(&file, expected)?;
                let new = rewrite(&old, &self.path(&file)?, renames)?;
                Ok(Prepared {
                    src: file,
                    dst: None,
                    old: Some(old),
                    new: Some(new),
                    saved_page: None,
                })
            }
            Step::Move {
                file,
                expected,
                to,
                renames,
            } => {
                if to.as_str().starts_with("logseq/.tine-") {
                    return Err(Why::Refused(Refusal::InvalidTarget(to.as_str().into())));
                }
                let old = self.stage(file, expected)?;
                self.absent(to)?;
                self.twin(to, Some(file))?;
                let new = match renames {
                    Some(map) => rewrite(&old, &self.path(to)?, map)?,
                    None => old.clone(),
                };
                Ok(Prepared {
                    src: file.clone(),
                    dst: Some(to.clone()),
                    old: Some(old),
                    new: Some(new),
                    saved_page: None,
                })
            }
            Step::Trash { file, expected } => {
                if file.as_str().starts_with("logseq/.tine-trash/") {
                    return Err(Why::Refused(Refusal::InvalidTarget(file.as_str().into())));
                }
                let old = self.stage(file, expected)?;
                Ok(Prepared {
                    src: file.clone(),
                    dst: None,
                    old: Some(old),
                    new: None,
                    saved_page: None,
                })
            }
        }
    }

    fn verify(&self, file: &FileId, old: Option<&[u8]>, index: usize) -> Result<(), Why> {
        let path = self.path(file)?;
        if fault(self.store, FaultPoint::Stage2ConfigExternal) {
            atomic_write(&path, b"{:external true :start-of-week 1}\n").map_err(failed)?;
        }
        if fault(self.store, FaultPoint::Stage2ValidSidecar) {
            let external = b"{:highlights [] :foreign \"external\"}";
            let result = if old.is_some() {
                atomic_write(&path, external)
            } else {
                atomic_write_new(&path, external)
            };
            result.map_err(failed)?;
        }
        if fault(self.store, FaultPoint::Stage2Mismatch)
            || fault(self.store, FaultPoint::Stage2MismatchAt(index))
        {
            let result = if old.is_some() {
                atomic_write(&path, b"external stage-2")
            } else {
                atomic_write_new(&path, b"external stage-2")
            };
            result.map_err(failed)?;
        }
        match fs::read(path) {
            Ok(now) if old == Some(now.as_slice()) => Ok(()),
            Ok(now) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(FileRev::from_bytes(&now)),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound && old.is_none() => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Why::Conflict {
                file: file.clone(),
                disk: None,
            }),
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    fn fixed_step_names(&self) -> HashSet<FileId> {
        let mut names = HashSet::new();
        for step in &self.steps {
            match step {
                Step::Save { id, .. } | Step::Rewrite { id, .. } => {
                    names.insert(id.file());
                }
                Step::Create { file, .. }
                | Step::Replace { file, .. }
                | Step::Trash { file, .. } => {
                    names.insert(file.clone());
                }
                Step::Move { file, to, .. } => {
                    names.insert(file.clone());
                    names.insert(to.clone());
                }
                Step::Unique { .. } => {}
            }
        }
        names
    }

    fn trash_id(&self, file: &FileId) -> FileId {
        let rel = file.as_str();
        let page_area = rel
            .starts_with(&format!("{}/", self.store.graph.current_config().pages_dir))
            || rel.starts_with(&format!(
                "{}/",
                self.store.graph.current_config().journals_dir
            ));
        let stem = Path::new(rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let kind = if page_area && tine_core::model::is_sync_conflict(stem) {
            "conflicts"
        } else if rel.starts_with(&format!("{}/", self.store.graph.current_config().pages_dir)) {
            "pages"
        } else if rel.starts_with(&format!(
            "{}/",
            self.store.graph.current_config().journals_dir
        )) {
            "journals"
        } else if rel.starts_with("assets/") {
            "assets"
        } else {
            "other"
        };
        let name = Path::new(rel)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        FileId::from(format!(
            "logseq/.tine-trash/{kind}/{}__{name}",
            trash_stamp()
        ))
    }

    fn fault_collision(&self, path: &Path) {
        if fault(self.store, FaultPoint::NoReplaceCollision) {
            let _ = atomic_write_new(path, b"external collision");
        }
    }

    fn fault_mid_step(&self, index: usize) -> Result<(), Why> {
        if fault(self.store, FaultPoint::MidStepIo)
            || fault(self.store, FaultPoint::MidStepIoAt(index))
        {
            Err(failed(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected mid-step I/O error",
            )))
        } else {
            Ok(())
        }
    }

    fn fault_twin(&self, file: &FileId) {
        if !self.page(file) || !fault(self.store, FaultPoint::TwinAfterPublish) {
            return;
        }
        let path = self.store.graph.root.join(file.as_str());
        let other = if path.extension().and_then(|s| s.to_str()) == Some("org") {
            path.with_extension("md")
        } else {
            path.with_extension("org")
        };
        let _ = atomic_write_new(&other, b"external twin");
    }

    fn apply(
        &self,
        index: usize,
        step: &mut Step,
        plan: &Prepared,
        undo: &mut Undo,
        temps: &mut Vec<PathBuf>,
        fixed_names: &HashSet<FileId>,
    ) -> Result<StepResult, Why> {
        let src = self.path(&plan.src)?;
        let unique_info = match &*step {
            Step::Unique {
                area, stem, ext, ..
            } => Some((*area, stem.clone(), ext.clone())),
            _ => None,
        };
        match step {
            Step::Save { .. } | Step::Replace { .. } | Step::Rewrite { .. } => {
                let new = plan.new.as_ref().expect("prepared write");
                let old = plan.old.as_deref();
                if old == Some(new.as_slice()) {
                    return Ok(StepResult::Unchanged {
                        file: plan.src.clone(),
                        rev: FileRev::from_bytes(new),
                    });
                }
                self.verify(&plan.src, old, index)?;
                if let Some(parent) = src.parent() {
                    fs::create_dir_all(parent).map_err(failed)?;
                }
                if self.page(&plan.src) {
                    self.store.graph.transaction_note_page(&src, new);
                }
                undo.new = Some(Expected::Bytes(new.clone()));
                if old.is_none() {
                    self.fault_collision(&src);
                }
                let result = if let Some(old) = old {
                    atomic_write_with_check(&src, new, || {
                        if fault(self.store, FaultPoint::AfterTempSync) {
                            atomic_write(&src, b"external after temp sync")?;
                        }
                        let unchanged = match fs::read(&src) {
                            Ok(current) => current == old,
                            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                            Err(error) => return Err(error),
                        };
                        if !unchanged {
                            return Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                "source changed after temp sync",
                            ));
                        }
                        Ok(())
                    })
                } else {
                    atomic_write_new(&src, new)
                };
                result.map_err(|error| collision(&plan.src, error, &src))?;
                undo.created = true;
                self.fault_mid_step(index)?;
                if old.is_none() {
                    self.fault_twin(&plan.src);
                    if let Some(twin) = self.disk_twin(&plan.src)? {
                        return Err(Why::Conflict {
                            file: twin.clone(),
                            disk: disk_rev(&self.path(&twin)?),
                        });
                    }
                }
                Ok(StepResult::Written {
                    file: plan.src.clone(),
                    rev: FileRev::from_bytes(new),
                })
            }
            Step::Create { content, .. } | Step::Unique { content, .. } => {
                let unique = unique_info.is_some();
                let (area, stem, ext) =
                    unique_info.unwrap_or((Area::Assets, String::new(), String::new()));
                for attempt in 0usize.. {
                    let file = if unique {
                        let rel = if attempt == 0 {
                            format!("{stem}{ext}")
                        } else {
                            format!("{stem}_{attempt}{ext}")
                        };
                        self.store
                            .file_id(area, &rel)
                            .map_err(|_| Why::Refused(Refusal::InvalidTarget(rel)))?
                    } else {
                        plan.src.clone()
                    };
                    if unique && fixed_names.contains(&file) {
                        return Err(Why::Refused(Refusal::RepeatedFile(file)));
                    }
                    let path = self.path(&file)?;
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(failed)?;
                    }
                    let expected = match content {
                        Content::Bytes(bytes) => {
                            if self.page(&file) && std::str::from_utf8(bytes).is_err() {
                                return Err(Why::Refused(Refusal::Undecodable));
                            }
                            if self.page(&file) {
                                self.store.graph.transaction_note_page(&path, bytes);
                            }
                            Expected::Bytes(bytes.clone())
                        }
                        Content::Stream { source, max_bytes } => {
                            source.seek(SeekFrom::Start(0)).map_err(failed)?;
                            let stage =
                                path.with_file_name(format!(".tine-tx-{}.tmp", trash_stamp()));
                            atomic_copy_file_new(source, &stage, *max_bytes).map_err(failed)?;
                            if self.page(&file) && !valid_utf8_file(&stage).map_err(failed)? {
                                let _ = fs::remove_file(&stage);
                                return Err(Why::Refused(Refusal::Undecodable));
                            }
                            temps.push(stage.clone());
                            Expected::File(stage)
                        }
                    };
                    undo.src = file.clone();
                    undo.new = Some(expected);
                    self.fault_collision(&path);
                    let result = match undo.new.as_ref().unwrap() {
                        Expected::Bytes(bytes) => atomic_write_new(&path, bytes),
                        Expected::File(stage) => atomic_copy_new(stage, &path),
                    };
                    match result {
                        Ok(()) => {
                            undo.created = true;
                            self.fault_mid_step(index)?;
                            self.fault_twin(&file);
                            if let Some(twin) = self.disk_twin(&file)? {
                                return Err(Why::Conflict {
                                    file: twin.clone(),
                                    disk: disk_rev(&self.path(&twin)?),
                                });
                            }
                            let rev = match undo.new.as_ref().unwrap() {
                                Expected::Bytes(bytes) => FileRev::from_bytes(bytes),
                                Expected::File(stage) => {
                                    FileRev::from_file(stage).map_err(failed)?
                                }
                            };
                            return Ok(StepResult::Written { file, rev });
                        }
                        Err(error) if unique && error.kind() == io::ErrorKind::AlreadyExists => {
                            continue
                        }
                        Err(error) => return Err(collision(&file, error, &path)),
                    }
                }
                unreachable!()
            }
            Step::Move { .. } => {
                let dst_id = plan.dst.as_ref().expect("move destination");
                let dst = self.path(dst_id)?;
                let old = plan.old.as_deref().expect("move baseline");
                let new = plan.new.as_ref().expect("move bytes");
                self.verify(&plan.src, Some(old), index)?;
                self.verify(dst_id, None, index)?;
                if let Some(parent) = dst.parent() {
                    fs::create_dir_all(parent).map_err(failed)?;
                }
                if new.as_slice() == old {
                    // Content unchanged: a guarded no-replace rename, so no copy
                    // of the source is left in the trash. Undo withdraws the
                    // destination and writes the baseline back under `src`.
                    undo.kind = UndoKind::Rename;
                    undo.new = Some(Expected::Bytes(old.to_vec()));
                    if self.page(dst_id) {
                        self.store.graph.transaction_note_page(&dst, old);
                    }
                    if self.page(&plan.src) {
                        self.store.graph.transaction_note_delete(&src);
                    }
                    self.fault_collision(&dst);
                    move_file_noreplace(&src, &dst)
                        .map_err(|error| collision(dst_id, error, &dst))?;
                    undo.created = true;
                    sync_move_dirs(&src, &dst);
                    self.fault_mid_step(index)?;
                    self.fault_twin(dst_id);
                    if let Some(twin) = self.disk_twin(dst_id)? {
                        return Err(Why::Conflict {
                            file: twin.clone(),
                            disk: disk_rev(&self.path(&twin)?),
                        });
                    }
                    if fs::read(&dst).map_err(failed)? != old {
                        return Err(Why::Conflict {
                            file: plan.src.clone(),
                            disk: disk_rev(&dst),
                        });
                    }
                    return Ok(StepResult::Moved {
                        to: dst_id.clone(),
                        rev: FileRev::from_bytes(old),
                    });
                }
                if self.page(dst_id) {
                    self.store.graph.transaction_note_page(&dst, new);
                }
                undo.new = Some(Expected::Bytes(new.clone()));
                self.fault_collision(&dst);
                atomic_write_new(&dst, new).map_err(|e| collision(dst_id, e, &dst))?;
                undo.created = true;
                self.fault_mid_step(index)?;
                self.fault_twin(dst_id);
                if let Some(twin) = self.disk_twin(dst_id)? {
                    return Err(Why::Conflict {
                        file: twin.clone(),
                        disk: disk_rev(&self.path(&twin)?),
                    });
                }
                let trash_id = self.trash_id(&plan.src);
                let trash = self.path(&trash_id)?;
                if let Some(parent) = trash.parent() {
                    fs::create_dir_all(parent).map_err(|error| failed_trash_dir(error, parent))?;
                }
                undo.trash = Some(trash_id.clone());
                if self.page(&plan.src) {
                    self.store.graph.transaction_note_delete(&src);
                }
                move_file_noreplace(&src, &trash).map_err(|e| collision(&plan.src, e, &src))?;
                sync_move_dirs(&src, &trash);
                undo.moved = true;
                self.fault_mid_step(index)?;
                if fs::read(&trash).map_err(failed)? != old {
                    return Err(Why::Conflict {
                        file: plan.src.clone(),
                        disk: disk_rev(&trash),
                    });
                }
                Ok(StepResult::Moved {
                    to: dst_id.clone(),
                    rev: FileRev::from_bytes(new),
                })
            }
            Step::Trash { .. } => {
                let old = plan.old.as_deref().expect("trash baseline");
                self.verify(&plan.src, Some(old), index)?;
                let trash_id = self.trash_id(&plan.src);
                let trash = self.path(&trash_id)?;
                if let Some(parent) = trash.parent() {
                    fs::create_dir_all(parent).map_err(|error| failed_trash_dir(error, parent))?;
                }
                undo.trash = Some(trash_id.clone());
                if self.page(&plan.src) {
                    self.store.graph.transaction_note_delete(&src);
                }
                move_file_noreplace(&src, &trash).map_err(|e| collision(&plan.src, e, &src))?;
                sync_move_dirs(&src, &trash);
                undo.moved = true;
                self.fault_mid_step(index)?;
                if fs::read(&trash).map_err(failed)? != old {
                    return Err(Why::Conflict {
                        file: plan.src.clone(),
                        disk: disk_rev(&trash),
                    });
                }
                Ok(StepResult::Trashed {
                    file: plan.src.clone(),
                    trashed: trash_id,
                })
            }
        }
    }

    fn undo<'b>(
        &self,
        record: &'b Undo,
        rollback: &mut Rollback,
        exact_copies: &mut Vec<(PathBuf, &'b Expected)>,
    ) {
        let live = match self.path(&record.src) {
            Ok(path) => path,
            Err(error) => {
                rollback.undo_failed.push((
                    record.src.clone(),
                    IoError {
                        kind: io::ErrorKind::InvalidInput,
                        message: format!("{error:?}"),
                    },
                ));
                return;
            }
        };
        if record.created {
            let (id, path) = match &record.dst {
                Some(dst) => match self.path(dst) {
                    Ok(path) => (dst, path),
                    Err(error) => {
                        rollback.undo_failed.push((
                            dst.clone(),
                            IoError {
                                kind: io::ErrorKind::InvalidInput,
                                message: format!("{error:?}"),
                            },
                        ));
                        return;
                    }
                },
                None => (&record.src, live.clone()),
            };
            if fault(self.store, FaultPoint::UndoLiveWrite) {
                if let Err(error) = atomic_write(&path, b"external during undo") {
                    rollback.undo_failed.push((id.clone(), error.into()));
                }
            }
            let result = match record.new.as_ref().expect("undo expected") {
                Expected::Bytes(bytes) => self
                    .store
                    .graph
                    .transaction_withdraw_exact(&path, bytes, "tx-undo"),
                Expected::File(stage) => self
                    .store
                    .graph
                    .withdraw_file_to_conflict_if_matching_file(&path, stage, "tx-undo"),
            };
            let withdrawn = result.is_ok();
            match result {
                Ok(Withdrawal::Exact(staged)) => {
                    exact_copies.push((staged, record.new.as_ref().expect("undo expected")));
                    if matches!(record.kind, UndoKind::Replace) {
                        if let Some(old) = &record.old {
                            if let Err(error) = atomic_write_new(&path, old) {
                                rollback.undo_failed.push((id.clone(), error.into()));
                            }
                        }
                    }
                }
                Ok(Withdrawal::Missing) => {
                    if matches!(record.kind, UndoKind::Replace) {
                        if let Some(old) = &record.old {
                            if let Err(error) = atomic_write_new(&path, old) {
                                rollback.undo_failed.push((id.clone(), error.into()));
                            }
                        }
                    }
                }
                // The old bytes of a replaced file are preserved once, by the
                // sweep in `commit` that compares every name with its baseline.
                Ok(Withdrawal::ExternalLive) => {
                    rollback.kept_external.push((id.clone(), None));
                }
                Ok(Withdrawal::ExternalRecovery(recovery)) => {
                    rollback.kept_external.push((
                        id.clone(),
                        Some(FileId::from(self.store.graph.rel_path(&recovery))),
                    ));
                }
                Err(error) => rollback.undo_failed.push((id.clone(), error.into())),
            }
            // A rename left nothing under the source name: put the baseline
            // back there. If a third party took that name, the sweep in
            // `commit` preserves the baseline in recovery.
            if withdrawn && matches!(record.kind, UndoKind::Rename) {
                if let Some(old) = &record.old {
                    if let Err(error) = atomic_write_new(&live, old) {
                        rollback
                            .undo_failed
                            .push((record.src.clone(), error.into()));
                    }
                }
            }
        }
        if record.moved {
            let trash_id = record.trash.as_ref().expect("moved trash");
            let trash = match self.path(trash_id) {
                Ok(path) => path,
                Err(error) => {
                    rollback.undo_failed.push((
                        record.src.clone(),
                        IoError {
                            kind: io::ErrorKind::InvalidInput,
                            message: format!("{error:?}"),
                        },
                    ));
                    return;
                }
            };
            if let Err(error) = move_file_noreplace(&trash, &live) {
                rollback
                    .undo_failed
                    .push((record.src.clone(), error.into()));
            } else {
                sync_move_dirs(&trash, &live);
            }
        }
    }

    fn preserve_old(&self, id: &FileId, bytes: &[u8], rollback: &mut Rollback) {
        let name = Path::new(id.as_str())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let recovery = self
            .store
            .graph
            .root
            .join("logseq/.tine-trash/conflicts")
            .join(format!("{}__tx-old__{name}", trash_stamp()));
        let result = self
            .store
            .graph
            .ensure_write_target(&recovery)
            .and_then(|()| recovery.parent().map(fs::create_dir_all).unwrap_or(Ok(())))
            .and_then(|()| atomic_write_new(&recovery, bytes));
        if let Err(error) = result {
            rollback.undo_failed.push((id.clone(), error.into()));
        }
    }

    /// Check all guards before writing, apply queued steps, then publish the
    /// final state before returning. Each guard hashes its current disk file.
    /// A changed transaction can scan O(P) page and journal metadata and
    /// may wait for the initial graph parse. It blocks other writes for its
    /// duration without a timeout. On apply failure, attempts undo and reports
    /// remaining disk differences; a process crash can leave partial changes.
    /// Preflight reports the first failing step. Neither a preflight nor an
    /// apply guard conflict publishes the observed external bytes by itself;
    /// a changed final state after undo can publish. Apply rechecks each changed
    /// source against the preflight bytes after syncing the replacement temp
    /// file and immediately before rename, subject to the remaining external
    /// writer window between that check and rename. Undo
    /// stages live bytes in recoverable conflict trash and uses no-replace
    /// moves; another writer can still race those filesystem operations.
    /// Prior page bytes are kept in memory until commit finishes, so undo can
    /// require O(changed bytes) memory and additional file reads and writes.
    pub fn commit(mut self) -> TxOutcome {
        let _writer = self.store.writer.lock().unwrap();
        let rev = || self.store.changes.rev();
        if self.store.is_closed() {
            return TxOutcome::NotCommitted {
                step: 0,
                why: Why::Refused(Refusal::Closed),
                rollback: Rollback::default(),
                graph_rev: rev(),
            };
        }
        let starting_rev = self.store.graph.cache_generation();
        let mut names = Vec::new();
        for step in &self.steps {
            match step {
                Step::Save { id, .. } | Step::Rewrite { id, .. } => names.push(id.file()),
                Step::Create { file, .. }
                | Step::Replace { file, .. }
                | Step::Trash { file, .. } => names.push(file.clone()),
                Step::Move { file, to, .. } => {
                    names.push(file.clone());
                    names.push(to.clone());
                }
                Step::Unique { .. } => {}
            }
        }
        let mut seen = HashSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            let ids: Vec<FileId> = match step {
                Step::Save { id, .. } | Step::Rewrite { id, .. } => vec![id.file()],
                Step::Create { file, .. }
                | Step::Replace { file, .. }
                | Step::Trash { file, .. } => vec![file.clone()],
                Step::Move { file, to, .. } => vec![file.clone(), to.clone()],
                Step::Unique { .. } => Vec::new(),
            };
            for id in ids {
                if !seen.insert(id.clone()) {
                    return TxOutcome::NotCommitted {
                        step: index,
                        why: Why::Refused(Refusal::RepeatedFile(id)),
                        rollback: Rollback::default(),
                        graph_rev: rev(),
                    };
                }
            }
        }
        names.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        names.dedup();
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|id| self.store.graph.root.join(id.as_str()))
            .collect();
        let locks: Vec<_> = paths
            .iter()
            .map(|path| self.store.graph.page_lock(path))
            .collect();
        let _guards: Vec<_> = locks.iter().map(|lock| lock.lock().unwrap()).collect();
        let mut plans = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            match self.preflight(step) {
                Ok(plan) => plans.push(plan),
                Err(why) => {
                    return TxOutcome::NotCommitted {
                        step: index,
                        why,
                        rollback: Rollback::default(),
                        graph_rev: rev(),
                    }
                }
            }
        }
        let mut before = BTreeMap::new();
        for (plan, step) in plans.iter().zip(&self.steps) {
            if matches!(step, Step::Unique { .. }) {
                continue;
            }
            before.insert(plan.src.as_str().to_owned(), plan.old.clone());
            if let Some(dst) = &plan.dst {
                before.insert(dst.as_str().to_owned(), None);
            }
        }
        let original_pages = self.store.graph.list_pages_shared();
        let fixed_names = self.fixed_step_names();
        let mut steps = std::mem::take(&mut self.steps);
        let mut done = Vec::new();
        let mut results = Vec::new();
        let mut temps = Vec::new();
        let mut failure = None;
        for index in 0..steps.len() {
            let plan = &plans[index];
            let kind = match &steps[index] {
                Step::Save { .. } | Step::Replace { .. } | Step::Rewrite { .. } => {
                    UndoKind::Replace
                }
                Step::Create { .. } | Step::Unique { .. } => UndoKind::Create,
                Step::Move { .. } => UndoKind::Move,
                Step::Trash { .. } => UndoKind::Trash,
            };
            let mut undo = Undo {
                kind,
                src: plan.src.clone(),
                dst: plan.dst.clone(),
                trash: None,
                old: plan.old.clone(),
                new: None,
                created: false,
                moved: false,
            };
            match self.apply(
                index,
                &mut steps[index],
                plan,
                &mut undo,
                &mut temps,
                &fixed_names,
            ) {
                Ok(result) => {
                    results.push(result);
                    done.push(undo);
                    if fault(self.store, FaultPoint::AbortAfterStep(index)) {
                        std::process::abort();
                    }
                }
                Err(why) => {
                    done.push(undo);
                    failure = Some((index, why));
                    break;
                }
            }
        }
        let mut rollback = Rollback::default();
        let mut exact_copies = Vec::new();
        if failure.is_some() {
            for undo in done.iter().rev() {
                self.undo(undo, &mut rollback, &mut exact_copies);
            }
        }
        for undo in &done {
            if !before.contains_key(undo.src.as_str()) {
                before.insert(undo.src.as_str().into(), None);
            }
            if let Some(dst) = &undo.dst {
                if !before.contains_key(dst.as_str()) {
                    before.insert(dst.as_str().into(), None);
                }
            }
        }
        let mut changed_any = false;
        let mut published_own = Vec::new();
        let mut published_external = Vec::new();
        for (name, baseline) in &before {
            let id = FileId::from(name.clone());
            let path = match self.path(&id) {
                Ok(path) => path,
                Err(_) => {
                    self.store.graph.invalidate_cache();
                    continue;
                }
            };
            let now = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(_) => {
                    self.store.graph.invalidate_cache();
                    continue;
                }
            };
            if failure.is_some() && baseline.is_none() && now.is_some() {
                if !rollback.kept_external.iter().any(|(kept, _)| *kept == id) {
                    rollback.kept_external.push((id.clone(), None));
                }
            }
            if failure.is_some()
                && baseline
                    .as_ref()
                    .is_some_and(|old| now.as_ref() != Some(old))
            {
                if now.is_some() && !rollback.kept_external.iter().any(|(kept, _)| *kept == id) {
                    rollback.kept_external.push((id.clone(), None));
                }
                self.preserve_old(&id, baseline.as_ref().unwrap(), &mut rollback);
            }
            if now.as_ref() != baseline.as_ref() {
                let kind = match (baseline, &now) {
                    (None, Some(_)) => ChangeKind::Created,
                    (Some(_), None) => ChangeKind::Removed,
                    _ => ChangeKind::Modified,
                };
                let tuple = (
                    id.clone(),
                    kind,
                    now.as_ref().map(|bytes| FileRev::from_bytes(bytes)),
                );
                let external_after_undo = failure.is_some()
                    && (rollback
                        .kept_external
                        .iter()
                        .any(|(kept, recovery)| kept == &id && recovery.is_none())
                        || !done.iter().any(|record| {
                            if record.src == id && record.moved && now.is_none() {
                                return true;
                            }
                            if record.src != id && record.dst.as_ref() != Some(&id) {
                                return false;
                            }
                            match (&record.new, &now) {
                                (Some(Expected::Bytes(written)), Some(bytes)) => written == bytes,
                                (Some(Expected::File(stage)), Some(bytes)) => {
                                    fs::read(stage).is_ok_and(|written| written == *bytes)
                                }
                                _ => false,
                            }
                        }));
                if external_after_undo {
                    published_external.push(tuple);
                } else {
                    published_own.push(tuple);
                }
            }
            if self.page(&id) {
                if now.as_ref() != baseline.as_ref() {
                    changed_any = true;
                    let saved_page = if failure.is_none() {
                        plans
                            .iter()
                            .find(|plan| plan.src == id)
                            .and_then(|plan| plan.saved_page.as_ref())
                    } else {
                        None
                    };
                    self.store.graph.transaction_publish_page(&path, saved_page);
                } else {
                    self.store.graph.transaction_clear_page_marker(&path);
                }
            } else if now.as_ref() != baseline.as_ref() {
                changed_any = true;
            }
        }
        if changed_any && self.store.graph.cache_generation() == starting_rev {
            self.store.graph.transaction_bump_generation();
        }
        let mut published_rev = self.store.changes.rev();
        if !published_own.is_empty() {
            published_rev = self.store.publish_own(published_own);
        }
        if !published_external.is_empty() {
            let current_pages = self.store.graph.list_pages_shared();
            let pages = published_external
                .iter()
                .filter_map(|(id, _, _)| {
                    current_pages
                        .iter()
                        .chain(original_pages.iter())
                        .find(|entry| {
                            entry
                                .rel_path
                                .as_ref()
                                .is_some_and(|path| path.file() == *id)
                        })
                        .map(|entry| (id.clone(), entry.kind, entry.name.clone()))
                })
                .collect();
            published_rev =
                self.store
                    .publish_transaction_change(Origin::External, published_external, pages);
        }
        // A clean rollback needs no second copy of bytes written by this
        // transaction. Keep every staged inode if recovery failed or an
        // external writer won; otherwise verify the entire named baseline
        // before discarding transaction-owned copies from conflict trash.
        if failure.is_some()
            && rollback.undo_failed.is_empty()
            && rollback.kept_external.is_empty()
            && before.iter().all(|(name, expected)| {
                let Ok(path) = self.path(&FileId::from(name.clone())) else {
                    return false;
                };
                match fs::read(path) {
                    Ok(bytes) => expected.as_ref() == Some(&bytes),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => expected.is_none(),
                    Err(_) => false,
                }
            })
        {
            for (copy, expected) in exact_copies {
                let valid = self
                    .path(&FileId::from(self.store.graph.rel_path(&copy)))
                    .is_ok()
                    && match expected {
                        Expected::Bytes(bytes) => {
                            fs::read(&copy).is_ok_and(|found| found == *bytes)
                        }
                        Expected::File(stage) => fs::read(&copy)
                            .and_then(|found| fs::read(stage).map(|wanted| found == wanted))
                            .unwrap_or(false),
                    };
                if valid {
                    let _ = fs::remove_file(copy);
                }
            }
        }
        for temp in temps {
            let _ = fs::remove_file(temp);
        }
        match failure {
            Some((step, why)) => TxOutcome::NotCommitted {
                step,
                why,
                rollback,
                graph_rev: published_rev,
            },
            None => TxOutcome::Committed {
                steps: results,
                graph_rev: published_rev,
            },
        }
    }
}

fn failed(error: io::Error) -> Why {
    Why::Failed(error.into())
}

/// Reading a directory fails with EISDIR on Unix but ERROR_ACCESS_DENIED on
/// Windows. Report both as `IsADirectory` so a caller that treats an occupying
/// directory as "name taken" (the Guide copy) behaves the same on every
/// platform instead of failing with "Access is denied" on Windows.
fn directory_read_error(error: io::Error, path: &Path) -> io::Error {
    if error.kind() != io::ErrorKind::IsADirectory
        && fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    {
        io::Error::new(
            io::ErrorKind::IsADirectory,
            format!("{}: is a directory ({error})", path.display()),
        )
    } else {
        error
    }
}

fn failed_trash_dir(error: io::Error, parent: &Path) -> Why {
    Why::Failed(IoError {
        kind: error.kind(),
        message: format!(
            "could not create trash directory {}: {error}",
            parent.display()
        ),
    })
}

fn sync_move_dirs(source: &Path, destination: &Path) {
    for parent in [source.parent(), destination.parent()]
        .into_iter()
        .flatten()
    {
        let _ = File::open(parent).and_then(|dir| dir.sync_all());
    }
}

fn disk_rev(path: &Path) -> Option<FileRev> {
    fs::read(path).ok().map(|bytes| FileRev::from_bytes(&bytes))
}

fn collision(file: &FileId, error: io::Error, path: &Path) -> Why {
    if error.kind() == io::ErrorKind::AlreadyExists {
        Why::Conflict {
            file: file.clone(),
            disk: disk_rev(path),
        }
    } else {
        failed(error)
    }
}

fn rewrite(old: &[u8], path: &Path, map: &RenameMap) -> Result<Vec<u8>, Why> {
    let text = std::str::from_utf8(old).map_err(|_| Why::Refused(Refusal::Undecodable))?;
    let is_org = path.extension().and_then(|ext| ext.to_str()) == Some("org");
    let renames: std::collections::HashMap<String, String> = map
        .0
        .iter()
        .map(|(from, to)| (tine_core::refs::normalize(from), to.clone()))
        .collect();
    let rewritten = tine_core::refs::rename_tags_property_multi(
        &tine_core::refs::rename_refs_multi(text, &renames, is_org),
        &renames,
        is_org,
    );
    if is_org && rewritten != text && !tine_core::org::org_editable(text) {
        return Err(Why::Refused(Refusal::ReadOnly(
            "org file is read-only (does not round-trip)".into(),
        )));
    }
    Ok(rewritten.into_bytes())
}

fn valid_utf8_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut carry = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok(carry.is_empty());
        }
        carry.extend_from_slice(&buf[..n]);
        match std::str::from_utf8(&carry) {
            Ok(_) => carry.clear(),
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                carry.drain(..valid);
            }
            Err(_) => return Ok(false),
        }
    }
}

fn validate_stream(source: &File, max_bytes: u64, utf8: bool) -> Result<(), Why> {
    use std::io::Read;
    let mut input = source.try_clone().map_err(failed)?;
    input.seek(SeekFrom::Start(0)).map_err(failed)?;
    let mut total = 0u64;
    let mut carry = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf).map_err(failed)?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > max_bytes {
            return Err(failed(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("stream exceeds {max_bytes} byte limit"),
            )));
        }
        if utf8 {
            carry.extend_from_slice(&buf[..n]);
            match std::str::from_utf8(&carry) {
                Ok(_) => carry.clear(),
                Err(error) if error.error_len().is_none() => {
                    let valid = error.valid_up_to();
                    carry.drain(..valid);
                }
                Err(_) => return Err(Why::Refused(Refusal::Undecodable)),
            }
        }
    }
    if utf8 && !carry.is_empty() {
        return Err(Why::Refused(Refusal::Undecodable));
    }
    Ok(())
}
