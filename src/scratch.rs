use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::sync::Mutex;

#[cfg(windows)]
use cap_fs_ext::OsMetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    ensure_directory_nofollow, open_dir_nofollow, sync_dir_required, ContentDigest, FilesystemError,
};

pub const SCRATCH_DIR: &str = "engine-scratch-v2";
pub const SCRATCH_MARKER_FILE: &str = "marker";
pub const SCRATCH_LEASE_FILE: &str = "lease";
pub const SCRATCH_PAGES_FILE: &str = "pages.index";
pub const SCRATCH_BLOBS_FILE: &str = "blobs.data";
pub const SCRATCH_SCHEMA_VERSION: u32 = 13;

const MAX_MARKER_BYTES: u64 = 4 * 1024;

/// Durable retention mode authenticated by a scratch run's marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ScratchRetention {
    Ephemeral,
    Retained,
}

/// Generic schema-13 marker. Field order and representations are persistent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScratchRunMarker<Owner> {
    schema_version: u32,
    owner: Owner,
    run_id: Uuid,
    retention: ScratchRetention,
    random_owner_nonce: [u8; 32],
}

/// A failure at the generic physical scratch-run boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScratchRunError {
    Io(String),
    UnsafeEntry(String),
    MalformedMarker(String),
    MalformedEncoding,
    Poisoned,
}

impl fmt::Display for ScratchRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "scratch I/O failed: {error}"),
            Self::UnsafeEntry(reason) => write!(f, "unsafe scratch entry: {reason}"),
            Self::MalformedMarker(run) => write!(f, "malformed scratch marker in {run}"),
            Self::MalformedEncoding => f.write_str("malformed or non-canonical scratch page"),
            Self::Poisoned => f.write_str("scratch file lock was poisoned"),
        }
    }
}

impl std::error::Error for ScratchRunError {}

impl From<std::io::Error> for ScratchRunError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<FilesystemError> for ScratchRunError {
    fn from(error: FilesystemError) -> Self {
        let error = match error {
            FilesystemError::Io(error) => error.to_string(),
            FilesystemError::UnsafeEntry(message) => format!("unsafe store entry: {message}"),
            FilesystemError::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => format!(
                "stored file length mismatch at {path}: expected {expected}, found {actual}"
            ),
            FilesystemError::StoredFileTooLarge {
                path,
                length,
                limit,
            } => format!("stored file at {path} is {length} bytes, exceeding limit {limit}"),
            FilesystemError::ByteCollision => "immutable immutable publication collision".into(),
        };
        Self::Io(error)
    }
}

/// Fallible physical boundaries observable by a core-owned test fault policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScratchConstructionBoundary {
    AfterRunDirectory,
    AfterNamespaceSync,
    AfterRunOpen,
    AfterMarkerWrite,
    AfterLeaseCreate,
    AfterLeaseLock,
    AfterPagesCreate,
    AfterBlobsCreate,
    InspectSibling,
    AfterReclaim,
}

/// Counts from opportunistic cleanup performed while creating a fresh run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScratchRunLifecycleStats {
    pub stale_runs_reclaimed: usize,
    pub live_runs_skipped: usize,
    pub retained_runs_preserved: usize,
    pub unclassified_runs_preserved: usize,
}

/// One exclusively leased physical scratch run and its raw address spaces.
pub struct ScratchRun<Owner> {
    namespace: Dir,
    run: Dir,
    run_name: String,
    marker: ScratchRunMarker<Owner>,
    lease: fs::File,
    pages: Mutex<fs::File>,
    blobs: Mutex<fs::File>,
    lifecycle_stats: ScratchRunLifecycleStats,
}

impl<Owner: fmt::Debug> fmt::Debug for ScratchRun<Owner> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScratchRun")
            .field("run_name", &self.run_name)
            .field("owner", &self.marker.owner)
            .finish_non_exhaustive()
    }
}

impl<Owner> ScratchRun<Owner>
where
    Owner: Clone + Eq + Serialize + DeserializeOwned,
{
    pub fn create_ephemeral(
        archive_capability: &Dir,
        owner: Owner,
    ) -> Result<Self, ScratchRunError> {
        Self::create_ephemeral_observed(archive_capability, owner, |_| Ok(()))
    }

    pub fn create_ephemeral_observed(
        archive_capability: &Dir,
        owner: Owner,
        observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        Self::create_run(
            archive_capability,
            owner,
            ScratchRetention::Ephemeral,
            observer,
        )
    }

    pub fn create_retained(
        archive_capability: &Dir,
        owner: Owner,
    ) -> Result<Self, ScratchRunError> {
        Self::create_retained_observed(archive_capability, owner, |_| Ok(()))
    }

    pub fn create_retained_observed(
        archive_capability: &Dir,
        owner: Owner,
        observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        Self::create_run(
            archive_capability,
            owner,
            ScratchRetention::Retained,
            observer,
        )
    }

    fn create_run(
        archive_capability: &Dir,
        owner: Owner,
        retention: ScratchRetention,
        mut observer: impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        ensure_directory_nofollow(archive_capability, SCRATCH_DIR)?;
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_id = Uuid::new_v4();
        let run_name = format!("run-{run_id}");
        namespace.create_dir(&run_name)?;
        let construction = Self::construct_own_run(
            &namespace,
            &run_name,
            run_id,
            owner,
            retention,
            &mut observer,
        );
        let mut run = match construction {
            Ok(run) => run,
            Err(error) => {
                remove_partial_own_run(&namespace, &run_name);
                return Err(error);
            }
        };
        run.reclaim_stale_runs(&mut observer);
        if let Err(error) = observer(ScratchConstructionBoundary::AfterReclaim) {
            run.cleanup_own_run();
            return Err(error);
        }
        Ok(run)
    }

    fn construct_own_run(
        namespace: &Dir,
        run_name: &str,
        run_id: Uuid,
        owner: Owner,
        retention: ScratchRetention,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<Self, ScratchRunError> {
        observer(ScratchConstructionBoundary::AfterRunDirectory)?;
        sync_dir_required(namespace)?;
        observer(ScratchConstructionBoundary::AfterNamespaceSync)?;
        let run = open_dir_nofollow(namespace, run_name)?;
        observer(ScratchConstructionBoundary::AfterRunOpen)?;
        let nonce_a = Uuid::new_v4();
        let nonce_b = Uuid::new_v4();
        let mut random_owner_nonce = [0_u8; 32];
        random_owner_nonce[..16].copy_from_slice(nonce_a.as_bytes());
        random_owner_nonce[16..].copy_from_slice(nonce_b.as_bytes());
        let marker = ScratchRunMarker {
            schema_version: SCRATCH_SCHEMA_VERSION,
            owner,
            run_id,
            retention,
            random_owner_nonce,
        };
        write_new_regular(&run, SCRATCH_MARKER_FILE, &encode_canonical(&marker)?)?;
        observer(ScratchConstructionBoundary::AfterMarkerWrite)?;
        let lease = create_new_regular(&run, SCRATCH_LEASE_FILE)?;
        observer(ScratchConstructionBoundary::AfterLeaseCreate)?;
        lock_exclusive_nonblocking(&lease)?
            .then_some(())
            .ok_or_else(|| {
                ScratchRunError::UnsafeEntry("new scratch lease was already locked".into())
            })?;
        observer(ScratchConstructionBoundary::AfterLeaseLock)?;
        let pages = create_new_regular(&run, SCRATCH_PAGES_FILE)?;
        observer(ScratchConstructionBoundary::AfterPagesCreate)?;
        let blobs = create_new_regular(&run, SCRATCH_BLOBS_FILE)?;
        observer(ScratchConstructionBoundary::AfterBlobsCreate)?;
        Ok(Self {
            namespace: namespace.try_clone()?,
            run,
            run_name: run_name.to_owned(),
            marker,
            lease,
            pages: Mutex::new(pages),
            blobs: Mutex::new(blobs),
            lifecycle_stats: ScratchRunLifecycleStats::default(),
        })
    }

    pub fn adopt_retained(
        archive_capability: &Dir,
        owner: Owner,
        run_id: Uuid,
    ) -> Result<Self, ScratchRunError> {
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_name = format!("run-{run_id}");
        if parse_run_name(&run_name)? != run_id {
            return Err(ScratchRunError::MalformedMarker(run_name));
        }
        let run = open_dir_nofollow(&namespace, &run_name)?;
        let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
        if !lock_exclusive_nonblocking(&lease)? {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "retained scratch run {run_name:?} is still leased"
            )));
        }
        let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
        let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
        if marker.schema_version != SCRATCH_SCHEMA_VERSION
            || marker.owner != owner
            || marker.run_id != run_id
            || marker.retention != ScratchRetention::Retained
        {
            return Err(ScratchRunError::MalformedMarker(run_name));
        }
        validate_run_entries(&run)?;
        let pages = open_regular_read_write_nofollow(&run, SCRATCH_PAGES_FILE)?;
        let blobs = open_regular_read_write_nofollow(&run, SCRATCH_BLOBS_FILE)?;
        Ok(Self {
            namespace,
            run,
            run_name,
            marker,
            lease,
            pages: Mutex::new(pages),
            blobs: Mutex::new(blobs),
            lifecycle_stats: ScratchRunLifecycleStats::default(),
        })
    }

    pub fn clone_retained_into(&self, archive_capability: &Dir) -> Result<Self, ScratchRunError> {
        if self.marker.retention != ScratchRetention::Retained {
            return Err(ScratchRunError::UnsafeEntry(
                "scratch migration source is not retained".into(),
            ));
        }
        ensure_directory_nofollow(archive_capability, SCRATCH_DIR)?;
        let namespace = open_dir_nofollow(archive_capability, SCRATCH_DIR)?;
        let run_name = format!("run-{}", self.run_id());
        namespace.create_dir(&run_name)?;
        let construction = (|| {
            sync_dir_required(&namespace)?;
            let run = open_dir_nofollow(&namespace, &run_name)?;
            write_new_regular(&run, SCRATCH_MARKER_FILE, &encode_canonical(&self.marker)?)?;
            let lease = create_new_regular(&run, SCRATCH_LEASE_FILE)?;
            lock_exclusive_nonblocking(&lease)?
                .then_some(())
                .ok_or_else(|| {
                    ScratchRunError::UnsafeEntry("migrated scratch lease was already locked".into())
                })?;
            let pages = create_new_regular(&run, SCRATCH_PAGES_FILE)?;
            let blobs = create_new_regular(&run, SCRATCH_BLOBS_FILE)?;
            let migrated = Self {
                namespace: namespace.try_clone()?,
                run,
                run_name: run_name.clone(),
                marker: self.marker.clone(),
                lease,
                pages: Mutex::new(pages),
                blobs: Mutex::new(blobs),
                lifecycle_stats: ScratchRunLifecycleStats::default(),
            };
            migrated.copy_exact_from(self)?;
            Ok(migrated)
        })();
        match construction {
            Ok(migrated) => Ok(migrated),
            Err(error) => {
                remove_partial_own_run(&namespace, &run_name);
                Err(error)
            }
        }
    }

    pub const fn run_id(&self) -> Uuid {
        self.marker.run_id
    }

    pub const fn retention(&self) -> ScratchRetention {
        self.marker.retention
    }

    pub const fn owner(&self) -> &Owner {
        &self.marker.owner
    }

    pub fn binding_digest(&self) -> Result<ContentDigest, ScratchRunError> {
        Ok(ContentDigest::of(&encode_canonical(&self.marker)?))
    }

    pub const fn lifecycle_stats(&self) -> ScratchRunLifecycleStats {
        self.lifecycle_stats
    }

    /// Execute one operation against the locked raw page-file address space.
    pub fn with_pages<T>(
        &self,
        operation: impl FnOnce(&mut fs::File) -> T,
    ) -> Result<T, ScratchRunError> {
        let mut file = self.pages.lock().map_err(|_| ScratchRunError::Poisoned)?;
        Ok(operation(&mut file))
    }

    /// Execute one operation against the locked raw blob-file address space.
    pub fn with_blobs<T>(
        &self,
        operation: impl FnOnce(&mut fs::File) -> T,
    ) -> Result<T, ScratchRunError> {
        let mut file = self.blobs.lock().map_err(|_| ScratchRunError::Poisoned)?;
        Ok(operation(&mut file))
    }

    pub fn clone_pages_file(&self) -> Result<fs::File, ScratchRunError> {
        self.pages
            .lock()
            .map_err(|_| ScratchRunError::Poisoned)?
            .try_clone()
            .map_err(Into::into)
    }

    fn copy_exact_from(&self, source: &Self) -> Result<(), ScratchRunError> {
        if self.marker != source.marker || self.binding_digest()? != source.binding_digest()? {
            return Err(ScratchRunError::UnsafeEntry(
                "scratch migration source and destination identity mismatch".into(),
            ));
        }

        fn copy_file(
            source: &Mutex<fs::File>,
            destination: &Mutex<fs::File>,
        ) -> Result<(), ScratchRunError> {
            let mut source = source.lock().map_err(|_| ScratchRunError::Poisoned)?;
            let mut destination = destination.lock().map_err(|_| ScratchRunError::Poisoned)?;
            if destination.metadata()?.len() != 0 {
                return Err(ScratchRunError::UnsafeEntry(
                    "scratch migration destination is not empty".into(),
                ));
            }
            source.seek(SeekFrom::Start(0))?;
            destination.seek(SeekFrom::Start(0))?;
            let expected = source.metadata()?.len();
            let copied = std::io::copy(&mut *source, &mut *destination)?;
            if copied != expected || destination.metadata()?.len() != expected {
                return Err(ScratchRunError::UnsafeEntry(
                    "scratch migration did not copy the exact byte extent".into(),
                ));
            }
            Ok(())
        }

        copy_file(&source.pages, &self.pages)?;
        copy_file(&source.blobs, &self.blobs)
    }

    fn reclaim_stale_runs(
        &mut self,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) {
        let Ok(entries) = self.namespace.entries() else {
            self.lifecycle_stats.unclassified_runs_preserved += 1;
            return;
        };
        for entry in entries {
            let disposition = entry
                .map_err(ScratchRunError::from)
                .and_then(|entry| self.classify_stale_sibling(&entry, observer))
                .unwrap_or(StaleRunDisposition::Unclassified);
            match disposition {
                StaleRunDisposition::OwnRun => {}
                StaleRunDisposition::Reclaimed => self.lifecycle_stats.stale_runs_reclaimed += 1,
                StaleRunDisposition::LivePreserved => self.lifecycle_stats.live_runs_skipped += 1,
                StaleRunDisposition::RetainedPreserved => {
                    self.lifecycle_stats.retained_runs_preserved += 1;
                }
                StaleRunDisposition::Unclassified => {
                    self.lifecycle_stats.unclassified_runs_preserved += 1;
                }
            }
        }
    }

    fn classify_stale_sibling(
        &self,
        entry: &cap_std::fs::DirEntry,
        observer: &mut impl FnMut(ScratchConstructionBoundary) -> Result<(), ScratchRunError>,
    ) -> Result<StaleRunDisposition, ScratchRunError> {
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch run".into()))?
            .to_owned();
        if name == self.run_name {
            return Ok(StaleRunDisposition::OwnRun);
        }
        observer(ScratchConstructionBoundary::InspectSibling)?;
        let run_id = parse_run_name(&name)?;
        require_real_directory(entry, &name)?;
        let run = open_dir_nofollow(&self.namespace, &name)?;
        let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
        let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
        if marker.schema_version != SCRATCH_SCHEMA_VERSION
            || marker.owner != self.marker.owner
            || marker.run_id != run_id
        {
            return Err(ScratchRunError::MalformedMarker(name));
        }
        validate_run_entries(&run)?;
        if marker.retention == ScratchRetention::Retained {
            return Ok(StaleRunDisposition::RetainedPreserved);
        }
        let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
        if !lock_exclusive_nonblocking(&lease)? {
            return Ok(StaleRunDisposition::LivePreserved);
        }
        remove_stale_run(&self.namespace, &run, &name, lease)?;
        Ok(StaleRunDisposition::Reclaimed)
    }

    fn cleanup_own_run(&self) {
        for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
            let _ = self.run.remove_file(name);
        }
        unlock(&self.lease);
        let _ = self.run.remove_file(SCRATCH_LEASE_FILE);
        let _ = self.namespace.remove_dir(&self.run_name);
    }
}

impl<Owner> Drop for ScratchRun<Owner> {
    fn drop(&mut self) {
        match self.marker.retention {
            ScratchRetention::Ephemeral => self.cleanup_own_run_unbounded(),
            ScratchRetention::Retained => unlock(&self.lease),
        }
    }
}

impl<Owner> ScratchRun<Owner> {
    fn cleanup_own_run_unbounded(&self) {
        for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
            let _ = self.run.remove_file(name);
        }
        unlock(&self.lease);
        let _ = self.run.remove_file(SCRATCH_LEASE_FILE);
        let _ = self.namespace.remove_dir(&self.run_name);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StaleRunDisposition {
    OwnRun,
    Reclaimed,
    LivePreserved,
    RetainedPreserved,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedRunCensus {
    pub retained: usize,
    pub ephemeral: usize,
    pub unclassified: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedRunReclamation {
    pub retained_reachable: usize,
    pub retained_reclaimed: usize,
    pub retained_live_skipped: usize,
    pub ephemeral_preserved: usize,
    pub unclassified_preserved: usize,
}

pub fn census_retained_runs<Owner>(
    archive_capability: &Dir,
    owner: &Owner,
) -> Result<RetainedRunCensus, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let mut census = RetainedRunCensus::default();
    let Some(namespace) = open_scratch_namespace(archive_capability)? else {
        return Ok(census);
    };
    for entry in namespace.entries()? {
        match entry
            .map_err(ScratchRunError::from)
            .and_then(|entry| authenticate_scratch_sibling(&namespace, &entry, owner))
        {
            Ok((_, AuthenticatedScratchSibling::Retained(_))) => census.retained += 1,
            Ok((_, AuthenticatedScratchSibling::Ephemeral)) => census.ephemeral += 1,
            Err(_) => census.unclassified += 1,
        }
    }
    Ok(census)
}

/// Delete free authenticated retained runs excluded by a caller-proved set.
///
/// # Safety
///
/// The caller must have authenticated a complete authoritative reachability
/// scan and must pass every retained run identity reachable by that scan.
/// This is unsafe so an ordinary safe downstream call site cannot turn a
/// partial or guessed set into deletion authority.
pub unsafe fn reclaim_unreachable_retained_runs<Owner>(
    archive_capability: &Dir,
    owner: &Owner,
    reachable: impl Fn(Uuid) -> bool,
) -> Result<RetainedRunReclamation, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let mut outcome = RetainedRunReclamation::default();
    let Some(namespace) = open_scratch_namespace(archive_capability)? else {
        return Ok(outcome);
    };
    for entry in namespace.entries()? {
        let disposition = entry
            .map_err(ScratchRunError::from)
            .and_then(|entry| classify_retained_sibling(&namespace, &entry, owner, &reachable))
            .unwrap_or(RetainedRunDisposition::Unclassified);
        match disposition {
            RetainedRunDisposition::Reachable => outcome.retained_reachable += 1,
            RetainedRunDisposition::Reclaimed => outcome.retained_reclaimed += 1,
            RetainedRunDisposition::LivePreserved => outcome.retained_live_skipped += 1,
            RetainedRunDisposition::EphemeralPreserved => outcome.ephemeral_preserved += 1,
            RetainedRunDisposition::Unclassified => outcome.unclassified_preserved += 1,
        }
    }
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedRunDisposition {
    Reachable,
    Reclaimed,
    LivePreserved,
    EphemeralPreserved,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthenticatedScratchSibling {
    Retained(Uuid),
    Ephemeral,
}

fn open_scratch_namespace(archive_capability: &Dir) -> Result<Option<Dir>, ScratchRunError> {
    match archive_capability.symlink_metadata(SCRATCH_DIR) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{SCRATCH_DIR} is not a real no-follow directory"
            )));
        }
        Ok(_) => {}
    }
    Ok(Some(open_dir_nofollow(archive_capability, SCRATCH_DIR)?))
}

fn authenticate_scratch_sibling<Owner>(
    namespace: &Dir,
    entry: &cap_std::fs::DirEntry,
    owner: &Owner,
) -> Result<(String, AuthenticatedScratchSibling), ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let name = entry
        .file_name()
        .to_str()
        .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch run".into()))?
        .to_owned();
    let run_id = parse_run_name(&name)?;
    require_real_directory(entry, &name)?;
    let run = open_dir_nofollow(namespace, &name)?;
    let marker_bytes = read_regular_nofollow(&run, SCRATCH_MARKER_FILE, MAX_MARKER_BYTES)?;
    let marker: ScratchRunMarker<Owner> = decode_canonical(&marker_bytes)?;
    if marker.schema_version != SCRATCH_SCHEMA_VERSION
        || &marker.owner != owner
        || marker.run_id != run_id
    {
        return Err(ScratchRunError::MalformedMarker(name));
    }
    validate_run_entries(&run)?;
    let sibling = match marker.retention {
        ScratchRetention::Retained => AuthenticatedScratchSibling::Retained(run_id),
        ScratchRetention::Ephemeral => AuthenticatedScratchSibling::Ephemeral,
    };
    Ok((name, sibling))
}

fn classify_retained_sibling<Owner>(
    namespace: &Dir,
    entry: &cap_std::fs::DirEntry,
    owner: &Owner,
    reachable: &impl Fn(Uuid) -> bool,
) -> Result<RetainedRunDisposition, ScratchRunError>
where
    Owner: Eq + Serialize + DeserializeOwned,
{
    let (name, sibling) = authenticate_scratch_sibling(namespace, entry, owner)?;
    let AuthenticatedScratchSibling::Retained(run_id) = sibling else {
        return Ok(RetainedRunDisposition::EphemeralPreserved);
    };
    if reachable(run_id) {
        return Ok(RetainedRunDisposition::Reachable);
    }
    let run = open_dir_nofollow(namespace, &name)?;
    let lease = open_regular_read_write_nofollow(&run, SCRATCH_LEASE_FILE)?;
    if !lock_exclusive_nonblocking(&lease)? {
        return Ok(RetainedRunDisposition::LivePreserved);
    }
    remove_stale_run(namespace, &run, &name, lease)?;
    Ok(RetainedRunDisposition::Reclaimed)
}

fn parse_run_name(name: &str) -> Result<Uuid, ScratchRunError> {
    let suffix = name
        .strip_prefix("run-")
        .ok_or_else(|| ScratchRunError::UnsafeEntry(format!("unknown scratch entry {name:?}")))?;
    let run_id = Uuid::parse_str(suffix)
        .map_err(|_| ScratchRunError::UnsafeEntry(format!("malformed scratch run {name:?}")))?;
    if format!("run-{run_id}") != name {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "non-canonical scratch run {name:?}"
        )));
    }
    Ok(run_id)
}

fn validate_run_entries(run: &Dir) -> Result<(), ScratchRunError> {
    let mut seen = BTreeSet::new();
    for entry in run.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| ScratchRunError::UnsafeEntry("non-UTF-8 scratch entry".into()))?
            .to_owned();
        if ![
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
        ]
        .contains(&name.as_str())
        {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "unknown scratch run entry {name:?}"
            )));
        }
        require_regular_entry(&entry, &name)?;
        if !seen.insert(name.clone()) {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "duplicate scratch run entry {name:?}"
            )));
        }
    }
    for required in [
        SCRATCH_MARKER_FILE,
        SCRATCH_LEASE_FILE,
        SCRATCH_PAGES_FILE,
        SCRATCH_BLOBS_FILE,
    ] {
        if !seen.contains(required) {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "scratch run is missing {required:?}"
            )));
        }
    }
    Ok(())
}

fn remove_stale_run(
    namespace: &Dir,
    run: &Dir,
    run_name: &str,
    lease: fs::File,
) -> Result<(), ScratchRunError> {
    validate_run_entries(run)?;
    for name in [SCRATCH_PAGES_FILE, SCRATCH_BLOBS_FILE, SCRATCH_MARKER_FILE] {
        run.remove_file(name)?;
    }
    unlock(&lease);
    drop(lease);
    run.remove_file(SCRATCH_LEASE_FILE)?;
    namespace.remove_dir(run_name)?;
    Ok(())
}

fn remove_partial_own_run(namespace: &Dir, run_name: &str) {
    if let Ok(run) = open_dir_nofollow(namespace, run_name) {
        for name in [
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
        ] {
            let _ = run.remove_file(name);
        }
    }
    let _ = namespace.remove_dir(run_name);
}

fn create_new_regular(dir: &Dir, name: &str) -> Result<fs::File, ScratchRunError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    let file = dir.open_with(name, &options)?.into_std();
    ensure_opened_regular(&file, name)?;
    Ok(file)
}

fn write_new_regular(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), ScratchRunError> {
    let mut file = create_new_regular(dir, name)?;
    file.write_all(bytes)?;
    Ok(())
}

fn open_regular_read_write_nofollow(dir: &Dir, name: &str) -> Result<fs::File, ScratchRunError> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::fd::AsFd as _;
        let path = CString::new(name)
            .map_err(|_| ScratchRunError::UnsafeEntry("invalid scratch filename".into()))?;
        let fd = unsafe {
            libc::openat(
                dir.as_fd().as_raw_fd(),
                path.as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { fs::File::from_raw_fd(fd) };
        ensure_opened_regular(&file, name)?;
        Ok(file)
    }
    #[cfg(windows)]
    {
        use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        options.follow(FollowSymlinks::No);
        let file = dir.open_with(name, &options)?.into_std();
        ensure_opened_regular(&file, name)?;
        Ok(file)
    }
}

fn read_regular_nofollow(dir: &Dir, name: &str, limit: u64) -> Result<Vec<u8>, ScratchRunError> {
    let mut file = open_regular_read_write_nofollow(dir, name)?;
    let metadata = file.metadata()?;
    if metadata.len() > limit {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "scratch file {name:?} exceeds its bound"
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn ensure_opened_regular(file: &fs::File, name: &str) -> Result<(), ScratchRunError> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    if !metadata.is_file() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    Ok(())
}

fn require_real_directory(
    entry: &cap_std::fs::DirEntry,
    name: &str,
) -> Result<(), ScratchRunError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a real directory"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn require_regular_entry(entry: &cap_std::fs::DirEntry, name: &str) -> Result<(), ScratchRunError> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(ScratchRunError::UnsafeEntry(format!(
            "{name:?} is not a regular file"
        )));
    }
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        if entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ScratchRunError::UnsafeEntry(format!(
                "{name:?} is a reparse point"
            )));
        }
    }
    Ok(())
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, ScratchRunError> {
    postcard::to_allocvec(value).map_err(|_| ScratchRunError::MalformedEncoding)
}

fn decode_canonical<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, ScratchRunError> {
    let value: T = postcard::from_bytes(bytes).map_err(|_| ScratchRunError::MalformedEncoding)?;
    if encode_canonical(&value)? != bytes {
        return Err(ScratchRunError::MalformedEncoding);
    }
    Ok(value)
}

#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchRunError> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(error.into())
}

#[cfg(unix)]
fn unlock(file: &fs::File) {
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, ScratchRunError> {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, FALSE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != FALSE {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        return Ok(false);
    }
    Err(error.into())
}

#[cfg(windows)]
fn unlock(file: &fs::File) {
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    let mut overlapped = unsafe { std::mem::zeroed() };
    let _ = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as _,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(transparent)]
    struct TestOwner(Uuid);

    fn scratch_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-storage-scratch-{label}-{}", Uuid::new_v4()))
    }

    fn archive(root: &Path) -> Dir {
        fs::create_dir_all(root).unwrap();
        Dir::open_ambient_dir(root, ambient_authority()).unwrap()
    }

    fn run_path(root: &Path, run_id: Uuid) -> PathBuf {
        root.join(SCRATCH_DIR).join(format!("run-{run_id}"))
    }

    fn run_snapshot(root: &Path, run_id: Uuid) -> BTreeMap<&'static str, Vec<u8>> {
        let run = run_path(root, run_id);
        [
            SCRATCH_MARKER_FILE,
            SCRATCH_LEASE_FILE,
            SCRATCH_PAGES_FILE,
            SCRATCH_BLOBS_FILE,
        ]
        .into_iter()
        .map(|name| (name, fs::read(run.join(name)).unwrap()))
        .collect()
    }

    fn namespace_names(root: &Path) -> BTreeSet<String> {
        fs::read_dir(root.join(SCRATCH_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    /// Exact bytes produced by the pre-extraction schema-13 marker codec for:
    /// schema 13, owner UUID bytes 00..0f, run UUID bytes 10..1f, retained,
    /// and owner nonce bytes 20..3f.
    const PRE_EXTRACTION_SCHEMA_13_MARKER: [u8; 68] = [
        0x0d, 0x10, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        0x0d, 0x0e, 0x0f, 0x10, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a,
        0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x01, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28,
        0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
    ];

    #[test]
    fn pre_extraction_schema_13_run_reopens_and_clones_byte_exactly() {
        let source_root = scratch_root("schema-13-source");
        let destination_root = scratch_root("schema-13-destination");
        let source = archive(&source_root);
        let destination = archive(&destination_root);
        let owner = TestOwner(Uuid::from_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ]));
        let run_id = Uuid::from_bytes([
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ]);
        let run = run_path(&source_root, run_id);
        fs::create_dir_all(&run).unwrap();
        fs::write(
            run.join(SCRATCH_MARKER_FILE),
            PRE_EXTRACTION_SCHEMA_13_MARKER,
        )
        .unwrap();
        fs::write(run.join(SCRATCH_LEASE_FILE), []).unwrap();
        fs::write(run.join(SCRATCH_PAGES_FILE), b"existing page extent").unwrap();
        fs::write(run.join(SCRATCH_BLOBS_FILE), b"existing blob extent").unwrap();
        let baseline = run_snapshot(&source_root, run_id);

        let adopted = ScratchRun::adopt_retained(&source, owner.clone(), run_id).unwrap();
        assert_eq!(
            adopted.binding_digest().unwrap(),
            ContentDigest::of(&PRE_EXTRACTION_SCHEMA_13_MARKER)
        );
        assert_eq!(adopted.retention(), ScratchRetention::Retained);
        let cloned = adopted.clone_retained_into(&destination).unwrap();
        assert_eq!(run_snapshot(&source_root, run_id), baseline);
        assert_eq!(run_snapshot(&destination_root, run_id), baseline);
        assert_eq!(
            namespace_names(&source_root),
            BTreeSet::from([format!("run-{run_id}")])
        );
        assert_eq!(
            baseline.keys().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from(["blobs.data", "lease", "marker", "pages.index",])
        );
        drop(cloned);
        drop(adopted);

        let reopened = ScratchRun::adopt_retained(&source, owner, run_id).unwrap();
        assert_eq!(run_snapshot(&source_root, run_id), baseline);
        drop(reopened);
        fs::remove_dir_all(source_root).unwrap();
        fs::remove_dir_all(destination_root).unwrap();
    }

    #[test]
    fn census_is_read_only_and_reclamation_deletes_only_proved_free_orphans() {
        let root = scratch_root("census-reclamation");
        let archive = archive(&root);
        let owner = TestOwner(Uuid::from_u128(1));
        let foreign_owner = TestOwner(Uuid::from_u128(2));

        let reachable = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let reachable_id = reachable.run_id();
        drop(reachable);
        let orphan = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let orphan_id = orphan.run_id();
        drop(orphan);
        let live = ScratchRun::create_retained(&archive, owner.clone()).unwrap();
        let live_id = live.run_id();
        let ephemeral = ScratchRun::create_ephemeral(&archive, owner.clone()).unwrap();
        let ephemeral_id = ephemeral.run_id();
        let foreign = ScratchRun::create_retained(&archive, foreign_owner).unwrap();
        let foreign_id = foreign.run_id();
        drop(foreign);
        let conflict = root
            .join(SCRATCH_DIR)
            .join(format!("run-{reachable_id} (1)"));
        fs::create_dir(&conflict).unwrap();
        fs::write(conflict.join(SCRATCH_MARKER_FILE), b"conflict copy").unwrap();

        let before = [reachable_id, orphan_id, live_id, ephemeral_id, foreign_id]
            .map(|run_id| (run_id, run_snapshot(&root, run_id)));
        let census = census_retained_runs(&archive, &owner).unwrap();
        assert_eq!(
            census,
            RetainedRunCensus {
                retained: 3,
                ephemeral: 1,
                unclassified: 2,
            }
        );
        for (run_id, bytes) in &before {
            assert_eq!(run_snapshot(&root, *run_id), *bytes);
        }
        assert_eq!(
            fs::read(conflict.join(SCRATCH_MARKER_FILE)).unwrap(),
            b"conflict copy"
        );

        // SAFETY: this synthetic test enumerated the complete authoritative
        // reachable set above; exactly `reachable_id` is reachable.
        let outcome = unsafe {
            reclaim_unreachable_retained_runs(&archive, &owner, |run_id| run_id == reachable_id)
        }
        .unwrap();
        assert_eq!(
            outcome,
            RetainedRunReclamation {
                retained_reachable: 1,
                retained_reclaimed: 1,
                retained_live_skipped: 1,
                ephemeral_preserved: 1,
                unclassified_preserved: 2,
            }
        );
        assert!(!run_path(&root, orphan_id).exists());
        for run_id in [reachable_id, live_id, ephemeral_id, foreign_id] {
            let baseline = before.iter().find(|(id, _)| *id == run_id).unwrap();
            assert_eq!(run_snapshot(&root, run_id), baseline.1);
        }
        assert_eq!(
            fs::read(conflict.join(SCRATCH_MARKER_FILE)).unwrap(),
            b"conflict copy"
        );

        drop(ephemeral);
        drop(live);
        fs::remove_dir_all(root).unwrap();
    }
}
