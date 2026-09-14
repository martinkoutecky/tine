#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _, OpenOptionsMaybeDirExt as _};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
#[cfg(any(test, target_os = "android"))]
use std::io;
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions, ReadDir};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::enrollment::EnrollmentBindingV1;
use super::sync_layout::{
    ARCHIVE_BATCHES_DIR as BATCHES_DIR, ARCHIVE_OBJECTS_DIR as OBJECTS_DIR, LINEAGE_CLAIM_FILE,
};
use super::{
    BatchError, BatchId, BatchOrigin, ContentDigest, LineageDigest, ObjectDescriptor,
    OperationBatch, OperationObject, PreparedBatch, ValidatedBatch, WorkspaceId,
    MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES,
};

/// Stable semantic value carried by enrollment anchors that predate removal
/// of the physical engine-history index. This does not name a live store.
pub(crate) fn empty_engine_history_root_digest() -> ContentDigest {
    ContentDigest::of(b"tine/oplog-engine-history/radix-v1/empty")
}

/// Retained, O(1)-memory enumeration of immutable manifest commit markers.
///
/// The cursor deliberately preserves the filesystem iterator instead of
/// materializing and sorting the complete archive. Callers which need a full
/// audit continue to use [`ObjectStore::committed_manifests`].
pub(crate) struct ObjectStoreManifestCursor {
    entries: ReadDir,
}
thread_local! {
    // This one hook is also used by the crate-private deterministic simulator.
    // It is deliberately narrower than a general object-store fault injector:
    // the only observable boundary is after every immutable object is durable
    // and before the manifest commit marker is published.
    static HARNESS_PUBLISH_FAIL_AFTER_OBJECTS: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    static ENROLLED_OPEN_USE_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static ENROLLED_OPEN_ACT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn fail_next_publish_after_objects() {
    HARNESS_PUBLISH_FAIL_AFTER_OBJECTS.with(|fail| fail.set(true));
}

thread_local! {
    static ARCHIVE_INSTALL_CUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Arm one deterministic cut immediately after the next staged archive
/// artifact is installed under its final name. On a strict publication this is
/// after the data barrier; on a turn-covered publication the first object cut
/// is deliberately before it, while the exact journal frame remains undrained.
#[cfg(test)]
pub(crate) fn cut_after_next_archive_install() {
    ARCHIVE_INSTALL_CUT.with(|armed| armed.set(true));
}

fn archive_install_cut_hook() -> Result<(), StoreError> {
    ARCHIVE_INSTALL_CUT.with(|armed| {
        if armed.replace(false) {
            Err(StoreError::Io(std::io::Error::other(
                "deterministic cut after an archive install",
            )))
        } else {
            Ok(())
        }
    })
}

fn publish_after_objects_hook() -> Result<(), StoreError> {
    HARNESS_PUBLISH_FAIL_AFTER_OBJECTS.with(|fail| {
        if fail.replace(false) {
            Err(StoreError::Io(std::io::Error::other(
                "deterministic failure after object publication",
            )))
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
pub(crate) fn set_enrolled_open_use_hook(hook: impl FnOnce() + 'static) {
    ENROLLED_OPEN_USE_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
pub(crate) fn set_enrolled_open_act_hook(hook: impl FnOnce() + 'static) {
    ENROLLED_OPEN_ACT_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn enrolled_open_use_hook() {
    ENROLLED_OPEN_USE_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn enrolled_open_act_hook() {
    ENROLLED_OPEN_ACT_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

/// A caller-rooted, v2-candidate immutable object and batch-manifest store.
///
/// Opening this type is the only persistence trigger. It is intentionally not
/// connected to graph startup, enrollment, or the legacy managed-sync store.
#[derive(Debug)]
pub struct ObjectStore {
    root_path: PathBuf,
    workspace_id: WorkspaceId,
    capability: Dir,
    counters: Arc<StoreCounters>,
    /// Fingerprints collected while the ordinary namespace validator already
    /// has every manifest body in memory. Checkpoint roster comparison reads
    /// this cache, never the pre-roster manifest bodies a second time.
    validated_manifest_fingerprints: Arc<std::sync::RwLock<BTreeMap<BatchId, ContentDigest>>>,
    validated_manifest_names: Arc<std::sync::RwLock<BTreeSet<BatchId>>>,
    validated_object_names: Arc<std::sync::RwLock<BTreeSet<ContentDigest>>>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlDirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlDirectoryIdentity {
    volume: u64,
    file_id: [u8; 16],
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlDirectoryIdentity;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AcceptedReadStats {
    pub manifest_reads: usize,
    pub object_reads: usize,
}

/// Process-wide `inspect_batch` cost, for the F49 quadratic probe only.
///
/// The per-store `counters` are reset with the store; these are not, so an
/// import's *total* re-read volume can be read once at the end of a run.
/// Diagnostic only — nothing reads these outside the probe.
pub static INSPECT_BATCH_CALLS: AtomicUsize = AtomicUsize::new(0);
/// `caller file:line -> (calls, required objects)`. `#[track_caller]` on
/// `inspect_batch` makes this exact without touching a single call site --
/// which matters, because guessing which callers dominate has already been
/// wrong once.
pub static INSPECT_BATCH_SITES: std::sync::Mutex<
    std::collections::BTreeMap<String, (usize, usize)>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());
pub static INSPECT_BATCH_OBJECT_READS: AtomicUsize = AtomicUsize::new(0);
pub static INSPECT_BATCH_OBJECT_BYTES: AtomicUsize = AtomicUsize::new(0);
pub static INSPECT_BATCH_DIGEST_BYTES: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ObjectStoreStats {
    pub directory_enumerations: usize,
    pub namespace_manifest_decodes: usize,
    pub namespace_object_decodes: usize,
    /// These stay zero on a generation-aware open. They are explicit so the
    /// bounded-open ratchet observes work rather than inferring it from time.
    pub accepted_manifest_reads: usize,
    pub accepted_object_reads: usize,
    pub dag_manifest_reads: usize,
    pub inspected_manifest_operations: usize,
    pub inspected_manifest_bytes: usize,
    pub inspected_object_operations: usize,
    pub inspected_object_bytes: usize,
    /// Logical objects resolved out of cold packs. Ordinary hot paths must
    /// leave this at zero; it is the oracle for MS-02's per-consumer
    /// hot-only / indexed-cold classification.
    pub cold_object_reads: usize,
    pub cold_manifest_reads: usize,
    pub hot_object_bytes: usize,
    pub hot_manifest_bytes: usize,
    pub cold_object_bytes: usize,
    pub cold_manifest_bytes: usize,
    /// Logical batch inventories considered by generation relocation.  Later
    /// generations increment this only for C+1..=new C.
    pub relocation_batch_visits: usize,
    pub retired_hot_manifests: usize,
    pub retired_hot_objects: usize,
}

#[derive(Debug, Default)]
struct StoreCounters {
    directory_enumerations: AtomicUsize,
    namespace_manifest_decodes: AtomicUsize,
    namespace_object_decodes: AtomicUsize,
    accepted_manifest_reads: AtomicUsize,
    accepted_object_reads: AtomicUsize,
    dag_manifest_reads: AtomicUsize,
    inspected_manifest_operations: AtomicUsize,
    inspected_manifest_bytes: AtomicUsize,
    inspected_object_operations: AtomicUsize,
    inspected_object_bytes: AtomicUsize,
    cold_object_reads: AtomicUsize,
    cold_manifest_reads: AtomicUsize,
    hot_object_bytes: AtomicUsize,
    hot_manifest_bytes: AtomicUsize,
    cold_object_bytes: AtomicUsize,
    cold_manifest_bytes: AtomicUsize,
    relocation_batch_visits: AtomicUsize,
    retired_hot_manifests: AtomicUsize,
    retired_hot_objects: AtomicUsize,
}

/// How one logical read is permitted to resolve its bytes.
///
/// MS-02 5.3 gives every consumer a permanent hot-only / indexed-cold /
/// forbidden classification. This type is where that classification lives in
/// code: an ordinary path cannot reach a pack byte by accident, because the
/// hot-only reader has no cold branch at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LogicalReadClass {
    /// Hot originals only. An absent hot name is ordinary absence. Page
    /// open/save/move, the SQLite applier, projection payload pins and the
    /// operational coordinator's admission path all read this way.
    HotOnly,
    /// Hot original first, then one indexed cold lookup bounded by this
    /// object's own index path and byte range. Never an enumeration, never a
    /// history scan.
    HotThenIndexedCold,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ArchiveDiscoveryInspection {
    Absent,
    Residue,
}

/// Inspect one explicit existing archive root without constructing an
/// [`ObjectStore`] or any writer/runtime authority.
///
/// This is intentionally only a presence probe, used to distinguish true
/// absence from unexplained pre-0.7 archive residue. The expected binding is
/// retained at the caller boundary while the old bound engine-history reader
/// is gone; neither case constructs an archive writer or runtime authority.
pub(crate) fn inspect_existing_archive_at(
    archive_root: &Path,
    _expected_binding: Option<&EnrollmentBindingV1>,
) -> Result<ArchiveDiscoveryInspection, StoreError> {
    let Some(_archive) = open_existing_archive_root_nofollow(archive_root)? else {
        return Ok(ArchiveDiscoveryInspection::Absent);
    };
    // Every archive in this namespace predates the clean 0.7 storage format.
    // Preserve it through the caller archive-aside flow; never decode it into
    // current runtime authority.
    Ok(ArchiveDiscoveryInspection::Residue)
}

fn open_existing_archive_root_nofollow(root: &Path) -> Result<Option<Dir>, StoreError> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StoreError::UnsafeEntry(
            "archive root is not a real no-follow directory".into(),
        ));
    }
    let name = root
        .file_name()
        .ok_or_else(|| StoreError::UnsafeEntry("archive root has no final component".into()))?;
    if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
        return Err(StoreError::UnsafeEntry(
            "archive root must end in a normal path component".into(),
        ));
    }
    let name = name.to_str().ok_or_else(|| {
        StoreError::UnsafeEntry("archive root final component is not UTF-8".into())
    })?;
    let parent = root.parent().ok_or_else(|| {
        StoreError::UnsafeEntry("archive root must have an existing parent".into())
    })?;
    let canonical_parent = fs::canonicalize(parent)?;
    let parent = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
    open_existing_dir_nofollow(&parent, name)
}

#[cfg(test)]
mod discovery_inspector_tests {
    use super::*;

    #[test]
    fn explicit_archive_presence_probe_creates_nothing() {
        let parent =
            std::env::temp_dir().join(format!("tine-archive-discovery-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&parent).unwrap();
        let archive = parent.join("archive");
        assert_eq!(
            inspect_existing_archive_at(&archive, None).unwrap(),
            ArchiveDiscoveryInspection::Absent
        );
        assert!(!archive.exists());

        std::fs::create_dir(&archive).unwrap();
        assert_eq!(
            inspect_existing_archive_at(&archive, None).unwrap(),
            ArchiveDiscoveryInspection::Residue
        );
        assert!(std::fs::read_dir(&archive).unwrap().next().is_none());
        crate::test_support::remove_dir_all(parent);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchInspection {
    /// No manifest commit marker exists. Object-only residue remains invisible.
    Absent,
    /// The manifest is valid, but these canonical descriptors are not present.
    Staged {
        manifest: OperationBatch,
        missing: Vec<ObjectDescriptor>,
    },
    /// The manifest and its exact closed object set have been validated.
    Ready(ValidatedBatch),
}

fn parse_available_kib(meminfo: &str) -> Option<u64> {
    let value = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    value.checked_mul(1024)
}

fn finite_cgroup_available(maximum: &str, current: &str) -> Option<u64> {
    let maximum = maximum.trim().parse::<u64>().ok()?;
    if maximum >= (1_u64 << 60) {
        return None;
    }
    let current = current.trim().parse::<u64>().ok()?;
    Some(maximum.saturating_sub(current))
}

pub(crate) fn available_memory_bytes() -> Option<u64> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let host = fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|meminfo| parse_available_kib(&meminfo));
        let cgroup_v2 = fs::read_to_string("/sys/fs/cgroup/memory.max")
            .ok()
            .zip(fs::read_to_string("/sys/fs/cgroup/memory.current").ok())
            .and_then(|(maximum, current)| finite_cgroup_available(&maximum, &current));
        let cgroup_v1 = fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes")
            .ok()
            .zip(fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes").ok())
            .and_then(|(maximum, current)| finite_cgroup_available(&maximum, &current));
        return [host, cgroup_v2, cgroup_v1].into_iter().flatten().min();
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: `status` is initialized with the exact ABI size required by
        // GlobalMemoryStatusEx and remains exclusively borrowed for the call.
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullAvailPhys)
    }
}

impl ObjectStore {
    /// Open or create a store at an explicit root and retain the opened
    /// directory capability for all later operations.
    pub fn open(root: &Path, workspace_id: WorkspaceId) -> Result<Self, StoreError> {
        let store = Self::open_structural(root, workspace_id)?;
        store.validate_namespace()?;
        Ok(store)
    }

    /// Retain and structurally authenticate the archive without yet trusting
    /// immutable object contents. This narrow cold-open seam exists so the
    /// caller can acquire the workspace lease, recover undrained journal bytes,
    /// repair exactly covered torn object names, and only then run the ordinary
    /// full namespace validation.
    pub(crate) fn open_structural(
        root: &Path,
        workspace_id: WorkspaceId,
    ) -> Result<Self, StoreError> {
        let name = root
            .file_name()
            .ok_or_else(|| StoreError::UnsafeEntry("store root has no final component".into()))?;
        if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
            return Err(StoreError::UnsafeEntry(
                "store root must end in a normal path component".into(),
            ));
        }
        let parent = root.parent().ok_or_else(|| {
            StoreError::UnsafeEntry("store root must have an existing parent".into())
        })?;
        let canonical_parent = fs::canonicalize(parent)?;
        let parent_capability = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
        let relative = Path::new(name);
        let name = name.to_str().ok_or_else(|| {
            StoreError::UnsafeEntry("store root final component is not UTF-8".into())
        })?;

        match parent_capability.symlink_metadata(relative) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StoreError::UnsafeEntry(
                    "store root is not a real no-follow directory".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                parent_capability.create_dir(relative)?;
                sync_dir_required(&parent_capability)?;
            }
            Err(error) => return Err(error.into()),
        }

        let capability = open_dir_nofollow(&parent_capability, name)?;
        ensure_directory(&capability, OBJECTS_DIR)?;
        ensure_directory(&capability, BATCHES_DIR)?;
        Ok(Self {
            root_path: canonical_parent.join(name),
            workspace_id,
            capability,
            counters: Arc::new(StoreCounters::default()),
            validated_manifest_fingerprints: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            validated_manifest_names: Arc::new(std::sync::RwLock::new(BTreeSet::new())),
            validated_object_names: Arc::new(std::sync::RwLock::new(BTreeSet::new())),
        })
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    /// Duplicate the retained no-follow archive-root capability that roots the
    /// workspace runtime lease.
    ///
    /// The lease is deliberately archive-rooted rather than app-data-rooted:
    /// the returned handle is the same physical directory resource this store
    /// already authenticated, so two processes with different XDG, HOME, or
    /// Flatpak roots still contend on one lock, and renaming the archive
    /// pathname cannot split it.
    pub(crate) fn workspace_runtime_lease_capability(&self) -> std::io::Result<Dir> {
        self.capability.try_clone()
    }

    /// Duplicate the retained archive capability for a lease-owned private
    /// derived namespace. The caller never reopens `root_path`, so an archive
    /// rename cannot redirect completion-index reads or writes.
    pub(crate) fn private_derived_root_capability(&self) -> std::io::Result<Dir> {
        self.capability.try_clone()
    }

    /// Install one coalesced group of immutable private derived objects with
    /// the archive batch protocol: data first, final names second, and one
    /// directory barrier for the shared namespace.
    pub(crate) fn publish_coalesced_private_derived(
        &self,
        namespace: &Dir,
        artifacts: &[(&str, &[u8], u64)],
        collision_kind: &'static str,
    ) -> Result<(), StoreError> {
        let mut publication = ArchiveBatchPublication::strict(&self.capability)?;
        let namespace_index = publication.namespace(namespace)?;
        for (name, bytes, limit) in artifacts {
            publication.stage(
                namespace_index,
                name,
                bytes,
                *limit,
                Collision::Exact(collision_kind),
                false,
            )?;
        }
        publication.commit()
    }

    /// Duplicate this store directly from its retained no-follow archive-root
    /// capability.
    ///
    /// The duplicate is the *same* physical directory resource, never a fresh
    /// ambient pathname open, so a caller that already authenticated one exact
    /// archive can hand a consuming API (`seal_history_only`, `seal_enrolled_projection`)
    /// its own store value without ever reintroducing a pathname race. An
    /// archive renamed while retained open stays bound to the enrolled archive,
    /// and a look-alike directory that appears at the old pathname is not
    /// reachable through the duplicate at all.
    pub(crate) fn duplicate_retained_capability(&self) -> Result<Self, StoreError> {
        Ok(Self {
            root_path: self.root_path.clone(),
            workspace_id: self.workspace_id,
            capability: self.capability.try_clone()?,
            counters: Arc::clone(&self.counters),
            validated_manifest_fingerprints: Arc::clone(&self.validated_manifest_fingerprints),
            validated_manifest_names: Arc::clone(&self.validated_manifest_names),
            validated_object_names: Arc::clone(&self.validated_object_names),
        })
    }

    /// Prove this store's retained capability and its enrolled archive pathname
    /// still name one and the same physical directory.
    ///
    /// The retained capability remains the authority; this only refuses an
    /// *ambiguous* archive. If the archive was renamed while it stayed retained
    /// open and a look-alike directory now occupies the enrolled pathname, then
    /// two different directories both answer to "the enrolled archive": one by
    /// resource identity, one by pathname. A one-shot durable publication must
    /// block there rather than silently pick a winner, because the two
    /// candidates diverge immediately afterwards.
    ///
    /// Nothing is created, repaired, claimed, or written. The check is one
    /// ambient parent open, one no-follow child open, and one identity stat.
    pub(crate) fn authenticate_unambiguous_archive_pathname(&self) -> Result<(), StoreError> {
        let parent = self.root_path.parent().ok_or_else(|| {
            StoreError::UnsafeEntry("store root must have an existing parent".into())
        })?;
        let name = self
            .root_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                StoreError::UnsafeEntry("store root final component is not UTF-8".into())
            })?;
        let parent_capability = Dir::open_ambient_dir(parent, ambient_authority())?;
        let named = open_existing_dir_nofollow(&parent_capability, name)?.ok_or_else(|| {
            StoreError::UnsafeEntry(
                "enrolled archive pathname no longer names a real no-follow directory".into(),
            )
        })?;
        if control_directory_identity(&named)? != self.canonical_archive_identity()? {
            return Err(StoreError::UnsafeEntry(
                "enrolled archive pathname no longer names the retained archive capability".into(),
            ));
        }
        Ok(())
    }

    /// Validate and retain one object independently of any manifest delivery.
    pub fn stage_object_bytes(&self, bytes: &[u8]) -> Result<ContentDigest, StoreError> {
        let object = OperationObject::decode(bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let digest = ContentDigest::of(bytes);
        let objects = self.open_namespace(OBJECTS_DIR)?;
        publish_immutable(
            &objects,
            &object_filename(digest),
            bytes,
            Collision::Object(digest),
        )?;
        self.validated_object_names
            .write()
            .map_err(|_| StoreError::UnsafeEntry("object-name cache is poisoned".into()))?
            .insert(digest);
        Ok(digest)
    }

    /// Validate and publish the sole batch commit marker. Missing objects do
    /// not prevent staging the marker and remain invisible until complete.
    pub fn stage_manifest_bytes(&self, bytes: &[u8]) -> Result<BatchId, StoreError> {
        let manifest = OperationBatch::decode(bytes)?;
        if manifest.origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        let batch_id = manifest.batch_id();
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        if read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?.is_some() {
            self.check_or_establish_lineage(manifest.lineage_digest())?;
            publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
            self.record_validated_manifest_fingerprint(batch_id, ContentDigest::of(bytes))?;
            return Ok(batch_id);
        }
        self.check_or_establish_lineage(manifest.lineage_digest())?;
        publish_immutable(&batches, &filename, bytes, Collision::Batch(batch_id))?;
        self.record_validated_manifest_fingerprint(batch_id, ContentDigest::of(bytes))?;
        Ok(batch_id)
    }

    /// Publish a prevalidated complete batch in the required order: every
    /// content-addressed object first, then the manifest commit marker.
    pub fn publish_prepared(&self, batch: &PreparedBatch) -> Result<(), StoreError> {
        if batch.manifest().origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        self.publish_prepared_impl(batch, false, false)
    }

    /// Legacy semantic-engine fixtures construct synthetic bootstrap batches
    /// without the retired bootstrap publisher. Keep their archive oracle
    /// test-only; production callers must use the current direct publication
    /// boundary and remain rejected by `publish_prepared` above.
    #[cfg(test)]
    pub(crate) fn publish_prepared_fixture(&self, batch: &PreparedBatch) -> Result<(), StoreError> {
        self.publish_prepared_impl(batch, true, false)
    }

    /// Publish an ordinary batch whose exact bytes are still retained by the
    /// caller's undrained managed-local journal frame. Only that durable turn
    /// permits object installation without a pre-install data barrier.
    pub(crate) fn publish_turn_covered_prepared(
        &self,
        batch: &PreparedBatch,
    ) -> Result<(), StoreError> {
        if batch.manifest().origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        self.publish_prepared_impl(batch, false, true)
    }

    /// Publish one accepted batch's complete archive materialization.
    ///
    /// Ordinary callers use a strict batch barrier before any immutable name
    /// becomes visible. The managed-local drain may instead set
    /// `turn_covered_objects`: its durable, still-undrained journal frame is the
    /// exact recovery authority for object bytes, so object temporaries may be
    /// installed before the batch-wide data flush. That flush then covers both
    /// the installed object inodes and the staged manifest; the manifest is
    /// installed only afterward, and the journal checkpoint can advance only
    /// after publication returns. Cold open repairs only torn object names
    /// covered byte-for-byte by an undrained record, then runs the ordinary full
    /// namespace validation.
    fn publish_prepared_impl(
        &self,
        batch: &PreparedBatch,
        allow_bootstrap: bool,
        turn_covered_objects: bool,
    ) -> Result<(), StoreError> {
        let manifest = batch.manifest();
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        if !allow_bootstrap && manifest.origin() == BatchOrigin::BootstrapImport {
            return Err(StoreError::BootstrapBatchRequiresDirectPublication);
        }
        let manifest_bytes = manifest.encode()?;
        let batch_id = manifest.batch_id();

        // The lineage claim is written once in an archive's lifetime and is a
        // precondition of every manifest, not part of this batch. It keeps its
        // own barrier so a manifest can never be durable before the lineage it
        // asserts.
        self.check_or_establish_lineage(manifest.lineage_digest())
            .map_err(|error| publication_stage_error("publish operation manifest", error))?;

        let objects = self.open_namespace(OBJECTS_DIR)?;
        let batches = self.open_namespace(BATCHES_DIR)?;
        let mut publication = if turn_covered_objects {
            ArchiveBatchPublication::turn_covered(&self.capability)?
        } else {
            ArchiveBatchPublication::strict(&self.capability)?
        };
        let objects_namespace = publication.namespace(&objects)?;
        let batches_namespace = publication.namespace(&batches)?;
        for object in batch.objects() {
            let bytes = object.encode()?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            let digest = ContentDigest::of(&bytes);
            publication
                .stage(
                    objects_namespace,
                    &object_filename(digest),
                    &bytes,
                    MAX_OBJECT_BYTES as u64,
                    Collision::Object(digest),
                    false,
                )
                .map_err(|error| publication_stage_error("publish operation object", error))?;
        }
        publish_after_objects_hook()?;
        publication
            .stage(
                batches_namespace,
                &manifest_filename(batch_id),
                &manifest_bytes,
                MAX_MANIFEST_BYTES as u64,
                Collision::Batch(batch_id),
                true,
            )
            .map_err(|error| publication_stage_error("publish operation manifest", error))?;

        publication.commit().map_err(|error| {
            publication_stage_error("publish operation batch durability", error)
        })?;
        self.record_validated_manifest_fingerprint(batch_id, ContentDigest::of(&manifest_bytes))?;
        Ok(())
    }

    /// Inspect a single manifest and validate every present required object.
    /// Missing objects stage the batch; corrupt or mismatched objects reject it.
    ///
    /// This is the ordinary hot-only path: it never touches the cold locator
    /// index or a pack byte. Named historical consumers use
    /// [`Self::inspect_batch_with_cold_history`] instead.
    #[track_caller]
    pub fn inspect_batch(&self, batch_id: BatchId) -> Result<BatchInspection, StoreError> {
        self.inspect_batch_impl(batch_id, LogicalReadClass::HotOnly)
    }

    #[track_caller]
    fn inspect_batch_impl(
        &self,
        batch_id: BatchId,
        class: LogicalReadClass,
    ) -> Result<BatchInspection, StoreError> {
        INSPECT_BATCH_CALLS.fetch_add(1, Ordering::Relaxed);
        let site = super::inspect_site_trace_enabled().then(|| {
            let caller = std::panic::Location::caller();
            format!("{}:{}", caller.file(), caller.line())
        });
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let manifest_bytes =
            match read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)? {
                Some(bytes) => bytes,
                None => match class {
                    LogicalReadClass::HotOnly => return Ok(BatchInspection::Absent),
                    LogicalReadClass::HotThenIndexedCold => {
                        match self.cold_manifest_bytes(batch_id)? {
                            Some(bytes) => bytes,
                            None => return Ok(BatchInspection::Absent),
                        }
                    }
                },
            };
        self.counters
            .inspected_manifest_operations
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .inspected_manifest_bytes
            .fetch_add(manifest_bytes.len(), Ordering::Relaxed);
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }

        if let Some(site) = site {
            if let Ok(mut sites) = INSPECT_BATCH_SITES.lock() {
                let entry = sites.entry(site).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += manifest.required_objects().len();
            }
        }
        let objects_dir = self.open_namespace(OBJECTS_DIR)?;
        let mut missing = Vec::new();
        let mut objects = Vec::with_capacity(manifest.required_objects().len());
        for descriptor in manifest.required_objects() {
            self.counters
                .inspected_object_operations
                .fetch_add(1, Ordering::Relaxed);
            let filename = object_filename(descriptor.content_digest());
            let hot = read_optional_regular(
                &objects_dir,
                &filename,
                MAX_OBJECT_BYTES as u64,
                Some(descriptor.encoded_byte_length()),
            )?;
            let bytes = match hot {
                Some(bytes) => bytes,
                None => match class {
                    LogicalReadClass::HotOnly => {
                        missing.push(descriptor.clone());
                        continue;
                    }
                    LogicalReadClass::HotThenIndexedCold => {
                        match self.cold_object_bytes(descriptor.content_digest())? {
                            Some(bytes) => bytes,
                            None => {
                                missing.push(descriptor.clone());
                                continue;
                            }
                        }
                    }
                },
            };
            self.counters
                .inspected_object_bytes
                .fetch_add(bytes.len(), Ordering::Relaxed);
            INSPECT_BATCH_OBJECT_READS.fetch_add(1, Ordering::Relaxed);
            INSPECT_BATCH_OBJECT_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
            INSPECT_BATCH_DIGEST_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
            let content_digest = ContentDigest::of(&bytes);
            if content_digest != descriptor.content_digest() {
                return Err(StoreError::ObjectPathMismatch(descriptor.content_digest()));
            }
            let object = OperationObject::decode(&bytes)?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            let actual = ObjectDescriptor::new(
                object.document_id(),
                object.kind(),
                content_digest,
                bytes.len() as u64,
            )?;
            if actual != *descriptor {
                return Err(StoreError::Batch(BatchError::DescriptorMismatch {
                    expected: descriptor.clone(),
                    actual,
                }));
            }
            objects.push(object);
        }

        if !missing.is_empty() {
            return Ok(BatchInspection::Staged { manifest, missing });
        }
        // Exact lookup against the atomically established immutable lineage
        // claim keeps the Ready path independent of archive cardinality.
        // Store open and explicit `committed_manifests` remain full audits.
        self.require_lineage(manifest.lineage_digest())?;
        let prepared = PreparedBatch::new(manifest, objects)?;
        Ok(BatchInspection::Ready(ValidatedBatch::new(prepared)))
    }

    pub(crate) fn reload_accepted_document_object(
        &self,
        manifest: &OperationBatch,
        document_id: super::DocumentId,
    ) -> Result<OperationObject, StoreError> {
        let batch_id = manifest.batch_id();
        let descriptor = manifest
            .required_objects()
            .iter()
            .find(|descriptor| {
                descriptor.kind() == super::ObjectKind::CrdtUpdate
                    && descriptor.document_id() == document_id
            })
            .ok_or(StoreError::AcceptedDocumentUpdateMissing {
                batch_id,
                document_id,
            })?;
        self.counters
            .accepted_object_reads
            .fetch_add(1, Ordering::Relaxed);
        crate::fast_commit::note_archive_object_read();
        // Accepted-history reload is a named cold consumer: the hot original
        // may have been retired after exact cold publication. Stay on the one
        // logical resolver rather than opening a second inventory.
        let bytes = self.resolve_logical_object_bytes(descriptor.content_digest())?;
        if ContentDigest::of(&bytes) != descriptor.content_digest() {
            return Err(StoreError::ObjectPathMismatch(descriptor.content_digest()));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        let actual = object.descriptor()?;
        if actual != *descriptor {
            return Err(StoreError::Batch(BatchError::DescriptorMismatch {
                expected: descriptor.clone(),
                actual,
            }));
        }
        Ok(object)
    }

    pub(crate) fn reload_accepted_manifest(
        &self,
        batch_id: BatchId,
        expected_manifest_fingerprint: ContentDigest,
    ) -> Result<OperationBatch, StoreError> {
        self.counters
            .accepted_manifest_reads
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .dag_manifest_reads
            .fetch_add(1, Ordering::Relaxed);
        // Full-history reconstruction can evict a prepared batch from the hot
        // engine and reload it after Packet 4 has retired its hot manifest.
        // Resolve the immutable original through the single hot/cold surface.
        let bytes = self.resolve_logical_manifest_bytes(batch_id)?;
        let actual = ContentDigest::of(&bytes);
        if actual != expected_manifest_fingerprint {
            return Err(StoreError::AcceptedManifestMismatch {
                batch_id,
                expected: expected_manifest_fingerprint,
                actual,
            });
        }
        let manifest = OperationBatch::decode(&bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(manifest)
    }

    pub(crate) fn accepted_read_stats(&self) -> AcceptedReadStats {
        AcceptedReadStats {
            manifest_reads: self
                .counters
                .accepted_manifest_reads
                .load(Ordering::Relaxed),
            object_reads: self.counters.accepted_object_reads.load(Ordering::Relaxed),
        }
    }

    pub fn instrumentation(&self) -> ObjectStoreStats {
        self.counters.snapshot()
    }

    fn record_validated_manifest_fingerprint(
        &self,
        batch_id: BatchId,
        fingerprint: ContentDigest,
    ) -> Result<(), StoreError> {
        let mut fingerprints = self.validated_manifest_fingerprints.write().map_err(|_| {
            StoreError::UnsafeEntry("manifest fingerprint cache is poisoned".into())
        })?;
        if let Some(existing) = fingerprints.insert(batch_id, fingerprint) {
            if existing != fingerprint {
                return Err(StoreError::BatchCollision(batch_id));
            }
        }
        self.validated_manifest_names
            .write()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))?
            .insert(batch_id);
        Ok(())
    }

    /// Fingerprints collected by the already-required namespace validation.
    /// This is observational cache output only; it grants no archive authority.
    pub(crate) fn validated_manifest_fingerprints(
        &self,
    ) -> Result<BTreeMap<BatchId, ContentDigest>, StoreError> {
        self.validated_manifest_fingerprints
            .read()
            .map(|fingerprints| fingerprints.clone())
            .map_err(|_| StoreError::UnsafeEntry("manifest fingerprint cache is poisoned".into()))
    }

    /// Stable identity of the retained no-follow archive root capability.
    ///
    /// This is derived from the opened directory resource, never from an
    /// ambient path string, so two `ObjectStore` values opened over the same
    /// archive compare equal while a substituted directory does not.
    pub(crate) fn canonical_archive_identity(
        &self,
    ) -> Result<ControlDirectoryIdentity, StoreError> {
        control_directory_identity(&self.capability)
    }

    /// Enumerate all manifest commit markers in deterministic BatchId order.
    /// Staged manifests are included; readiness is determined by `inspect_batch`.
    pub fn committed_manifests(&self) -> Result<Vec<OperationBatch>, StoreError> {
        self.counters
            .directory_enumerations
            .fetch_add(1, Ordering::Relaxed);
        let batches = self.open_namespace(BATCHES_DIR)?;
        let mut manifests = Vec::new();
        for entry in batches.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| StoreError::MalformedPath("non-UTF-8 batch entry".into()))?;
            if is_temp_name(name) {
                require_regular_entry(&entry.file_type()?, name)?;
                continue;
            }
            require_regular_entry(&entry.file_type()?, name)?;
            let batch_id = parse_manifest_filename(name)?;
            let bytes = read_required_regular(&batches, name, MAX_MANIFEST_BYTES as u64, None)?;
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            if manifest.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: manifest.workspace_id(),
                });
            }
            manifests.push(manifest);
        }
        manifests.sort_unstable_by_key(OperationBatch::batch_id);
        if let Some(first) = manifests.first() {
            for manifest in &manifests[1..] {
                if manifest.lineage_digest() != first.lineage_digest() {
                    return Err(StoreError::LineageMismatch {
                        expected: first.lineage_digest(),
                        found: manifest.lineage_digest(),
                    });
                }
            }
        }
        Ok(manifests)
    }

    /// Enumerate only canonical manifest commit-marker names. No manifest or
    /// object body is opened or decoded.
    pub(crate) fn committed_manifest_names(&self) -> Result<BTreeSet<BatchId>, StoreError> {
        self.validated_manifest_names
            .read()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))
            .map(|names| names.clone())
    }

    /// The one committed-history discovery used by full reconstruction: hot
    /// commit-marker names plus authenticated cold manifest membership,
    /// deduplicated by immutable BatchId. Payload resolution remains routed
    /// through the logical hot-then-indexed-cold resolver.
    pub(crate) fn committed_manifest_names_with_cold_history(
        &self,
    ) -> Result<BTreeSet<BatchId>, StoreError> {
        let mut names = self.committed_manifest_names()?;
        if let Some(cold) = super::cold_object_store::SealedArchiveReader::open(self)? {
            names.extend(cold.manifest_batch_ids()?);
        }
        Ok(names)
    }

    /// Enumerate only canonical content-addressed object names. No object body
    /// is opened or decoded.
    pub(crate) fn object_names(&self) -> Result<BTreeSet<ContentDigest>, StoreError> {
        self.validated_object_names
            .read()
            .map_err(|_| StoreError::UnsafeEntry("object-name cache is poisoned".into()))
            .map(|names| names.clone())
    }

    pub(crate) fn checkpoint_namespace_delta(
        &self,
        roster: &BTreeSet<BatchId>,
        required_objects: &BTreeSet<ContentDigest>,
    ) -> Result<(BTreeSet<BatchId>, Option<BatchId>, Option<ContentDigest>), StoreError> {
        let manifests = self
            .validated_manifest_names
            .read()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))?;
        let missing_manifest = roster
            .iter()
            .find(|batch_id| !manifests.contains(batch_id))
            .copied();
        let tail = manifests.difference(roster).copied().collect();
        let objects = self
            .validated_object_names
            .read()
            .map_err(|_| StoreError::UnsafeEntry("object-name cache is poisoned".into()))?;
        let missing_object = required_objects
            .iter()
            .find(|digest| !objects.contains(digest))
            .copied();
        Ok((tail, missing_manifest, missing_object))
    }

    /// Recovery-only checkpoint qualification over the authenticated logical
    /// archive. Ordinary open keeps its hot cached path; full reconstruction
    /// has already authorized the lifetime-sized cold manifest-map walk.
    pub(crate) fn checkpoint_namespace_delta_with_cold_history(
        &self,
        roster: &BTreeSet<BatchId>,
        required_objects: &BTreeSet<ContentDigest>,
    ) -> Result<(BTreeSet<BatchId>, Option<BatchId>, Option<ContentDigest>), StoreError> {
        let manifests = self.committed_manifest_names_with_cold_history()?;
        let missing_manifest = roster
            .iter()
            .find(|batch_id| !manifests.contains(batch_id))
            .copied();
        let tail = manifests.difference(roster).copied().collect();
        let mut missing_object = None;
        for digest in required_objects {
            if !self.contains_logical_object(*digest)? {
                missing_object = Some(*digest);
                break;
            }
        }
        Ok((tail, missing_manifest, missing_object))
    }

    /// Begin an incremental manifest enumeration without opening any manifest
    /// or object bytes.
    pub(crate) fn manifest_cursor(&self) -> Result<ObjectStoreManifestCursor, StoreError> {
        self.counters
            .directory_enumerations
            .fetch_add(1, Ordering::Relaxed);
        Ok(ObjectStoreManifestCursor {
            entries: self.open_namespace(BATCHES_DIR)?.entries()?,
        })
    }

    /// Visit at most one immutable manifest from a retained cursor.
    ///
    /// Temporary publication names are validated and skipped. A returned
    /// manifest is decoded and workspace-bound, but its objects are not opened.
    pub(crate) fn next_manifest(
        &self,
        cursor: &mut ObjectStoreManifestCursor,
    ) -> Result<Option<OperationBatch>, StoreError> {
        loop {
            let Some(entry) = cursor.entries.next() else {
                return Ok(None);
            };
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| StoreError::MalformedPath("non-UTF-8 batch entry".into()))?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            let batch_id = parse_manifest_filename(name)?;
            let bytes = self.read_manifest_bytes(batch_id)?;
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            return Ok(Some(manifest));
        }
    }

    pub(crate) fn read_manifest_bytes(&self, batch_id: BatchId) -> Result<Vec<u8>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let bytes = read_required_regular(
            &batches,
            &manifest_filename(batch_id),
            MAX_MANIFEST_BYTES as u64,
            None,
        )?;
        let manifest = OperationBatch::decode(&bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(bytes)
    }

    pub fn contains_object(&self, digest: ContentDigest) -> Result<bool, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?
        else {
            return Ok(false);
        };
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(true)
    }

    /// Read and validate only a batch's manifest, without touching its objects.
    ///
    /// The manifest's descriptors already carry `document_id`, `kind`,
    /// `content_digest` and `encoded_byte_length` for every object, so a caller
    /// that needs object *metadata* -- which documents a batch updates, how many
    /// bytes it retains, which object holds a given kind -- never needs to read
    /// the objects themselves. Pair this with [`Self::read_object`] to fetch the
    /// one payload that is genuinely required.
    pub(crate) fn read_manifest(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<OperationBatch>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        let filename = manifest_filename(batch_id);
        let Some(manifest_bytes) =
            read_optional_regular(&batches, &filename, MAX_MANIFEST_BYTES as u64, None)?
        else {
            return Ok(None);
        };
        self.counters
            .inspected_manifest_operations
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .inspected_manifest_bytes
            .fetch_add(manifest_bytes.len(), Ordering::Relaxed);
        let manifest = OperationBatch::decode(&manifest_bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        Ok(Some(manifest))
    }

    /// Read exactly the one content-addressed object a caller already has the
    /// digest for.
    ///
    /// This is the access path [`Self::inspect_batch`] is *not*: inspecting a
    /// batch to obtain a single object costs O(whole batch) — it reads,
    /// SHA-256s and decodes every object the manifest requires. Callers that
    /// hold a digest are asking for a file whose name already *is* that digest,
    /// so there is nothing to search and nothing to re-prove about the rest of
    /// the batch.
    ///
    /// The object's own integrity is still checked here (digest + workspace),
    /// because that is O(one object) and keeps the content-addressing contract.
    /// What is dropped is re-proving *batch completeness* per object, which is
    /// established once, by [`Self::inspect_batch`]: it reports
    /// [`BatchInspection::Ready`] only after reading and digesting every object
    /// its manifest names, and reports [`BatchInspection::Staged`] otherwise.
    /// Every production path that admits a batch goes through that call and
    /// refuses anything but `Ready` — the coordinator's projection drain and
    /// `sync_runtime`'s clean outbound publication both do. A caller reaching
    /// `read_object` is therefore working inside a batch whose completeness has
    /// already been proved; only this one object's integrity is left to check.
    pub(crate) fn read_object(&self, digest: ContentDigest) -> Result<OperationObject, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        // Counted on the same counters as `inspect_batch`'s per-object reads.
        // These measure *object reads*, not one particular access path, and
        // tests use them as an oracle for how much an operation reconstructs
        // (`ordinary_drain_reconstructs_each_accepted_event_once`). A new path
        // that read objects silently would make that oracle lie.
        self.counters
            .inspected_object_operations
            .fetch_add(1, Ordering::Relaxed);
        let bytes = read_required_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?;
        self.counters
            .inspected_object_bytes
            .fetch_add(bytes.len(), Ordering::Relaxed);
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(object)
    }

    pub(crate) fn read_object_bytes(&self, digest: ContentDigest) -> Result<Vec<u8>, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let bytes = read_required_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?;
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(bytes)
    }

    /// Repair only content-addressed object names whose canonical bytes remain
    /// authoritative in an undrained managed-local journal record.
    ///
    /// The caller holds the archive's sole-writer workspace lease. Each
    /// replacement is still bound to the exact bad bytes observed here, staged
    /// and flushed before an atomic same-directory replacement, followed by a
    /// directory barrier. Uncovered, ambiguous, malformed, or foreign objects
    /// are deliberately left for `validate_namespace` to refuse unchanged.
    pub(crate) fn repair_covered_object_mismatches(
        &self,
        covered: &BTreeMap<ContentDigest, Vec<u8>>,
    ) -> Result<usize, StoreError> {
        if covered.is_empty() {
            return Ok(0);
        }
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let mut repaired = 0_usize;
        for (expected, replacement) in covered {
            let name = object_filename(*expected);
            let Some(observed) =
                read_optional_regular(&objects, &name, MAX_OBJECT_BYTES as u64, None)?
            else {
                continue;
            };
            if ContentDigest::of(&observed) == *expected {
                continue;
            }
            if ContentDigest::of(replacement) != *expected {
                return Err(StoreError::ObjectPathMismatch(*expected));
            }
            let object = OperationObject::decode(replacement)?;
            if object.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: object.workspace_id(),
                });
            }
            if object.encode()?.as_slice() != replacement {
                return Err(StoreError::ObjectPathMismatch(*expected));
            }
            tine_storage::DurableDirectoryPublication::open(&objects)
                .map_err(|error| publication_error(error, Collision::Object(*expected)))?
                .replace_exact(&name, &observed, replacement)
                .map_err(|error| publication_error(error, Collision::Object(*expected)))?;
            let installed = read_required_regular(
                &objects,
                &name,
                MAX_OBJECT_BYTES as u64,
                Some(replacement.len() as u64),
            )?;
            if installed != *replacement {
                return Err(StoreError::ObjectPathMismatch(*expected));
            }
            repaired = repaired.saturating_add(1);
        }
        Ok(repaired)
    }

    pub(crate) fn validate_namespace(&self) -> Result<(), StoreError> {
        self.validate_namespace_impl(None, &BTreeSet::new())?;
        Ok(())
    }

    /// Validate only the hot tail selected by a qualified generation. A hot
    /// duplicate of covered cold history is not an ordinary-open input.
    pub(crate) fn validate_namespace_for_generation(
        &self,
        history: &super::checkpoint_generation::SealedAcceptedHistory,
        pinned_batches: &BTreeSet<BatchId>,
    ) -> Result<(), StoreError> {
        let covered_hot = self.validate_namespace_impl(Some(history), pinned_batches)?;
        // Resume a marker-last retirement interrupted by a crash. This is
        // empty on every steady open; a recovery open verifies exact cold
        // bytes before repeating the idempotent removals.
        self.retire_hot_history_for_batches(&covered_hot, pinned_batches)?;
        Ok(())
    }

    fn validate_namespace_impl(
        &self,
        covered: Option<&super::checkpoint_generation::SealedAcceptedHistory>,
        pinned_batches: &BTreeSet<BatchId>,
    ) -> Result<BTreeSet<BatchId>, StoreError> {
        let mut manifests = Vec::new();
        let mut manifest_fingerprints = BTreeMap::new();
        let mut manifest_names = BTreeSet::new();
        let mut object_names = BTreeSet::new();
        let mut covered_hot = BTreeSet::new();
        for (directory, kind) in [
            (OBJECTS_DIR, NamespaceKind::Objects),
            (BATCHES_DIR, NamespaceKind::Batches),
        ] {
            self.counters
                .directory_enumerations
                .fetch_add(1, Ordering::Relaxed);
            let dir = self.open_namespace(directory)?;
            for entry in dir.entries()? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_str().ok_or_else(|| {
                    StoreError::MalformedPath(format!("non-UTF-8 entry under {directory}"))
                })?;
                require_regular_entry(&entry.file_type()?, name)?;
                if is_temp_name(name) {
                    let limit = match kind {
                        NamespaceKind::Objects => MAX_OBJECT_BYTES as u64,
                        NamespaceKind::Batches => MAX_MANIFEST_BYTES as u64,
                    };
                    read_required_regular(&dir, name, limit, None)?;
                    continue;
                }
                match kind {
                    NamespaceKind::Objects => {
                        let expected = parse_object_filename(name)?;
                        let is_covered = covered
                            .map(|history| history.contains_object(expected))
                            .transpose()
                            .map_err(StoreError::UnsafeEntry)?
                            .unwrap_or(false);
                        if is_covered {
                            continue;
                        }
                        let bytes =
                            read_required_regular(&dir, name, MAX_OBJECT_BYTES as u64, None)?;
                        self.counters
                            .namespace_object_decodes
                            .fetch_add(1, Ordering::Relaxed);
                        if ContentDigest::of(&bytes) != expected {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                        let object = OperationObject::decode(&bytes)?;
                        if object.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: object.workspace_id(),
                            });
                        }
                        if object.encode()?.as_slice() != bytes {
                            return Err(StoreError::ObjectPathMismatch(expected));
                        }
                        object_names.insert(expected);
                    }
                    NamespaceKind::Batches => {
                        let expected = parse_manifest_filename(name)?;
                        let is_covered = covered
                            .map(|history| history.contains_batch(expected))
                            .transpose()
                            .map_err(StoreError::UnsafeEntry)?
                            .unwrap_or(false);
                        if is_covered && !pinned_batches.contains(&expected) {
                            covered_hot.insert(expected);
                            continue;
                        }
                        let bytes =
                            read_required_regular(&dir, name, MAX_MANIFEST_BYTES as u64, None)?;
                        self.counters
                            .namespace_manifest_decodes
                            .fetch_add(1, Ordering::Relaxed);
                        let manifest = OperationBatch::decode(&bytes)?;
                        if manifest.batch_id() != expected {
                            return Err(StoreError::ManifestPathMismatch {
                                expected,
                                found: manifest.batch_id(),
                            });
                        }
                        if manifest.workspace_id() != self.workspace_id {
                            return Err(StoreError::WorkspaceMismatch {
                                expected: self.workspace_id,
                                found: manifest.workspace_id(),
                            });
                        }
                        manifest_fingerprints.insert(expected, ContentDigest::of(&bytes));
                        manifest_names.insert(expected);
                        manifests.push(manifest);
                    }
                }
            }
        }
        ensure_single_lineage(&manifests)?;
        if let Some(first) = manifests.first() {
            self.check_or_establish_lineage(first.lineage_digest())?;
        } else {
            let _ = read_optional_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
        }
        *self.validated_manifest_fingerprints.write().map_err(|_| {
            StoreError::UnsafeEntry("manifest fingerprint cache is poisoned".into())
        })? = manifest_fingerprints;
        *self
            .validated_manifest_names
            .write()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))? =
            manifest_names;
        *self
            .validated_object_names
            .write()
            .map_err(|_| StoreError::UnsafeEntry("object-name cache is poisoned".into()))? =
            object_names;
        Ok(covered_hot)
    }

    fn check_or_establish_lineage(&self, lineage: LineageDigest) -> Result<(), StoreError> {
        if let Some(bytes) =
            read_optional_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?
        {
            return require_lineage_bytes(lineage, &bytes);
        }
        match publish_immutable(
            &self.capability,
            LINEAGE_CLAIM_FILE,
            lineage.as_bytes(),
            Collision::Lineage(lineage),
        ) {
            Ok(()) => Ok(()),
            Err(StoreError::LineageClaimCollision(_)) => {
                let bytes =
                    read_required_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
                require_lineage_bytes(lineage, &bytes)
            }
            Err(error) => Err(error),
        }
    }

    fn require_lineage(&self, lineage: LineageDigest) -> Result<(), StoreError> {
        let bytes = read_required_regular(&self.capability, LINEAGE_CLAIM_FILE, 32, Some(32))?;
        require_lineage_bytes(lineage, &bytes)
    }

    fn open_namespace(&self, name: &str) -> Result<Dir, StoreError> {
        let metadata = self.capability.symlink_metadata(name)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeEntry(format!(
                "{name} is not a real no-follow directory"
            )));
        }
        open_dir_nofollow(&self.capability, name)
    }
}

/// The single logical-object resolver.
///
/// Production already consumes it: SQLite projection rebuild -- the full replay
/// of accepted history -- resolves its manifests, semantic effects and whole
/// batches here (`oplog::sqlite::AcceptedBatchEvent::{from_accepted,
/// from_indexed}`). The remaining indexed-cold consumers -- Restore, historical
/// admission, republication and previous-generation fallback -- live in
/// `sync_runtime.rs` and `hot_engine.rs`, which other live lanes own; RECEIPT.md
/// hands those exact call sites to the manager. `#[allow(dead_code)]` covers the
/// members only that gap and the maintenance entry points still reach.
#[allow(dead_code)]
impl ObjectStore {
    /// Inspect a batch whose hot originals may already have been relocated.
    ///
    /// Identical to [`Self::inspect_batch`] except that an absent hot manifest
    /// or object falls through to the single cold resolver. Reserved for the
    /// named indexed-cold consumers -- Restore, historical admission,
    /// republication, previous-generation fallback and full replay.
    #[track_caller]
    pub(crate) fn inspect_batch_with_cold_history(
        &self,
        batch_id: BatchId,
    ) -> Result<BatchInspection, StoreError> {
        self.inspect_batch_impl(batch_id, LogicalReadClass::HotThenIndexedCold)
    }

    // -----------------------------------------------------------------------
    // The single logical-object resolver
    //
    // Every logical read in this crate already funnels through this type, so
    // routing the cold tier here -- rather than into any consumer -- is what
    // makes "one resolver, no hidden second reader" structural rather than a
    // convention. `read_object`/`read_manifest`/`inspect_batch` stay hot-only;
    // the `resolve_*` family below is the indexed-cold surface, and nothing
    // else in the crate opens a pack.
    // -----------------------------------------------------------------------

    /// Hot object bytes, or `None` when the hot name is absent. Integrity is
    /// still proved for a present object.
    fn optional_hot_object_bytes(
        &self,
        digest: ContentDigest,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &objects,
            &object_filename(digest),
            MAX_OBJECT_BYTES as u64,
            None,
        )?
        else {
            return Ok(None);
        };
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        Ok(Some(bytes))
    }

    fn optional_hot_manifest_bytes(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let batches = self.open_namespace(BATCHES_DIR)?;
        read_optional_regular(
            &batches,
            &manifest_filename(batch_id),
            MAX_MANIFEST_BYTES as u64,
            None,
        )
    }

    /// One indexed cold object lookup. Absence of any cold history at all is
    /// `None`, not a refusal: an archive that never relocated anything is
    /// healthy.
    fn cold_object_bytes(&self, digest: ContentDigest) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(reader) = super::cold_object_store::SealedArchiveReader::open(self)? else {
            return Ok(None);
        };
        let Some(bytes) = reader.object_bytes(digest)? else {
            return Ok(None);
        };
        // Cold bytes are the canonical original: re-prove the content address
        // and workspace binding exactly as the hot reader does.
        if ContentDigest::of(&bytes) != digest {
            return Err(StoreError::ObjectPathMismatch(digest));
        }
        let object = OperationObject::decode(&bytes)?;
        if object.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: object.workspace_id(),
            });
        }
        self.counters
            .cold_object_reads
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .cold_object_bytes
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(Some(bytes))
    }

    fn cold_manifest_bytes(&self, batch_id: BatchId) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(reader) = super::cold_object_store::SealedArchiveReader::open(self)? else {
            return Ok(None);
        };
        let Some(bytes) = reader.manifest_bytes(batch_id)? else {
            return Ok(None);
        };
        let manifest = OperationBatch::decode(&bytes)?;
        if manifest.batch_id() != batch_id {
            return Err(StoreError::ManifestPathMismatch {
                expected: batch_id,
                found: manifest.batch_id(),
            });
        }
        if manifest.workspace_id() != self.workspace_id {
            return Err(StoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: manifest.workspace_id(),
            });
        }
        self.counters
            .cold_manifest_reads
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .cold_manifest_bytes
            .fetch_add(bytes.len(), Ordering::Relaxed);
        Ok(Some(bytes))
    }

    /// Resolve one logical object's exact canonical bytes, hot original first.
    ///
    /// Reserved for the named indexed-cold consumers. Work is bounded by this
    /// object's index path and byte range; no pack or manifest namespace is
    /// enumerated, and unrelated history is never touched.
    pub(crate) fn resolve_logical_object_bytes(
        &self,
        digest: ContentDigest,
    ) -> Result<Vec<u8>, StoreError> {
        if let Some(bytes) = self.optional_hot_object_bytes(digest)? {
            self.counters
                .inspected_object_operations
                .fetch_add(1, Ordering::Relaxed);
            self.counters
                .inspected_object_bytes
                .fetch_add(bytes.len(), Ordering::Relaxed);
            self.counters
                .hot_object_bytes
                .fetch_add(bytes.len(), Ordering::Relaxed);
            return Ok(bytes);
        }
        self.cold_object_bytes(digest)?
            .ok_or(StoreError::ColdObjectUnavailable {
                digest,
                reason: "no hot original and no cold locator".into(),
            })
    }

    /// Resolve and decode one logical object, hot original first.
    pub(crate) fn resolve_logical_object(
        &self,
        digest: ContentDigest,
    ) -> Result<OperationObject, StoreError> {
        Ok(OperationObject::decode(
            &self.resolve_logical_object_bytes(digest)?,
        )?)
    }

    /// Resolve one batch manifest's exact canonical bytes, hot original first.
    pub(crate) fn resolve_logical_manifest_bytes(
        &self,
        batch_id: BatchId,
    ) -> Result<Vec<u8>, StoreError> {
        if let Some(bytes) = self.optional_hot_manifest_bytes(batch_id)? {
            let manifest = OperationBatch::decode(&bytes)?;
            if manifest.batch_id() != batch_id {
                return Err(StoreError::ManifestPathMismatch {
                    expected: batch_id,
                    found: manifest.batch_id(),
                });
            }
            if manifest.workspace_id() != self.workspace_id {
                return Err(StoreError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: manifest.workspace_id(),
                });
            }
            self.counters
                .hot_manifest_bytes
                .fetch_add(bytes.len(), Ordering::Relaxed);
            return Ok(bytes);
        }
        self.cold_manifest_bytes(batch_id)?
            .ok_or(StoreError::ColdManifestUnavailable {
                batch_id,
                reason: "no hot original and no cold locator".into(),
            })
    }

    /// Resolve and decode one batch manifest, hot original first. `None` means
    /// the batch is unknown to both tiers.
    pub(crate) fn resolve_logical_manifest(
        &self,
        batch_id: BatchId,
    ) -> Result<Option<OperationBatch>, StoreError> {
        if let Some(manifest) = self.read_manifest(batch_id)? {
            return Ok(Some(manifest));
        }
        let Some(bytes) = self.cold_manifest_bytes(batch_id)? else {
            return Ok(None);
        };
        Ok(Some(OperationBatch::decode(&bytes)?))
    }

    /// Whether this logical object is retrievable from either tier.
    pub(crate) fn contains_logical_object(
        &self,
        digest: ContentDigest,
    ) -> Result<bool, StoreError> {
        if self.contains_object(digest)? {
            return Ok(true);
        }
        Ok(self.cold_object_bytes(digest)?.is_some())
    }

    /// Additively relocate these batches' exact hot originals into cold packs.
    /// Nothing hot is removed here; see `cold_object_store` for why.
    pub(crate) fn publish_cold_history_for_batches(
        &self,
        batches: &BTreeSet<BatchId>,
    ) -> Result<super::cold_object_store::ColdPublicationOutcome, StoreError> {
        self.counters
            .relocation_batch_visits
            .fetch_add(batches.len(), Ordering::Relaxed);
        super::cold_object_store::publish_cold_history_for_batches(self, batches)
    }

    /// Retire exact hot names only after the cold root has made the same bytes
    /// point-readable.  Removal is idempotent and the two namespace barriers
    /// make every crash prefix resolve from hot, cold, or both.
    pub(crate) fn retire_hot_history_for_batches(
        &self,
        batches: &BTreeSet<BatchId>,
        pinned_batches: &BTreeSet<BatchId>,
    ) -> Result<(usize, usize), StoreError> {
        if batches.is_empty() {
            return Ok((0, 0));
        }
        let cold = super::cold_object_store::SealedArchiveReader::open(self)?
            .ok_or_else(|| StoreError::ColdHistoryRootMissing)?;
        let mut pinned_objects = BTreeSet::new();
        for batch_id in pinned_batches {
            if let Some(bytes) = cold.manifest_bytes(*batch_id)? {
                let manifest = OperationBatch::decode(&bytes)?;
                pinned_objects.extend(
                    manifest
                        .required_objects()
                        .iter()
                        .map(ObjectDescriptor::content_digest),
                );
            }
        }
        // A batch accepted after the generation capture is not part of
        // `batches`, but it can still share a content-addressed object with a
        // covered batch. Keep every object named by that live hot tail. The
        // validated manifest cache is already bounded by tail + explicit pins:
        // each retirement below removes the covered, unpinned names from it.
        let live_hot_batches = self
            .validated_manifest_names
            .read()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))?
            .clone();
        for batch_id in live_hot_batches.difference(batches) {
            if let Some(manifest) = self.read_manifest(*batch_id)? {
                pinned_objects.extend(
                    manifest
                        .required_objects()
                        .iter()
                        .map(ObjectDescriptor::content_digest),
                );
            }
        }
        let mut retire_objects = BTreeSet::new();
        let mut retire_manifests = Vec::new();
        for batch_id in batches {
            let cold_bytes =
                cold.manifest_bytes(*batch_id)?
                    .ok_or(StoreError::ColdManifestUnavailable {
                        batch_id: *batch_id,
                        reason: "generation relocation root omits a covered manifest".into(),
                    })?;
            let manifest = OperationBatch::decode(&cold_bytes)?;
            for descriptor in manifest.required_objects() {
                let digest = descriptor.content_digest();
                let bytes =
                    cold.object_bytes(digest)?
                        .ok_or(StoreError::ColdObjectUnavailable {
                            digest,
                            reason: "generation relocation root omits a covered object".into(),
                        })?;
                if ContentDigest::of(&bytes) != digest {
                    return Err(StoreError::ObjectPathMismatch(digest));
                }
                if !pinned_objects.contains(&digest) {
                    retire_objects.insert(digest);
                }
            }
            if !pinned_batches.contains(batch_id) {
                retire_manifests.push(*batch_id);
            }
        }
        let objects = self.open_namespace(OBJECTS_DIR)?;
        let manifests = self.open_namespace(BATCHES_DIR)?;
        let retired_object_names = retire_objects.clone();
        let mut removed_objects = 0;
        for digest in retire_objects {
            match objects.remove_file(object_filename(digest)) {
                Ok(()) => removed_objects += 1,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut removed_manifests = 0;
        for batch_id in retire_manifests {
            match manifests.remove_file(manifest_filename(batch_id)) {
                Ok(()) => removed_manifests += 1,
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if removed_objects != 0 {
            sync_dir_required(&objects)?;
        }
        if removed_manifests != 0 {
            sync_dir_required(&manifests)?;
        }
        self.counters
            .retired_hot_objects
            .fetch_add(removed_objects, Ordering::Relaxed);
        self.counters
            .retired_hot_manifests
            .fetch_add(removed_manifests, Ordering::Relaxed);
        {
            let mut names = self
                .validated_object_names
                .write()
                .map_err(|_| StoreError::UnsafeEntry("object-name cache is poisoned".into()))?;
            names.retain(|digest| !retired_object_names.contains(digest));
        }
        self.validated_manifest_names
            .write()
            .map_err(|_| StoreError::UnsafeEntry("manifest-name cache is poisoned".into()))?
            .retain(|batch_id| !batches.contains(batch_id) || pinned_batches.contains(batch_id));
        self.validated_manifest_fingerprints
            .write()
            .map_err(|_| StoreError::UnsafeEntry("manifest fingerprint cache is poisoned".into()))?
            .retain(|batch_id, _| !batches.contains(batch_id) || pinned_batches.contains(batch_id));
        Ok((removed_manifests, removed_objects))
    }

    /// Republish every cold record into fresh packs and rebuild the locator
    /// index from pack footers alone.
    pub(crate) fn repack_cold_history(
        &self,
    ) -> Result<super::cold_object_store::ColdPublicationOutcome, StoreError> {
        super::cold_object_store::repack_cold_history(self)
    }

    /// Recover a lost cold-history root marker from the preserved packs.
    ///
    /// The explicit damaged-state operation behind
    /// [`StoreError::ColdHistoryRootMissing`]. Healthy archives are untouched.
    pub(crate) fn repair_cold_history_root(
        &self,
    ) -> Result<super::cold_object_store::ColdRepairOutcome, StoreError> {
        super::cold_object_store::repair_cold_history_root(self)
    }
}

impl StoreCounters {
    fn snapshot(&self) -> ObjectStoreStats {
        ObjectStoreStats {
            directory_enumerations: self.directory_enumerations.load(Ordering::Relaxed),
            namespace_manifest_decodes: self.namespace_manifest_decodes.load(Ordering::Relaxed),
            namespace_object_decodes: self.namespace_object_decodes.load(Ordering::Relaxed),
            accepted_manifest_reads: self.accepted_manifest_reads.load(Ordering::Relaxed),
            accepted_object_reads: self.accepted_object_reads.load(Ordering::Relaxed),
            dag_manifest_reads: self.dag_manifest_reads.load(Ordering::Relaxed),
            inspected_manifest_operations: self
                .inspected_manifest_operations
                .load(Ordering::Relaxed),
            inspected_manifest_bytes: self.inspected_manifest_bytes.load(Ordering::Relaxed),
            inspected_object_operations: self.inspected_object_operations.load(Ordering::Relaxed),
            inspected_object_bytes: self.inspected_object_bytes.load(Ordering::Relaxed),
            cold_object_reads: self.cold_object_reads.load(Ordering::Relaxed),
            cold_manifest_reads: self.cold_manifest_reads.load(Ordering::Relaxed),
            hot_object_bytes: self.hot_object_bytes.load(Ordering::Relaxed),
            hot_manifest_bytes: self.hot_manifest_bytes.load(Ordering::Relaxed),
            cold_object_bytes: self.cold_object_bytes.load(Ordering::Relaxed),
            cold_manifest_bytes: self.cold_manifest_bytes.load(Ordering::Relaxed),
            relocation_batch_visits: self.relocation_batch_visits.load(Ordering::Relaxed),
            retired_hot_manifests: self.retired_hot_manifests.load(Ordering::Relaxed),
            retired_hot_objects: self.retired_hot_objects.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone, Copy)]
enum NamespaceKind {
    Objects,
    Batches,
}

/// `TINE_PUBLISH_TRACE=1` names the artifact class of every individually
/// published immutable artifact on stderr.
///
/// The barrier budget in `docs/storage-sync-contract.md` 2.10a-i is a number;
/// this is how you find out WHICH artifacts make it up when the number moves.
/// Read once per process, like the other `TINE_*_TRACE` switches in this crate.
fn publication_trace_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TINE_PUBLISH_TRACE").is_some())
}

impl Collision {
    /// The artifact class this publication belongs to, for the publication
    /// trace.
    fn artifact_class(&self) -> String {
        match self {
            Self::Object(_) => "archive-object".to_owned(),
            Self::Batch(_) => "archive-manifest".to_owned(),
            Self::Lineage(_) => "archive-lineage-claim".to_owned(),
            Self::Exact(kind) => (*kind).to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
enum Collision {
    Object(ContentDigest),
    Batch(BatchId),
    Lineage(LineageDigest),
    Exact(&'static str),
}

fn ensure_single_lineage(manifests: &[OperationBatch]) -> Result<(), StoreError> {
    if let Some(first) = manifests.first() {
        for manifest in &manifests[1..] {
            if manifest.lineage_digest() != first.lineage_digest() {
                return Err(StoreError::LineageMismatch {
                    expected: first.lineage_digest(),
                    found: manifest.lineage_digest(),
                });
            }
        }
    }
    Ok(())
}

fn require_lineage_bytes(expected: LineageDigest, bytes: &[u8]) -> Result<(), StoreError> {
    let found_bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| StoreError::MalformedPath(LINEAGE_CLAIM_FILE.into()))?;
    let found = LineageDigest::from_bytes(found_bytes);
    if found != expected {
        return Err(StoreError::LineageMismatch { expected, found });
    }
    Ok(())
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Batch(BatchError),
    Bootstrap(String),
    UnsafeEntry(String),
    MalformedPath(String),
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    LineageMismatch {
        expected: LineageDigest,
        found: LineageDigest,
    },
    ObjectCollision(ContentDigest),
    BatchCollision(BatchId),
    ObjectPathMismatch(ContentDigest),
    ManifestPathMismatch {
        expected: BatchId,
        found: BatchId,
    },
    AcceptedManifestMismatch {
        batch_id: BatchId,
        expected: ContentDigest,
        actual: ContentDigest,
    },
    AcceptedDocumentUpdateMissing {
        batch_id: BatchId,
        document_id: super::DocumentId,
    },
    UpgradeRequired {
        store: &'static str,
        found: u32,
        current: u32,
    },
    UnsupportedStoreVersion {
        store: &'static str,
        version: u32,
    },
    MalformedPortablePathIndex,
    MissingExactLogicalPageNameBlob(ContentDigest),
    ExactLogicalPageNameBlobPathMismatch(ContentDigest),
    MalformedPageNameIndex,
    PageNamePointBatchTooLarge {
        actual: usize,
        limit: usize,
    },
    NonCanonicalPageNamePointKeys,
    MissingPageNameCatalogFrontier,
    MisboundPageNameCatalogFrontier,
    LineageClaimCollision(LineageDigest),
    ImmutableCollision(&'static str),
    /// The cold locator index itself could not be read. Hot and current state
    /// remain usable; only indexed historical lookups are affected.
    ColdHistoryIndexUnavailable(String),
    /// Cold packs are preserved but their derived root marker is gone. This is
    /// a named repair condition (I-8), never ordinary absence: the packs are
    /// self-describing, so `repair_cold_history_root` recovers the index from
    /// them. Hot and current state remain usable throughout.
    ColdHistoryRootMissing,
    /// Two different canonical manifests claim the same `BatchId`. A `BatchId`
    /// is an identity, not a content address, so this is a collision and never
    /// a successfully archived exact copy. The predecessor bytes are preserved.
    ColdManifestConflict {
        batch_id: BatchId,
        reason: String,
    },
    /// This exact logical object is not retrievable from cold history. The
    /// refusal names the affected logical object (I-8) and leaves every other
    /// object, hot or cold, unaffected.
    ColdObjectUnavailable {
        digest: ContentDigest,
        reason: String,
    },
    /// This exact batch manifest is not retrievable from cold history.
    ColdManifestUnavailable {
        batch_id: BatchId,
        reason: String,
    },
    BootstrapBatchRequiresDirectPublication,
    InactiveBootstrapHistory,
    StoredLengthMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    StoredFileTooLarge {
        path: String,
        length: u64,
        limit: u64,
    },
}

impl StoreError {
    /// Physical damage to the archive's own bytes, as opposed to a logical
    /// refusal. These are exactly the variants the clean-open taxonomy reports
    /// as `MS-REF-DISK-CORRUPT`; the in-scope scenarios are a crash or power
    /// loss mid-write, a truncated write, a disk/media error, and a sync
    /// service delivering a partially copied archive.
    ///
    /// It exists so a caller that must convert this error into another type
    /// can carry the classification across the conversion instead of leaving
    /// the next layer to recover control flow from display text.
    pub(crate) fn is_physical_archive_damage(&self) -> bool {
        matches!(
            self,
            Self::ColdHistoryIndexUnavailable(_)
                | Self::ColdHistoryRootMissing
                | Self::ColdManifestConflict { .. }
                | Self::ColdObjectUnavailable { .. }
                | Self::ColdManifestUnavailable { .. }
        )
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Batch(error) => error.fmt(f),
            Self::Bootstrap(error) => error.fmt(f),
            Self::UnsafeEntry(message) => write!(f, "unsafe store entry: {message}"),
            Self::MalformedPath(path) => write!(f, "malformed store path: {path}"),
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::LineageMismatch { expected, found } => {
                write!(f, "lineage mismatch: expected {expected}, found {found}")
            }
            Self::ObjectCollision(digest) => write!(f, "content-address collision at {digest}"),
            Self::BatchCollision(batch_id) => {
                write!(f, "fatal manifest collision for batch {batch_id}")
            }
            Self::ObjectPathMismatch(digest) => {
                write!(f, "stored object bytes do not match path {digest}")
            }
            Self::ManifestPathMismatch { expected, found } => write!(
                f,
                "manifest path names batch {expected}, but bytes name {found}"
            ),
            Self::AcceptedManifestMismatch {
                batch_id,
                expected,
                actual,
            } => write!(
                f,
                "accepted manifest {batch_id} fingerprint mismatch: expected {expected}, found {actual}"
            ),
            Self::AcceptedDocumentUpdateMissing {
                batch_id,
                document_id,
            } => write!(
                f,
                "accepted manifest {batch_id} has no CRDT update for document {document_id}"
            ),
            Self::UpgradeRequired {
                store,
                found,
                current,
            } => write!(f, "{store} version {found} requires upgrade to {current}"),
            Self::UnsupportedStoreVersion { store, version } => {
                write!(f, "{store} version {version} is unsupported")
            }
            Self::MalformedPortablePathIndex => {
                f.write_str("authenticated portable-path index is malformed or non-canonical")
            }
            Self::MissingExactLogicalPageNameBlob(digest) => {
                write!(f, "exact logical page-name blob {digest} is missing")
            }
            Self::ExactLogicalPageNameBlobPathMismatch(digest) => {
                write!(
                    f,
                    "exact logical page-name blob bytes do not match path {digest}"
                )
            }
            Self::MalformedPageNameIndex => {
                f.write_str("authenticated page-name ownership index is malformed or non-canonical")
            }
            Self::PageNamePointBatchTooLarge { actual, limit } => write!(
                f,
                "page-name point batch has {actual} entries, exceeding {limit}"
            ),
            Self::NonCanonicalPageNamePointKeys => {
                f.write_str("page-name point keys are not strictly sorted and unique")
            }
            Self::MissingPageNameCatalogFrontier => {
                f.write_str("exact page-name catalog-frontier binding is missing")
            }
            Self::MisboundPageNameCatalogFrontier => {
                f.write_str("exact page-name catalog-frontier binding is misbound")
            }
            Self::LineageClaimCollision(lineage) => {
                write!(f, "immutable lineage claim collision for {lineage}")
            }
            Self::ImmutableCollision(kind) => {
                write!(f, "immutable {kind} collision")
            }
            Self::ColdHistoryIndexUnavailable(reason) => {
                write!(f, "cold history index is unavailable: {reason}")
            }
            Self::ColdHistoryRootMissing => f.write_str(
                "cold history packs are preserved but their root marker is missing: \
                 repair the cold history index before publishing or reading it",
            ),
            Self::ColdManifestConflict { batch_id, reason } => write!(
                f,
                "cold logical manifest {batch_id} conflicts with archived history: {reason}"
            ),
            Self::ColdObjectUnavailable { digest, reason } => write!(
                f,
                "cold logical object {digest} is unavailable: {reason}"
            ),
            Self::ColdManifestUnavailable { batch_id, reason } => write!(
                f,
                "cold logical manifest {batch_id} is unavailable: {reason}"
            ),
            Self::BootstrapBatchRequiresDirectPublication => {
                f.write_str("bootstrap batches require bootstrap-specific direct publication")
            }
            Self::InactiveBootstrapHistory => {
                f.write_str("inactive bootstrap history cannot be opened as ordinary runtime")
            }
            Self::StoredLengthMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "stored file length mismatch at {path}: expected {expected}, found {actual}"
            ),
            Self::StoredFileTooLarge {
                path,
                length,
                limit,
            } => write!(
                f,
                "stored file at {path} is {length} bytes, exceeding limit {limit}"
            ),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Batch(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BatchError> for StoreError {
    fn from(error: BatchError) -> Self {
        Self::Batch(error)
    }
}

pub(crate) fn open_existing_dir_nofollow(
    root: &Dir,
    name: &str,
) -> Result<Option<Dir>, StoreError> {
    tine_storage::open_existing_dir_nofollow(root, name).map_err(filesystem_error_without_collision)
}

#[cfg(unix)]
pub fn control_directory_identity(dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = dir.try_clone()?.into_std_file().metadata()?;
    Ok(ControlDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
pub fn control_directory_identity(dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let file = dir.try_clone()?.into_std_file();
    let mut information = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err(StoreError::Io(std::io::Error::last_os_error()));
    }
    Ok(ControlDirectoryIdentity {
        volume: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

#[cfg(not(any(unix, windows)))]
pub fn control_directory_identity(_dir: &Dir) -> Result<ControlDirectoryIdentity, StoreError> {
    Err(StoreError::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "directory identity is unavailable on this platform",
    )))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivateDirectoryDurability {
    StrictAuthority,
    Reconstructible,
}

fn ensure_directory_nofollow_with_durability(
    root: &Dir,
    name: &str,
    durability: PrivateDirectoryDurability,
) -> Result<(), StoreError> {
    #[cfg(target_os = "android")]
    {
        let component = Path::new(name);
        if !matches!(component.components().next(), Some(Component::Normal(_)))
            || component.components().count() != 1
        {
            return Err(StoreError::UnsafeEntry(format!(
                "managed private directory name is not one normal component: {name}"
            )));
        }
        match root.symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(StoreError::UnsafeEntry(format!(
                    "managed private directory is not a real no-follow directory: {name}"
                )));
            }
            Ok(_) => {
                if durability == PrivateDirectoryDurability::StrictAuthority {
                    sync_dir_required(root)?;
                }
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::NotFound => match root.create_dir(component) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(StoreError::Io(error)),
            },
            Err(error) => return Err(StoreError::Io(error)),
        }
        let metadata = root.symlink_metadata(component)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StoreError::UnsafeEntry(format!(
                "managed private directory is not a real no-follow directory: {name}"
            )));
        }
        match durability {
            PrivateDirectoryDurability::StrictAuthority => sync_dir_required(root)?,
            PrivateDirectoryDurability::Reconstructible => {
                crate::filesystem_durability::sync_reconstructible_directory(root)
                    .map_err(StoreError::Io)?;
            }
        }
        return Ok(());
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = durability;
        tine_storage::ensure_directory_nofollow(root, name)
            .map_err(filesystem_error_without_collision)
    }
}

/// Create or reopen a directory whose descendants can carry private durable
/// authority. Android must prove the parent namespace durable even when the
/// exact directory already exists after an earlier refused barrier or restart.
pub(crate) fn ensure_directory_nofollow(root: &Dir, name: &str) -> Result<(), StoreError> {
    ensure_directory_nofollow_with_durability(
        root,
        name,
        PrivateDirectoryDurability::StrictAuthority,
    )
}

/// Create or reopen a directory whose complete contents are disposable or
/// reconstructible from accepted authority elsewhere.
pub(crate) fn ensure_reconstructible_directory_nofollow(
    root: &Dir,
    name: &str,
) -> Result<(), StoreError> {
    ensure_directory_nofollow_with_durability(
        root,
        name,
        PrivateDirectoryDurability::Reconstructible,
    )
}

fn ensure_directory(root: &Dir, name: &str) -> Result<(), StoreError> {
    ensure_directory_nofollow(root, name)
}

fn publish_immutable(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    collision: Collision,
) -> Result<(), StoreError> {
    // ObjectStore is rooted in the app-private archive and is only mutated
    // while the managed runtime owns its sole-writer lease. This is distinct
    // from shared/provider publication, which must retain strict no-replace
    // behavior across processes.
    if publication_trace_enabled() {
        eprintln!("PUBLISH kind={}", collision.artifact_class());
    }
    crate::durability_counters::note_immutable_publication();
    tine_storage::publish_immutable_exact_single_writer(dir, filename, bytes)
        .map_err(|error| publication_error(error, collision))
}

/// One accepted batch's archive artifacts, published under either the strict
/// or journal-turn-covered protocol.
///
/// ## Why this exists
///
/// The per-artifact publisher (`tine_storage::publish_immutable_exact*`) pays
/// two device round trips — the file's `fsync` and its directory's `fsync` —
/// for every artifact. An ordinary one-block managed edit produces four to
/// eight archive artifacts, a cross-page move roughly eight, so artifact-local
/// durability alone cost ten to sixteen barriers per accepted edit
/// (2026-08-26 managed-storage cost-model audit, defect D1).
///
/// ## The protocol, and why each step is where it is
///
/// 1. **Stage.** Every artifact is written to a temporary name in its own
///    namespace with no barrier. Temporary names are invisible to every
///    reader: `ObjectStore::validate_namespace` and every replay path address
///    artifacts by their content-addressed or batch-addressed final names.
/// 2. **Object install for a journal-covered turn.** Its object final names are
///    inserted no-replace while the exact journal frame remains undrained.
///    Strict callers skip this step.
/// 3. **One data barrier.** `syncfs` flushes every staged inode. Strict callers
///    take it before any final-name install; a journal-covered turn takes it
///    after object install and before manifest install.
/// 4. **Remaining installs.** Strict callers insert every final name; the
///    journal-covered turn inserts its now-durable manifest commit marker.
/// 5. **Directory barriers.** One `fsync` per distinct namespace makes the
///    name insertions durable — two for an ordinary batch (objects, batches).
///
/// For a strict caller, the data barrier before install ensures no visible name
/// can refer to unflushed bytes. For the managed-local drain, a crash during
/// step 2 may leave a torn object final name; cold open replaces it only when
/// an uncheckpointed local journal record supplies the exact canonical bytes.
/// After step 3 every object is durable, so publication may return and the
/// caller may eventually checkpoint without losing repair authority too early.
/// The manifest is never installed before its data barrier.
///
/// ## Crash points
///
/// | crash at | on disk | recovery |
/// |---|---|---|
/// | during staging | temporary names only, contents arbitrary | the batch is not accepted; the journal frame is undrained and the drain republishes. Temporaries are ignored by every reader. |
/// | after staging, before an install | temporary names only | as above |
/// | during journal-covered object installs, before the barrier | a prefix of possibly torn object names; no manifest name | cold open repairs only exact journal-covered object mismatches before validation |
/// | after the barrier, during remaining installs | every surviving final name has durable correct bytes | the drain republishes and verifies exact existing names |
/// | after installs, before a directory barrier | possibly no final name durable after reboot; any surviving name has durable correct bytes | the drain republishes |
/// | after the directory barriers | the whole batch durable | the drain proceeds to checkpoint |
///
/// In every row the accepted operation itself is unaffected: it became durable
/// in the local journal during the foreground save, before this runs, and its
/// journal frame is checkpointed only after this returns.
///
/// On platforms without `syncfs` (Windows, macOS), `stage` publishes each
/// artifact through the ordinary durable publisher and `commit` is inert. The
/// barrier count there is unchanged.
struct ArchiveBatchPublication {
    archive: Dir,
    namespaces: Vec<Dir>,
    strict_filesystem_barrier: bool,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    staged: Vec<StagedArchiveArtifact>,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
struct StagedArchiveArtifact {
    namespace: usize,
    temp_name: String,
    final_name: String,
    limit: u64,
    requires_preinstall_flush: bool,
}

impl ArchiveBatchPublication {
    fn strict(archive: &Dir) -> Result<Self, StoreError> {
        Self::new(archive, true)
    }

    fn turn_covered(archive: &Dir) -> Result<Self, StoreError> {
        Self::new(archive, false)
    }

    fn new(archive: &Dir, strict_filesystem_barrier: bool) -> Result<Self, StoreError> {
        Ok(Self {
            archive: archive.try_clone()?,
            namespaces: Vec::new(),
            strict_filesystem_barrier,
            #[cfg(any(target_os = "linux", target_os = "android"))]
            staged: Vec::new(),
        })
    }

    /// Retain one namespace capability for the batch and return its index.
    fn namespace(&mut self, dir: &Dir) -> Result<usize, StoreError> {
        self.namespaces.push(dir.try_clone()?);
        Ok(self.namespaces.len() - 1)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn stage(
        &mut self,
        namespace: usize,
        final_name: &str,
        bytes: &[u8],
        limit: u64,
        _collision: Collision,
        requires_preinstall_flush: bool,
    ) -> Result<(), StoreError> {
        let dir = &self.namespaces[namespace];
        let temp_name = format!(".tmp-{}", Uuid::new_v4());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let mut temp = dir.open_with(&temp_name, &options)?;
        if let Err(error) = temp.write_all(bytes) {
            drop(temp);
            let _ = dir.remove_file(&temp_name);
            return Err(error.into());
        }
        drop(temp);
        self.staged.push(StagedArchiveArtifact {
            namespace,
            temp_name,
            final_name: final_name.to_owned(),
            limit,
            requires_preinstall_flush,
        });
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn stage(
        &mut self,
        namespace: usize,
        final_name: &str,
        bytes: &[u8],
        _limit: u64,
        collision: Collision,
        _requires_preinstall_flush: bool,
    ) -> Result<(), StoreError> {
        publish_immutable(&self.namespaces[namespace], final_name, bytes, collision)
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn commit(self) -> Result<(), StoreError> {
        // `Drop` removes the temporaries whether this succeeds or not, so an
        // abandoned batch cannot leak names into the archive namespaces.
        self.commit_staged()
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn commit_staged(&self) -> Result<(), StoreError> {
        if self.staged.is_empty() {
            return Ok(());
        }
        if self.strict_filesystem_barrier {
            self.barrier_staged_data()?;
            self.install_staged_where(|_| true)?;
        } else {
            // The undrained journal is exact recovery authority while object
            // names install. Flush the whole staged set only after those names
            // exist, then install the now-durable manifest commit marker. The
            // journal checkpoint cannot advance until this method returns, so
            // every crash before the barrier remains repairable and every crash
            // after it has durable object bytes.
            self.install_staged_where(|artifact| !artifact.requires_preinstall_flush)?;
            self.barrier_staged_data()?;
            self.install_staged_where(|artifact| artifact.requires_preinstall_flush)?;
        }
        let mut barriered: Vec<usize> = Vec::new();
        for artifact in &self.staged {
            if barriered.contains(&artifact.namespace) {
                continue;
            }
            barriered.push(artifact.namespace);
            sync_dir_required(&self.namespaces[artifact.namespace])?;
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn install_staged_where(
        &self,
        predicate: impl Fn(&StagedArchiveArtifact) -> bool,
    ) -> Result<(), StoreError> {
        for artifact in self.staged.iter().filter(|artifact| predicate(artifact)) {
            install_staged_artifact(
                &self.namespaces[artifact.namespace],
                &artifact.temp_name,
                &artifact.final_name,
                artifact.limit,
            )?;
            archive_install_cut_hook()?;
        }
        Ok(())
    }

    /// Flush every staged inode. Strict callers take this before all installs;
    /// a journal-covered caller takes it after object installs but before the
    /// manifest install and before the journal checkpoint can advance.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn barrier_staged_data(&self) -> Result<(), StoreError> {
        let result = crate::filesystem_durability::sync_filesystem_containing(&self.archive);
        #[cfg(target_os = "android")]
        {
            return self.finish_android_staged_data_barrier(result);
        }
        #[cfg(not(target_os = "android"))]
        {
            result.map_err(Into::into)
        }
    }

    /// Android app-private storage can deny `syncfs` while still supporting
    /// exact file durability. Keep the branch host-executable so its selection
    /// and exact staged-file fallback are unit-tested rather than merely
    /// cross-compiled.
    #[cfg(any(target_os = "android", all(test, target_os = "linux")))]
    fn finish_android_staged_data_barrier(&self, result: io::Result<()>) -> Result<(), StoreError> {
        match result {
            Ok(()) => Ok(()),
            // Android app-private storage on some vendor filesystems denies the
            // filesystem-wide flush while permitting per-file flush. Falling
            // back to one `fsync` per staged artifact costs the barriers this
            // batch exists to avoid, but it is a correct durability point and
            // it keeps managed storage available on those devices. Every other
            // errno is a real I/O failure and stays fatal.
            Err(error) if crate::filesystem_durability::is_capability_refusal(&error) => {
                for artifact in &self.staged {
                    let name = match self.namespaces[artifact.namespace]
                        .symlink_metadata(&artifact.temp_name)
                    {
                        Ok(_) => artifact.temp_name.as_str(),
                        Err(error) if error.kind() == ErrorKind::NotFound => {
                            artifact.final_name.as_str()
                        }
                        Err(error) => return Err(error.into()),
                    };
                    let file = tine_storage::open_file_nofollow(
                        &self.namespaces[artifact.namespace],
                        name,
                    )?;
                    crate::durability_counters::sync_file(&file)?;
                }
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn commit(self) -> Result<(), StoreError> {
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl Drop for ArchiveBatchPublication {
    /// Remove every temporary this batch created, on the committed and the
    /// abandoned path alike. A batch abandoned mid-stage (a validation error,
    /// or the deterministic cut a crash test arms) must not leave temporaries
    /// behind: they are invisible to readers, but they are still bytes in the
    /// user's archive.
    fn drop(&mut self) {
        for artifact in &self.staged {
            let _ = self.namespaces[artifact.namespace].remove_file(&artifact.temp_name);
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
mod archive_batch_android_fallback_tests {
    use super::*;
    use crate::durability_counters::{Barrier, BarrierSession};

    #[test]
    fn android_group_commit_capability_refusal_flushes_every_exact_staged_file() {
        let root =
            std::env::temp_dir().join(format!("tine-android-archive-fallback-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("objects")).unwrap();
        let archive = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
        let objects = open_dir_nofollow(&archive, "objects").unwrap();
        let mut publication = ArchiveBatchPublication::strict(&archive).unwrap();
        let namespace = publication.namespace(&objects).unwrap();
        publication
            .stage(
                namespace,
                "first",
                b"first exact staged object",
                1024,
                Collision::Exact("test object"),
                false,
            )
            .unwrap();
        publication
            .stage(
                namespace,
                "second",
                b"second exact staged object",
                1024,
                Collision::Exact("test object"),
                false,
            )
            .unwrap();

        let barriers = BarrierSession::begin();
        publication
            .finish_android_staged_data_barrier(Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "simulated Android syncfs capability refusal",
            )))
            .unwrap();
        assert_eq!(barriers.counts().get(Barrier::File), 2);
        BarrierSession::detach_current_thread();

        assert!(objects.symlink_metadata("first").is_err());
        assert!(objects.symlink_metadata("second").is_err());
        drop(publication);
        drop(objects);
        drop(archive);
        crate::test_support::remove_dir_all(root);
    }

    #[test]
    fn android_group_commit_does_not_hide_a_real_io_failure() {
        let root =
            std::env::temp_dir().join(format!("tine-android-archive-real-io-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let archive = Dir::open_ambient_dir(&root, ambient_authority()).unwrap();
        let publication = ArchiveBatchPublication::strict(&archive).unwrap();
        let error = publication
            .finish_android_staged_data_barrier(Err(io::Error::new(
                ErrorKind::WriteZero,
                "simulated media failure",
            )))
            .unwrap_err();
        assert!(matches!(error, StoreError::Io(error) if error.kind() == ErrorKind::WriteZero));
        drop(publication);
        drop(archive);
        crate::test_support::remove_dir_all(root);
    }
}

/// Insert one already-durable staged artifact under its final immutable name.
///
/// No-replace, because an immutable name is authored once. A name that is
/// already present is not an error: the drain republishes the byte-identical
/// batch after any crash, so "already there with exactly these bytes" is the
/// expected outcome of resuming. Anything else is a genuine collision.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn install_staged_artifact(
    dir: &Dir,
    temp_name: &str,
    final_name: &str,
    limit: u64,
) -> Result<(), StoreError> {
    match dir.hard_link(temp_name, dir, final_name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let staged = tine_storage::read_required_regular(dir, temp_name, limit, None)
                .map_err(filesystem_error_without_collision)?;
            let existing = tine_storage::read_required_regular(dir, final_name, limit, None)
                .map_err(filesystem_error_without_collision)?;
            if existing == staged {
                Ok(())
            } else {
                Err(StoreError::ImmutableCollision("archive batch publication"))
            }
        }
        // Some Android app-private filesystems permit atomic same-directory
        // renames but deny the hard-link primitive. The archive is a
        // single-writer namespace, so proving the name absent and renaming is
        // equivalent there. This mirrors
        // `tine_storage::publish_immutable_exact_single_writer`.
        #[cfg(target_os = "android")]
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::PermissionDenied | ErrorKind::Unsupported | ErrorKind::InvalidInput
            ) =>
        {
            match dir.symlink_metadata(final_name) {
                Ok(_) => {
                    let staged = tine_storage::read_required_regular(dir, temp_name, limit, None)
                        .map_err(filesystem_error_without_collision)?;
                    let existing =
                        tine_storage::read_required_regular(dir, final_name, limit, None)
                            .map_err(filesystem_error_without_collision)?;
                    if existing == staged {
                        Ok(())
                    } else {
                        Err(StoreError::ImmutableCollision("archive batch publication"))
                    }
                }
                Err(absent) if absent.kind() == ErrorKind::NotFound => {
                    dir.rename(temp_name, dir, final_name)?;
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn publication_stage_error(stage: &'static str, error: StoreError) -> StoreError {
    match error {
        StoreError::Io(error) => StoreError::Io(std::io::Error::new(
            error.kind(),
            format!("{stage}: {error}"),
        )),
        error => error,
    }
}

pub(crate) fn publish_immutable_exact(
    dir: &Dir,
    filename: &str,
    bytes: &[u8],
    kind: &'static str,
) -> Result<(), StoreError> {
    publish_immutable(dir, filename, bytes, Collision::Exact(kind))
}

fn publication_error(error: tine_storage::FilesystemError, collision: Collision) -> StoreError {
    match error {
        tine_storage::FilesystemError::ByteCollision => collision_error(collision),
        error => filesystem_error_without_collision(error),
    }
}

pub(crate) fn filesystem_error_without_collision(
    error: tine_storage::FilesystemError,
) -> StoreError {
    match error {
        tine_storage::FilesystemError::Io(error) => StoreError::Io(error),
        tine_storage::FilesystemError::DurableNameOperationUnavailable(message) => {
            StoreError::Io(std::io::Error::new(ErrorKind::Unsupported, message))
        }
        tine_storage::FilesystemError::UnsafeEntry(message) => StoreError::UnsafeEntry(message),
        tine_storage::FilesystemError::StoredLengthMismatch {
            path,
            expected,
            actual,
        } => StoreError::StoredLengthMismatch {
            path,
            expected,
            actual,
        },
        tine_storage::FilesystemError::StoredFileTooLarge {
            path,
            length,
            limit,
        } => StoreError::StoredFileTooLarge {
            path,
            length,
            limit,
        },
        tine_storage::FilesystemError::ByteCollision => {
            StoreError::ImmutableCollision("immutable publication")
        }
    }
}

fn collision_error(collision: Collision) -> StoreError {
    match collision {
        Collision::Object(digest) => StoreError::ObjectCollision(digest),
        Collision::Batch(batch_id) => StoreError::BatchCollision(batch_id),
        Collision::Lineage(lineage) => StoreError::LineageClaimCollision(lineage),
        Collision::Exact(kind) => StoreError::ImmutableCollision(kind),
    }
}

pub(crate) fn open_dir_nofollow(dir: &Dir, path: &str) -> Result<Dir, StoreError> {
    tine_storage::open_dir_nofollow(dir, path).map_err(filesystem_error_without_collision)
    // SAFETY: `openat` returned a newly owned directory descriptor.
}

pub(crate) fn read_optional_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Option<Vec<u8>>, StoreError> {
    tine_storage::read_optional_regular(dir, path, limit, expected_length)
        .map_err(filesystem_error_without_collision)
}

fn read_required_regular(
    dir: &Dir,
    path: &str,
    limit: u64,
    expected_length: Option<u64>,
) -> Result<Vec<u8>, StoreError> {
    tine_storage::read_required_regular(dir, path, limit, expected_length)
        .map_err(filesystem_error_without_collision)
}

fn object_filename(digest: ContentDigest) -> String {
    format!("{digest}.object")
}

fn manifest_filename(batch_id: BatchId) -> String {
    format!("{batch_id}.manifest")
}

fn parse_object_filename(name: &str) -> Result<ContentDigest, StoreError> {
    let Some(digest) = name.strip_suffix(".object") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    if digest.len() != 64
        || digest
            .bytes()
            .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(StoreError::MalformedPath(name.into()));
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in digest.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]).expect("validated hex") << 4)
            | hex_nibble(pair[1]).expect("validated hex");
    }
    Ok(ContentDigest::from_bytes(bytes))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_manifest_filename(name: &str) -> Result<BatchId, StoreError> {
    let Some(batch_id) = name.strip_suffix(".manifest") else {
        return Err(StoreError::MalformedPath(name.into()));
    };
    let parsed = batch_id
        .parse::<BatchId>()
        .map_err(|_| StoreError::MalformedPath(name.into()))?;
    if parsed.to_string() != batch_id {
        return Err(StoreError::MalformedPath(name.into()));
    }
    Ok(parsed)
}

pub(crate) fn is_temp_name(name: &str) -> bool {
    name.strip_prefix(".tmp-")
        .and_then(|value| Uuid::parse_str(value).ok())
        .is_some()
}

pub(crate) fn require_regular_entry(
    file_type: &cap_std::fs::FileType,
    name: &str,
) -> Result<(), StoreError> {
    tine_storage::require_regular_entry(file_type, name).map_err(filesystem_error_without_collision)
}

pub(crate) fn sync_dir_required(dir: &Dir) -> Result<(), StoreError> {
    crate::durability_counters::note(crate::durability_counters::Barrier::Directory);
    tine_storage::sync_dir_required(dir).map_err(Into::into)
}
