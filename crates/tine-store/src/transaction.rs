//! Guarded multi-file graph writes. A transaction queues steps without disk
//! I/O; commit checks them, applies them in order, and attempts undo on a
//! failed apply. Changed final bytes publish before return, with external
//! bytes from undo in a separate `Origin::External` change. A config-file
//! write validates managed directories before disk mutation and reloads
//! effective settings before publication of the final state;
//! undo or a failed apply reconciles them
//! against the final disk file.
//! Transactions are not crash atomic and cannot exclude external processes.
//! Steps may mix page, journal, asset, and metadata files; changed final
//! files share one publication after a successful initial load. A failed final
//! read or revision check returns the affected file locations and makes the
//! publication incomplete; callers inspect disk and refresh before retrying.

use std::collections::{BTreeMap, HashMap, HashSet};

use faults::fault;
#[cfg(any(test, feature = "test-faults"))]
use faults::inject_external_delete;
#[cfg(any(test, feature = "test-faults"))]
pub use faults::FaultPoint;
#[cfg(not(any(test, feature = "test-faults")))]
pub(crate) use faults::FaultPoint;

mod faults;
mod io_helpers;
mod move_file;
#[cfg(feature = "test-faults")]
#[path = "../tests/support/og_k1_pause.rs"]
mod og_k1_pause;
mod preflight;
mod prepared;
use prepared::PreparedRewrite;
mod publication;
mod read_checks;
mod validation;
use io_helpers::{
    collision, content_refusal, directory_read_error, disk_rev, failed, failed_trash_dir,
    publication_path_error, sync_move_dirs,
};
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use validation::{
    refuse_read_only_org, rewrite, rewrite_move, valid_utf8_file, validate_config_bytes,
    validate_config_content, validate_page_content, validate_stream,
};

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
    /// `Why::Failed` for non-page files or a content refusal for page files;
    /// bytes are not truncated.
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
/// without being rewritten. Code spans are not rewritten. Bracketed references
/// inside `alias::`, `title::`, and query arguments are rewritten because the
/// raw-text rewriter scans those regions too.
#[derive(Clone, Debug, Default)]
pub struct RenameMap(pub Vec<(String, String)>);

/// I/O error with a stable kind for handling and a message for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoError {
    /// Platform-independent I/O error category.
    pub kind: io::ErrorKind,
    /// Human-readable cause.
    pub message: String,
    /// Fixed name of the platform step that failed, when known (GH #538).
    pub operation: Option<&'static str>,
    /// OS error code of that step, when known.
    pub os_error: Option<i32>,
}

impl From<io::Error> for IoError {
    fn from(error: io::Error) -> Self {
        let step = crate::directory_durability::failure_step(&error);
        Self {
            kind: error.kind(),
            message: error.to_string(),
            operation: step.map(|(operation, _)| operation),
            os_error: step.map_or(error.raw_os_error(), |(_, os_error)| os_error),
        }
    }
}

impl IoError {
    /// Platform-independent error category for caller dispatch.
    pub fn kind(&self) -> io::ErrorKind {
        self.kind
    }
}

impl std::fmt::Display for IoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IoError {}

impl From<IoError> for io::Error {
    fn from(error: IoError) -> Self {
        Self::new(error.kind, error.message)
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
    /// Save, replace, or reference rewrite found the same bytes on disk, or a
    /// read-only expectation matched. A stale base still returns `Why::Conflict`.
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
    /// Invalid or unsafe destination or page content; reason is for display.
    InvalidTarget(String),
    /// Another page file claims the same name or journal day.
    Twin {
        /// Existing claimant.
        existing: PageId,
    },
    /// Page bytes are not UTF-8.
    Undecodable,
    /// A name-only page creation was refused because a graph-text file Tine
    /// cannot read (`file`) could already be that page
    /// (R-CREATE-UNREADABLE-OWNER, docs/storage-contract.md).
    UnreadableOwner {
        /// The unreadable file or directory that could own the name.
        file: FileId,
    },
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
        /// Bounded derived-answer delta of this publication; absent when unchanged.
        change: Option<crate::Change>,
        /// Generation publishing the disk state, or current one if unchanged.
        /// During a failed initial load this is the unchanged current revision:
        /// no view covers the write until recovery's first view does.
        graph_rev: GraphRev,
    },
    /// Disk steps applied, but at least one final file could not be read for
    /// publication. The caller must inspect disk and refresh before retrying.
    PublicationIncomplete {
        /// Applied step results in input order.
        steps: Vec<StepResult>,
        /// Graph-relative files whose final state could not be observed.
        files: Vec<(FileId, IoError)>,
        /// Last successfully published graph generation.
        graph_rev: GraphRev,
    },
    /// The transaction did not commit. Preflight checks every step before any
    /// write; an apply failure attempts undo of prior steps and the failed
    /// step, which may already have written, in reverse application order.
    /// Inspect the final disk state and
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
        /// Final files that could not be read for publication after rollback.
        publication_errors: Vec<(FileId, IoError)>,
        /// Last generation published for the final disk state. If both own and
        /// external changes survive undo, the external publication comes last.
        /// During a failed initial load this is the unchanged current revision:
        /// no view covers a changed result until recovery's first view does.
        graph_rev: GraphRev,
    },
}

/// Whether a queued page save may replace a file that carries VCS merge
/// conflict markers. Scoped to the one step that carries it: there is no
/// store-wide mode, so the exemption cannot outlive the resolving transaction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Markers {
    /// Ordinary save: a marker-bearing file on disk is refused in preflight.
    Refuse,
    /// `SaveBase::ResolvingMarkers`: the resolution itself. The old bytes
    /// are staged byte-exact in conflict trash before the replacement.
    Resolve,
}

enum Step {
    Save {
        id: PageId,
        base: SaveBase,
        doc: PageDto,
        markers: Markers,
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
    Expect {
        file: FileId,
        expected: FileRev,
    },
    Rewrite {
        id: PageId,
        expected: FileRev,
        renames: RenameMap,
        rebind_title: bool,
        prepared: Option<Box<PreparedRewrite>>,
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
        orphan_only: bool,
    },
}

struct Prepared {
    src: FileId,
    dst: Option<FileId>,
    old: Option<Vec<u8>>,
    new: Option<Vec<u8>>,
    saved_page: Option<Document>,
    opaque_rev: Option<FileRev>,
}

enum Expected {
    Bytes(Vec<u8>),
    File(PathBuf),
}

enum UndoKind {
    Expect,
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
    new_rev: Option<FileRev>,
    opaque_rev: Option<FileRev>,
    created: bool,
    moved: bool,
}

/// Builder for guarded file changes, applied by [`Self::commit`]. Dropping
/// without commit performs no disk I/O.
pub struct Transaction<'a> {
    store: &'a Store,
    steps: Vec<Step>,
    /// Declared intent for page-file steps other than `save_page` (which
    /// carries its own). Debug builds assert at commit that a page-file step
    /// has one; release builds do not refuse, because the kind is request
    /// metadata and refusing would strand the user's write.
    kinds: Vec<crate::EditKind>,
    // Exact names sampled once per directory per commit phase.
    spelling_entries:
        std::cell::RefCell<BTreeMap<PathBuf, BTreeMap<std::ffi::OsString, std::ffi::OsString>>>,
    // Reference rewrites prepared from the caller's planning read (GH #623).
    prepared: HashMap<FileId, PreparedRewrite>,
}

impl Store {
    /// Begin a transaction. Commit serializes writes through this Store; it
    /// does not lock other Store instances or external processes. Two stores
    /// on one graph have separate `GraphRev` sequences and can observe one
    /// another's writes as external changes; both still use revision guards.
    pub fn transaction(&self, kind: Option<crate::EditKind>) -> Transaction<'_> {
        Transaction {
            store: self,
            steps: Vec::new(),
            kinds: kind.into_iter().collect(),
            spelling_entries: Default::default(),
            prepared: HashMap::new(),
        }
    }
}

impl<'a> Transaction<'a> {
    /// Queue a guarded page save. `CreateNew` requires absence; `Existing`
    /// compares the current raw-byte revision. A Guide DTO is refused as
    /// ephemeral before disk access. `CreateNew` checks both the exact target
    /// and an alternate extension, and refuses an indexed name or day twin.
    /// No disk I/O until commit. `ResolvingMarkers` is the one save allowed to
    /// replace a file carrying VCS conflict markers (R-VCS-MARKERS): commit
    /// first stages a byte-exact copy of the old file under
    /// `logseq/.tine-trash/conflicts/<stamp>__markers__<name>` and withdraws it
    /// if undo restores the original (one extra write + fsync of the old bytes).
    /// Commit cost includes page bytes and O(P) graph metadata on publication.
    pub fn save_page(
        &mut self,
        kinds: &[crate::EditKind],
        id: &PageId,
        base: SaveBase,
        doc: &PageDto,
    ) -> &mut Self {
        assert!(!kinds.is_empty(), "OG-RULES Rule 8: page save needs a kind; exemplar crates/tine-graph-features/src/pages.rs");
        let (base, markers) = match base {
            SaveBase::ResolvingMarkers(rev) => (SaveBase::Existing(rev), Markers::Resolve),
            other => (other, Markers::Refuse),
        };
        self.steps.push(Step::Save {
            id: id.clone(),
            base,
            doc: doc.clone(),
            markers,
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
    /// It checks target safety, UTF-8, the 64 MiB input cap, and the 512-level
    /// source depth cap for page files, but does not run the Org round-trip
    /// editability check because no prior page is rewritten.
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
    /// `Area::Trash` and `Area::Meta` with `config` + `.edn` are refused.
    /// The chosen id appears in the step result.
    /// Cost O(new bytes + collisions): an occupied candidate costs one
    /// `metadata` call; its content is never read.
    /// A page target uses the same raw UTF-8 and
    /// target-safety and indexed twin checks as `create`, not `PageDto`
    /// serialization. An exact occupied candidate tries the next suffix;
    /// an alternate name or journal-day claimant refuses creation.
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
    /// `Store::file_id(Area::Meta, "config.edn")` to replace graph config;
    /// unsafe configured pages or journals directories are refused before
    /// any transaction file changes.
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

    /// Queue a read-only revision guard for an existing file. Commit checks
    /// `expected` during preflight and again at this step under the same
    /// per-file lock as writes; absence or changed bytes return `Why::Conflict`.
    /// It writes and publishes nothing, and needs no page edit kind. I/O
    /// errors return `Why::Failed`; unsafe page input can be refused. An
    /// external process can still change the file between this final check
    /// and a later step's write. Cost is two full reads and hashes of this
    /// file, including parse validation when it is a page.
    pub fn expect(&mut self, file: &FileId, expected: FileRev) -> &mut Self {
        self.steps.push(Step::Expect {
            file: file.clone(),
            expected,
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
    /// each named referrer, O(its text bytes), plus publication metadata; a
    /// rewrite kept by [`Self::prepare_ref_rewrite`] is reused, not recomputed.
    pub fn rewrite_refs(
        &mut self,
        id: &PageId,
        expected: FileRev,
        renames: &RenameMap,
    ) -> &mut Self {
        let prepared = self.take_prepared(&id.file(), renames);
        self.steps.push(Step::Rewrite {
            id: id.clone(),
            expected,
            renames: renames.clone(),
            rebind_title: false,
            prepared,
        });
        self
    }

    /// Queue a guarded no-replace move. With a rename map and identical source
    /// and destination IDs, complete an interrupted move by rewriting the
    /// file's explicit title and references in place under the same revision
    /// guard; stale revisions or non-round-tripping Org content refuse it.
    /// On a normal move, optional ref rewrites rebind an own Markdown `title::`
    /// matching the mapped old identity and destination; other titles, aliases
    /// and namespace children stay untouched. Changed bytes leave an old-byte
    /// copy in trash; unchanged bytes rename directly. An in-place completion
    /// rewrites the existing file and makes no new trash copy.
    /// `resolve` follows the destination; later referrers need a later query.
    /// Twin claims are refused. A read-only Org source may move without a
    /// rewrite; asking to rewrite its bytes invokes the round-trip check.
    /// A case-only rename changes spelling on case-sensitive and case-folding
    /// filesystems when the destination is absent or is the unique source's
    /// alternate spelling. Distinct directory entries (including hard links)
    /// refuse. It moves through existing graph trash with both moves synced:
    /// a crash between them leaves bytes recoverable via a guarded move from trash; a
    /// crash after publication leaves the page at its new spelling. An I/O
    /// failure attempts ordinary transaction undo. See docs/storage-contract.md.
    /// Moves between pages and journals change the file's area identity and
    /// require a guarded source and free destination. Moving into graph
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
        if file == to {
            if let Some(renames) = renames {
                self.steps.push(Step::Rewrite {
                    id: PageId::from(file.as_str()),
                    expected,
                    renames: renames.clone(),
                    rebind_title: true,
                    prepared: None,
                });
                return self;
            }
        }
        self.steps.push(Step::Move {
            file: file.clone(),
            expected,
            to: to.clone(),
            renames: renames.cloned(),
        });
        self
    }

    /// Guarded one-file trash; caller chooses twin claimants. Refusals preserve bytes.
    /// Returns a Journal/Conflict trash id; Org read-only can move. Cost O(bytes + P metadata).
    pub fn trash(&mut self, file: &FileId, expected: FileRev) -> &mut Self {
        self.steps.push(Step::Trash {
            file: file.clone(),
            expected,
            orphan_only: false,
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
                    failed_trash_dir(error.into(), target.parent().unwrap_or(&target))
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
        let claimant = if moving_from.is_some() {
            self.store.move_claimant(&entry.name, entry.kind)
        } else {
            self.store.graph.find_entry(&entry.name, entry.kind)
        };
        if let Some(existing) = claimant {
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

    /// Refusal R-CREATE-UNREADABLE-OWNER (docs/storage-contract.md; master
    /// 69e0a885ddf9 + 69525c055f0b, GH #543). Threat scenario: sync delivery,
    /// an interrupted external write or malformed imported Markdown/Org leaves
    /// a page file Tine cannot read, so its name is unknown; creating a page of
    /// a name it could carry gives that name two files once the bad file is
    /// repaired. Only the names that file could be are refused
    /// (`Graph::unreadable_page_could_own`), and the refusal names the file;
    /// every other creation proceeds. `names` are the names the new file
    /// would claim: its file-name name and, for a page save, the DTO's name.
    /// Cost: O(unreadable rows) metadata checks, each at most one bounded read.
    fn unreadable_owner(&self, file: &FileId, names: &[&str]) -> Result<(), Why> {
        if !self.page(file) {
            return Ok(());
        }
        let path = self.path(file)?;
        let Some(entry) = self.store.graph.entry_for_path(&path) else {
            return Ok(());
        };
        if entry.kind != tine_core::model::PageKind::Page {
            return Ok(());
        }
        let unreadable = self.store.graph.unreadable_pages();
        if unreadable.is_empty() {
            return Ok(());
        }
        let mut keys: Vec<String> = std::iter::once(entry.name.as_str())
            .chain(names.iter().copied())
            .map(tine_core::refs::page_key)
            .filter(|key| !key.is_empty())
            .collect();
        keys.dedup();
        for (failed, _) in unreadable.iter() {
            if failed == file {
                continue;
            }
            if keys
                .iter()
                .any(|key| self.store.graph.unreadable_page_could_own(failed, key))
            {
                return Err(Why::Refused(Refusal::UnreadableOwner {
                    file: failed.clone(),
                }));
            }
        }
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
            Err(error) if crate::atomic_file::names_nothing(&error) => Ok(None),
            Err(error) if self.page(file) && error.kind() == io::ErrorKind::InvalidData => {
                Err(content_refusal(error))
            }
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    fn stage(&self, file: &FileId, expected: &FileRev) -> Result<Vec<u8>, Why> {
        let path = self.path(file)?;
        match if self.page(file) {
            crate::model::read_parse_bytes(&path)
        } else {
            fs::read(&path)
        } {
            Ok(bytes) if FileRev::from_bytes(&bytes) == *expected => Ok(bytes),
            Ok(bytes) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(FileRev::from_bytes(&bytes)),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Why::Conflict {
                file: file.clone(),
                disk: None,
            }),
            Err(error) if self.page(file) && error.kind() == io::ErrorKind::InvalidData => {
                Err(content_refusal(error))
            }
            Err(error) => Err(Why::Failed(error.into())),
        }
    }

    fn oversized_page_rev(
        &self,
        file: &FileId,
        expected: &FileRev,
    ) -> Result<Option<FileRev>, Why> {
        if !self.page(file) {
            return Ok(None);
        }
        let path = self.path(file)?;
        let len = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Why::Conflict {
                    file: file.clone(),
                    disk: None,
                })
            }
            Err(error) => return Err(failed(error)),
        };
        if len <= crate::model::PARSE_INPUT_MAX_BYTES {
            return Ok(None);
        }
        let disk = FileRev::from_file(&path).map_err(failed)?;
        if disk != *expected {
            return Err(Why::Conflict {
                file: file.clone(),
                disk: Some(disk),
            });
        }
        Ok(Some(disk))
    }

    /// Whether `file`'s name is taken, from one `metadata` call: never its
    /// content, so probing a name held by a large opaque asset costs no
    /// memory or reads (I-22). A directory at the name is a failure, as when
    /// [`Self::absent`] reads it.
    fn occupied(&self, file: &FileId) -> Result<bool, Why> {
        let path = self.path(file)?;
        match fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => Err(Why::Failed(
                directory_read_error(io::Error::from(io::ErrorKind::IsADirectory), &path).into(),
            )),
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(Why::Failed(directory_read_error(error, &path).into())),
        }
    }

    /// `Ok` when `file` does not exist; otherwise a conflict carrying its
    /// revision, hashed by streaming in 64 KiB chunks (bounded memory for any
    /// occupant size).
    fn absent(&self, file: &FileId) -> Result<(), Why> {
        if !self.occupied(file)? {
            return Ok(());
        }
        let path = self.path(file)?;
        match FileRev::from_file(&path) {
            Ok(rev) => Err(Why::Conflict {
                file: file.clone(),
                disk: Some(rev),
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Why::Failed(directory_read_error(error, &path).into())),
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
                | Step::Expect { file, .. }
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
            "logseq/.tine-trash/{kind}/{}",
            crate::atomic_file::prefixed_name(&format!("{}__", trash_stamp()), name)
        ))
    }

    /// Stage the pre-resolution bytes of a marker-bearing page in conflict
    /// trash (no-replace, fsync'd) before a `SaveBase::ResolvingMarkers`
    /// replaces them. A crash after this point leaves the old file plus its
    /// copy, or the resolved file plus the copy: never a lost side.
    fn stage_marker_file(&self, file: &FileId, old: &[u8]) -> Result<FileId, Why> {
        let name = Path::new(file.as_str())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let id = FileId::from(format!(
            "logseq/.tine-trash/conflicts/{}",
            crate::atomic_file::prefixed_name(&format!("{}__markers__", trash_stamp()), name)
        ));
        self.write_trash_copy(&id, old)?;
        #[cfg(feature = "test-faults")]
        if fault(self.store, FaultPoint::AbortAfterMarkerStage) {
            std::process::abort();
        }
        Ok(id)
    }

    /// Write a byte-exact copy of retired bytes into graph trash. The copy is
    /// outside the live page area and uses the audited no-replace publication.
    fn write_trash_copy(&self, id: &FileId, old: &[u8]) -> Result<PathBuf, Why> {
        let path = self.path(id)?;
        if let Some(parent) = path.parent() {
            crate::directory_durability::create_dir_all_durable(parent)
                .map_err(|error| failed_trash_dir(error, parent))?;
        }
        atomic_write_new(&path, old).map_err(failed)?;
        Ok(path)
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

    fn arm_directory_sync_fault(&self) {
        #[cfg(all(feature = "test-faults", unix))]
        if fault(self.store, FaultPoint::DirectorySyncIo) {
            crate::directory_durability::fail_next_sync();
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
            Step::Expect { .. } => {
                self.verify(&plan.src, plan.old.as_deref(), index)?;
                Ok(StepResult::Unchanged {
                    file: plan.src.clone(),
                    rev: FileRev::from_bytes(plan.old.as_deref().expect("guard baseline")),
                })
            }
            Step::Save { .. } | Step::Replace { .. } | Step::Rewrite { .. } => {
                #[cfg(feature = "test-faults")]
                og_k1_pause::before_apply(&src);
                let new = plan.new.as_ref().expect("prepared write");
                let old = plan.old.as_deref();
                if Self::final_guard_covers(step, old, new) {
                    self.stage2_faults(&plan.src, old, index)?;
                } else {
                    self.verify(&plan.src, old, index)?;
                }
                if old == Some(new.as_slice()) {
                    return Ok(StepResult::Unchanged {
                        file: plan.src.clone(),
                        rev: FileRev::from_bytes(new),
                    });
                }
                if let Some(parent) = src.parent() {
                    crate::directory_durability::create_dir_all_durable(parent).map_err(failed)?;
                }
                if self.page(&plan.src) {
                    self.store.graph.transaction_note_page(&src, new);
                }
                undo.new = Some(Expected::Bytes(new.clone()));
                if old.is_none() {
                    self.fault_collision(&src);
                }
                if let (
                    Step::Save {
                        markers: Markers::Resolve,
                        ..
                    },
                    Some(old),
                ) = (&*step, old)
                {
                    undo.trash = Some(self.stage_marker_file(&plan.src, old)?);
                }
                self.arm_directory_sync_fault();
                let result = if let Some(old) = old {
                    atomic_write_with_check(&src, new, || {
                        if fault(self.store, FaultPoint::AfterTempSync) {
                            atomic_write(&src, b"external after temp sync")?;
                        }
                        let unchanged = match if self.page(&plan.src) {
                            crate::model::read_parse_bytes(&src)
                        } else {
                            fs::read(&src)
                        } {
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
                if let Err(error) = result {
                    if crate::directory_durability::is_directory_sync_failure(&error) {
                        undo.created = true;
                    }
                    return Err(collision(&plan.src, error, &src));
                }
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
                            crate::atomic_file::marked_name(&stem, &format!("_{attempt}"), &ext)
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
                    if unique {
                        self.twin(&file, None)?;
                    }
                    let path = self.path(&file)?;
                    if let Some(parent) = path.parent() {
                        crate::directory_durability::create_dir_all_durable(parent)
                            .map_err(failed)?;
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
                            if let Err(error) = atomic_copy_file_new(source, &stage, *max_bytes) {
                                if crate::directory_durability::is_directory_sync_failure(&error) {
                                    let _ = fs::remove_file(&stage);
                                }
                                return Err(failed(error));
                            }
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
                    self.arm_directory_sync_fault();
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
                            undo.new_rev = Some(rev.clone());
                            if let Some(Expected::File(stage)) = undo.new.as_ref() {
                                if fault(self.store, FaultPoint::RemoveStreamStageAfterWrite) {
                                    fs::remove_file(stage).map_err(failed)?;
                                }
                            }
                            return Ok(StepResult::Written { file, rev });
                        }
                        Err(error) if unique && error.kind() == io::ErrorKind::AlreadyExists => {
                            continue
                        }
                        Err(error) => {
                            if crate::directory_durability::is_directory_sync_failure(&error) {
                                undo.created = true;
                            }
                            return Err(collision(&file, error, &path));
                        }
                    }
                }
                unreachable!()
            }
            Step::Move { .. } => self.apply_move(plan, undo, index),
            Step::Trash { .. } => {
                if let Some(rev) = &plan.opaque_rev {
                    self.verify_opaque(&plan.src, rev)?;
                    let trash_id = self.trash_id(&plan.src);
                    let trash = self.path(&trash_id)?;
                    if let Some(parent) = trash.parent() {
                        crate::directory_durability::create_dir_all_durable(parent)
                            .map_err(|error| failed_trash_dir(error, parent))?;
                    }
                    undo.trash = Some(trash_id.clone());
                    undo.opaque_rev = Some(rev.clone());
                    self.store.graph.transaction_note_delete(&src);
                    move_file_noreplace(&src, &trash)
                        .map_err(|error| collision(&plan.src, error, &src))?;
                    undo.moved = true;
                    sync_move_dirs(self.store, &src, &trash).map_err(failed)?;
                    self.fault_mid_step(index)?;
                    self.verify_opaque(&trash_id, rev)?;
                    return Ok(StepResult::Trashed {
                        file: plan.src.clone(),
                        trashed: trash_id,
                    });
                }
                let old = plan.old.as_deref().expect("trash baseline");
                self.verify(&plan.src, Some(old), index)?;
                let trash_id = self.trash_id(&plan.src);
                let trash = self.path(&trash_id)?;
                if let Some(parent) = trash.parent() {
                    crate::directory_durability::create_dir_all_durable(parent)
                        .map_err(|error| failed_trash_dir(error, parent))?;
                }
                undo.trash = Some(trash_id.clone());
                if self.page(&plan.src) {
                    self.store.graph.transaction_note_delete(&src);
                }
                move_file_noreplace(&src, &trash).map_err(|e| collision(&plan.src, e, &src))?;
                undo.moved = true;
                sync_move_dirs(self.store, &src, &trash).map_err(failed)?;
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
        old_copies: &mut Vec<(&'b Undo, FileId)>,
    ) {
        if matches!(record.kind, UndoKind::Expect) {
            return;
        }
        let spelling_move = record.dst.as_ref().is_some_and(|dst| {
            move_file::case_only(Path::new(record.src.as_str()), Path::new(dst.as_str()))
        });
        let live = match if spelling_move {
            self.spelled_path(&record.src)
        } else {
            self.path(&record.src)
        } {
            Ok(path) => path,
            Err(error) => {
                rollback
                    .undo_failed
                    .push((record.src.clone(), io_helpers::unresolved_undo_path(&error)));
                return;
            }
        };
        if record.created && matches!(record.kind, UndoKind::Rename) {
            if let (Some(dst_id), Some(expected)) = (&record.dst, &record.opaque_rev) {
                let result = (|| -> io::Result<()> {
                    let dst = self.path(dst_id).map_err(|error| {
                        io::Error::new(io::ErrorKind::InvalidInput, format!("{error:?}"))
                    })?;
                    let current = FileRev::from_file(&dst)?;
                    if current != *expected {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "destination changed during undo",
                        ));
                    }
                    self.undo_opaque_move(record, dst_id, &dst, &live)?;
                    Ok(())
                })();
                match result {
                    Ok(_) => {}
                    Err(error) => rollback
                        .undo_failed
                        .push((record.src.clone(), error.into())),
                }
                return;
            }
        }
        if record.created {
            let (id, path) = match &record.dst {
                Some(dst) => match self.path(dst) {
                    Ok(path) => (dst, path),
                    Err(error) => {
                        rollback
                            .undo_failed
                            .push((dst.clone(), io_helpers::unresolved_undo_path(&error)));
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
            if fault(self.store, FaultPoint::UndoWithdrawalIo) {
                rollback.undo_failed.push((
                    id.clone(),
                    io::Error::new(io::ErrorKind::Other, "injected undo withdrawal error").into(),
                ));
                return;
            }
            // I-2 (C3 L07): the withdrawal below leaves no live name until the
            // old bytes are rewritten. Stage them in conflict trash first, as a
            // rewritten move does, so a crash or a failed rewrite (disk full) in
            // that window keeps them on disk; if they cannot be staged, withdraw
            // nothing and leave this transaction's bytes live.
            if let (UndoKind::Replace, None, Some(old)) = (&record.kind, &record.trash, &record.old)
            {
                match self.write_old_copy(id, old) {
                    Ok(copy) => old_copies.push((record, copy)),
                    Err(error) => {
                        rollback.undo_failed.push((id.clone(), error.into()));
                        return;
                    }
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
            #[cfg(feature = "test-faults")]
            if result.is_ok() && fault(self.store, FaultPoint::AbortAfterUndoWithdraw) {
                std::process::abort();
            }
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
                    rollback
                        .undo_failed
                        .push((record.src.clone(), io_helpers::unresolved_undo_path(&error)));
                    return;
                }
            };
            if let Err(error) = move_file_noreplace(&trash, &live) {
                rollback
                    .undo_failed
                    .push((record.src.clone(), error.into()));
            } else {
                if let Err(error) = sync_move_dirs(self.store, &trash, &live) {
                    rollback
                        .undo_failed
                        .push((record.src.clone(), error.into()));
                }
            }
        }
    }

    fn preserve_old(&self, id: &FileId, bytes: &[u8], rollback: &mut Rollback) {
        if let Err(error) = self.write_old_copy(id, bytes) {
            rollback.undo_failed.push((id.clone(), error.into()));
        }
    }

    /// Write a file's pre-transaction bytes to a fresh `tx-old` conflict-trash
    /// copy and return its id.
    fn write_old_copy(&self, id: &FileId, bytes: &[u8]) -> io::Result<FileId> {
        let name = Path::new(id.as_str())
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let copy = FileId::from(format!(
            "logseq/.tine-trash/conflicts/{}",
            crate::atomic_file::prefixed_name(&format!("{}__tx-old__", trash_stamp()), name)
        ));
        let recovery = self.store.graph.root.join(copy.as_str());
        self.store.graph.ensure_write_target(&recovery)?;
        recovery
            .parent()
            .map(crate::directory_durability::create_dir_all_durable)
            .unwrap_or(Ok(()))?;
        atomic_write_new(&recovery, bytes)?;
        Ok(copy)
    }

    /// Check guards, apply queued steps, then publish final state. Each guard
    /// hashes its disk file; apply checks preflight bytes even for unchanged
    /// output (I-2, external-editor race). Changed writes check again after temp
    /// sync, before rename; the external check-to-rename window remains.
    /// A changed transaction can scan O(P) page/journal metadata and wait for
    /// initial parsing. It blocks other writes for its duration without timeout.
    /// Preflight reports the first failing step. Guard conflicts alone do not
    /// publish external bytes; apply failure attempts undo and reports remaining
    /// differences. A process crash can leave partial changes.
    /// Undo stages live bytes in recoverable conflict trash and uses no-replace
    /// moves. Racing external bytes may remain live, enter recovery, or be
    /// overwritten by a later external write: inspect disk state and `rollback`.
    /// Prior page bytes stay in memory through commit, so undo can require
    /// O(changed bytes) memory and extra file reads/writes. A clean undo publishes
    /// no change and leaves the graph revision unchanged.
    pub fn commit(self) -> TxOutcome {
        let store = self.store;
        let mut timing = crate::launch_diag::SaveTiming::start(self.steps.len());
        let outcome = self.commit_timed(&mut timing);
        store
            .graph
            .diag
            .save(timing, matches!(outcome, TxOutcome::Committed { .. }));
        outcome
    }

    /// The body of `commit`; `timing` splits the wait for reference
    /// publication and for the writer lock from the work itself.
    fn commit_timed(mut self, timing: &mut crate::launch_diag::SaveTiming) -> TxOutcome {
        // OG-RULES Rule 8: a raw step that touches a page file runs in a
        // transaction that declared its edit kind (`save_page` asserts its own).
        // A missing kind is a caller bug, not an in-scope storage threat, so it
        // is a debug assertion, never a production refusal.
        #[cfg(debug_assertions)]
        if self.kinds.is_empty() {
            for step in &self.steps {
                let touches_page = match step {
                    Step::Save { .. } => false,
                    Step::Expect { .. } => false,
                    Step::Rewrite { .. } => true,
                    Step::Create { file, .. }
                    | Step::Replace { file, .. }
                    | Step::Trash { file, .. } => self.store.as_page(file).is_some(),
                    Step::Move { file, to, .. } => {
                        self.store.as_page(file).is_some() || self.store.as_page(to).is_some()
                    }
                    Step::Unique { area, .. } => {
                        matches!(area, crate::Area::Pages | crate::Area::Journals)
                    }
                };
                assert!(!touches_page, "OG-RULES Rule 8: a page-file write needs an edit kind; exemplar crates/tine-graph-features/src/pages.rs");
            }
        }
        self.await_reference_publication();
        timing.publication_waited();
        let _writer = self.store.writer.lock().unwrap();
        timing.writer_acquired();
        let rev = || self.store.changes.rev();
        if self.store.is_closed() {
            return TxOutcome::NotCommitted {
                step: 0,
                why: Why::Refused(Refusal::Closed),
                rollback: Rollback::default(),
                publication_errors: Vec::new(),
                graph_rev: rev(),
            };
        }
        if let Some(failure) = self.config_write_failure() {
            return failure;
        }
        let starting_rev = self.store.graph.cache_generation();
        let mut names = Vec::new();
        for step in &self.steps {
            match step {
                Step::Save { id, .. } | Step::Rewrite { id, .. } => names.push(id.file()),
                Step::Create { file, .. }
                | Step::Replace { file, .. }
                | Step::Expect { file, .. }
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
                | Step::Expect { file, .. }
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
                        publication_errors: Vec::new(),
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
                        publication_errors: Vec::new(),
                        graph_rev: rev(),
                    }
                }
            }
        }
        self.spelling_entries.borrow_mut().clear();
        let mut before = BTreeMap::new();
        for (plan, step) in plans.iter().zip(&self.steps) {
            if matches!(step, Step::Unique { .. } | Step::Expect { .. }) {
                continue;
            }
            before.insert(plan.src.as_str().to_owned(), plan.old.clone());
            if let Some(dst) = &plan.dst {
                before.insert(dst.as_str().to_owned(), None);
            }
        }
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
                Step::Expect { .. } => UndoKind::Expect,
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
                new_rev: None,
                opaque_rev: plan.opaque_rev.clone(),
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
                    if !matches!(steps[index], Step::Expect { .. }) {
                        done.push(undo);
                    }
                    if fault(self.store, FaultPoint::AbortAfterStep(index)) {
                        std::process::abort();
                    }
                }
                Err(why) => {
                    if !matches!(steps[index], Step::Expect { .. }) {
                        done.push(undo);
                    }
                    failure = Some((index, why));
                    break;
                }
            }
        }
        let mut rollback = Rollback::default();
        let mut exact_copies = Vec::new();
        let mut old_copies = Vec::new();
        // Files whose old bytes stay in an undo's staged copy need no second one.
        let mut kept_old = Vec::new();
        if failure.is_some() {
            for undo in done.iter().rev() {
                self.undo(undo, &mut rollback, &mut exact_copies, &mut old_copies);
            }
            // A rewritten move copies the old bytes to trash before replacing
            // the destination. Withdraw that copy only after undo has restored
            // the source; leave it recoverable if restoration was incomplete.
            // A marker resolution's staged copy and undo's own old-byte copy
            // (both `UndoKind::Replace`) are withdrawn under the same rule.
            let staged = done
                .iter()
                .filter(|record| {
                    matches!(record.kind, UndoKind::Rename | UndoKind::Replace) && !record.moved
                })
                .filter_map(|record| Some((record, record.trash.clone()?)))
                .chain(old_copies);
            for (record, trash_id) in staged {
                let trash_id = &trash_id;
                let Some(old) = &record.old else {
                    continue;
                };
                let result = (|| -> io::Result<()> {
                    let live = self
                        .path(&record.src)
                        .map_err(|error| io::Error::other(format!("{error:?}")))?;
                    let trash = self
                        .path(trash_id)
                        .map_err(|error| io::Error::other(format!("{error:?}")))?;
                    if matches!(record.kind, UndoKind::Replace) && fs::read(&live)? != *old {
                        // The original is not back at its name (an external
                        // writer raced the resolution), so this copy may be the
                        // only one left: keep it, recoverable in trash.
                        return Ok(());
                    }
                    if fs::read(&live)? != *old || fs::read(&trash)? != *old {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "move bytes changed during undo",
                        ));
                    }
                    fs::remove_file(&trash)?;
                    crate::directory_durability::sync_directory_entry(
                        trash.parent().expect("trash parent"),
                    )
                })();
                if let Err(error) = result {
                    rollback.undo_failed.push((trash_id.clone(), error.into()));
                }
                if self.path(trash_id).is_ok_and(|trash| trash.exists()) {
                    kept_old.push(record.src.clone());
                }
            }
        }
        for undo in &done {
            before.entry(undo.src.as_str().into()).or_insert(None);
            if let Some(dst) = &undo.dst {
                if !before.contains_key(dst.as_str()) {
                    before.insert(dst.as_str().into(), None);
                }
            }
        }
        self.spelling_entries.borrow_mut().clear();
        let publication::FilePublication {
            changed_any,
            spelling_moves,
            publication_errors,
            published_own,
            published_external,
            observations,
        } = self.publish_final_files(
            &plans,
            &done,
            &before,
            failure.is_some(),
            &mut rollback,
            &kept_old,
            &steps,
        );
        if changed_any && self.store.graph.cache_generation() == starting_rev {
            self.store.graph.transaction_bump_generation();
        }
        let mut published_rev = self.store.changes.rev();
        let mut change = None;
        if !published_own.is_empty() {
            (published_rev, change) = self.store.publish_own(published_own, observations);
        }
        if !published_external.is_empty() {
            let pages = published_external
                .iter()
                .filter_map(|(id, _, _)| {
                    self.store
                        .graph
                        .entry_for_path(&self.store.graph.root.join(id.as_str()))
                        .map(|entry| (id.clone(), entry.kind, entry.name.clone()))
                })
                .collect();
            let (rev, _) = self.store.publish_transaction_change(
                Origin::External,
                published_external,
                pages,
                Default::default(),
            );
            published_rev = rev;
        }
        // A clean rollback needs no second copy of bytes written by this
        // transaction. Keep every staged inode if recovery failed or an
        // external writer won; otherwise verify the entire named baseline
        // before discarding transaction-owned copies from conflict trash.
        if failure.is_some()
            && rollback.undo_failed.is_empty()
            && rollback.kept_external.is_empty()
            && before.iter().all(|(name, expected)| {
                let id = FileId::from(name.clone());
                let spelling_move = spelling_moves.contains(&id);
                let Ok(mut path) = (if spelling_move {
                    self.spelled_path(&id)
                } else {
                    self.path(&id)
                }) else {
                    return false;
                };
                if spelling_move {
                    match self.listed_path(&path) {
                        Ok(Some(actual)) => path = actual,
                        Ok(None) => return expected.is_none(),
                        Err(_) => return false,
                    }
                }
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
                publication_errors,
                graph_rev: published_rev,
            },
            None if !publication_errors.is_empty() => TxOutcome::PublicationIncomplete {
                steps: results,
                files: publication_errors,
                graph_rev: published_rev,
            },
            None => TxOutcome::Committed {
                steps: results,
                change,
                graph_rev: published_rev,
            },
        }
    }
}
