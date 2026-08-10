use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead as _, Write as _};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::*;
use crate::model::Graph;
use crate::oplog::authenticated_patricia::{
    fail_next_patricia_reclamation_for_test, PatriciaReclamationFailureForTest,
};
use crate::oplog::enrollment::{
    compose_verified_local, enrollment_application_root_for_test, fail_next_enrollment_head_read,
    CommitCut, EnrollmentOpen, EnrollmentReader, PreparationId,
};
use crate::oplog::hot_engine::{
    fail_next_ephemeral_bootstrap_predecessor_restore_for_test, take_last_admitted_local_author,
    AcceptedFrontierRoot, AuthenticatedPatriciaIndexKind, PackedPatriciaMaintenanceOutcome,
    ProjectionEndpointBinding, ProjectionStorageBinding, MAX_EPHEMERAL_BLOCK_CLAIMS,
};
use crate::oplog::identity::ARCHIVE_INSTANCE_CLAIM_FILE;
use crate::oplog::import::{
    force_next_bootstrap_part_operation_limit, prepare_inactive_bootstrap_import,
    publish_install_verify_inactive_bootstrap, reopen_inactive_bootstrap_accepted_authority,
    InactiveBootstrapAcceptedAuthority, InactiveBootstrapPreparedPublication,
    InactiveBootstrapVerifiedPublication,
};
use crate::oplog::migration_backup::{
    verify_migration_source_backup, MigrationBackupRoot, VerifiedSourceBackup,
};
use crate::oplog::object_store::{
    fail_next_resume_clear, fail_next_resume_publication_at, ResumePublishBoundary,
    RetainedRunMaintenanceOutcome,
};
use crate::oplog::operational_coordinator::{
    act_once_at, fail_once_at, LocalMutationBlockReason, LocalMutationCoordinatorState,
    LocalMutationRecovery, OperationalCoordinator, OperationalCoordinatorError,
    OperationalCoordinatorState, OperationalFaultPoint, OperationalPhase, RetainedBlockReason,
};
use crate::oplog::projection::write_projection_exact;
use crate::oplog::reconciliation_baseline::{
    BaselineTimestamp, ReconciliationBaseline, ReconciliationBaselineBinding,
    TrustedPrivateApplicationRuntimeRoot,
};
use crate::oplog::reconciliation_scan::{
    scan_graph_text, take_bootstrap_page_materializations_for_test, GraphTextCandidateKind,
    GraphTextScanLimits, JoinedAuthenticatedExpectedPathSource, ReconciliationSchedulerLimits,
    ReconciliationTrigger,
};
use crate::oplog::reconciliation_session::{
    ReconciliationSession, ReconciliationSessionDependencies, ReconciliationSessionStep,
};
use crate::oplog::shadow_projection::{
    verify_inactive_bootstrap_shadow_projection, VerifiedShadowProjection,
};
use crate::oplog::sqlite::{
    fail_next_workspace_lease_identity_check, ApplicationRuntimeRoot, ProjectionError,
    RebuildSource, SqliteFrontier, TailOverlay, WorkspaceRuntimeLease,
};
use crate::oplog::watcher_queue::WatcherObservation;
use crate::oplog::{
    execute_manifested_projection_work, AuthorBatch, BatchDisposition, BatchId, BatchOrigin,
    BlockId, BlockLocation, CanonicalArchiveResourceId, CrdtPeerId, DeviceId, DocumentId,
    LineageDigest, LogicalPageName, ManagedPath, ManagedTextKind, ObjectStore,
    OperationTransaction, PageId, ProjectionClaim, ProjectionEndpointId, ProjectionReceiptStore,
    ReferenceCatalogPolicyV1, SemanticOperation, ShardedHotEngine, WorkspaceId,
};

#[test]
fn promoted_recovery_debug_classifications_are_bounded_and_content_free() {
    let retained = EngineScratchRetentionPlan::Retained { retained_runs: 2 };
    assert_eq!(retention_plan_diagnostic(&retained), ("retained", 2));
    assert_eq!(resume_candidate_diagnostic(&retained, None), "not_read");
    assert_eq!(
        resume_candidate_diagnostic(
            &retained,
            Some(&ResumeAdoptionCandidate::Unavailable(
                ResumeAcceleratorUnavailable::NeverPublished,
            )),
        ),
        "never_published"
    );

    let ephemeral = EngineScratchRetentionPlan::Ephemeral {
        retained_runs: 3,
        reason: crate::oplog::resume_point::ResumePointError::TooManyPoints(3),
    };
    assert_eq!(retention_plan_diagnostic(&ephemeral), ("ephemeral", 3));
    assert_eq!(
        resume_candidate_diagnostic(&ephemeral, None),
        "not_read_ephemeral"
    );
    assert_eq!(
        recovery_diagnostic_class(RuntimeRecoveryState::TookOverCrashedUnsafe {
            previous_session: SessionId::new(),
        }),
        "crash_takeover"
    );
}

#[test]
fn activation_uses_the_enrollment_transitions_post_commit_reopen_without_repeating_it() {
    let source = include_str!("../local_active.rs");
    let start = source.find("fn activate_with_optional_cut(").unwrap();
    let end = source[start..]
        .find("/// The sole fresh-process")
        .map(|offset| start + offset)
        .unwrap();
    let activation = &source[start..end];
    assert!(
        !activation.contains("reopen_local_active_record("),
        "activate_verified_local_record already returns a fresh post-commit reopen; repeating it is graph-sized duplicate construction"
    );
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tine-local-active-{label}-{}", Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One complete inactive enrollment: real capture, publication, backup, SQLite
/// bootstrap, shadow projection, and receipt namespace over one real graph.
struct Fixture {
    root: TestRoot,
    graph_root: PathBuf,
    graph: Graph,
    receipts: ProjectionReceiptStore,
    archive_root: PathBuf,
    workspace: WorkspaceId,
    lineage: LineageDigest,
    catalog_document_id: DocumentId,
    prepared: InactiveBootstrapPreparedPublication,
    verified: InactiveBootstrapVerifiedPublication,
    authority: InactiveBootstrapAcceptedAuthority,
    roots: MigrationBackupRoot,
    backup: VerifiedSourceBackup,
    /// The production inactive-bootstrap session: the database *and* the
    /// archive-rooted workspace runtime lease it was opened under, as one
    /// value. Phase two of promotion is reachable only through it, so the
    /// bootstrap -> promoted handoff cannot release the workspace lock.
    sqlite: Option<InactiveBootstrapRuntimeSession>,
    archive_resource_id: CanonicalArchiveResourceId,
    shadow: VerifiedShadowProjection,
    preparation: PreparationId,
    original_graph: BTreeMap<String, Vec<u8>>,
}

impl Fixture {
    fn new(label: &str, config: Option<&[u8]>, files: Vec<(String, Vec<u8>)>) -> Self {
        let root = TestRoot::new(label);
        let graph_root = root.path().join("graph");
        fs::create_dir(&graph_root).unwrap();
        if let Some(config) = config {
            fs::create_dir(graph_root.join("logseq")).unwrap();
            fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
        }
        for (path, bytes) in &files {
            let destination = graph_root.join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        let original_graph = snapshot_files(&graph_root);
        let graph = Graph::open(&graph_root);

        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x9100));
        let lineage = LineageDigest::of(b"local-active-activation-test");
        let catalog_document_id = DocumentId::from_uuid(Uuid::from_u128(0x9101));

        // A real receipt namespace supplies the enrolled endpoint and receipt
        // store identity, so the enrollment binding is never synthetic.
        let receipt_root = root.path().join("receipts");
        fs::create_dir(&receipt_root).unwrap();
        let endpoint = ProjectionEndpointBinding::enroll_graph(
            &graph,
            ProjectionEndpointId::from_uuid(Uuid::from_u128(0x9102)),
            DeviceId::from_uuid(Uuid::from_u128(0x9103)),
        )
        .unwrap();
        let receipts =
            ProjectionReceiptStore::open_for_endpoint(&receipt_root, workspace, endpoint).unwrap();

        let capture_root = root.path().join("capture");
        let preparation_root = root.path().join("preparation");
        fs::create_dir(&capture_root).unwrap();
        fs::create_dir(&preparation_root).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_root)
            .unwrap_or_else(|error| {
                panic!(
                    "bootstrap source capture failed during `{}`: {error:?}",
                    crate::model::bootstrap_source_io_stage_for_test()
                )
            });
        // The bootstrap is authored for exactly this archive: its accepted cold
        // records bind reference-catalog roots that live in this archive's
        // durable authenticated store.
        let archive_root = root.path().join("archive");
        let prepared = prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            lineage,
            catalog_document_id,
            ReferenceCatalogPolicyV1::default(),
            &ObjectStore::open(&archive_root, workspace)
                .unwrap()
                .bootstrap_authoring_capability()
                .unwrap(),
            &preparation_root,
        )
        .unwrap();
        let storage_binding = ProjectionStorageBinding {
            endpoint,
            receipt_store_id: receipts.store_id(),
        };
        let verified = publish_install_verify_inactive_bootstrap(
            &prepared,
            ObjectStore::open(&archive_root, workspace).unwrap(),
            storage_binding,
        )
        .unwrap();
        let authority = reopen_inactive_bootstrap_accepted_authority(
            &verified,
            ObjectStore::open(&archive_root, workspace).unwrap(),
        )
        .unwrap();

        let device_root = root.path().join("device-local");
        fs::create_dir(&device_root).unwrap();
        let roots = MigrationBackupRoot::open(&device_root, &graph_root).unwrap();
        let backup = verify_migration_source_backup(&roots, &prepared, &verified).unwrap();
        let runtime = ApplicationRuntimeRoot::open_for_test(&root.path().join("runtime")).unwrap();
        let sqlite = InactiveBootstrapRuntimeSession::open(
            &archive_root,
            workspace,
            &root.path().join("bootstrap.sqlite"),
            &runtime,
            &authority,
            None,
        )
        .expect("inactive bootstrap runtime session");
        let archive_resource_id = authority
            .store()
            .provision_enrolled_archive_resource_id()
            .unwrap();
        let shadow = verify_inactive_bootstrap_shadow_projection(
            &graph,
            &roots,
            &prepared,
            &verified,
            &backup,
            &authority,
            sqlite.sqlite_proof(),
        )
        .unwrap();

        Self {
            root,
            graph_root,
            graph,
            receipts,
            archive_root,
            workspace,
            lineage,
            catalog_document_id,
            prepared,
            verified,
            authority,
            roots,
            backup,
            sqlite: Some(sqlite),
            archive_resource_id,
            shadow,
            preparation: PreparationId::new(),
            original_graph,
        }
    }

    fn bootstrap(&self) -> &InactiveBootstrapRuntimeSession {
        self.sqlite
            .as_ref()
            .expect("retained inactive bootstrap projection")
    }

    fn sqlite(&self) -> &OpenProjection {
        self.bootstrap().projection()
    }

    /// Reopen the production inactive-bootstrap session under a lease this
    /// process is still holding — the retry path after a refused promotion,
    /// which must not go through releasing the archive either.
    fn reopen_bootstrap_session(
        &self,
        lease: WorkspaceRuntimeLease,
    ) -> InactiveBootstrapRuntimeSession {
        let runtime =
            ApplicationRuntimeRoot::open_for_test(&self.root.path().join("runtime")).unwrap();
        InactiveBootstrapRuntimeSession::reopen_under(
            lease,
            &self.root.path().join("bootstrap.sqlite"),
            &runtime,
            &self.authority,
        )
        .map_err(|(_returned_lease, error)| error)
        .expect("reopened inactive bootstrap session")
    }

    /// Drop the retained inactive bootstrap session *and* its archive-rooted
    /// workspace lease, so a promoted open must acquire the workspace lease
    /// itself.
    fn release_bootstrap_projection(&mut self) {
        self.sqlite = None;
    }

    /// Take the production inactive-bootstrap session out of the fixture.
    ///
    /// Phase two of promotion runs on the session's own retained lease, so the
    /// workspace lock is never released between the two databases, and a
    /// refusal returns that exact lease.
    fn take_bootstrap_session(&mut self) -> InactiveBootstrapRuntimeSession {
        self.sqlite
            .take()
            .expect("retained inactive bootstrap projection")
    }

    fn proofs(&self) -> VerifiedLocalProofSet<'_> {
        VerifiedLocalProofSet {
            graph: &self.graph,
            roots: &self.roots,
            prepared: &self.prepared,
            verified_publication: &self.verified,
            source_backup: &self.backup,
            accepted_authority: &self.authority,
            sqlite: self.sqlite(),
            sqlite_projection: self.bootstrap().sqlite_proof(),
            shadow_projection: &self.shadow,
        }
    }

    fn enrollment_binding(&self) -> EnrollmentBindingV1 {
        let accepted = self.authority.binding();
        let storage = accepted.storage_binding();
        EnrollmentBindingV1::new(
            accepted.workspace_id(),
            accepted.lineage_digest(),
            self.verified.catalog_document_id(),
            storage.endpoint.endpoint_id(),
            storage.endpoint.device_id(),
            accepted.graph_resource(),
            storage.receipt_store_id,
            self.archive_resource_id,
            self.graph.graph_text_scope_binding().unwrap(),
        )
        .unwrap()
    }

    fn enrollment_root(&self, label: &str) -> EnrollmentApplicationRoot {
        enrollment_application_root_for_test(
            &self
                .root
                .path()
                .join(format!("enrollment-{}-{label}", Uuid::new_v4())),
        )
        .unwrap()
    }

    fn runtime(&self) -> LocalActiveRuntime<'_> {
        LocalActiveRuntime {
            engine: self.authority.accepted_engine(),
            projection: self.sqlite(),
        }
    }

    fn archive(&self) -> ObjectStore {
        ObjectStore::open(&self.archive_root, self.workspace).unwrap()
    }

    /// A live ordinary runtime engine enrolled to the exact endpoint, receipt
    /// store, workspace, lineage, and catalog document this enrollment binds.
    ///
    /// It is opened over a separate ordinary archive root on purpose: an
    /// inactive-bootstrap archive is explicitly fenced from ordinary runtime
    /// opening ("inactive bootstrap history cannot be opened as ordinary
    /// runtime"), and promoting it is a later packet. The gate under test here
    /// is the runtime identity/enrollment binding, which this engine reproduces
    /// exactly.
    fn runtime_engine(&self, label: &str) -> ShardedHotEngine {
        let archive_root = self.root.path().join(format!("runtime-archive-{label}"));
        ShardedHotEngine::with_enrolled_projection(
            ObjectStore::open(&archive_root, self.workspace).unwrap(),
            self.lineage,
            self.catalog_document_id,
            &self.graph,
            &self.receipts,
        )
    }

    /// A device-local SQLite projection bound to one exact live runtime engine.
    fn runtime_projection(
        &self,
        engine: &ShardedHotEngine,
        archive: &ObjectStore,
        label: &str,
    ) -> SqliteFrontier {
        let runtime =
            ApplicationRuntimeRoot::open_for_test(&self.root.path().join(format!("rt-{label}")))
                .unwrap();
        SqliteFrontier::open_or_rebuild(
            &self.root.path().join(format!("rt-{label}.sqlite")),
            &runtime,
            ProjectionClaim::current(self.workspace, self.lineage),
            RebuildSource::new(engine, archive).unwrap(),
        )
        .unwrap()
        .database
    }

    /// A fresh device-local reconciliation baseline bound to this exact
    /// enrolled workspace, endpoint, graph resource, and graph-text scope.
    fn reconciliation_baseline(&self, label: &str) -> ReconciliationBaseline {
        let runtime = ApplicationRuntimeRoot::open_for_test(
            &self.root.path().join(format!("baseline-rt-{label}")),
        )
        .unwrap();
        let binding = ReconciliationBaselineBinding::new(
            self.workspace,
            self.authority
                .binding()
                .storage_binding()
                .endpoint
                .endpoint_id(),
            self.graph.canonical_resource_id().unwrap(),
            self.graph.graph_text_scope_binding().unwrap(),
        )
        .unwrap();
        ReconciliationBaseline::create_fresh(
            &TrustedPrivateApplicationRuntimeRoot::from_application_runtime_root(&runtime),
            binding,
        )
        .unwrap()
    }

    fn compose(&self, root: &EnrollmentApplicationRoot) -> VerifiedLocalEvidence {
        compose_verified_local(
            root,
            self.enrollment_binding(),
            self.preparation,
            &self.proofs(),
        )
        .unwrap()
    }

    fn assert_graph_unchanged(&self) {
        assert_eq!(snapshot_files(&self.graph_root), self.original_graph);
    }
}

fn snapshot_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut output = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries = fs::read_dir(&directory)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if fs::symlink_metadata(&path).unwrap().is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                output.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    output
}

/// Byte identity of a directory, reported compactly so a failure prints
/// digests instead of whole databases.
fn snapshot_file_digests(root: &Path) -> BTreeMap<String, ContentDigest> {
    snapshot_files(root)
        .into_iter()
        .map(|(path, bytes)| (path, ContentDigest::of(&bytes)))
        .collect()
}

/// Hash only entries selected before their contents are opened.
///
/// The predicate receives the normalized relative path and whether the entry
/// is a directory. Returning false for a directory prunes the whole subtree.
fn snapshot_file_digests_matching_with_reader(
    root: &Path,
    include: impl Fn(&str, bool) -> bool,
    mut read_file: impl FnMut(&Path) -> Vec<u8>,
) -> BTreeMap<String, ContentDigest> {
    let mut output = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries = fs::read_dir(&directory)
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = fs::symlink_metadata(&path).unwrap();
            let is_directory = metadata.is_dir();
            if !include(&relative, is_directory) {
                continue;
            }
            if is_directory {
                stack.push(path);
            } else {
                output.insert(relative, ContentDigest::of(&read_file(&path)));
            }
        }
    }
    output
}

fn snapshot_file_digests_matching(
    root: &Path,
    include: impl Fn(&str, bool) -> bool,
) -> BTreeMap<String, ContentDigest> {
    snapshot_file_digests_matching_with_reader(root, include, |path| {
        fs::read(path).unwrap_or_else(|error| {
            let relative = path.strip_prefix(root).unwrap_or(path);
            panic!(
                "failed to read snapshot file `{}` beneath `{}`: {error:?}",
                relative.display(),
                root.display()
            )
        })
    })
}

fn in_top_level_namespace(path: &str, namespaces: &[&str]) -> bool {
    path.split('/')
        .next()
        .is_some_and(|component| namespaces.contains(&component))
}

fn is_enrollment_lease_path(path: &str) -> bool {
    let mut components = path.split('/');
    matches!(
        (
            components.next(),
            components.next(),
            components.next(),
            components.next(),
            components.next(),
            components.next(),
            components.next(),
        ),
        (
            Some("sparse-storage"),
            Some("v2"),
            Some("local"),
            Some(_),
            Some("enrollment"),
            Some("lease"),
            None
        )
    )
}

/// Durable byte identity of a SQLite database directory.
///
/// The `-shm` sidecar is a volatile shared-memory index that ordinary read
/// transactions legitimately update, so durable identity covers the database
/// file and its write-ahead log, where every committed row actually lands.
fn durable_sqlite_digests(directory: &Path) -> BTreeMap<String, ContentDigest> {
    snapshot_file_digests_matching(directory, |name, is_directory| {
        is_directory || !name.ends_with("-shm")
    })
}

#[test]
fn authoritative_snapshot_prunes_lock_namespaces_before_reading_files() {
    let root = TestRoot::new("authoritative-snapshot-prunes-runtime");
    fs::create_dir(root.path().join(".tine-runtime")).unwrap();
    fs::create_dir(root.path().join("engine-scratch-v2")).unwrap();
    fs::create_dir(root.path().join("batches")).unwrap();
    fs::create_dir_all(root.path().join("sparse-storage/v2/local/graph/enrollment")).unwrap();
    fs::write(root.path().join(".tine-runtime/locked"), b"live lease").unwrap();
    fs::write(root.path().join("engine-scratch-v2/run"), b"scratch").unwrap();
    fs::write(root.path().join("batches/authoritative"), b"durable").unwrap();
    fs::write(root.path().join("head"), b"authoritative head").unwrap();
    fs::write(
        root.path()
            .join("sparse-storage/v2/local/graph/enrollment/lease"),
        b"locked enrollment lease",
    )
    .unwrap();
    fs::write(
        root.path()
            .join("sparse-storage/v2/local/graph/enrollment/head"),
        b"authenticated enrollment head",
    )
    .unwrap();

    let mut opened = Vec::new();
    let snapshot = snapshot_file_digests_matching_with_reader(
        root.path(),
        |path, _| {
            !in_top_level_namespace(path, &[".tine-runtime", "engine-scratch-v2"])
                && !is_enrollment_lease_path(path)
        },
        |path| {
            let relative = path
                .strip_prefix(root.path())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            assert!(
                !in_top_level_namespace(&relative, &[".tine-runtime", "engine-scratch-v2"])
                    && !is_enrollment_lease_path(&relative),
                "an excluded runtime file must never reach the reader: {relative}"
            );
            opened.push(relative);
            fs::read(path).unwrap()
        },
    );

    assert_eq!(
        snapshot.keys().cloned().collect::<Vec<_>>(),
        vec![
            "batches/authoritative".to_owned(),
            "head".to_owned(),
            "sparse-storage/v2/local/graph/enrollment/head".to_owned()
        ]
    );
    opened.sort();
    assert_eq!(
        opened,
        vec![
            "batches/authoritative".to_owned(),
            "head".to_owned(),
            "sparse-storage/v2/local/graph/enrollment/head".to_owned()
        ]
    );
}

/// Nested, non-standard, Unicode, CRLF, BOM, and multi-chunk graph layout.
fn rich_fixture(label: &str) -> Fixture {
    let mut deep = String::from("notes");
    for ordinal in 0..80 {
        deep.push_str(&format!("/層{ordinal:02}"));
    }
    deep.push_str("/Déjà___計画.markdown");
    Fixture::new(
        label,
        Some(
            br#"{:pages-directory "notes"
                :journals-directory "diary"
                :file/name-format :triple-lowbar
                :journal/file-name-format "dd-MM-yyyy"
                :journal/page-title-format "yyyy-MM-dd"}"#,
        ),
        vec![
            (
                "Root.md".into(),
                b"title:: Root logical\r\n\r\n- CRLF\r\n".to_vec(),
            ),
            (
                "notes/a/same.md".into(),
                b"- same bytes, distinct identity\n".to_vec(),
            ),
            (
                "notes/b/same-copy.org".into(),
                b"- same bytes, distinct identity\n".to_vec(),
            ),
            (deep, "\u{feff}- Unicode caf\u{e9}\r\n".as_bytes().to_vec()),
            ("diary/nested/25-07-2026.org".into(), Vec::new()),
        ],
    )
}

fn enrollment_head(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
) -> ContentDigest {
    match EnrollmentReader::open_existing(root, binding).unwrap() {
        EnrollmentOpen::Present(reader) => reader.current().digest(),
        EnrollmentOpen::Absent => panic!("expected an enrollment head"),
    }
}

fn enrollment_generation(root: &EnrollmentApplicationRoot, binding: &EnrollmentBindingV1) -> u64 {
    match EnrollmentReader::open_existing(root, binding).unwrap() {
        EnrollmentOpen::Present(reader) => reader.current().generation(),
        EnrollmentOpen::Absent => panic!("expected an enrollment head"),
    }
}

fn find_file_with_prefix(root: &Path, prefix: &str) -> PathBuf {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(directory).unwrap().map(Result::unwrap) {
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else if entry.file_name().to_string_lossy().starts_with(prefix) {
                return entry.path();
            }
        }
    }
    panic!("missing file with prefix {prefix}");
}

/// The same semantic activation shapes exercised by both the initial commit
/// and the durable restart path.
fn local_active_shape_fixtures(label: &str) -> [Fixture; 4] {
    let mut multipart_bytes = Vec::new();
    for ordinal in 0..4096 {
        multipart_bytes.extend_from_slice(format!("- operation {ordinal:04}\n").as_bytes());
    }
    let zero = Fixture::new(&format!("{label}-zero"), None, Vec::new());
    let one = Fixture::new(
        &format!("{label}-one"),
        None,
        vec![("pages/one.md".into(), b"- one\n".to_vec())],
    );
    // Production's page/manifest cap grew beyond this deliberately compact
    // fixture. Keep the semantic matrix genuinely multipart through the
    // existing one-shot test hook rather than manufacturing thousands more
    // source files and slowing every lifecycle test that uses it.
    force_next_bootstrap_part_operation_limit(4_096);
    let multipart = Fixture::new(
        &format!("{label}-multipart-4096"),
        None,
        vec![("pages/multipart.md".into(), multipart_bytes)],
    );
    [
        zero,
        one,
        multipart,
        rich_fixture(&format!("{label}-rich-nested-unicode")),
    ]
}

fn assert_local_active_fixture_shapes(cases: &[Fixture; 4]) {
    let zero_parts = cases[0].verified.part_count();
    let one_source_parts = cases[1].verified.part_count();
    let multipart_parts = cases[2].verified.part_count();

    assert_eq!(zero_parts, 0);
    assert!(
        one_source_parts > zero_parts,
        "one managed source must produce a nonempty bootstrap"
    );
    assert!(
        multipart_parts >= 2 && multipart_parts >= one_source_parts,
        "the large source must remain genuinely multipart and use no fewer parts than the one-source bootstrap: one={one_source_parts}, multipart={multipart_parts}"
    );
}

#[test]
fn activation_of_zero_one_and_multipart_verified_local_is_exact_and_writes_no_graph_bytes() {
    let cases = local_active_shape_fixtures("activate");
    assert_local_active_fixture_shapes(&cases);

    for fixture in &cases {
        let root = fixture.enrollment_root("activate");
        let binding = fixture.enrollment_binding();
        let evidence = fixture.compose(&root);
        let verified_head = evidence.enrollment_head();
        let verification_digest = evidence.verification_digest();
        let session = SessionId::new();

        let before = snapshot_files(&fixture.graph_root);
        let authority = activate_verified_local(
            &root,
            evidence,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();

        assert_eq!(authority.session_id(), session);
        assert_eq!(authority.verification_digest(), verification_digest);
        assert_eq!(
            authority.handoff(),
            LocalActiveHandoff::Unsafe {
                session_id: session
            }
        );
        assert_eq!(authority.binding(), &binding);
        assert_eq!(
            authority.enrollment_head(),
            enrollment_head(&root, &binding)
        );
        assert_ne!(authority.enrollment_head(), verified_head);

        // Activation changes only device-local enrollment/runtime state.
        assert_eq!(snapshot_files(&fixture.graph_root), before);
        fixture.assert_graph_unchanged();
    }
}

/// A genuine process restart: every in-memory `VerifiedLocalEvidence` and
/// `LocalActiveAuthority` is destroyed before the reopen, which therefore has
/// nothing but the durable enrollment chain, the retained proof set, and the
/// live runtime handles to work from.
#[test]
fn restart_reopens_local_active_from_durable_state_without_any_retained_evidence() {
    let cases = local_active_shape_fixtures("restart");
    assert_local_active_fixture_shapes(&cases);

    for fixture in &cases {
        let root = fixture.enrollment_root("restart");
        let binding = fixture.enrollment_binding();
        let session = SessionId::new();
        let evidence = fixture.compose(&root);
        let verified_head = evidence.enrollment_head();
        let verification_digest = evidence.verification_digest();

        let authority = activate_verified_local(
            &root,
            evidence,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        let activated_head = authority.enrollment_head();
        let activated_generation = enrollment_generation(&root, &binding);
        // The previous process is gone: `evidence` was consumed by the
        // activation and the authority is dropped here. Nothing below may
        // depend on either.
        drop(authority);

        // The predecessor boundaries genuinely cannot reconstruct this state:
        // the VerifiedLocal reopen refuses a committed LocalActive head, and
        // the LocalActive record reopen requires the evidence this process no
        // longer has.
        assert!(crate::oplog::enrollment::reopen_verified_local(
            &root,
            &binding,
            &fixture.proofs()
        )
        .is_err());

        let reopened = reopen_local_active_authority(
            &root,
            &binding,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        assert_eq!(reopened.session_id(), session);
        assert_eq!(reopened.enrollment_head(), activated_head);
        assert_eq!(reopened.verification_digest(), verification_digest);
        assert_eq!(
            reopened.handoff(),
            LocalActiveHandoff::Unsafe {
                session_id: session
            }
        );
        assert_eq!(reopened.binding(), &binding);
        assert_ne!(activated_head, verified_head);
        // A reopen of a committed Unsafe record persists nothing at all.
        assert_eq!(enrollment_head(&root, &binding), activated_head);
        assert_eq!(enrollment_generation(&root, &binding), activated_generation);
        drop(reopened);

        // Any other requested session fails closed and never advances.
        assert!(
            matches!(
                reopen_local_active_authority(
                    &root,
                    &binding,
                    SessionId::new(),
                    &fixture.proofs(),
                    &fixture.runtime(),
                ),
                Err(LocalActivationError::Enrollment(
                    VerifiedLocalCompositionError::CompetingSession
                ))
            ),
            "a competing restart session must fail closed"
        );
        assert_eq!(enrollment_head(&root, &binding), activated_head);
        assert_eq!(enrollment_generation(&root, &binding), activated_generation);

        // Repeating the exact restart stays idempotent.
        let again = reopen_local_active_authority(
            &root,
            &binding,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        assert_eq!(again.enrollment_head(), activated_head);
        assert_eq!(enrollment_generation(&root, &binding), activated_generation);

        // A LocalActive enrollment can never be recomposed as VerifiedLocal.
        assert!(compose_verified_local(
            &root,
            binding.clone(),
            fixture.preparation,
            &fixture.proofs(),
        )
        .is_err());
        assert_eq!(enrollment_head(&root, &binding), activated_head);
        fixture.assert_graph_unchanged();
    }
}

/// One coherent live runtime: engine, device-local SQLite, and tail overlay
/// over a single engine identity, as the safe-handoff drain requires.
struct LiveRuntime {
    engine: ShardedHotEngine,
    database: SqliteFrontier,
    tail: TailOverlay,
}

impl LiveRuntime {
    fn open(fixture: &Fixture, label: &str) -> Self {
        let engine = fixture.runtime_engine(label);
        let archive = ObjectStore::open(
            &fixture.root.path().join(format!("runtime-archive-{label}")),
            fixture.workspace,
        )
        .unwrap();
        let database = fixture.runtime_projection(&engine, &archive, label);
        let source = RebuildSource::new(&engine, &archive).unwrap();
        let tail = TailOverlay::from_durable(&database, &source).unwrap();
        Self {
            engine,
            database,
            tail,
        }
    }

    fn mark_safe(&self, fixture: &Fixture, authority: &mut LocalActiveAuthority) -> ContentDigest {
        authority
            .quiesce_and_mark_safe_without_watcher_dependency(
                &fixture.graph,
                &self.engine,
                &self.database,
                &self.tail,
            )
            .unwrap()
            .enrollment_head()
    }
}

/// A restart over a cleanly handed-off `Safe` record may adopt exactly one new
/// session, through the ordinary durable record/head protocol.
#[test]
fn restart_from_a_safe_handoff_adopts_exactly_the_requested_new_session() {
    let fixture = Fixture::new("restart-safe", None, Vec::new());
    let root = fixture.enrollment_root("restart-safe");
    let binding = fixture.enrollment_binding();
    let runtime = LiveRuntime::open(&fixture, "restart-safe");

    let first_session = SessionId::new();
    let mut authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        first_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let safe_head = runtime.mark_safe(&fixture, &mut authority);
    let safe_generation = enrollment_generation(&root, &binding);
    assert_eq!(authority.handoff(), LocalActiveHandoff::Safe);
    // The previous process is gone.
    drop(authority);

    let second_session = SessionId::new();
    let mut reopened = reopen_local_active_authority(
        &root,
        &binding,
        second_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    assert_eq!(
        reopened.handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: second_session
        }
    );
    assert_ne!(reopened.enrollment_head(), safe_head);
    assert_eq!(reopened.enrollment_head(), enrollment_head(&root, &binding));
    assert_eq!(enrollment_generation(&root, &binding), safe_generation + 1);
    let resumed_head = reopened.enrollment_head();

    // The reopened value is a genuine authority: it admits a live mutation
    // window over the exact enrolled runtime.
    {
        let permit = reopened
            .admit_local_mutation(&fixture.graph, &runtime.engine)
            .unwrap();
        assert_eq!(permit.session_id(), second_session);
        assert_eq!(permit.enrollment_head(), resumed_head);
    }
    assert_eq!(enrollment_generation(&root, &binding), safe_generation + 1);
    drop(reopened);

    // The same session restarts idempotently: no second transition.
    let again = reopen_local_active_authority(
        &root,
        &binding,
        second_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    assert_eq!(again.enrollment_head(), resumed_head);
    assert_eq!(enrollment_generation(&root, &binding), safe_generation + 1);
    drop(again);

    // A third session cannot take over the committed Unsafe record.
    assert!(matches!(
        reopen_local_active_authority(
            &root,
            &binding,
            SessionId::new(),
            &fixture.proofs(),
            &fixture.runtime(),
        ),
        Err(LocalActivationError::Enrollment(
            VerifiedLocalCompositionError::CompetingSession
        ))
    ));
    assert_eq!(enrollment_head(&root, &binding), resumed_head);
    assert_eq!(enrollment_generation(&root, &binding), safe_generation + 1);
    fixture.assert_graph_unchanged();
}

/// The reopen never assumes that the committed `LocalActive` record directly
/// succeeds `VerifiedLocal`: it traverses any valid sequence of `Safe`/`Unsafe`
/// handoff records back to the exact predecessor.
#[test]
fn restart_traverses_a_long_valid_handoff_record_chain() {
    let fixture = Fixture::new("restart-chain", None, Vec::new());
    let root = fixture.enrollment_root("restart-chain");
    let binding = fixture.enrollment_binding();
    let runtime = LiveRuntime::open(&fixture, "restart-chain");

    let mut authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let mut session = SessionId::new();
    for _ in 0..3 {
        runtime.mark_safe(&fixture, &mut authority);
        drop(authority);
        session = SessionId::new();
        authority = reopen_local_active_authority(
            &root,
            &binding,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
    }
    let head = authority.enrollment_head();
    drop(authority);

    // ShadowImport, VerifiedLocal, the original activation, and three
    // Safe/Unsafe handoff pairs.
    assert_eq!(enrollment_generation(&root, &binding), 9);

    let reopened = reopen_local_active_authority(
        &root,
        &binding,
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    assert_eq!(reopened.enrollment_head(), head);
    assert_eq!(
        reopened.handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: session
        }
    );
    assert_eq!(enrollment_generation(&root, &binding), 9);
    fixture.assert_graph_unchanged();
}

/// Every durability cut of the `Safe -> Unsafe { new session }` reopen
/// transition leaves exactly one resumable head.
#[test]
fn restart_from_safe_at_every_durability_cut_resumes_one_exact_head() {
    let fixture = Fixture::new("restart-cuts", None, Vec::new());
    let runtime = LiveRuntime::open(&fixture, "restart-cuts");
    let cuts = [
        CommitCut::AfterRecordTempCreate,
        CommitCut::AfterRecordWrite,
        CommitCut::AfterRecordFileSync,
        CommitCut::AfterRecordLink,
        CommitCut::AfterRecordInsert,
        CommitCut::AfterRecordsDirectorySync,
        CommitCut::AfterHeadTempCreate,
        CommitCut::AfterHeadWrite,
        CommitCut::AfterHeadFileSync,
        CommitCut::AfterHeadReplace,
        CommitCut::AfterEnrollmentDirectorySync,
    ];
    for cut in cuts {
        let root = fixture.enrollment_root("restart-cut");
        let binding = fixture.enrollment_binding();
        let mut authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            SessionId::new(),
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        let verification_digest = authority.verification_digest();
        let safe_head = runtime.mark_safe(&fixture, &mut authority);
        drop(authority);

        let session = SessionId::new();
        let interrupted = super::reopen_local_active_authority_at_cut_for_test(
            &root,
            &binding,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
            cut,
        );
        assert!(interrupted.is_err(), "{cut:?} must not return an authority");

        // Whatever the cut left behind, the durable state is either still the
        // exact Safe predecessor or exactly one Unsafe successor for this
        // requested session. Both resume to one head.
        let head_after_crash = enrollment_head(&root, &binding);
        let committed = crate::oplog::enrollment::reopen_committed_local_active_for_session(
            &root,
            &binding,
            verification_digest,
        )
        .unwrap();
        assert_eq!(committed.sync(), LocalActiveSync::Idle, "{cut:?}");
        match committed.handoff() {
            LocalActiveHandoff::Safe => assert_eq!(head_after_crash, safe_head, "{cut:?}"),
            LocalActiveHandoff::Unsafe { session_id } => {
                assert_eq!(session_id, session, "{cut:?}");
                assert_ne!(head_after_crash, safe_head, "{cut:?}");
            }
        }

        let resumed = reopen_local_active_authority(
            &root,
            &binding,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        assert_eq!(
            resumed.handoff(),
            LocalActiveHandoff::Unsafe {
                session_id: session
            },
            "{cut:?}"
        );
        assert_eq!(resumed.enrollment_head(), enrollment_head(&root, &binding));
        assert_ne!(resumed.enrollment_head(), safe_head, "{cut:?}");
    }
    fixture.assert_graph_unchanged();
}

/// Every wrong durable lifecycle, malformed head, mixed proof set, foreign
/// binding, and cross-bound runtime fails the restart closed.
#[test]
fn restart_reopen_rejects_wrong_state_proofs_bindings_and_runtimes() {
    let fixture = Fixture::new(
        "restart-reject",
        None,
        vec![("pages/reject.md".into(), b"- reject\n".to_vec())],
    );
    let other = Fixture::new(
        "restart-reject-other",
        None,
        vec![("pages/other.md".into(), b"- other\n".to_vec())],
    );
    let binding = fixture.enrollment_binding();

    // Absent enrollment.
    let absent = fixture.enrollment_root("reject-absent");
    assert!(reopen_local_active_authority(
        &absent,
        &binding,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());

    // A committed ShadowImport head, left behind by an interrupted
    // VerifiedLocal composition.
    let shadow_root = fixture.enrollment_root("reject-shadow");
    assert!(
        crate::oplog::enrollment::compose_verified_local_at_cut_for_test(
            &shadow_root,
            binding.clone(),
            fixture.preparation,
            &fixture.proofs(),
            CommitCut::AfterRecordWrite,
        )
        .is_err()
    );
    let shadow_head = enrollment_head(&shadow_root, &binding);
    assert!(reopen_local_active_authority(
        &shadow_root,
        &binding,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());
    assert_eq!(enrollment_head(&shadow_root, &binding), shadow_head);

    // A committed VerifiedLocal head is not LocalActive authority.
    let verified_root = fixture.enrollment_root("reject-verified");
    let verified_head = fixture.compose(&verified_root).enrollment_head();
    assert!(reopen_local_active_authority(
        &verified_root,
        &binding,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());
    assert_eq!(enrollment_head(&verified_root, &binding), verified_head);

    // One genuinely activated enrollment for the remaining cases.
    let root = fixture.enrollment_root("reject-active");
    let session = SessionId::new();
    let authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let active_head = authority.enrollment_head();
    let verification_digest = authority.verification_digest();
    let active_generation = enrollment_generation(&root, &binding);
    drop(authority);

    // Every single-proof substitution from a second genuinely enrolled graph.
    for proofs in [
        VerifiedLocalProofSet {
            accepted_authority: &other.authority,
            ..fixture.proofs()
        },
        VerifiedLocalProofSet {
            sqlite_projection: other.bootstrap().sqlite_proof(),
            ..fixture.proofs()
        },
        VerifiedLocalProofSet {
            shadow_projection: &other.shadow,
            ..fixture.proofs()
        },
        VerifiedLocalProofSet {
            source_backup: &other.backup,
            ..fixture.proofs()
        },
        VerifiedLocalProofSet {
            roots: &other.roots,
            ..fixture.proofs()
        },
        VerifiedLocalProofSet {
            graph: &other.graph,
            ..fixture.proofs()
        },
    ] {
        assert!(
            reopen_local_active_authority(&root, &binding, session, &proofs, &fixture.runtime())
                .is_err(),
            "a mixed proof set must never reopen an authority"
        );
        assert_eq!(enrollment_head(&root, &binding), active_head);
    }

    // A cross-bound runtime and a foreign enrollment binding.
    let foreign_runtime = LocalActiveRuntime {
        engine: other.authority.accepted_engine(),
        projection: other.sqlite(),
    };
    assert!(reopen_local_active_authority(
        &root,
        &binding,
        session,
        &fixture.proofs(),
        &foreign_runtime,
    )
    .is_err());
    assert!(reopen_local_active_authority(
        &root,
        &other.enrollment_binding(),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());
    assert_eq!(enrollment_head(&root, &binding), active_head);
    assert_eq!(enrollment_generation(&root, &binding), active_generation);

    // A non-Idle (published) sync state never reopens.
    let published_root = fixture.enrollment_root("reject-published");
    let published_session = SessionId::new();
    let published_authority = activate_verified_local(
        &published_root,
        fixture.compose(&published_root),
        published_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let published_head = crate::oplog::enrollment::publish_local_active_for_test(
        &published_root,
        &binding,
        published_authority.enrollment_head(),
        published_authority.verification_digest(),
        published_session,
    )
    .unwrap();
    drop(published_authority);
    let published_error = reopen_local_active_authority(
        &published_root,
        &binding,
        published_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap_err();
    assert!(
        matches!(
            published_error,
            LocalActivationError::Enrollment(VerifiedLocalCompositionError::WrongLifecycle(
                detail
            )) if detail.contains("Idle")
        ),
        "unexpected published-state outcome: {published_error}"
    );
    assert_eq!(enrollment_head(&published_root, &binding), published_head);

    // A blocked enrollment never reopens.
    let blocked_root = fixture.enrollment_root("reject-blocked");
    let blocked_session = SessionId::new();
    let blocked_authority = activate_verified_local(
        &blocked_root,
        fixture.compose(&blocked_root),
        blocked_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let blocked_head = crate::oplog::enrollment::block_current_for_test(
        &blocked_root,
        &binding,
        blocked_authority.enrollment_head(),
        "restart.test".into(),
    )
    .unwrap();
    drop(blocked_authority);
    assert!(reopen_local_active_authority(
        &blocked_root,
        &binding,
        blocked_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());
    assert_eq!(enrollment_head(&blocked_root, &binding), blocked_head);

    // A truncated committed head is malformed and never reopens.
    let truncated_root = fixture.enrollment_root("reject-truncated");
    let truncated_session = SessionId::new();
    drop(
        activate_verified_local(
            &truncated_root,
            fixture.compose(&truncated_root),
            truncated_session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap(),
    );
    let head_file = find_file_with_prefix(truncated_root.path(), "head");
    assert_eq!(head_file.file_name().unwrap(), "head");
    fs::OpenOptions::new()
        .write(true)
        .open(&head_file)
        .unwrap()
        .set_len(7)
        .unwrap();
    assert!(reopen_local_active_authority(
        &truncated_root,
        &binding,
        truncated_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());

    // The genuine restart still works, and the retained verification digest is
    // unchanged throughout.
    let reopened = reopen_local_active_authority(
        &root,
        &binding,
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    assert_eq!(reopened.enrollment_head(), active_head);
    assert_eq!(reopened.verification_digest(), verification_digest);
    fixture.assert_graph_unchanged();
    other.assert_graph_unchanged();
}

#[test]
fn activation_resume_requires_the_exact_session_and_rejects_a_competing_one() {
    let fixture = Fixture::new(
        "competing-session",
        None,
        vec![("pages/session.md".into(), b"- session\n".to_vec())],
    );
    let root = fixture.enrollment_root("session");
    let binding = fixture.enrollment_binding();
    let evidence = fixture.compose(&root);
    let retained = fixture.compose(&root);
    let session = SessionId::new();

    let authority = activate_verified_local(
        &root,
        evidence,
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let activated_head = authority.enrollment_head();
    let activated_generation = enrollment_generation(&root, &binding);
    drop(authority);

    // The identical retained evidence under a competing session fails closed
    // and never advances the committed head.
    let competing = activate_verified_local(
        &root,
        retained,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    );
    assert!(
        matches!(
            competing,
            Err(LocalActivationError::Enrollment(
                VerifiedLocalCompositionError::CompetingSession
            ))
        ),
        "a competing activation session must fail closed"
    );
    assert_eq!(enrollment_head(&root, &binding), activated_head);
    assert_eq!(enrollment_generation(&root, &binding), activated_generation);
    fixture.assert_graph_unchanged();
}

#[test]
fn activation_rejects_stale_and_cross_bound_evidence_without_advancing() {
    let first = Fixture::new(
        "cross-first",
        None,
        vec![("pages/first.md".into(), b"- first\n".to_vec())],
    );
    let second = Fixture::new(
        "cross-second",
        None,
        vec![("pages/second.md".into(), b"- second\n".to_vec())],
    );

    let root = first.enrollment_root("cross");
    let binding = first.enrollment_binding();
    let evidence = first.compose(&root);
    let verified_head = evidence.enrollment_head();

    // Every single-proof substitution from a second genuinely enrolled graph
    // fails closed and leaves the committed VerifiedLocal head untouched.
    for proofs in [
        VerifiedLocalProofSet {
            accepted_authority: &second.authority,
            ..first.proofs()
        },
        VerifiedLocalProofSet {
            sqlite_projection: second.bootstrap().sqlite_proof(),
            ..first.proofs()
        },
        VerifiedLocalProofSet {
            shadow_projection: &second.shadow,
            ..first.proofs()
        },
        VerifiedLocalProofSet {
            source_backup: &second.backup,
            ..first.proofs()
        },
        VerifiedLocalProofSet {
            roots: &second.roots,
            ..first.proofs()
        },
        VerifiedLocalProofSet {
            graph: &second.graph,
            ..first.proofs()
        },
    ] {
        let attempt = activate_verified_local(
            &root,
            first.compose(&root),
            SessionId::new(),
            &proofs,
            &first.runtime(),
        );
        assert!(attempt.is_err(), "mixed proof sets must never activate");
        assert_eq!(enrollment_head(&root, &binding), verified_head);
    }

    // A runtime component from the other enrollment is also refused.
    let foreign_runtime = LocalActiveRuntime {
        engine: second.authority.accepted_engine(),
        projection: second.sqlite(),
    };
    assert!(activate_verified_local(
        &root,
        first.compose(&root),
        SessionId::new(),
        &first.proofs(),
        &foreign_runtime,
    )
    .is_err());
    assert_eq!(enrollment_head(&root, &binding), verified_head);

    // The genuine proof set and runtime still activate exactly once.
    let authority = activate_verified_local(
        &root,
        evidence,
        SessionId::new(),
        &first.proofs(),
        &first.runtime(),
    )
    .unwrap();
    assert_ne!(authority.enrollment_head(), verified_head);
    first.assert_graph_unchanged();
    second.assert_graph_unchanged();
}

#[test]
fn activation_at_every_enrollment_durability_cut_resumes_one_exact_head() {
    let fixture = Fixture::new(
        "activation-cuts",
        None,
        vec![("pages/cuts.md".into(), b"- durability\n".to_vec())],
    );
    let cuts = [
        CommitCut::AfterRecordTempCreate,
        CommitCut::AfterRecordWrite,
        CommitCut::AfterRecordFileSync,
        CommitCut::AfterRecordLink,
        CommitCut::AfterRecordInsert,
        CommitCut::AfterRecordsDirectorySync,
        CommitCut::AfterHeadTempCreate,
        CommitCut::AfterHeadWrite,
        CommitCut::AfterHeadFileSync,
        CommitCut::AfterHeadReplace,
        CommitCut::AfterEnrollmentDirectorySync,
    ];
    for cut in cuts {
        let root = fixture.enrollment_root("cut");
        let binding = fixture.enrollment_binding();
        let evidence = fixture.compose(&root);
        let verified_head = evidence.enrollment_head();
        let verification_digest = evidence.verification_digest();
        let session = SessionId::new();

        let interrupted = super::activate_verified_local_at_cut_for_test(
            &root,
            evidence,
            session,
            &fixture.proofs(),
            &fixture.runtime(),
            cut,
        );
        assert!(interrupted.is_err(), "{cut:?} must not return an authority");

        // Whatever the cut left behind, a crash always resumes conservatively
        // to exactly one head: either still VerifiedLocal, or the exact
        // Unsafe+Idle LocalActive record for this session.
        let head_after_crash = enrollment_head(&root, &binding);
        match crate::oplog::enrollment::reopen_verified_local(&root, &binding, &fixture.proofs()) {
            Ok(evidence) => {
                assert_eq!(evidence.enrollment_head(), verified_head, "{cut:?}");
                let resumed = activate_verified_local(
                    &root,
                    evidence,
                    session,
                    &fixture.proofs(),
                    &fixture.runtime(),
                )
                .unwrap();
                assert_eq!(resumed.enrollment_head(), enrollment_head(&root, &binding));
                assert_eq!(
                    resumed.handoff(),
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    }
                );
            }
            Err(_) => {
                // The head already advanced past VerifiedLocal, so the record
                // must be exactly this session's Unsafe+Idle activation.
                assert_ne!(head_after_crash, verified_head, "{cut:?}");
                let committed =
                    crate::oplog::enrollment::reopen_committed_local_active_for_session(
                        &root,
                        &binding,
                        verification_digest,
                    )
                    .unwrap();
                assert_eq!(
                    committed.handoff(),
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    },
                    "{cut:?}"
                );
                assert_eq!(committed.sync(), LocalActiveSync::Idle, "{cut:?}");
                assert_eq!(committed.enrollment_head(), head_after_crash);
            }
        }
    }
    fixture.assert_graph_unchanged();
}

#[test]
fn activation_partial_record_and_head_temporaries_fail_closed_or_resume_exactly() {
    let fixture = Fixture::new(
        "activation-partial",
        None,
        vec![("pages/partial.md".into(), b"- partial\n".to_vec())],
    );

    // A truncated record temporary is ambiguous and must never advance.
    let record_root = fixture.enrollment_root("partial-record");
    let binding = fixture.enrollment_binding();
    let record_evidence = fixture.compose(&record_root);
    let verified_head = record_evidence.enrollment_head();
    assert!(super::activate_verified_local_at_cut_for_test(
        &record_root,
        record_evidence,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
        CommitCut::AfterRecordWrite,
    )
    .is_err());
    let temp = find_file_with_prefix(record_root.path(), ".record-tmp-");
    let length = fs::metadata(&temp).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(&temp)
        .unwrap()
        .set_len(length / 2)
        .unwrap();
    let resumed =
        crate::oplog::enrollment::reopen_verified_local(&record_root, &binding, &fixture.proofs())
            .unwrap();
    assert_eq!(resumed.enrollment_head(), verified_head);
    let session = SessionId::new();
    // A stranded partial record never yields a divergent head: activation either
    // fails closed at the exact VerifiedLocal head, or commits exactly one
    // Unsafe+Idle LocalActive record for this session.
    match activate_verified_local(
        &record_root,
        resumed,
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    ) {
        Ok(authority) => {
            assert_eq!(
                authority.enrollment_head(),
                enrollment_head(&record_root, &binding)
            );
            assert_eq!(
                authority.handoff(),
                LocalActiveHandoff::Unsafe {
                    session_id: session
                }
            );
        }
        Err(_) => assert_eq!(enrollment_head(&record_root, &binding), verified_head),
    }

    // A truncated head temporary is discardable, so activation resumes.
    let head_root = fixture.enrollment_root("partial-head");
    let head_evidence = fixture.compose(&head_root);
    let head_session = SessionId::new();
    assert!(super::activate_verified_local_at_cut_for_test(
        &head_root,
        head_evidence,
        head_session,
        &fixture.proofs(),
        &fixture.runtime(),
        CommitCut::AfterHeadWrite,
    )
    .is_err());
    let head_temp = find_file_with_prefix(head_root.path(), ".head-tmp-");
    fs::OpenOptions::new()
        .write(true)
        .open(&head_temp)
        .unwrap()
        .set_len(7)
        .unwrap();
    let evidence =
        crate::oplog::enrollment::reopen_verified_local(&head_root, &binding, &fixture.proofs())
            .unwrap();
    let authority = activate_verified_local(
        &head_root,
        evidence,
        head_session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    assert_eq!(
        authority.enrollment_head(),
        enrollment_head(&head_root, &binding)
    );
    fixture.assert_graph_unchanged();
}

#[test]
fn blocked_and_non_verified_lifecycles_never_activate() {
    let fixture = Fixture::new(
        "blocked-lifecycle",
        None,
        vec![("pages/blocked.md".into(), b"- blocked\n".to_vec())],
    );
    let root = fixture.enrollment_root("blocked");
    let binding = fixture.enrollment_binding();
    let evidence = fixture.compose(&root);

    crate::oplog::enrollment::block_current_for_test(
        &root,
        &binding,
        evidence.enrollment_head(),
        "activation.test".into(),
    )
    .unwrap();
    let blocked_head = enrollment_head(&root, &binding);

    assert!(activate_verified_local(
        &root,
        evidence,
        SessionId::new(),
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .is_err());
    assert_eq!(enrollment_head(&root, &binding), blocked_head);
    fixture.assert_graph_unchanged();
}

/// A full-scan reconciliation dispatch owns the first durable baseline
/// mutation of a step (`begin_epoch` plus the scan row appends). It must
/// therefore authorize the exact live graph and engine *before* that mutation,
/// not only later inside the coordinator.
///
/// The refused authority here is a genuine promoted admission — a real
/// `LocalActiveAuthority` plus the real `PromotedLocalRuntime` minted for a
/// second, separately enrolled graph — so this is a live wrong-authority
/// dispatch rather than a synthetic admission value.
#[test]
fn full_scan_dispatch_with_wrong_authority_never_mutates_the_baseline() {
    let fixture = Fixture::new(
        "full-scan-authority",
        None,
        vec![("pages/scan.md".into(), b"- scan\n".to_vec())],
    );
    // The foreign enrollment is deliberately empty so its own live runtime
    // engine is admissible: the refusal under test must come from the graph and
    // engine identity, not from a stale accepted frontier.
    let mut foreign = Fixture::new("full-scan-foreign", None, Vec::new());

    // A genuine live promoted admission that belongs to the *other* graph.
    let foreign_root = foreign.enrollment_root("foreign-admission");
    let foreign_paths = PromotedPaths::new(&foreign, "foreign-admission");
    let (mut foreign_authority, mut foreign_runtime) = promote(
        &mut foreign,
        &foreign_root,
        SessionId::new(),
        &foreign_paths,
    );
    let foreign_session = foreign_runtime
        .admit_promoted_mutation(&mut foreign_authority, &foreign.graph)
        .unwrap();
    let wrong_admission = foreign_session.admission();

    // One coherent live runtime for the graph actually being reconciled.
    let mut engine = fixture.runtime_engine("scan");
    let archive = ObjectStore::open(
        &fixture.root.path().join("runtime-archive-scan"),
        fixture.workspace,
    )
    .unwrap();
    let mut database = fixture.runtime_projection(&engine, &archive, "scan");
    let source = RebuildSource::new(&engine, &archive).unwrap();
    let mut tail = TailOverlay::from_durable(&database, &source).unwrap();
    let mut baseline = fixture.reconciliation_baseline("scan");
    let baseline_directory = baseline.path().parent().unwrap().to_path_buf();

    // A fresh baseline has no clean head yet, so the head observation is the
    // exact `Option`, not an unwrapped value.
    let head_before = baseline.head().ok();
    let epochs_before = baseline.epoch_rows_for_test();
    let bytes_before = durable_sqlite_digests(&baseline_directory);
    let projection_before = ContentDigest::of(&fs::read(database.path()).unwrap());
    let frontier_before = database.frontier_root().unwrap();
    let accepted_before = engine.accepted_frontier_root().unwrap();

    let mut session = ReconciliationSession::new(ReconciliationSchedulerLimits::default());
    session.trigger(ReconciliationTrigger::Explicit);
    assert_eq!(
        session.step(ReconciliationSessionDependencies {
            admission: &wrong_admission,
            graph: &fixture.graph,
            receipts: &fixture.receipts,
            engine: &mut engine,
            database: &mut database,
            tail: &mut tail,
            bootstrap: None,
            baseline: &mut baseline,
            observed_at: BaselineTimestamp::from_millis(1).unwrap(),
        }),
        Ok(ReconciliationSessionStep::Blocked),
        "a full scan that is not admitted must fail closed"
    );

    // Nothing scan-owned may have been written: not the baseline bytes, not a
    // building epoch, not a row, not the head, and not the projection.
    assert_eq!(
        durable_sqlite_digests(&baseline_directory),
        bytes_before,
        "a refused full scan must leave the baseline database byte-identical"
    );
    assert_eq!(baseline.epoch_rows_for_test(), epochs_before);
    assert_eq!(baseline.head().ok(), head_before);
    assert_eq!(
        ContentDigest::of(&fs::read(database.path()).unwrap()),
        projection_before
    );
    assert_eq!(database.frontier_root().unwrap(), frontier_before);
    assert_eq!(engine.accepted_frontier_root().unwrap(), accepted_before);
    fixture.assert_graph_unchanged();
    foreign.assert_graph_unchanged();

    // Control: the identical dispatch under an admitted runtime does reach the
    // baseline, so the assertions above are not vacuous.
    let admitted = LocalRuntimeAdmission::unenrolled_pre_activation();
    let mut control = ReconciliationSession::new(ReconciliationSchedulerLimits::default());
    control.trigger(ReconciliationTrigger::Explicit);
    assert!(control
        .step(ReconciliationSessionDependencies {
            admission: &admitted,
            graph: &fixture.graph,
            receipts: &fixture.receipts,
            engine: &mut engine,
            database: &mut database,
            tail: &mut tail,
            bootstrap: None,
            baseline: &mut baseline,
            observed_at: BaselineTimestamp::from_millis(2).unwrap(),
        })
        .is_ok());
    assert_ne!(
        baseline.epoch_rows_for_test(),
        epochs_before,
        "an admitted full scan must reach the baseline"
    );
    fixture.assert_graph_unchanged();
}

#[test]
fn unchanged_bootstrap_full_scan_never_materializes_crdt_pages_at_any_graph_size() {
    for page_count in [1, 17] {
        let files = (0..page_count)
            .map(|index| {
                (
                    format!("pages/unchanged-{index:03}.md"),
                    format!("- unchanged {index}\n").into_bytes(),
                )
            })
            .collect();
        let label = format!("unchanged-bootstrap-scan-{page_count}");
        let mut fixture = Fixture::new(&label, None, files);
        let root = fixture.enrollment_root(&label);
        let paths = PromotedPaths::new(&fixture, &label);
        let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
        let mut window = runtime
            .admit_promoted_mutation(&mut authority, &fixture.graph)
            .unwrap();
        let (_admission, engine, database, _tail, bootstrap) =
            window.parts_with_bootstrap().unwrap();
        engine.reconcile_expected_path_history().unwrap();
        let projection = engine.projection_work_index().unwrap();
        let source = JoinedAuthenticatedExpectedPathSource::with_bootstrap(
            engine, projection, bootstrap, database,
        );

        take_bootstrap_page_materializations_for_test();
        let scan = scan_graph_text(&fixture.graph, &source, GraphTextScanLimits::default())
            .expect("unchanged bootstrap scan must be semantically exact");
        let materialized = take_bootstrap_page_materializations_for_test();

        assert!(scan.candidates.is_empty());
        assert_eq!(
            materialized, 0,
            "{page_count} unchanged bootstrap pages must require no CRDT materialization"
        );
    }
}

#[test]
fn one_accepted_pending_page_uses_authenticated_work_without_touching_bootstrap_pages() {
    let files = (0..17)
        .map(|index| {
            (
                format!("pages/pending-control-{index:03}.md"),
                format!("- unchanged control {index}\n").into_bytes(),
            )
        })
        .collect();
    let mut fixture = Fixture::new("pending-bootstrap-suffix", None, files);
    let root = fixture.enrollment_root("pending-bootstrap-suffix");
    let paths = PromotedPaths::new(&fixture, "pending-bootstrap-suffix");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    const SEED: u128 = 0xB005_7000;
    append_local_batch(&fixture, &mut authority, &mut runtime, SEED);
    let pending_path = ManagedPath::parse(&format!("pages/promoted-{SEED}.md")).unwrap();

    let mut window = runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .unwrap();
    let (_admission, engine, database, _tail, bootstrap) = window.parts_with_bootstrap().unwrap();
    engine.reconcile_expected_path_history().unwrap();
    let exceptions = database
        .authenticated_bootstrap_projection_exceptions(
            engine,
            bootstrap.binding(),
            1_000_000,
            512 * 1024 * 1024,
        )
        .unwrap();
    assert_eq!(exceptions.len(), 1);

    let projection = engine.projection_work_index().unwrap();
    let work = projection
        .next()
        .unwrap()
        .expect("one Ready projection row");
    assert_eq!(work.path(), &pending_path);
    let source = JoinedAuthenticatedExpectedPathSource::with_bootstrap(
        engine, projection, bootstrap, database,
    );
    take_bootstrap_page_materializations_for_test();
    let pending_scan = scan_graph_text(&fixture.graph, &source, GraphTextScanLimits::default())
        .expect("authenticated Ready target must drive the pending scan");
    assert_eq!(take_bootstrap_page_materializations_for_test(), 0);
    assert_eq!(pending_scan.candidates.len(), 1);
    assert_eq!(pending_scan.candidates[0].path, pending_path);
    assert_eq!(
        pending_scan.candidates[0].change,
        GraphTextCandidateKind::Absence
    );
    drop(source);

    execute_manifested_projection_work(&fixture.graph, &fixture.receipts, engine, &work).unwrap();
    let projection = engine.projection_work_index().unwrap();
    let source = JoinedAuthenticatedExpectedPathSource::with_bootstrap(
        engine, projection, bootstrap, database,
    );
    take_bootstrap_page_materializations_for_test();
    let completed_scan = scan_graph_text(&fixture.graph, &source, GraphTextScanLimits::default())
        .expect("completed projection and Ready target must describe the same bytes");
    assert!(completed_scan.candidates.is_empty());
    assert_eq!(take_bootstrap_page_materializations_for_test(), 0);
}

#[test]
fn corrupt_bootstrap_payload_is_refused_without_crdt_fallback() {
    let fixture = Fixture::new(
        "corrupt-bootstrap-reconciliation",
        None,
        vec![("pages/corrupt.md".into(), b"- exact bootstrap\n".to_vec())],
    );
    let pack_name = fixture.prepared.aggregate().parts()[0]
        .part_id()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let payload = fixture
        .archive_root
        .join("bootstrap-v1/part-object-packs")
        .join(pack_name);
    fs::write(payload, b"unauthenticated replacement pack").unwrap();

    take_bootstrap_page_materializations_for_test();
    assert!(
        reopen_inactive_bootstrap_accepted_authority(&fixture.verified, fixture.archive(),)
            .is_err()
    );
    assert_eq!(take_bootstrap_page_materializations_for_test(), 0);
    fixture.assert_graph_unchanged();
}

#[test]
fn stale_sqlite_bootstrap_suffix_binding_is_never_treated_as_unchanged() {
    let mut fixture = Fixture::new(
        "stale-bootstrap-suffix",
        None,
        vec![("pages/stale.md".into(), b"- bootstrap\n".to_vec())],
    );
    let root = fixture.enrollment_root("stale-bootstrap-suffix");
    let paths = PromotedPaths::new(&fixture, "stale-bootstrap-suffix");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    stage_local_batch_at(
        &fixture,
        &mut authority,
        &mut runtime,
        0xB005_7100,
        "pages",
        false,
    );

    let engine = &runtime.engine;
    let projection = engine.projection_work_index().unwrap();
    let source = JoinedAuthenticatedExpectedPathSource::with_bootstrap(
        engine,
        projection,
        &runtime.bootstrap_projection,
        runtime.projection.database(),
    );
    assert!(source
        .current_scan_identity(GraphTextScanLimits::default().retained_bytes)
        .is_err());
}

/// Compile-time proof that the authority cannot be cloned, serialized, or
/// deserialized. The inherent associated const wins whenever the bound holds.
struct Probe<T>(PhantomData<T>);

trait NegativeProbe {
    const CLONEABLE: bool = false;
    const SERIALIZABLE: bool = false;
    const DESERIALIZABLE: bool = false;
}

impl<T> NegativeProbe for Probe<T> {}

impl<T: Clone> Probe<T> {
    const CLONEABLE: bool = true;
}

struct SerdeProbe<T>(PhantomData<T>);

trait NegativeSerdeProbe {
    const SERIALIZABLE: bool = false;
}

impl<T> NegativeSerdeProbe for SerdeProbe<T> {}

impl<T: serde::Serialize> SerdeProbe<T> {
    const SERIALIZABLE: bool = true;
}

struct DeserializeProbe<T>(PhantomData<T>);

trait NegativeDeserializeProbe {
    const DESERIALIZABLE: bool = false;
}

impl<T> NegativeDeserializeProbe for DeserializeProbe<T> {}

impl<T: serde::de::DeserializeOwned> DeserializeProbe<T> {
    const DESERIALIZABLE: bool = true;
}

#[test]
fn local_active_authority_cannot_be_cloned_serialized_or_deserialized() {
    assert!(!Probe::<LocalActiveAuthority>::CLONEABLE);
    assert!(!SerdeProbe::<LocalActiveAuthority>::SERIALIZABLE);
    assert!(!DeserializeProbe::<LocalActiveAuthority>::DESERIALIZABLE);
    assert!(!Probe::<SafeHandoffPermit>::CLONEABLE);
    assert!(!SerdeProbe::<SafeHandoffPermit>::SERIALIZABLE);
    // The positive control proves the probe actually discriminates.
    assert!(Probe::<ContentDigest>::CLONEABLE);
    assert!(SerdeProbe::<ContentDigest>::SERIALIZABLE);
}

#[test]
fn runtime_mutation_is_denied_without_wrong_or_stale_authority_and_allowed_with_the_exact_one() {
    let fixture = Fixture::new("runtime-gate", None, Vec::new());
    let root = fixture.enrollment_root("gate");
    let binding = fixture.enrollment_binding();
    let session = SessionId::new();
    let mut authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();

    let engine = fixture.runtime_engine("gate");

    // Allowed: the exact current authority, live graph, and live engine.
    {
        let permit = authority
            .admit_local_mutation(&fixture.graph, &engine)
            .unwrap();
        assert_eq!(permit.session_id(), session);
        assert_eq!(permit.enrollment_head(), enrollment_head(&root, &binding));
    }

    // Denied: a foreign graph and a foreign engine.
    let other = Fixture::new(
        "runtime-gate-foreign",
        None,
        vec![("pages/foreign.md".into(), b"- foreign\n".to_vec())],
    );
    let foreign_engine = other.runtime_engine("foreign");
    assert!(authority
        .admit_local_mutation(&other.graph, &engine)
        .is_err());
    assert!(authority
        .admit_local_mutation(&fixture.graph, &foreign_engine)
        .is_err());

    // Denied: a runtime engine behind the activated accepted frontier. The
    // enrollment identity matches exactly; only the accepted sequence is stale.
    let advanced = Fixture::new(
        "runtime-gate-advanced",
        None,
        vec![("pages/advanced.md".into(), b"- advanced\n".to_vec())],
    );
    let advanced_root = advanced.enrollment_root("advanced");
    let mut advanced_authority = activate_verified_local(
        &advanced_root,
        advanced.compose(&advanced_root),
        SessionId::new(),
        &advanced.proofs(),
        &advanced.runtime(),
    )
    .unwrap();
    let behind = advanced.runtime_engine("behind");
    assert_eq!(
        behind
            .accepted_frontier_root()
            .unwrap()
            .acceptance_sequence(),
        0
    );
    assert!(advanced.verified.part_count() >= 1);
    assert!(
        advanced_authority
            .admit_local_mutation(&advanced.graph, &behind)
            .is_err(),
        "a runtime engine behind the activated frontier must never be admitted"
    );

    // Denied: an unenrolled engine has no endpoint at all.
    let unenrolled = ShardedHotEngine::new(
        WorkspaceId::from_uuid(Uuid::from_u128(0x9900)),
        LineageDigest::of(b"unenrolled"),
        DocumentId::from_uuid(Uuid::from_u128(0x9901)),
    );
    assert!(authority
        .admit_local_mutation(&fixture.graph, &unenrolled)
        .is_err());

    // Denied: the enrollment itself is no longer this session's LocalActive.
    crate::oplog::enrollment::block_current_for_test(
        &root,
        &binding,
        authority.enrollment_head(),
        "gate.test".into(),
    )
    .unwrap();
    let blocked_head = enrollment_head(&root, &binding);
    assert!(authority
        .admit_local_mutation(&fixture.graph, &engine)
        .is_err());
    assert_eq!(enrollment_head(&root, &binding), blocked_head);

    fixture.assert_graph_unchanged();
    other.assert_graph_unchanged();
    advanced.assert_graph_unchanged();
}

#[test]
fn safe_handoff_proves_every_core_drain_and_names_its_missing_dependency() {
    let fixture = Fixture::new("safe-handoff", None, Vec::new());
    let root = fixture.enrollment_root("safe");
    let binding = fixture.enrollment_binding();
    let session = SessionId::new();
    let mut authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let unsafe_head = authority.enrollment_head();

    // One coherent live runtime: engine, device-local SQLite, and tail overlay
    // all share a single engine identity.
    let engine = &fixture.runtime_engine("safe");
    let archive = ObjectStore::open(
        &fixture.root.path().join("runtime-archive-safe"),
        fixture.workspace,
    )
    .unwrap();
    let database = &fixture.runtime_projection(engine, &archive, "safe");
    let source = RebuildSource::new(engine, &archive).unwrap();
    let tail = TailOverlay::from_durable(database, &source).unwrap();

    // The production transition proves every core-checkable drain and then
    // refuses to mint Safe, naming the exact missing dependency.
    let unavailable = authority
        .quiesce_and_mark_safe(&fixture.graph, engine, database, &tail)
        .unwrap_err();
    assert!(
        matches!(
            unavailable,
            SafeHandoffUnavailable::MissingDependency(SAFE_HANDOFF_MISSING_DEPENDENCY)
        ),
        "unexpected safe-handoff outcome: {unavailable}"
    );
    assert_eq!(enrollment_head(&root, &binding), unsafe_head);
    assert_eq!(
        authority.handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: session
        }
    );

    // With that one dependency set aside, the same drain proof persists Safe
    // and a fresh committed-head reopen confirms it.
    let permit = authority
        .quiesce_and_mark_safe_without_watcher_dependency(&fixture.graph, engine, database, &tail)
        .unwrap();
    assert_eq!(permit.session_id(), session);
    assert_eq!(permit.enrollment_head(), enrollment_head(&root, &binding));
    assert_ne!(permit.enrollment_head(), unsafe_head);
    assert_eq!(authority.handoff(), LocalActiveHandoff::Safe);
    let safe_head = authority.enrollment_head();

    // Any mutation admission must durably move Safe back to Unsafe first.
    {
        let admitted = authority
            .admit_local_mutation(&fixture.graph, engine)
            .unwrap();
        assert_eq!(admitted.session_id(), session);
    }
    assert_eq!(
        authority.handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: session
        }
    );
    assert_ne!(authority.enrollment_head(), safe_head);
    assert_eq!(
        authority.enrollment_head(),
        enrollment_head(&root, &binding)
    );

    // An incomplete drain never reaches Safe.
    let mut pressured = tail;
    let _reservation = pressured
        .reserve_mutation(crate::oplog::TAIL_MAX_BYTES)
        .unwrap();
    let blocked = authority
        .quiesce_and_mark_safe_without_watcher_dependency(
            &fixture.graph,
            engine,
            database,
            &pressured,
        )
        .unwrap_err();
    assert!(
        matches!(blocked, SafeHandoffUnavailable::DrainIncomplete { .. }),
        "unexpected drain outcome: {blocked}"
    );
    assert_eq!(
        authority.handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: session
        }
    );
    fixture.assert_graph_unchanged();
}

// ---------------------------------------------------------------------------
// P2N8 runtime promotion.
// ---------------------------------------------------------------------------

/// The device-local paths one promoted runtime is opened over.
struct PromotedPaths {
    runtime_root: ApplicationRuntimeRoot,
    runtime_root_path: PathBuf,
    database_path: PathBuf,
}

impl PromotedPaths {
    fn new(fixture: &Fixture, label: &str) -> Self {
        let runtime_root_path = fixture.root.path().join(format!("promoted-rt-{label}"));
        Self {
            runtime_root: ApplicationRuntimeRoot::open_for_test(&runtime_root_path).unwrap(),
            runtime_root_path,
            database_path: fixture.root.path().join(format!("promoted-{label}.sqlite")),
        }
    }

    fn open<'a>(&'a self, fixture: &'a Fixture) -> PromotedRuntimeOpen<'a> {
        PromotedRuntimeOpen {
            graph: &fixture.graph,
            receipts: &fixture.receipts,
            archive_root: &fixture.archive_root,
            database_path: &self.database_path,
            application_runtime_root: &self.runtime_root,
            graph_root: &fixture.graph_root,
            migration_backup_root: fixture.roots.canonical_root(),
        }
    }
}

/// Prove the P2N7 fence is still real for this archive: an ordinary enrolled
/// open of an inactive bootstrap history must fail closed.
fn assert_ordinary_enrolled_open_is_fenced(fixture: &Fixture) {
    let storage = ProjectionStorageBinding {
        endpoint: fixture.authority.binding().storage_binding().endpoint,
        receipt_store_id: fixture.receipts.store_id(),
    };
    let error = fixture
        .archive()
        .seal_enrolled_projection(storage)
        .err()
        .expect("an inactive bootstrap archive must refuse an ordinary enrolled open")
        .1;
    assert!(
        matches!(
            error,
            crate::oplog::StoreError::InactiveBootstrapHistory
                | crate::oplog::StoreError::PromotedRuntimeStateMismatch(_)
        ),
        "unexpected ordinary-open error: {error}"
    );
}

/// Activate P2N7 and then complete the P2N8 promotion.
fn promote(
    fixture: &mut Fixture,
    root: &EnrollmentApplicationRoot,
    session: SessionId,
    paths: &PromotedPaths,
) -> (LocalActiveAuthority, PromotedLocalRuntime) {
    let authority = activate_verified_local(
        root,
        fixture.compose(root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let sealed =
        seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime()).unwrap();
    // The bootstrap database closes and the promoted one opens under the exact
    // same retained workspace lease. This is the production entry point, not a
    // hand-assembled twin of it.
    let bootstrap = fixture.take_bootstrap_session();
    let runtime = bootstrap
        .promote(sealed, &authority, &paths.open(fixture))
        .map_err(|refusal| refusal.into_parts().1)
        .unwrap();
    (authority, runtime)
}

/// Author, publish, and accept one ordinary local batch through the promoted
/// runtime's admitted mutation window.
fn append_local_batch(
    fixture: &Fixture,
    authority: &mut LocalActiveAuthority,
    runtime: &mut PromotedLocalRuntime,
    seed: u128,
) {
    append_local_batch_at(fixture, authority, runtime, seed, "pages")
}

/// `page_directory` must be the configured pages directory of `fixture`'s
/// graph, so the authored projection path is a valid managed path there.
fn append_local_batch_at(
    fixture: &Fixture,
    authority: &mut LocalActiveAuthority,
    runtime: &mut PromotedLocalRuntime,
    seed: u128,
    page_directory: &str,
) {
    stage_local_batch_at(fixture, authority, runtime, seed, page_directory, true);
}

fn stage_local_batch_at(
    fixture: &Fixture,
    authority: &mut LocalActiveAuthority,
    runtime: &mut PromotedLocalRuntime,
    seed: u128,
    page_directory: &str,
    drain_sqlite: bool,
) {
    let endpoint = authority.endpoint();
    let mut session = runtime
        .admit_promoted_mutation(authority, &fixture.graph)
        .unwrap();
    let transaction = OperationTransaction::new(vec![
        SemanticOperation::CreatePage {
            page_id: PageId::from_uuid(Uuid::from_u128(seed)),
            home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
            name: LogicalPageName::parse(&format!("Promoted {seed}")).unwrap(),
            path: ManagedPath::parse(&format!("{page_directory}/promoted-{seed}.md")).unwrap(),
            kind: ManagedTextKind::Page,
        },
        SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
            },
            page_id: PageId::from_uuid(Uuid::from_u128(seed)),
            parent: None,
            order: "a".into(),
            content: format!("promoted local batch {seed}"),
        },
    ])
    .unwrap();

    let (admission, engine, _database, _tail) = session.parts().unwrap();
    // Every mutation path authorizes first; a promoted admission proves the
    // whole binding, not merely a non-regressing acceptance sequence.
    admission.authorize(&fixture.graph, engine).unwrap();

    let draft = engine
        .draft_author_transaction(
            AuthorBatch {
                batch_id: BatchId::from_uuid(Uuid::from_u128(seed + 3)),
                author_device_id: endpoint.device_id(),
                author_session_id: SessionId::from_uuid(Uuid::from_u128(seed + 4)),
                crdt_peer_id: CrdtPeerId::from_u64((seed as u64) | 1),
            },
            BatchOrigin::LocalMutation,
            &transaction,
        )
        .unwrap();
    let prepared = engine
        .finalize_author_transaction(draft, &fixture.graph, &fixture.receipts, endpoint)
        .unwrap();
    ObjectStore::open(&fixture.archive_root, fixture.workspace)
        .unwrap()
        .publish_prepared(&prepared)
        .unwrap();
    let outcome = engine
        .stage_archive_batch(prepared.manifest().batch_id())
        .unwrap();
    assert!(
        matches!(outcome.disposition, BatchDisposition::Accepted { .. }),
        "promoted local batch was not accepted: {:?}",
        outcome.disposition
    );

    // A promoted admission requires the device-local SQLite projection to be at
    // the current accepted frontier, so every mutation drains before the next
    // window opens. This is the ordinary bounded tail drain.
    if drain_sqlite {
        let drained = session.drain_projection(16).unwrap();
        assert_eq!(drained, 1, "exactly the new accepted batch drains");
    }
}

#[test]
fn promoted_local_coordinator_mints_exact_session_device_batch_and_peer_identity() {
    let mut fixture = Fixture::new(
        "promoted-local-author-identity",
        None,
        vec![("pages/seed.md".into(), b"- seed\n".to_vec())],
    );
    let root = fixture.enrollment_root("promoted-local-author-identity");
    let paths = PromotedPaths::new(&fixture, "promoted-local-author-identity");
    let runtime_session = SessionId::new();
    let (mut authority, mut runtime) = promote(&mut fixture, &root, runtime_session, &paths);
    let endpoint_device = authority.endpoint().device_id();
    let mut observed = Vec::new();

    for index in 0..2u128 {
        let seed = 0xA110_0000 + index * 10;
        let transaction = OperationTransaction::new(vec![
            SemanticOperation::CreatePage {
                page_id: PageId::from_uuid(Uuid::from_u128(seed)),
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
                name: LogicalPageName::parse(&format!("Admitted Local {index}")).unwrap(),
                path: ManagedPath::parse(&format!("pages/admitted-local-{index}.md")).unwrap(),
                kind: ManagedTextKind::Page,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                    home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
                },
                page_id: PageId::from_uuid(Uuid::from_u128(seed)),
                parent: None,
                order: "a".into(),
                content: format!("admitted local {index}"),
            },
        ])
        .unwrap();
        let mut window = runtime
            .admit_promoted_mutation(&mut authority, &fixture.graph)
            .unwrap();
        let mut state = OperationalCoordinator::execute_local(
            &mut window,
            &fixture.graph,
            &fixture.receipts,
            &transaction,
        );
        let completion = loop {
            match state {
                LocalMutationCoordinatorState::Active(completion) => break completion,
                LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(
                    continuation,
                )) => {
                    let (admission, engine, database, tail) = window.parts().unwrap();
                    state = continuation.retry(
                        &admission,
                        &fixture.graph,
                        &fixture.receipts,
                        engine,
                        database,
                        tail,
                    );
                }
                LocalMutationCoordinatorState::Recovering(
                    LocalMutationRecovery::ReconciliationRequired(reconciliation),
                ) => panic!(
                    "positive promoted local authoring requested reconciliation: {:?}",
                    reconciliation.paths()
                ),
                LocalMutationCoordinatorState::Blocked(blocked) => {
                    panic!("promoted local authoring blocked: {}", blocked.failure())
                }
                LocalMutationCoordinatorState::Revoked(revoked) => {
                    panic!("promoted local authoring revoked: {}", revoked.failure())
                }
            }
        };
        let author =
            take_last_admitted_local_author().expect("the admitted draft records its test witness");
        assert_eq!(author.batch_id, completion.batch_id());
        assert_eq!(author.author_device_id, endpoint_device);
        assert_eq!(author.author_session_id, runtime_session);
        assert_ne!(author.crdt_peer_id.as_u64(), 0);
        drop(window);

        let archive = ObjectStore::open(&fixture.archive_root, fixture.workspace).unwrap();
        let batch = match archive.inspect_batch(completion.batch_id()).unwrap() {
            crate::oplog::BatchInspection::Ready(batch) => batch,
            other => panic!("admitted local batch is not immutable Ready: {other:?}"),
        };
        assert_eq!(batch.manifest().author_device_id(), endpoint_device);
        assert_eq!(batch.manifest().author_session_id(), runtime_session);
        assert_eq!(batch.manifest().origin(), BatchOrigin::LocalMutation);
        observed.push(author);
    }
    assert_ne!(observed[0].batch_id, observed[1].batch_id);
    assert_ne!(observed[0].crdt_peer_id, observed[1].crdt_peer_id);
}

fn promoted_local_create_page(seed: u128, label: &str) -> OperationTransaction {
    OperationTransaction::new(vec![
        SemanticOperation::CreatePage {
            page_id: PageId::from_uuid(Uuid::from_u128(seed)),
            home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
            name: LogicalPageName::parse(label).unwrap(),
            path: ManagedPath::parse(&format!("pages/{label}.md")).unwrap(),
            kind: ManagedTextKind::Page,
        },
        SemanticOperation::CreateBlock {
            block: BlockLocation {
                block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                home_document_id: DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
            },
            page_id: PageId::from_uuid(Uuid::from_u128(seed)),
            parent: None,
            order: "a".into(),
            content: label.into(),
        },
    ])
    .unwrap()
}

#[test]
fn deterministic_published_local_authentication_damage_retains_typed_blocked_state() {
    let mut fixture = Fixture::new(
        "published-local-authentication-damage",
        None,
        vec![("pages/seed.md".into(), b"- seed\n".to_vec())],
    );
    let root = fixture.enrollment_root("published-local-authentication-damage");
    let paths = PromotedPaths::new(&fixture, "published-local-authentication-damage");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    let transaction = promoted_local_create_page(0xAB40_0000, "published-auth-damage");

    let accepted_before = runtime.engine().accepted_frontier_root().unwrap();
    let history_before = runtime.engine().durable_history_authority().unwrap();
    let sqlite_before = runtime.database().frontier_root().unwrap();
    let projection_before = promoted_projection_digests(&paths.database_path);
    let graph_before = snapshot_files(&fixture.graph_root);
    let publications_before = published_immutable_digests(&fixture.archive_root);
    let releases_before = fixture.graph.handoff_release_count();

    fail_once_at(OperationalFaultPoint::AfterManifest);
    let mut window = runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .unwrap();
    let LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(continuation)) =
        OperationalCoordinator::execute_local(
            &mut window,
            &fixture.graph,
            &fixture.receipts,
            &transaction,
        )
    else {
        panic!("AfterManifest must return the genuine admitted-local continuation");
    };
    let batch_id = continuation.batch_id();
    let drafted =
        take_last_admitted_local_author().expect("the production local path drafted exactly once");
    assert_eq!(drafted.batch_id, batch_id);
    assert_ne!(
        published_immutable_digests(&fixture.archive_root),
        publications_before,
        "the failpoint must occur after immutable publication"
    );

    let manifest_path = fixture
        .archive_root
        .join("batches")
        .join(format!("{batch_id}.manifest"));
    fs::write(&manifest_path, b"{").unwrap();
    let damaged_publication = published_immutable_digests(&fixture.archive_root);

    let (admission, engine, database, tail) = window.parts().unwrap();
    let LocalMutationCoordinatorState::Blocked(blocked) = continuation.retry(
        &admission,
        &fixture.graph,
        &fixture.receipts,
        engine,
        database,
        tail,
    ) else {
        panic!("deterministic immutable decode damage must retain typed Blocked state");
    };
    assert_eq!(
        blocked.reason(),
        &LocalMutationBlockReason::Retained(RetainedBlockReason::PublishedAuthentication)
    );
    assert_eq!(
        blocked
            .continuation()
            .expect("the exact damaged publication remains retained")
            .batch_id(),
        batch_id
    );
    assert_eq!(engine.accepted_frontier_root().unwrap(), accepted_before);
    assert_eq!(engine.durable_history_authority().unwrap(), history_before);
    assert_eq!(database.frontier_root().unwrap(), sqlite_before);
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        projection_before
    );
    assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
    assert_eq!(
        published_immutable_digests(&fixture.archive_root),
        damaged_publication,
        "authentication retry must neither redraft nor republish"
    );
    assert!(
        take_last_admitted_local_author().is_none(),
        "authentication retry must not enter the local draft path"
    );
    assert_eq!(fixture.graph.handoff_release_count(), releases_before);
    assert!(
        fixture.graph.probe_managed_text_writer().is_err(),
        "the retained blocked continuation must keep the handoff latch closed"
    );
}

#[test]
fn transient_enrollment_read_keeps_published_local_recovery_exactly_resumable() {
    let mut fixture = Fixture::new(
        "published-local-transient-enrollment-read",
        None,
        vec![("pages/seed.md".into(), b"- seed\n".to_vec())],
    );
    let root = fixture.enrollment_root("published-local-transient-enrollment-read");
    let paths = PromotedPaths::new(&fixture, "published-local-transient-enrollment-read");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    let transaction = promoted_local_create_page(0xAB40_1000, "transient-enrollment-read");

    let accepted_before = runtime.engine().accepted_frontier_root().unwrap();
    let history_before = runtime.engine().durable_history_authority().unwrap();
    let sqlite_before = runtime.database().frontier_root().unwrap();
    let projection_before = promoted_projection_digests(&paths.database_path);
    let graph_before = snapshot_files(&fixture.graph_root);
    let releases_before = fixture.graph.handoff_release_count();

    fail_once_at(OperationalFaultPoint::AfterManifest);
    let mut window = runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .unwrap();
    let LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(continuation)) =
        OperationalCoordinator::execute_local(
            &mut window,
            &fixture.graph,
            &fixture.receipts,
            &transaction,
        )
    else {
        panic!("AfterManifest must return the genuine admitted-local continuation");
    };
    let batch_id = continuation.batch_id();
    let drafted =
        take_last_admitted_local_author().expect("the production local path drafted exactly once");
    assert_eq!(drafted.batch_id, batch_id);
    let publication = published_immutable_digests(&fixture.archive_root);

    let (admission, engine, database, tail) = window.parts().unwrap();
    fail_next_enrollment_head_read();
    let LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(continuation)) =
        continuation.retry(
            &admission,
            &fixture.graph,
            &fixture.receipts,
            engine,
            database,
            tail,
        )
    else {
        panic!("a transient enrollment read must remain Recovering");
    };
    assert_eq!(continuation.batch_id(), batch_id);
    assert_eq!(continuation.phase(), OperationalPhase::Bindings);
    assert_eq!(continuation.failure().retained_block_reason(), None);
    assert_eq!(engine.accepted_frontier_root().unwrap(), accepted_before);
    assert_eq!(engine.durable_history_authority().unwrap(), history_before);
    assert_eq!(database.frontier_root().unwrap(), sqlite_before);
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        projection_before
    );
    assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
    assert_eq!(
        published_immutable_digests(&fixture.archive_root),
        publication
    );
    assert_eq!(fixture.graph.handoff_release_count(), releases_before);
    assert!(fixture.graph.probe_managed_text_writer().is_err());
    assert!(take_last_admitted_local_author().is_none());

    let mut state = continuation.retry(
        &admission,
        &fixture.graph,
        &fixture.receipts,
        engine,
        database,
        tail,
    );
    let completion = loop {
        match state {
            LocalMutationCoordinatorState::Active(completion) => break completion,
            LocalMutationCoordinatorState::Recovering(LocalMutationRecovery::Published(
                continuation,
            )) => {
                state = continuation.retry(
                    &admission,
                    &fixture.graph,
                    &fixture.receipts,
                    engine,
                    database,
                    tail,
                );
            }
            LocalMutationCoordinatorState::Recovering(
                LocalMutationRecovery::ReconciliationRequired(reconciliation),
            ) => panic!(
                "exact continuation requested redraft reconciliation: {:?}",
                reconciliation.paths()
            ),
            LocalMutationCoordinatorState::Blocked(blocked) => {
                panic!(
                    "transient continuation became blocked: {}",
                    blocked.failure()
                )
            }
            LocalMutationCoordinatorState::Revoked(revoked) => {
                panic!(
                    "transient continuation became revoked: {}",
                    revoked.failure()
                )
            }
        }
    };
    assert_eq!(completion.batch_id(), batch_id);
    assert_eq!(
        published_immutable_digests(&fixture.archive_root),
        publication,
        "exact-continuation completion must not republish"
    );
    assert!(take_last_admitted_local_author().is_none());
    assert_eq!(fixture.graph.handoff_release_count(), releases_before + 1);
    fixture.graph.probe_managed_text_writer().unwrap();
}

/// The durable promotion state file inside one archive root.
fn promotion_state_path_in(archive_root: &Path, fixture: &Fixture) -> PathBuf {
    archive_root
        .join("engine-history")
        .join(
            fixture
                .authority
                .binding()
                .storage_binding()
                .endpoint
                .endpoint_id()
                .to_string(),
        )
        .join("promoted-runtime.state")
}

/// The durable promotion state file for one fixture archive.
fn promotion_state_path(fixture: &Fixture) -> PathBuf {
    promotion_state_path_in(&fixture.archive_root, fixture)
}

/// Recursively copy a directory tree, producing fresh directory inodes.
///
/// The copy is byte-identical and structurally identical, but every directory
/// in it is a distinct filesystem resource from its source. That distinction is
/// exactly what a retargeting attack cannot forge and what archive identity is
/// derived from.
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap().map(Result::unwrap) {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if fs::symlink_metadata(&from).unwrap().is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

/// The whole first-promotion boundary over a rich, nested, Unicode, CRLF,
/// multipart bootstrap: fenced before, writable after, byte-identical graph,
/// and an exactly resumable durable state.
#[test]
fn inactive_bootstrap_promotes_to_a_writable_runtime_and_resumes_exactly() {
    let mut fixture = rich_fixture("promote-rich");
    let root = fixture.enrollment_root("promote-rich");
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, "rich");
    let session = SessionId::new();

    // Fail-before: this archive cannot be opened as an ordinary runtime.
    assert_ordinary_enrolled_open_is_fenced(&fixture);
    assert!(!promotion_state_path(&fixture).exists());

    let authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let activated_head = enrollment_head(&root, &binding);
    let bootstrap_frontier = fixture
        .authority
        .accepted_engine()
        .accepted_frontier_root()
        .unwrap();
    let bootstrap_generation = fixture.authority.binding().history_generation();
    let bootstrap_root = fixture.authority.binding().history_root();
    assert!(
        fixture.verified.part_count() >= 1,
        "rich fixture is nonempty"
    );

    // Phase one is idempotent: repeating it with the same inputs resumes
    // against the identical committed bytes.
    let _first =
        seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime()).unwrap();
    let state_bytes = fs::read(promotion_state_path(&fixture)).unwrap();
    let sealed =
        seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime()).unwrap();
    assert_eq!(
        fs::read(promotion_state_path(&fixture)).unwrap(),
        state_bytes,
        "an idempotent resume must not rewrite the committed promotion state"
    );

    let bootstrap = fixture.take_bootstrap_session();
    let runtime = bootstrap
        .promote(sealed, &authority, &paths.open(&fixture))
        .map_err(|refusal| refusal.into_parts().1)
        .unwrap();

    // The promoted runtime is the bootstrap's own lineage at its own frontier.
    assert_eq!(
        runtime.bootstrap_anchor().generation,
        bootstrap_generation,
        "the promoted anchor must be the exact bootstrap history generation"
    );
    assert_eq!(runtime.bootstrap_anchor().index_root, bootstrap_root);
    // Every authenticated field of the frontier must be reproduced exactly.
    // `scratch_root` locates the run-local scratch LSM page holding this
    // frontier's point index; its file offset is reconstructible derived state
    // that legitimately differs between the inactive and promoted runs.
    assert!(
        runtime
            .engine()
            .accepted_frontier_root()
            .unwrap()
            .same_accepted_authority(&bootstrap_frontier),
        "promotion must reproduce the exact accepted bootstrap frontier"
    );
    assert_eq!(
        runtime
            .engine()
            .accepted_frontier_root()
            .unwrap()
            .reference_catalog_root(),
        bootstrap_frontier.reference_catalog_root(),
        "the promoted catalog root is the exact bootstrap catalog root"
    );
    assert!(runtime
        .database()
        .frontier_root()
        .unwrap()
        .same_accepted_authority(&bootstrap_frontier));
    assert_eq!(runtime.session_id(), session);
    assert_eq!(
        runtime.verification_digest(),
        authority.verification_digest()
    );

    // Promotion advanced no enrollment state and wrote no graph byte.
    assert_eq!(enrollment_head(&root, &binding), activated_head);
    fixture.assert_graph_unchanged();
}

/// A zero-file graph promotes exactly like a populated one: an empty bootstrap
/// anchor is a legitimate anchor, not a missing one.
#[test]
fn a_zero_part_bootstrap_promotes_at_the_empty_anchor() {
    let mut fixture = Fixture::new("promote-zero", None, Vec::new());
    let root = fixture.enrollment_root("promote-zero");
    let paths = PromotedPaths::new(&fixture, "zero");
    assert_eq!(fixture.verified.part_count(), 0);

    let (_authority, runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    assert_eq!(runtime.bootstrap_anchor().generation, 0);
    assert_eq!(
        runtime
            .engine()
            .accepted_frontier_root()
            .unwrap()
            .acceptance_sequence(),
        0
    );
    fixture.assert_graph_unchanged();
}

/// Every durable cut of the one-time promotion publication reopens as either
/// the unchanged inactive bootstrap or the one exact resumable promoted state.
/// Partial, truncated, and foreign residue fails closed and is preserved.
#[test]
fn promotion_state_residue_fails_closed_and_preserves_evidence() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "promote-residue",
            None,
            vec![("pages/residue.md".into(), b"- residue\n".to_vec())],
        );
        let root = fixture.enrollment_root("promote-residue");
        let paths = PromotedPaths::new(&fixture, "residue");
        let session = SessionId::new();
        let authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();

        // The pre-publication cut: no state file at all reopens as the unchanged
        // inactive bootstrap, and a promoted open refuses.
        let state_path = promotion_state_path(&fixture);
        assert!(!state_path.exists());
        let binding = fixture.enrollment_binding();
        assert!(
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).is_err(),
            "an unpromoted archive must never open a promoted runtime"
        );

        let sealed =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let committed = fs::read(&state_path).unwrap();

        // Truncated residue at every prefix fails closed rather than being
        // repaired or partially believed.
        for cut in [0, 1, committed.len() / 2, committed.len() - 1] {
            fs::write(&state_path, &committed[..cut]).unwrap();
            assert!(
                reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture))
                    .is_err(),
                "a truncated promotion state must fail closed at cut {cut}"
            );
            assert!(state_path.exists(), "evidence must be preserved");
        }

        // A byte-flipped, non-canonical, or foreign claim also fails closed.
        let mut corrupt = committed.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0xff;
        fs::write(&state_path, &corrupt).unwrap();
        assert!(
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).is_err()
        );

        // Restoring the exact committed bytes resumes the one promoted state.
        fs::write(&state_path, &committed).unwrap();
        let bootstrap = fixture.take_bootstrap_session();
        let runtime = bootstrap
            .promote(sealed, &authority, &paths.open(&fixture))
            .map_err(|refusal| refusal.into_parts().1)
            .unwrap();
        assert_eq!(runtime.session_id(), session);
        fixture.assert_graph_unchanged();
    });
}

/// A restarted process holds no evidence, no authority, no sealed promotion,
/// and no engine identity. It reconstructs everything from durable state.
#[test]
fn a_fresh_process_reopens_the_promoted_runtime_with_no_retained_evidence() {
    on_a_deep_stack(move || {
        let mut fixture = Fixture::new(
            "promote-restart",
            None,
            vec![("pages/restart.md".into(), b"- restart\n".to_vec())],
        );
        let root = fixture.enrollment_root("promote-restart");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "restart");
        let session = SessionId::new();

        let (authority, runtime) = promote(&mut fixture, &root, session, &paths);
        let anchor = runtime.bootstrap_anchor();
        let frontier = runtime.engine().accepted_frontier_root().unwrap();
        // The previous process is gone: every process-local value dies with it.
        drop(runtime);
        drop(authority);

        // A competing session is refused before any archive or lease work.
        assert!(
            reopen_promoted_local_runtime(&root, &binding, SessionId::new(), &paths.open(&fixture))
                .is_err(),
            "a crash resumes Unsafe for exactly the committed session"
        );

        let (reopened_authority, reopened) =
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).unwrap();
        assert_eq!(reopened.bootstrap_anchor(), anchor);
        assert_eq!(
            reopened.engine().accepted_frontier_root().unwrap(),
            frontier
        );
        assert_eq!(reopened.database().frontier_root().unwrap(), frontier);
        assert_eq!(
            reopened_authority.handoff(),
            LocalActiveHandoff::Unsafe {
                session_id: session
            },
            "a crash remains Unsafe"
        );
        assert_eq!(reopened_authority.session_id(), session);
        fixture.assert_graph_unchanged();
    });
}

/// Ordinary local batches extend the bootstrap without ever making it
/// unverifiable. After a restart the exact bootstrap ancestor is still proved,
/// the current frontier is reopened, and one more mutation is admitted.
#[test]
fn local_batches_extend_the_bootstrap_anchor_and_restart_proves_exact_ancestry() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "promote-append",
            None,
            vec![("pages/seed.md".into(), b"- seed\n".to_vec())],
        );
        let root = fixture.enrollment_root("promote-append");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "append");
        let session = SessionId::new();

        let (mut authority, mut runtime) = promote(&mut fixture, &root, session, &paths);
        let anchor = runtime.bootstrap_anchor();
        let anchor_frontier = runtime.engine().accepted_frontier_root().unwrap();

        append_local_batch(&fixture, &mut authority, &mut runtime, 0x9200);
        let after_one = runtime.engine().durable_history_authority().unwrap();
        assert!(
            after_one.generation > anchor.generation,
            "an ordinary local batch must extend the durable history"
        );
        append_local_batch(&fixture, &mut authority, &mut runtime, 0x9300);
        append_local_batch(&fixture, &mut authority, &mut runtime, 0x9400);
        let advanced = runtime.engine().durable_history_authority().unwrap();
        let advanced_frontier = runtime.engine().accepted_frontier_root().unwrap();
        assert_eq!(advanced.generation, anchor.generation + 3);
        assert!(advanced_frontier.acceptance_sequence() > anchor_frontier.acceptance_sequence());

        // The advanced history is still an authenticated descendant of the exact
        // bootstrap anchor, proved from the shared radix structure.
        let transition = runtime
            .engine()
            .authenticate_history_descends_from(anchor)
            .unwrap();
        assert_eq!(transition.before(), anchor);
        assert_eq!(transition.after(), advanced);

        drop(runtime);
        drop(authority);

        // Restart: the anchor is reconstructed from durable state alone, the live
        // history is proved to descend from it, and the current frontier reopens.
        let (mut authority, mut runtime) =
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).unwrap();
        assert_eq!(
            runtime.bootstrap_anchor(),
            anchor,
            "restart must reconstruct the exact original bootstrap anchor"
        );
        assert_eq!(
            runtime.engine().durable_history_authority().unwrap(),
            advanced
        );
        assert!(
            runtime
                .engine()
                .accepted_frontier_root()
                .unwrap()
                .same_accepted_authority(&advanced_frontier),
            "an adopted run may relocate the scratch page but must retain the exact accepted authority"
        );
        assert!(
            runtime
                .database()
                .frontier_root()
                .unwrap()
                .same_accepted_authority(&advanced_frontier),
            "SQLite must reopen the exact accepted authority"
        );

        // One more mutation is admitted after the restart.
        append_local_batch(&fixture, &mut authority, &mut runtime, 0x9500);
        assert_eq!(
            runtime
                .engine()
                .durable_history_authority()
                .unwrap()
                .generation,
            anchor.generation + 4
        );
        assert_eq!(
            enrollment_head(&root, &binding),
            authority.enrollment_head()
        );
        fixture.assert_graph_unchanged();
    });
}

/// The original `VerifiedLocal` bootstrap proof stays reopenable after the
/// promoted history advances, and the retained immutable publication is
/// unchanged.
#[test]
fn the_original_bootstrap_anchor_stays_reopenable_after_the_history_advances() {
    let mut fixture = Fixture::new(
        "promote-anchor",
        None,
        vec![("pages/anchor.md".into(), b"- anchor\n".to_vec())],
    );
    let root = fixture.enrollment_root("promote-anchor");
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, "anchor");
    let session = SessionId::new();

    let before = crate::oplog::enrollment::reopen_promoted_bootstrap_anchor(&root, &binding);
    assert!(
        before.is_err(),
        "no LocalActive record exists before activation"
    );

    let (mut authority, mut runtime) = promote(&mut fixture, &root, session, &paths);
    let anchor = crate::oplog::enrollment::reopen_promoted_bootstrap_anchor(&root, &binding)
        .expect("the committed anchor must be reconstructible immediately after promotion");
    let anchor_generation = anchor.history_generation();
    let anchor_root = anchor.history_root();
    let anchor_digest = anchor.verification_digest();

    for seed in [0x9600_u128, 0x9700, 0x9800] {
        append_local_batch(&fixture, &mut authority, &mut runtime, seed);
    }
    assert!(
        runtime
            .engine()
            .durable_history_authority()
            .unwrap()
            .generation
            > anchor_generation
    );

    let after = crate::oplog::enrollment::reopen_promoted_bootstrap_anchor(&root, &binding)
        .expect("the bootstrap anchor must stay reopenable after the history advances");
    assert_eq!(after.history_generation(), anchor_generation);
    assert_eq!(after.history_root(), anchor_root);
    assert_eq!(after.verification_digest(), anchor_digest);
    assert_eq!(
        after.bootstrap_part_count(),
        fixture.verified.part_count(),
        "the retained immutable publication identity is unchanged"
    );
    fixture.assert_graph_unchanged();
}

/// A promoted admission requires *both* the live authority and the exact
/// promoted runtime. Substituted graphs, engines, and enrollments are refused
/// before any durable or graph mutation.
#[test]
fn a_promoted_admission_rejects_substituted_runtime_components() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "promote-substitute",
            None,
            vec![("pages/subject.md".into(), b"- subject\n".to_vec())],
        );
        let root = fixture.enrollment_root("promote-substitute");
        let paths = PromotedPaths::new(&fixture, "substitute");
        let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);

        let mut foreign = Fixture::new(
            "promote-substitute-foreign",
            None,
            vec![("pages/foreign.md".into(), b"- foreign\n".to_vec())],
        );
        let foreign_root = foreign.enrollment_root("substitute-foreign");
        let foreign_paths = PromotedPaths::new(&foreign, "substitute-foreign");
        let (mut foreign_authority, mut foreign_runtime) = promote(
            &mut foreign,
            &foreign_root,
            SessionId::new(),
            &foreign_paths,
        );

        let advanced_before = runtime.engine().durable_history_authority().unwrap();
        let foreign_before = foreign_runtime
            .engine()
            .durable_history_authority()
            .unwrap();

        // A foreign graph is refused.
        assert!(runtime
            .admit_promoted_mutation(&mut authority, &foreign.graph)
            .is_err());
        // A foreign authority is refused for this runtime.
        assert!(runtime
            .admit_promoted_mutation(&mut foreign_authority, &fixture.graph)
            .is_err());
        // A foreign runtime is refused for this authority.
        assert!(foreign_runtime
            .admit_promoted_mutation(&mut authority, &foreign.graph)
            .is_err());

        // A genuine admission refuses a substituted engine, including one built
        // over the very same enrolled identity.
        {
            let session = runtime
                .admit_promoted_mutation(&mut authority, &fixture.graph)
                .unwrap();
            let admission = session.admission();
            let substitute = fixture.runtime_engine("substitute");
            assert!(
                admission.authorize(&fixture.graph, &substitute).is_err(),
                "a same-identity engine from another history must be refused"
            );
            assert!(admission
                .authorize(&foreign.graph, foreign_runtime.engine())
                .is_err());
        }

        assert_eq!(
            runtime.engine().durable_history_authority().unwrap(),
            advanced_before,
            "a refused admission must advance no durable history"
        );
        assert_eq!(
            foreign_runtime
                .engine()
                .durable_history_authority()
                .unwrap(),
            foreign_before
        );
        fixture.assert_graph_unchanged();
        foreign.assert_graph_unchanged();
    });
}

/// The bootstrap-anchor ancestry proof is bounded. An unchanged history costs
/// zero radix node reads, and a point extension costs the changed paths, never
/// the lifetime record count.
#[test]
fn the_bootstrap_ancestry_proof_is_bounded_by_the_changed_radix_paths() {
    let mut fixture = rich_fixture("promote-bounded");
    let root = fixture.enrollment_root("promote-bounded");
    let paths = PromotedPaths::new(&fixture, "bounded");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    let anchor = runtime.bootstrap_anchor();
    let archive = runtime
        .engine()
        .archive_store()
        .expect("the promoted engine retains its archive")
        .instrumentation();

    // Exact, unchanged history: the shared subtree terminates immediately.
    runtime
        .engine()
        .authenticate_history_descends_from(anchor)
        .unwrap();
    let unchanged = runtime.engine().archive_store().unwrap().instrumentation();
    assert_eq!(
        unchanged.history_index_reads, archive.history_index_reads,
        "an exact-history proof must read no radix nodes at all"
    );

    // The rich fixture configures `notes` as its pages directory.
    append_local_batch_at(&fixture, &mut authority, &mut runtime, 0x9900, "notes");
    let before_point = runtime
        .engine()
        .archive_store()
        .unwrap()
        .instrumentation()
        .history_index_reads;
    runtime
        .engine()
        .authenticate_history_descends_from(anchor)
        .unwrap();
    let point = runtime
        .engine()
        .archive_store()
        .unwrap()
        .instrumentation()
        .history_index_reads
        - before_point;
    // One inserted record touches one root-to-leaf path on each side.
    assert!(
        point
            <= 2 * (u64::from(crate::oplog::object_store::ENGINE_HISTORY_RADIX_DEPTH) + 1) as usize,
        "a point extension proof read {point} radix nodes"
    );
    fixture.assert_graph_unchanged();
}

/// The construction regression for the promoted reference catalog.
///
/// A non-empty inactive bootstrap binds a `reference_catalog_root` into every
/// accepted cold record. That root has exactly one construction — the target
/// archive's durable authenticated Patricia store — so every bound root must be
/// fully openable from a *fresh* archive open that holds no process-local
/// engine, candidate, or in-memory catalog, both while the bootstrap is still
/// inactive and after the runtime is promoted and its history has advanced.
///
/// Fail-before: authoring the bootstrap against the run-local ephemeral catalog
/// backend produced flat in-memory digests instead of Patricia roots, so this
/// validation failed with a missing authenticated node and promotion could
/// never open a non-empty bootstrap.
#[test]
fn a_non_empty_bootstrap_catalog_root_opens_from_a_fresh_archive_before_and_after_promotion() {
    // A genuinely multipart bootstrap, so more than one accepted cold record
    // binds a catalog root, over content that produces real reference sources.
    let mut multipart = String::from("title:: Catalog root\n\n");
    for ordinal in 0..4096 {
        // Deliberately syntax-free: the operation count alone forces a second
        // part, while reference evidence stays on the two pages below so the
        // catalog walk here is not an accidental 4096-target benchmark.
        multipart.push_str(&format!("- operation {ordinal:04}\n"));
    }
    force_next_bootstrap_part_operation_limit(4_096);
    let mut fixture = Fixture::new(
        "promote-catalog-root",
        None,
        vec![
            ("pages/multipart.md".into(), multipart.into_bytes()),
            (
                "pages/référence.md".into(),
                "- see [[Catalog root]] and #tag\r\n".as_bytes().to_vec(),
            ),
        ],
    );
    let root = fixture.enrollment_root("promote-catalog-root");
    let paths = PromotedPaths::new(&fixture, "catalog-root");
    assert!(
        fixture.verified.part_count() >= 2,
        "the fixture must bind more than one cold record: {}",
        fixture.verified.part_count()
    );

    // Every root bound by an accepted cold record, validated completely against
    // a freshly opened durable catalog.
    fn assert_every_bound_catalog_root_opens(fixture: &Fixture, label: &str) {
        let catalog = fixture.archive().open_reference_catalog().unwrap();
        let materials = fixture.prepared.engine_materials();
        assert_eq!(materials.len(), fixture.verified.part_count() as usize);
        let mut covered = 0;
        for material in materials {
            let bound = material.reference_catalog_root();
            catalog
                .validate_catalog_root(bound)
                .unwrap_or_else(|error| {
                    panic!("{label}: a bound bootstrap catalog root is not durable: {error}")
                });
            covered = covered.max(bound.source_count());
        }
        assert!(
            covered > 0,
            "{label}: the bootstrap covers reference sources"
        );
    }

    // Before promotion: the inactive bootstrap's own bound roots.
    assert_every_bound_catalog_root_opens(&fixture, "inactive bootstrap");

    let session = SessionId::new();
    let (mut authority, mut runtime) = promote(&mut fixture, &root, session, &paths);

    // After promotion, and after the history advances past the bootstrap.
    for seed in [0xA100_u128, 0xA200] {
        append_local_batch(&fixture, &mut authority, &mut runtime, seed);
    }
    assert!(
        runtime
            .engine()
            .durable_history_authority()
            .unwrap()
            .generation
            > u64::from(fixture.verified.part_count())
    );
    let advanced = runtime.engine().reference_catalog_root().unwrap().clone();
    drop(runtime);
    drop(authority);

    assert_every_bound_catalog_root_opens(&fixture, "promoted and advanced");
    // The live advanced catalog root is durable too, from a fresh archive open.
    fixture
        .archive()
        .open_reference_catalog()
        .unwrap()
        .validate_catalog_root(&advanced)
        .expect("the advanced promoted catalog root opens from a fresh archive");
    fixture.assert_graph_unchanged();
}

/// The promoted runtime token, its sealed promotion, and its mutation window
/// are opaque: no clone, no serde, and no way to reconstruct one from bytes.
#[test]
fn promoted_runtime_values_cannot_be_cloned_serialized_or_deserialized() {
    assert!(!Probe::<PromotedLocalRuntime>::CLONEABLE);
    assert!(!SerdeProbe::<PromotedLocalRuntime>::SERIALIZABLE);
    assert!(!DeserializeProbe::<PromotedLocalRuntime>::DESERIALIZABLE);
    assert!(!Probe::<super::SameProcessPromotionToken>::CLONEABLE);
    assert!(!SerdeProbe::<super::SameProcessPromotionToken>::SERIALIZABLE);
    assert!(!DeserializeProbe::<super::SameProcessPromotionToken>::DESERIALIZABLE);
    assert!(!Probe::<crate::oplog::import::RetainedBootstrapPromotionCandidate>::CLONEABLE);
    assert!(!SerdeProbe::<crate::oplog::import::RetainedBootstrapPromotionCandidate>::SERIALIZABLE);
    assert!(!DeserializeProbe::<
        crate::oplog::import::RetainedBootstrapPromotionCandidate,
    >::DESERIALIZABLE);
    assert!(!Probe::<SealedRuntimePromotion>::CLONEABLE);
    assert!(!SerdeProbe::<SealedRuntimePromotion>::SERIALIZABLE);
    assert!(!DeserializeProbe::<SealedRuntimePromotion>::DESERIALIZABLE);
    assert!(!Probe::<LocalRuntimeAdmission<'static>>::CLONEABLE);
    assert!(!SerdeProbe::<LocalRuntimeAdmission<'static>>::SERIALIZABLE);
    // The positive control proves the probe actually discriminates.
    assert!(Probe::<ContentDigest>::CLONEABLE);
    assert!(SerdeProbe::<ContentDigest>::SERIALIZABLE);
}

/// The pre-activation escape hatch can never authorize a promoted runtime.
///
/// It is `pub(crate)`, so no app or Tauri code can name it at all. This proves
/// the second, structural fence: even from inside the crate it refuses a
/// promoted engine, so a real activated user graph is only ever writable
/// through the authority-plus-runtime admission.
#[test]
fn the_pre_activation_admission_refuses_a_promoted_runtime_engine() {
    let mut fixture = Fixture::new(
        "promote-hatch",
        None,
        vec![("pages/hatch.md".into(), b"- hatch\n".to_vec())],
    );
    let root = fixture.enrollment_root("promote-hatch");
    let paths = PromotedPaths::new(&fixture, "hatch");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);

    let hatch = LocalRuntimeAdmission::unenrolled_pre_activation();
    assert!(
        hatch.authorize(&fixture.graph, runtime.engine()).is_err(),
        "the pre-activation hatch must never authorize a promoted runtime"
    );

    // An unpromoted fixture engine still passes, so the refusal is specific.
    let unpromoted = fixture.runtime_engine("hatch-unpromoted");
    assert!(hatch.authorize(&fixture.graph, &unpromoted).is_ok());

    // The genuine promoted admission still works, and the refused hatch
    // advanced no durable history.
    let before = runtime.engine().durable_history_authority().unwrap();
    append_local_batch(&fixture, &mut authority, &mut runtime, 0xA300);
    assert_eq!(
        runtime
            .engine()
            .durable_history_authority()
            .unwrap()
            .generation,
        before.generation + 1
    );
    fixture.assert_graph_unchanged();
}

/// Promotion publication is bound to the exact retained archive capability,
/// never to whatever currently answers to the archive's pathname.
///
/// The positive control proves the ordinary same-capability seal and its
/// readback still commit one exact immutable state and resume idempotently. The
/// negative case is the retargeting cut: an archive renamed while its retained
/// capability stays open, with a byte-identical recursive copy left at the old
/// pathname. That copy is a perfect forgery of everything content-addressed —
/// identical durable history, identical bootstrap publication, identical
/// canonical archive-resource claim bytes — and differs only in physical
/// directory identity. Publication must not durably land in either directory.
#[test]
fn promotion_publication_binds_the_exact_retained_archive_capability() {
    // --- positive control: ordinary same-capability seal and readback --------
    let control = Fixture::new(
        "promote-capability-control",
        None,
        vec![("pages/control.md".into(), b"- control\n".to_vec())],
    );
    let control_root = control.enrollment_root("promote-capability-control");
    let control_binding = control.enrollment_binding();
    let control_session = SessionId::new();
    let control_authority = activate_verified_local(
        &control_root,
        control.compose(&control_root),
        control_session,
        &control.proofs(),
        &control.runtime(),
    )
    .unwrap();
    let control_state = promotion_state_path(&control);
    assert!(!control_state.exists());

    let control_head = enrollment_head(&control_root, &control_binding);
    seal_local_runtime_promotion(&control_authority, &control.proofs(), &control.runtime())
        .unwrap();
    let committed = fs::read(&control_state).unwrap();
    // The readback inside the seal is a genuine fresh durable-history open over
    // the same retained capability, and repeating phase one resumes against the
    // identical committed bytes rather than rewriting them.
    seal_local_runtime_promotion(&control_authority, &control.proofs(), &control.runtime())
        .unwrap();
    assert_eq!(fs::read(&control_state).unwrap(), committed);
    assert_eq!(
        enrollment_head(&control_root, &control_binding),
        control_head
    );
    control.assert_graph_unchanged();
    drop(control_authority);

    // --- the retargeting cut -------------------------------------------------
    let mut fixture = Fixture::new(
        "promote-retarget",
        None,
        vec![("pages/retarget.md".into(), b"- retarget\n".to_vec())],
    );
    let root = fixture.enrollment_root("promote-retarget");
    let binding = fixture.enrollment_binding();
    let session = SessionId::new();
    let authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let activated_head = enrollment_head(&root, &binding);

    // Archive A is renamed while every retained capability in the proof set
    // stays open on it; a byte-identical copy B then takes its old pathname.
    let retained = fixture.archive_root.clone();
    let renamed = fixture.root.path().join("archive-renamed");
    fs::rename(&retained, &renamed).unwrap();
    copy_tree(&renamed, &retained);
    assert_eq!(
        snapshot_file_digests(&renamed),
        snapshot_file_digests(&retained),
        "the stale copy must be byte-identical, so only directory identity differs"
    );
    fixture.archive_root = renamed.clone();

    let retained_before = snapshot_file_digests(&renamed);
    let stale_before = snapshot_file_digests(&retained);
    let error = seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
        .err()
        .expect("an ambiguous archive must block promotion before publication");
    assert!(
        matches!(
            error,
            RuntimePromotionError::Activation(LocalActivationError::RuntimeBinding(_))
        ),
        "unexpected retargeting error: {error}"
    );

    // Neither archive may gain a promotion-state file, and neither archive's
    // durable history may have moved at all.
    assert!(
        !promotion_state_path_in(&renamed, &fixture).exists(),
        "the retained archive must not have been published into"
    );
    assert!(
        !promotion_state_path_in(&retained, &fixture).exists(),
        "the stale look-alike archive must not have been published into"
    );
    assert_eq!(snapshot_file_digests(&renamed), retained_before);
    assert_eq!(snapshot_file_digests(&retained), stale_before);
    assert_eq!(enrollment_head(&root, &binding), activated_head);
    fixture.assert_graph_unchanged();
}

/// The promoted-state authorization boundary itself refuses a foreign archive.
///
/// A restarted process holds no retained capability, so it must open the
/// configured archive pathname. Centralizing the exact archive binding inside
/// the durable-history control means the refusal happens at the state read —
/// before any promoted engine, projection-work index, SQLite lease, or replay
/// exists — rather than only later, at the promoted-runtime mint. A later
/// caller that reaches promoted state some other way inherits the same refusal.
#[test]
fn a_byte_identical_copy_of_a_promoted_archive_is_refused_at_the_state_boundary() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "promote-copied-archive",
            None,
            vec![("pages/copied.md".into(), b"- copied\n".to_vec())],
        );
        let root = fixture.enrollment_root("promote-copied-archive");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "copied");
        let session = SessionId::new();
        let (authority, runtime) = promote(&mut fixture, &root, session, &paths);
        drop(runtime);
        drop(authority);

        // A byte-identical recursive copy of the whole promoted archive, including
        // its committed promotion state. Only directory identity differs.
        let copy = fixture.root.path().join("archive-copy");
        copy_tree(&fixture.archive_root, &copy);
        assert_eq!(
            snapshot_file_digests(&fixture.archive_root),
            snapshot_file_digests(&copy)
        );
        assert!(promotion_state_path_in(&copy, &fixture).exists());

        let copied_paths = PromotedPaths::new(&fixture, "copied-target");
        fixture.archive_root = copy.clone();
        let error =
            reopen_promoted_local_runtime(&root, &binding, session, &copied_paths.open(&fixture))
                .err()
                .expect("a foreign archive must never adopt another archive's promotion state");
        assert!(
            matches!(
                error,
                RuntimePromotionError::Store(
                    crate::oplog::StoreError::PromotedRuntimeStateMismatch(_)
                )
            ),
            "the refusal must come from the promoted-state boundary, not a later mint: {error}"
        );
        // The refusal happened before any promoted runtime existed, so the copy
        // gained no device-local projection at all.
        assert!(!copied_paths.database_path.exists());
        fixture.assert_graph_unchanged();
    });
}

/// The promoted-state boundary authenticates the canonical archive-resource
/// claim, not only the physical archive directory identity.
///
/// `a_byte_identical_copy_of_a_promoted_archive_is_refused_at_the_state_boundary`
/// moves the archive to a new directory, so it can only exercise the
/// control-directory half of `require_promoted_state_binding`. This is the
/// residual half: the archive keeps its exact physical directory identity — no
/// directory is created, moved, or replaced — while its canonical
/// archive-resource claim goes missing, becomes corrupt, or is replaced by a
/// different canonical instance claim. Each case must still fail closed inside
/// `require_promoted_state_binding`, at the resource-claim check specifically,
/// before any promoted engine, SQLite projection, or replay is constructed, and
/// without moving the graph, the enrollment, the durable history, or the
/// committed promotion state.
#[test]
fn a_promoted_archive_with_an_unauthenticated_resource_claim_is_refused_at_the_state_boundary() {
    crate::test_support::run_on_deep_stack(|| {
        // Deleted outright.
        assert_promoted_reopen_refuses_a_tampered_archive_claim("missing", |path| {
            fs::remove_file(path).unwrap();
        });
        // Present and same-length-bounded, but no longer decodable.
        assert_promoted_reopen_refuses_a_tampered_archive_claim("corrupt", |path| {
            let mut bytes = fs::read(path).unwrap();
            bytes.truncate(bytes.len() / 2);
            assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_err());
            fs::write(path, &bytes).unwrap();
        });
        // A perfectly well-formed, canonical claim — for a different instance.
        assert_promoted_reopen_refuses_a_tampered_archive_claim("divergent", |path| {
            let divergent = divergent_archive_instance_claim(&fs::read(path).unwrap());
            fs::write(path, &divergent).unwrap();
        });
    });
}

/// Rewrite a canonical archive-instance claim to a different instance id.
///
/// Only the UUID text is substituted in place, so the result stays exactly as
/// canonical and schema-valid as the original: what fails is the derived
/// resource identity, not the encoding.
fn divergent_archive_instance_claim(bytes: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).expect("the archive instance claim is canonical JSON");
    const MARKER: &str = "\"instance_id\":\"";
    let start = text.find(MARKER).expect("the claim carries an instance id") + MARKER.len();
    let end = start + 36;
    text[start..end]
        .parse::<Uuid>()
        .expect("the claimed instance id is a UUID");
    let replacement = Uuid::new_v4().to_string();
    assert_eq!(replacement.len(), 36);
    let divergent = format!("{}{replacement}{}", &text[..start], &text[end..]).into_bytes();
    assert_ne!(divergent, bytes);
    assert_eq!(divergent.len(), bytes.len());
    divergent
}

/// Durable byte identity of one promoted device-local SQLite projection.
///
/// The volatile `-shm` sidecar is deliberately excluded, exactly as
/// [`durable_sqlite_digests`] does; everything committed lives in the database
/// file and its write-ahead log.
fn promoted_projection_digests(database_path: &Path) -> BTreeMap<String, ContentDigest> {
    let mut digests = BTreeMap::new();
    for suffix in ["", "-wal"] {
        let path = PathBuf::from(format!("{}{suffix}", database_path.display()));
        if let Ok(bytes) = fs::read(&path) {
            digests.insert(suffix.to_owned(), ContentDigest::of(&bytes));
        }
    }
    digests
}

/// Promote, prove the untampered reopen works, then tamper only the canonical
/// archive-resource claim and prove the same reopen fails closed and inert.
fn assert_promoted_reopen_refuses_a_tampered_archive_claim(
    label: &str,
    tamper: impl FnOnce(&Path),
) {
    let mut fixture = Fixture::new(
        &format!("promote-claim-{label}"),
        None,
        vec![(
            format!("pages/{label}.md").into(),
            format!("- {label} claim\n").into_bytes(),
        )],
    );
    let root = fixture.enrollment_root(&format!("promote-claim-{label}"));
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, label);
    let session = SessionId::new();
    let (authority, runtime) = promote(&mut fixture, &root, session, &paths);
    drop(runtime);
    drop(authority);

    // Necessity gate: this exact reopen succeeds while the claim is intact, so
    // the refusals below are caused by the tampered claim and nothing else.
    let (control_authority, control_runtime) =
        reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).unwrap();
    drop(control_runtime);
    drop(control_authority);

    let claim_path = fixture.archive_root.join(ARCHIVE_INSTANCE_CLAIM_FILE);
    assert!(
        claim_path.is_file(),
        "a promoted archive must carry its canonical resource claim at {}",
        claim_path.display()
    );
    tamper(&claim_path);

    // Everything the refused reopen must leave exactly as it found it, sampled
    // after the tamper so only the reopen's own effects can move it.
    let archive_before = snapshot_file_digests(&fixture.archive_root);
    let projection_before = promoted_projection_digests(&paths.database_path);
    assert!(
        !projection_before.is_empty(),
        "the promoted projection must exist before the refused reopen"
    );
    let enrollment_before = enrollment_head(&root, &binding);
    let generation_before = enrollment_generation(&root, &binding);
    let promotion_state_before = fs::read(promotion_state_path(&fixture)).unwrap();

    let error = reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture))
        .err()
        .unwrap_or_else(|| {
            panic!("a {label} archive-resource claim must never authorize a promoted reopen")
        });
    // The control-directory check runs first and passed — the directory identity
    // is genuinely unchanged — so the refusal is the resource-claim check.
    assert!(
        matches!(
            &error,
            RuntimePromotionError::Store(crate::oplog::StoreError::PromotedRuntimeStateMismatch(
                message
            )) if *message == "promoted runtime state archive resource claim does not authenticate"
        ),
        "the {label} claim must be refused at the promoted-state resource-claim check: {error}"
    );

    // Nothing may have moved: not the archive's durable history, not the
    // committed promotion state, not the enrollment, not the device-local
    // projection, and not Martin's graph.
    assert_eq!(
        snapshot_file_digests(&fixture.archive_root),
        archive_before,
        "the refused reopen must leave the {label} archive byte-identical"
    );
    assert_eq!(
        fs::read(promotion_state_path(&fixture)).unwrap(),
        promotion_state_before,
        "the committed promotion state must survive the {label} refusal as evidence"
    );
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        projection_before
    );
    assert_eq!(enrollment_head(&root, &binding), enrollment_before);
    assert_eq!(enrollment_generation(&root, &binding), generation_before);
    fixture.assert_graph_unchanged();

    // The refusal precedes construction, not merely mutation: pointed at a
    // never-used runtime root and database path, the same call still fails and
    // creates no projection at all.
    let fresh = PromotedPaths::new(&fixture, &format!("{label}-unbuilt"));
    let fresh_error =
        reopen_promoted_local_runtime(&root, &binding, session, &fresh.open(&fixture))
            .err()
            .unwrap_or_else(|| {
                panic!("a {label} archive-resource claim must never build a runtime")
            });
    assert!(matches!(
        fresh_error,
        RuntimePromotionError::Store(crate::oplog::StoreError::PromotedRuntimeStateMismatch(_))
    ));
    assert!(
        !fresh.database_path.exists(),
        "no SQLite projection may be constructed for the {label} refusal"
    );
    assert_eq!(snapshot_file_digests(&fixture.archive_root), archive_before);
    assert_eq!(enrollment_head(&root, &binding), enrollment_before);
    fixture.assert_graph_unchanged();
}

/// Uninterrupted promotion migrates its accepted candidate without replay;
/// fresh-process recovery reconstructs the same candidate from immutable parts
/// and adopts it as an authenticated bootstrap checkpoint.
///
/// Restart resident memory must be one bootstrap part, not the whole graph, so
/// the same-process promoted engine must report zero payload reads and zero
/// replayed generations. A fresh-process reopen that holds no retained evidence
/// must reconstruct the bootstrap once before entering ordinary enrolled
/// recovery.
///
/// The counter measures payload *ownership*, not a staging bracket, and this
/// test proves that executably rather than by assertion: the test-only preload
/// probe holds every loaded/prepared part at once through exactly the
/// production residency wrappers and must observe `max_live == part_count`,
/// while the production replay of the same publication observes exactly one.
///
/// Fail-before: recovery built a `Vec<PreparedBatch>` containing every part and
/// all of its objects before staging any of them, so the observed maximum was
/// the whole part count. The probe reproduces exactly that shape on demand, so
/// an instrument that could not see it would fail here.
#[test]
fn promoted_recovery_streams_exactly_one_bootstrap_part_at_a_time() {
    on_a_deep_stack(|| {
        // Two operations per part over a six-operation graph partitions into
        // exactly three parts, deterministically and without a four-thousand-block
        // fixture. Only the partition boundary is forced: every part is authored,
        // published, installed, and replayed through the ordinary path.
        force_next_bootstrap_part_operation_limit(2);
        let mut fixture = Fixture::new(
            "promote-stream-parts",
            None,
            vec![
                (
                    "pages/anchor.md".into(),
                    b"title:: Streamed anchor\n\n- one\n- two\n- three\n".to_vec(),
                ),
                // Real reference evidence, so the promoted open performs the
                // authenticated recovery replay rather than skipping it.
                (
                    "pages/referrer.md".into(),
                    "- see [[Streamed anchor]] and #tag\n".as_bytes().to_vec(),
                ),
            ],
        );
        let part_count = fixture.verified.part_count() as usize;
        assert!(
            part_count >= 3,
            "the streaming regression needs a genuinely multi-part bootstrap: {part_count}"
        );

        let root = fixture.enrollment_root("promote-stream-parts");
        let binding = fixture.enrollment_binding();
        let mut paths = PromotedPaths::new(&fixture, "stream-parts");
        // Production activation promotes the already-verified bootstrap database
        // in place. Use that exact path so this receipt also proves the successful
        // one-shot path opens existing SQLite without a rebuild.
        paths.database_path = fixture.root.path().join("bootstrap.sqlite");
        let session = SessionId::new();
        let (authority, runtime) = promote(&mut fixture, &root, session, &paths);

        let same_process = runtime.engine().bootstrap_recovery_instrumentation();
        assert_eq!(
            same_process.bootstrap_part_reads,
            0,
            "same-process promotion must not reread bootstrap parts: {:?}",
            runtime.resume_open_status()
        );
        assert_eq!(same_process.bootstrap_object_reads, 0);
        assert_eq!(same_process.max_live_bootstrap_parts, 0);
        let same_process_resume = runtime.resume_open_status().observation();
        assert!(same_process_resume.adopted);
        assert_eq!(
            same_process_resume.replay_base_generation,
            part_count as u64
        );
        assert_eq!(
            same_process_resume.live_history_generation,
            part_count as u64
        );
        assert_eq!(same_process_resume.replayed_generations, 0);
        assert!(matches!(
            runtime.projection().recovery,
            crate::oplog::sqlite::ProjectionRecovery::OpenedExisting
        ));
        assert_eq!(
            runtime.projection().rebuild,
            crate::oplog::sqlite::RebuildInstrumentation::default(),
            "same-process promotion must use the verified projection without rebuilding"
        );

        // The instrument itself is exercised against the forbidden shape. Holding
        // every loaded/prepared part at once reads the exact same parts and the
        // exact same objects as the streaming replay — only ownership overlaps
        // differ — so the residency counter is what separates the two, not the
        // accounting of reads.
        let (preloaded, live_after_release) = runtime
            .engine()
            .probe_preloaded_bootstrap_part_residency()
            .unwrap();
        assert_eq!(preloaded.bootstrap_part_reads, part_count);
        assert!(preloaded.bootstrap_object_reads > 0);
        assert_eq!(
            preloaded.max_live_bootstrap_parts, part_count,
            "holding every prepared part at once must be visible as {part_count} resident parts"
        );
        assert!(
            preloaded.max_live_bootstrap_parts > same_process.max_live_bootstrap_parts,
            "the streaming replay must own strictly fewer parts at once than a preload"
        );
        assert_eq!(
            live_after_release, 0,
            "dropping the owned payloads must release every counted residency"
        );
        // The probe must not have disturbed the engine's own instrumentation.
        assert_eq!(
            runtime.engine().bootstrap_recovery_instrumentation(),
            same_process
        );

        let same_process_observation = public_runtime_observation(&runtime);

        // A restarted process reconstructs the bootstrap from durable state in
        // one detached candidate. The ordinary enrolled engine adopts that
        // authenticated checkpoint instead of rebuilding the cumulative
        // catalog independently for every cold recovery step.
        drop(runtime);
        drop(authority);
        remove_every_resume_point(&fixture.archive_root);
        let (_reopened_authority, reopened) =
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).unwrap();
        let fresh_process = reopened.engine().bootstrap_recovery_instrumentation();
        assert_eq!(fresh_process.bootstrap_part_reads, 0);
        assert_eq!(fresh_process.bootstrap_object_reads, 0);
        assert_eq!(fresh_process.max_live_bootstrap_parts, 0);
        let fresh_resume = reopened.resume_open_status().observation();
        assert!(fresh_resume.adopted);
        assert_eq!(fresh_resume.replay_base_generation, part_count as u64);
        assert_eq!(fresh_resume.live_history_generation, part_count as u64);
        assert_eq!(fresh_resume.replayed_generations, 0);
        let (fresh_preloaded, fresh_live_after_release) = reopened
            .engine()
            .probe_preloaded_bootstrap_part_residency()
            .unwrap();
        assert_eq!(fresh_preloaded, preloaded);
        assert_eq!(fresh_live_after_release, 0);
        assert_eq!(
            reopened.engine().bootstrap_recovery_instrumentation(),
            fresh_process
        );
        let fresh_process_observation = public_runtime_observation(&reopened);
        assert_publicly_indistinguishable(
            &same_process_observation,
            &fresh_process_observation,
            "same-process bootstrap migration versus fresh full replay",
        );
        fixture.assert_graph_unchanged();
    });
}

/// A process token is an accelerator, never authority. If its exact typed
/// binding does not match, the retained workspace lease stays continuously
/// held and a fresh detached reconstruction supplies the bootstrap checkpoint.
#[test]
fn a_same_process_promotion_token_mismatch_reconstructs_under_the_retained_lease() {
    on_a_deep_stack(|| {
        force_next_bootstrap_part_operation_limit(1);
        let mut fixture = Fixture::new(
            "promotion-token-mismatch",
            None,
            vec![(
                "pages/token.md".into(),
                b"title:: Token\n\n- one\n- two\n".to_vec(),
            )],
        );
        let part_count = fixture.verified.part_count() as usize;
        assert!(part_count > 1);
        let root = fixture.enrollment_root("promotion-token-mismatch");
        let paths = PromotedPaths::new(&fixture, "promotion-token-mismatch");
        let session = SessionId::new();

        mismatch_next_same_process_promotion_token_for_test();
        let before = PromotedRuntimeInstrumentation::capture();
        let (_authority, runtime) = promote(&mut fixture, &root, session, &paths);
        let replay = runtime.engine().bootstrap_recovery_instrumentation();
        assert_eq!(replay.bootstrap_part_reads, 0);
        assert_eq!(replay.bootstrap_object_reads, 0);
        assert_eq!(replay.max_live_bootstrap_parts, 0);
        let observation = runtime.resume_open_status().observation();
        assert!(observation.adopted);
        assert_eq!(observation.replay_base_generation, part_count as u64);
        assert_eq!(observation.replayed_generations, 0);
        assert_eq!(
            before.since().workspace_lease_acquisitions,
            0,
            "discarding the token must not release and reacquire the workspace lease"
        );
        fixture.assert_graph_unchanged();
    });
}

/// A byte-identical archive substituted after the immutable promotion state
/// was published cannot consume the process token or the continuously held
/// workspace lease. Restoring the original directory lets the caller retry
/// with that exact returned lease and candidate.
#[test]
fn archive_replacement_after_promotion_publication_refuses_and_retries_under_the_same_lease() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "same-process-archive-replacement",
            None,
            vec![(
                "pages/archive.md".into(),
                b"title:: Archive\n\n- retained lease\n".to_vec(),
            )],
        );
        let root = fixture.enrollment_root("same-process-archive-replacement");
        let paths = PromotedPaths::new(&fixture, "same-process-archive-replacement");
        let session = SessionId::new();
        let authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        let refused_seal =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let retry_seal =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let bootstrap = fixture.take_bootstrap_session();

        // Keep the live archive and its retained lease open on the renamed inode,
        // then put a byte-identical but physically distinct archive at the enrolled
        // pathname. Content alone is deliberately insufficient authority.
        let enrolled_path = fixture.archive_root.clone();
        let relocated = fixture
            .root
            .path()
            .join("archive-relocated-after-publication");
        fs::rename(&enrolled_path, &relocated).unwrap();
        copy_tree(&relocated, &enrolled_path);
        let replacement_before = snapshot_file_digests(&enrolled_path);

        let before = PromotedRuntimeInstrumentation::capture();
        let (lease, error) = bootstrap
            .promote(refused_seal, &authority, &paths.open(&fixture))
            .err()
            .expect("a replacement archive must not receive writable authority")
            .into_parts();
        assert!(
            matches!(
                error,
                RuntimePromotionError::Store(crate::oplog::StoreError::Io(_))
            ),
            "unexpected archive replacement refusal: {error}"
        );
        assert_eq!(before.since().workspace_lease_acquisitions, 0);
        assert!(
            !paths.database_path.exists(),
            "the replacement must be refused before a promoted SQLite writer exists"
        );
        assert_eq!(
            snapshot_file_digests(&enrolled_path),
            replacement_before,
            "the refused replacement archive must remain byte-identical"
        );
        let relocated_store = ObjectStore::open(&relocated, fixture.workspace).unwrap();
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&relocated_store, fixture.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));

        // Put the exact original archive back. The refusal returned the same lease,
        // so reopening the bootstrap projection and retrying requires no archive
        // release/reacquire gap and can still use the process-only candidate.
        fs::remove_dir_all(&enrolled_path).unwrap();
        fs::rename(&relocated, &enrolled_path).unwrap();
        let bootstrap = fixture.reopen_bootstrap_session(lease);
        let runtime = bootstrap
            .promote(retry_seal, &authority, &paths.open(&fixture))
            .map_err(|refusal| refusal.into_parts().1)
            .unwrap();
        assert_eq!(
            runtime
                .engine()
                .bootstrap_recovery_instrumentation()
                .bootstrap_part_reads,
            0
        );
        assert_eq!(before.since().workspace_lease_acquisitions, 0);
        fixture.assert_graph_unchanged();
    });
}

/// Damage to reconstructible candidate bytes after the durable promotion state
/// was published cannot become writable authority. Resume restoration detects
/// it, rotates to an ordinary full replay, and keeps the retained workspace
/// lease throughout.
#[test]
fn corrupted_same_process_candidate_falls_back_before_writable_authority() {
    force_next_bootstrap_part_operation_limit(1);
    let mut fixture = Fixture::new(
        "promotion-candidate-corruption",
        None,
        vec![(
            "pages/corrupt-candidate.md".into(),
            b"title:: Candidate\n\n- one\n- two\n".to_vec(),
        )],
    );
    let root = fixture.enrollment_root("promotion-candidate-corruption");
    let paths = PromotedPaths::new(&fixture, "promotion-candidate-corruption");
    let session = SessionId::new();
    let authority = activate_verified_local(
        &root,
        fixture.compose(&root),
        session,
        &fixture.proofs(),
        &fixture.runtime(),
    )
    .unwrap();
    let sealed =
        seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime()).unwrap();
    fixture
        .authority
        .corrupt_retained_candidate_scratch_for_test();

    let before = PromotedRuntimeInstrumentation::capture();
    let bootstrap = fixture.take_bootstrap_session();
    let runtime = bootstrap
        .promote(sealed, &authority, &paths.open(&fixture))
        .map_err(|refusal| refusal.into_parts().1)
        .unwrap();
    let replay = runtime.engine().bootstrap_recovery_instrumentation();
    assert_eq!(replay.bootstrap_part_reads, 0);
    assert!(runtime.resume_open_status().observation().adopted);
    assert!(matches!(
        runtime.resume_open_status().unavailable(),
        Some(ResumeAcceleratorUnavailable::Unavailable(reason))
            if reason.contains("same-process bootstrap migration refused")
    ));
    assert_eq!(
        before.since().workspace_lease_acquisitions,
        0,
        "candidate refusal must remain under the continuously held lease"
    );
    fixture.assert_graph_unchanged();
}

/// The scratch-backed detached block-claim index removes the old fixed cap
/// instead of moving it.
///
/// Detached bootstrap authoring used to register block claims in the bounded
/// no-store in-memory map, which refuses the claim past
/// `MAX_EPHEMERAL_BLOCK_CLAIMS`. This is the smallest graph that crosses that
/// exact boundary, carried through the whole real path: preparation,
/// installation, promotion, and a fresh-process reopen.
///
/// Fail-before: preparing this exact fixture failed with "no-store block-claim
/// test index reached its fixed capacity". The bounded map itself still keeps
/// its cap — `no_store_block_claim_capacity_rejects_before_candidate_mutation`
/// covers that — so what changed is that authoring no longer uses it at all.
#[test]
fn a_bootstrap_one_block_past_the_old_claim_cap_promotes_and_reopens() {
    on_a_deep_stack(|| {
        let blocks = MAX_EPHEMERAL_BLOCK_CLAIMS + 1;
        let mut source = String::new();
        for ordinal in 0..blocks {
            source.push_str(&format!("- claim {ordinal:05}\n"));
        }
        let mut fixture = Fixture::new(
            "promote-claim-cap",
            None,
            vec![("pages/claims.md".into(), source.into_bytes())],
        );
        let root = fixture.enrollment_root("promote-claim-cap");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "claim-cap");
        let session = SessionId::new();

        let (authority, runtime) = promote(&mut fixture, &root, session, &paths);
        let frontier = runtime.engine().accepted_frontier_root().unwrap();
        // The cap is removed, not raised: the promoted engine holds its claims in
        // the scratch-backed point index, so the bounded map stays empty.
        assert_eq!(
            runtime.engine().instrumentation().block_claim_hot_entries,
            0,
            "a store-backed engine must hold no ephemeral block claims"
        );
        assert!(runtime
            .database()
            .frontier_root()
            .unwrap()
            .same_accepted_authority(&frontier));
        drop(runtime);
        drop(authority);

        let (_reopened_authority, reopened) =
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture)).unwrap();
        assert_eq!(
            reopened.engine().instrumentation().block_claim_hot_entries,
            0
        );
        assert_eq!(
            reopened.engine().accepted_frontier_root().unwrap(),
            frontier
        );
        fixture.assert_graph_unchanged();
    });
}

// ---------------------------------------------------------------------------
// Archive-lease-proved LocalActive crash takeover
// ---------------------------------------------------------------------------
//
// Every scheduling step below is caused by a request/response exchange over a
// pipe, a process exit, or an injected durability cut. Nothing sleeps, polls, or
// races on wall-clock time.

const HELPER_MARKER: &str = "local-active-helper:";
const HELPER_MODE: &str = "TINE_LOCAL_ACTIVE_HELPER_MODE";
const HELPER_WORLD: &str = "TINE_LOCAL_ACTIVE_HELPER_WORLD";

/// Everything a helper process needs to rebuild one fixture's durable world.
///
/// None of it is authority: every field is a path, an identity, or the durable
/// enrollment binding, and the child reauthenticates all of them from disk
/// exactly as a restarted Tine would. A process-local `LocalActiveAuthority`,
/// `PromotedLocalRuntime`, engine identity, or lease is unserializable and
/// deliberately absent.
#[derive(serde::Serialize, serde::Deserialize)]
struct HelperWorld {
    graph_root: PathBuf,
    migration_backup_root: PathBuf,
    receipt_root: PathBuf,
    archive_root: PathBuf,
    enrollment_root: PathBuf,
    database_path: PathBuf,
    runtime_root: PathBuf,
    workspace: WorkspaceId,
    binding: EnrollmentBindingV1,
    session: SessionId,
}

impl HelperWorld {
    fn new(
        fixture: &Fixture,
        root: &EnrollmentApplicationRoot,
        paths: &PromotedPaths,
        session: SessionId,
    ) -> Self {
        Self {
            graph_root: fixture.graph_root.clone(),
            migration_backup_root: fixture.roots.canonical_root().to_path_buf(),
            receipt_root: fixture.root.path().join("receipts"),
            archive_root: fixture.archive_root.clone(),
            enrollment_root: root.path().to_path_buf(),
            database_path: paths.database_path.clone(),
            runtime_root: paths.runtime_root_path.clone(),
            workspace: fixture.workspace,
            binding: fixture.enrollment_binding(),
            session,
        }
    }

    /// Reopen the device-local resources a promoted runtime is opened over.
    fn reopen(&self) -> HelperOpen {
        let endpoint = ProjectionEndpointBinding {
            endpoint_id: self.binding.endpoint_id(),
            device_id: self.binding.device_id(),
            graph_resource_id: self.binding.graph_resource_id(),
        };
        HelperOpen {
            graph: Graph::open(&self.graph_root),
            graph_root: self.graph_root.clone(),
            receipts: ProjectionReceiptStore::open_for_endpoint(
                &self.receipt_root,
                self.workspace,
                endpoint,
            )
            .unwrap(),
            archive_root: self.archive_root.clone(),
            migration_backup_root: self.migration_backup_root.clone(),
            database_path: self.database_path.clone(),
            runtime_root: ApplicationRuntimeRoot::open_for_test(&self.runtime_root).unwrap(),
            enrollment_root: enrollment_application_root_for_test(&self.enrollment_root).unwrap(),
        }
    }
}

struct HelperOpen {
    graph: Graph,
    graph_root: PathBuf,
    receipts: ProjectionReceiptStore,
    archive_root: PathBuf,
    migration_backup_root: PathBuf,
    database_path: PathBuf,
    runtime_root: ApplicationRuntimeRoot,
    enrollment_root: EnrollmentApplicationRoot,
}

impl HelperOpen {
    fn open(&self) -> PromotedRuntimeOpen<'_> {
        PromotedRuntimeOpen {
            graph: &self.graph,
            receipts: &self.receipts,
            archive_root: &self.archive_root,
            database_path: &self.database_path,
            application_runtime_root: &self.runtime_root,
            graph_root: &self.graph_root,
            migration_backup_root: &self.migration_backup_root,
        }
    }
}

fn helper_answer(answer: &str) {
    let mut output = std::io::stdout();
    writeln!(output, "{HELPER_MARKER}{answer}").unwrap();
    output.flush().unwrap();
}

fn helper_request() -> Option<String> {
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).unwrap() == 0 {
        return None;
    }
    Some(line.trim().to_string())
}

/// The one subprocess entry point. It is an ordinary `#[test]` so the test
/// binary can re-exec itself, and it returns immediately unless a mode is set.
#[test]
fn local_active_subprocess_helper() {
    let Ok(mode) = std::env::var(HELPER_MODE) else {
        return;
    };
    let world: HelperWorld = serde_json::from_str(&std::env::var(HELPER_WORLD).unwrap()).unwrap();
    match mode.as_str() {
        "archive-lease" => helper_archive_lease(&world),
        "takeover-hold" => helper_takeover_hold(&world),
        "takeover-loser" => helper_takeover_loser(&world),
        other => panic!("unknown local-active helper mode {other}"),
    }
}

/// A process under its own XDG/HOME roots that, on demand, takes or releases the
/// archive-rooted workspace runtime lease.
///
/// This is the exact lock a promoted open takes first, so this child models both
/// directions of the contention: another profile blocking us, and us blocking
/// another profile.
fn helper_archive_lease(world: &HelperWorld) {
    let mut held: Option<WorkspaceRuntimeLease> = None;
    while let Some(request) = helper_request() {
        match request.as_str() {
            "acquire" => {
                let store = ObjectStore::open(&world.archive_root, world.workspace).unwrap();
                match WorkspaceRuntimeLease::acquire(&store, world.workspace) {
                    Ok(lease) => {
                        held = Some(lease);
                        helper_answer("acquired");
                    }
                    Err(ProjectionError::LeaseContended(_)) => helper_answer("contended"),
                    Err(error) => panic!("unexpected archive lease error: {error}"),
                }
            }
            "release" => helper_answer(if held.take().is_some() {
                "released"
            } else {
                "not-held"
            }),
            "exit" => return,
            other => panic!("unknown archive-lease request {other}"),
        }
    }
}

/// A restarted process that takes over the crashed owner's `Unsafe` handoff and
/// then holds the whole writable runtime until it is killed or told to exit.
fn helper_takeover_hold(world: &HelperWorld) {
    let resources = world.reopen();
    match take_over_promoted_local_runtime(
        &resources.enrollment_root,
        &world.binding,
        world.session,
        &resources.open(),
    ) {
        Ok((authority, runtime)) => {
            helper_answer(&format!(
                "took-over:{}:{}",
                runtime
                    .engine()
                    .accepted_frontier_root()
                    .unwrap()
                    .acceptance_sequence(),
                runtime.database().frontier_root().unwrap().state_digest()
            ));
            while let Some(request) = helper_request() {
                if request == "exit" {
                    drop(runtime);
                    drop(authority);
                    helper_answer("exited");
                    return;
                }
            }
        }
        Err(error) => helper_answer(&format!("failed:{error}")),
    }
}

/// A newcomer that authenticates the crashed predecessor, is suspended at
/// exactly that point, and only then continues into the compare-and-swap.
///
/// Suspending it there is what makes the two-contender race deterministic: by
/// the time it resumes, another newcomer has already committed, so its
/// compare-and-swap must refuse on the exact predecessor it proved.
fn helper_takeover_loser(world: &HelperWorld) {
    super::set_takeover_predecessor_observed_hook_for_test(Box::new(|| {
        helper_answer("observed");
        assert_eq!(
            helper_request().expect("the loser was never resumed"),
            "resume"
        );
    }));
    let resources = world.reopen();
    match take_over_promoted_local_runtime(
        &resources.enrollment_root,
        &world.binding,
        world.session,
        &resources.open(),
    ) {
        Ok((_authority, _runtime)) => helper_answer("outcome:took-over"),
        Err(error) => helper_answer(&format!("outcome:failed:{error}")),
    }
}

/// A pipe-coordinated helper process.
///
/// It kills its child on drop, so a panicking test can never leave a helper
/// alive holding this run's archive-rooted workspace lease and poison the tests
/// that follow it.
struct HelperProcess {
    child: std::process::Child,
    answers: std::io::BufReader<std::process::ChildStdout>,
    requests: Option<std::process::ChildStdin>,
}

impl HelperProcess {
    fn spawn(mode: &str, world: &HelperWorld, profile: Option<&Path>) -> Self {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("oplog::local_active::tests::local_active_subprocess_helper")
            .arg("--nocapture")
            .env(HELPER_MODE, mode)
            .env(HELPER_WORLD, serde_json::to_string(world).unwrap())
            // The parent runs lifecycle tests on this same deep stack. A
            // re-exec helper instead enters through libtest's default worker,
            // so configure that process before libtest creates its worker.
            .env(
                "RUST_MIN_STACK",
                crate::test_support::TEST_DEEP_STACK_BYTES.to_string(),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        if let Some(profile) = profile {
            // A genuinely distinct application-data profile: different XDG data
            // home and different HOME. Nothing device-local is shared with this
            // process; the only thing both can reach is the archive.
            let xdg = profile.join("xdg");
            let home = profile.join("home");
            fs::create_dir_all(&xdg).unwrap();
            fs::create_dir_all(&home).unwrap();
            command.env("XDG_DATA_HOME", &xdg).env("HOME", &home);
        }
        let mut child = command.spawn().unwrap();
        let answers = std::io::BufReader::new(child.stdout.take().unwrap());
        let requests = Some(child.stdin.take().unwrap());
        Self {
            child,
            answers,
            requests,
        }
    }

    fn requests(&mut self) -> &mut std::process::ChildStdin {
        self.requests.as_mut().expect("the helper's stdin is open")
    }

    /// Read the next marked answer. The child is a libtest binary, so its own
    /// harness lines share this pipe; only marked lines are protocol.
    fn answer(&mut self) -> String {
        loop {
            let mut line = String::new();
            assert!(
                self.answers.read_line(&mut line).unwrap() != 0,
                "helper closed its output before answering"
            );
            if let Some((_, answer)) = line.rsplit_once(HELPER_MARKER) {
                return answer.trim().to_string();
            }
        }
    }

    fn ask(&mut self, request: &str) -> String {
        self.tell(request);
        self.answer()
    }

    fn tell(&mut self, request: &str) {
        let requests = self.requests();
        writeln!(requests, "{request}").unwrap();
        requests.flush().unwrap();
    }

    /// Kill the process outright. No destructor runs, so every lease it holds is
    /// released by the operating system exactly as it would be after a crash.
    fn kill(&mut self) {
        self.child.kill().unwrap();
        assert!(!self.child.wait().unwrap().success());
    }

    fn finish(mut self) {
        self.requests = None;
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for HelperProcess {
    fn drop(&mut self) {
        self.requests = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Every *authoritative* durable byte a takeover could touch: the enrollment
/// journal and the archive.
///
/// Four things are deliberately outside this set. The archive's
/// `.tine-runtime` namespace holds the workspace lease file, whose contents are
/// diagnostic metadata that ownership is never decided by — taking the lease
/// rewrites the recorded pid. The device-local enrollment lease's bytes are
/// likewise not authority: its stable file identity is bound into every
/// authenticated enrollment record and is revalidated separately, while the
/// empty file itself remains locked for the writer's lifetime. The device-local
/// SQLite projection is disposable
/// frontier-stamped materialization that can never authorize a write, so a
/// recovery legitimately rebuilds it before it has earned the right to change
/// anything authoritative. And the engine scratch namespace holds run-local
/// reconstructible state: a retained run is an accelerator whose every root is
/// re-proved against the sealed durable history before one byte of it is
/// reused, and an open that refuses one simply replays. Since P2N10 a refused
/// or failed open legitimately leaves a fresh retained run behind, which is
/// exactly the population `retained_run_directories` asserts on directly in the
/// resume-lifecycle tests rather than smuggling into "authoritative".
fn authoritative_world(
    fixture: &Fixture,
    root: &EnrollmentApplicationRoot,
) -> BTreeMap<String, ContentDigest> {
    let mut digests = BTreeMap::new();
    for (label, directory) in [
        ("enrollment", root.path()),
        ("archive", fixture.archive_root.as_path()),
    ] {
        for (path, digest) in snapshot_file_digests_matching(directory, |path, _| {
            !in_top_level_namespace(path, &[".tine-runtime", "engine-scratch-v2"])
                && (label != "enrollment" || !is_enrollment_lease_path(path))
        }) {
            digests.insert(format!("{label}/{path}"), digest);
        }
    }
    digests
}

fn committed_handoff(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    verification_digest: ContentDigest,
) -> LocalActiveHandoff {
    crate::oplog::enrollment::reopen_committed_local_active_for_session(
        root,
        binding,
        verification_digest,
    )
    .unwrap()
    .handoff()
}

/// Attempt a takeover and keep only its rendered error.
///
/// The opened runtime is enormous, so returning it into the caller's frame
/// would make a table of attempts overflow the test thread's stack. Every
/// attempt therefore lives and dies in this frame.
fn takeover_error(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    session: SessionId,
    open: &PromotedRuntimeOpen<'_>,
    expectation: &str,
) -> String {
    match take_over_promoted_local_runtime(root, binding, session, open) {
        Ok(_) => panic!("{expectation}"),
        Err(error) => error.to_string(),
    }
}

/// Take over, keep only the recovery state, and immediately release everything.
fn takeover_recovery(
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    session: SessionId,
    open: &PromotedRuntimeOpen<'_>,
) -> RuntimeRecoveryState {
    let (authority, runtime) = take_over_promoted_local_runtime(root, binding, session, open)
        .expect("the takeover must commit");
    let recovery = runtime.recovery();
    drop(runtime);
    drop(authority);
    recovery
}

/// Delete one device-local SQLite projection and its sidecars.
///
/// The projection is disposable derived state, so removing it forces the next
/// open to rebuild it from the authoritative oplog — which is where an injected
/// materialization fault can reach.
fn remove_device_local_database(database_path: &Path) {
    for suffix in ["", "-wal", "-shm", "-auth"] {
        let path = PathBuf::from(format!("{}{suffix}", database_path.display()));
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
    }
}

/// The ready projection work a runtime owes, as an exact ordered fingerprint.
fn projection_work_fingerprint(runtime: &PromotedLocalRuntime) -> Vec<String> {
    runtime
        .engine()
        .projection_work_index()
        .unwrap()
        .ready_page(None, 64)
        .unwrap()
        .work()
        .iter()
        .map(|work| format!("{work:?}"))
        .collect()
}

/// A live promoted runtime owns the archive. Every newcomer — this process's own
/// restart, and a process running under completely separate XDG/HOME roots — is
/// refused before one enrollment, archive, SQLite, or history byte moves. The
/// mirror image holds too: while another profile owns the archive-rooted
/// workspace lease, a takeover here fails on that lease.
#[test]
fn a_live_promoted_runtime_blocks_every_newcomer_before_any_durable_write() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-live",
            None,
            vec![("pages/live.md".into(), b"- live\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-live");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "live");
        let owner = SessionId::new();
        let world = HelperWorld::new(&fixture, &root, &paths, SessionId::new());
        let mut profile = HelperProcess::spawn(
            "archive-lease",
            &world,
            Some(&fixture.root.path().join("profile-a")),
        );
        let verification_digest = {
            let (mut authority, mut runtime) = promote(&mut fixture, &root, owner, &paths);
            append_local_batch(&fixture, &mut authority, &mut runtime, 0xC100);
            let before = authoritative_world(&fixture, &root);

            // A same-profile newcomer: the live runtime retains the exclusive
            // enrollment lease, so the newcomer cannot even read its way to a
            // durable write.
            let error = takeover_error(
                &root,
                &binding,
                SessionId::new(),
                &paths.open(&fixture),
                "a live runtime must block a same-profile takeover",
            );
            assert!(
                error.contains("enrollment lease"),
                "a same-profile newcomer must stop at the retained enrollment lease: {error}"
            );

            // A separate application-data profile shares nothing device-local with
            // this process, so only the archive-rooted lease can stop it. It does.
            assert_eq!(profile.ask("acquire"), "contended");
            assert_eq!(
                authoritative_world(&fixture, &root),
                before,
                "nothing may move"
            );
            authority.verification_digest()
        };

        // The old process is gone. Its record stays exactly Unsafe { owner }.
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe { session_id: owner }
        );

        // Now the other profile owns the archive. This process must fail on that
        // exact lease, after its own enrollment lease was free, and before writing.
        assert_eq!(profile.ask("acquire"), "acquired");
        let contended = authoritative_world(&fixture, &root);
        let before = PromotedRuntimeInstrumentation::capture();
        let error = takeover_error(
            &root,
            &binding,
            SessionId::new(),
            &paths.open(&fixture),
            "another profile's archive lease must block this takeover",
        );
        let refused = before.since();
        assert!(
            error.contains("sqlite-applier.lock"),
            "a cross-profile newcomer must stop at the archive-rooted workspace lease: {error}"
        );
        // The lease is taken before anything else this runtime does with the
        // archive, so the refusal happens before the archive is even authenticated,
        // let alone before the engine is recovered or SQLite is opened.
        assert_eq!(
            refused.archive_identity_reads, 0,
            "a lease-refused takeover must stop before it authenticates the archive"
        );
        assert_eq!(refused.sqlite_frontier_reads, 0);
        assert_eq!(authoritative_world(&fixture, &root), contended);
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe { session_id: owner },
            "a refused takeover leaves the crashed owner authoritative"
        );

        // Released, the takeover proceeds.
        assert_eq!(profile.ask("release"), "released");
        let successor = SessionId::new();
        assert_eq!(
            takeover_recovery(&root, &binding, successor, &paths.open(&fixture)),
            RuntimeRecoveryState::TookOverCrashedUnsafe {
                previous_session: owner
            }
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: successor
            }
        );
        profile.finish();
        fixture.assert_graph_unchanged();
    });
}

/// Process death releases the archive-rooted lease. A fresh process then
/// authenticates the whole crashed runtime, compare-and-swaps
/// `Unsafe { old } -> Unsafe { new }`, reopens exactly the data the crashed
/// process had, and admits a new mutation without any whole-history work or a
/// second lease acquisition.
#[test]
fn process_death_releases_the_lease_and_a_fresh_process_takes_over_and_admits_work() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-death",
            None,
            vec![("pages/death.md".into(), b"- death\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-death");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "death");
        let first = SessionId::new();
        let (mut authority, mut runtime) = promote(&mut fixture, &root, first, &paths);
        append_local_batch(&fixture, &mut authority, &mut runtime, 0xC200);
        let verification_digest = authority.verification_digest();
        let anchor = runtime.bootstrap_anchor();
        let frontier = runtime.engine().accepted_frontier_root().unwrap();
        let sqlite_frontier = runtime.database().frontier_root().unwrap();
        let work = projection_work_fingerprint(&runtime);
        drop(runtime);
        drop(authority);

        // A separate process takes over and holds the runtime.
        let second = SessionId::new();
        let world = HelperWorld::new(&fixture, &root, &paths, second);
        let mut holder = HelperProcess::spawn("takeover-hold", &world, None);
        assert_eq!(
            holder.answer(),
            format!(
                "took-over:{}:{}",
                frontier.acceptance_sequence(),
                sqlite_frontier.state_digest()
            ),
            "the takeover must reopen the crashed process's exact frontier"
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe { session_id: second }
        );

        // While it lives, nobody else can have the archive — not this process, and
        // not another profile.
        let mut profile = HelperProcess::spawn(
            "archive-lease",
            &world,
            Some(&fixture.root.path().join("profile-b")),
        );
        assert_eq!(profile.ask("acquire"), "contended");
        let held = authoritative_world(&fixture, &root);
        takeover_error(
            &root,
            &binding,
            SessionId::new(),
            &paths.open(&fixture),
            "a live takeover holder must block every newcomer",
        );
        assert_eq!(authoritative_world(&fixture, &root), held);

        // Kill it: no destructor runs, so this is a real crash, and the operating
        // system is what releases both leases.
        holder.kill();
        assert_eq!(profile.ask("acquire"), "acquired");
        assert_eq!(profile.ask("release"), "released");
        profile.finish();

        let third = SessionId::new();
        let before = PromotedRuntimeInstrumentation::capture();
        let (mut authority, mut runtime) =
            take_over_promoted_local_runtime(&root, &binding, third, &paths.open(&fixture))
                .unwrap();
        let opened = before.since();
        assert_eq!(
            opened.workspace_lease_acquisitions, 1,
            "a promoted open takes exactly one archive-rooted workspace lease"
        );

        // Exactly the crashed process's data, recovered.
        assert_eq!(runtime.bootstrap_anchor(), anchor);
        assert_eq!(runtime.engine().accepted_frontier_root().unwrap(), frontier);
        assert_eq!(runtime.database().frontier_root().unwrap(), sqlite_frontier);
        assert_eq!(projection_work_fingerprint(&runtime), work);
        assert!(!work.is_empty(), "the crashed process owed projection work");
        assert_eq!(runtime.tail().status().unapplied_batches, 0);
        assert_eq!(
            runtime.recovery(),
            RuntimeRecoveryState::TookOverCrashedUnsafe {
                previous_session: second
            }
        );
        assert_eq!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Blocked(
                "this runtime took over a crashed session's Unsafe handoff, whose drain was never \
             proved"
            ),
            "a crash must never authorize an automatic external import"
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe { session_id: third }
        );

        // A new mutation is admitted, and admission stays bounded: no lease is
        // reacquired, no enrollment chain is rewalked, no SQLite statement is run.
        let before = PromotedRuntimeInstrumentation::capture();
        {
            let _admitted = runtime
                .admit_promoted_mutation(&mut authority, &fixture.graph)
                .unwrap();
        }
        let admission = before.since();
        assert_eq!(admission.workspace_lease_acquisitions, 0);
        assert_eq!(admission.sqlite_frontier_reads, 0);
        assert_eq!(admission.enrollment.record_reads, 0);
        assert_eq!(admission.enrollment.lease_acquisitions, 0);
        assert_eq!(admission.enrollment.namespace_scans, 0);

        append_local_batch(&fixture, &mut authority, &mut runtime, 0xC300);
        assert!(
            runtime
                .engine()
                .accepted_frontier_root()
                .unwrap()
                .acceptance_sequence()
                > frontier.acceptance_sequence(),
            "the taken-over runtime accepts new local work"
        );
        fixture.assert_graph_unchanged();
    });
}

/// Two newcomers authenticate the same crashed `Unsafe { old }` record. Exactly
/// one compare-and-swap commits; the loser reauthenticates, observes the new
/// owner, and leaves every durable byte untouched.
#[test]
fn two_post_crash_contenders_commit_exactly_one_takeover() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-race",
            None,
            vec![("pages/race.md".into(), b"- race\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-race");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "race");
        let crashed = SessionId::new();
        let (authority, runtime) = promote(&mut fixture, &root, crashed, &paths);
        let verification_digest = authority.verification_digest();
        drop(runtime);
        drop(authority);

        // The loser starts first and is suspended the instant it has authenticated
        // the crashed predecessor, before it takes a single lease.
        let loser_session = SessionId::new();
        // The loser is an ordinary restart of the same profile: same enrollment
        // journal, same device-local database, same archive.
        let loser_world = HelperWorld::new(&fixture, &root, &paths, loser_session);
        let mut loser = HelperProcess::spawn("takeover-loser", &loser_world, None);
        assert_eq!(loser.answer(), "observed");

        // The winner completes its takeover and exits, so the loser will find every
        // lease free and only the record changed.
        let winner_session = SessionId::new();
        let (winner_authority, winner_runtime) = take_over_promoted_local_runtime(
            &root,
            &binding,
            winner_session,
            &paths.open(&fixture),
        )
        .unwrap();
        drop(winner_runtime);
        drop(winner_authority);
        let after_winner = authoritative_world(&fixture, &root);

        loser.tell("resume");
        let outcome = loser.answer();
        assert!(
            outcome.starts_with("outcome:failed:"),
            "the loser must not also win: {outcome}"
        );
        assert!(
            outcome.contains("authenticated unsafe predecessor"),
            "the loser must fail on its exact compare-and-swap predecessor: {outcome}"
        );
        loser.finish();

        assert_eq!(
            authoritative_world(&fixture, &root),
            after_winner,
            "a losing takeover writes nothing"
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: winner_session
            },
            "the winner stays the owner"
        );
        fixture.assert_graph_unchanged();
    });
}

/// Every wrong piece of evidence a takeover must authenticate refuses *before*
/// the compare-and-swap, leaving the crashed owner's record authoritative and
/// every authoritative byte untouched.
///
/// The rows are the evidence the durable takeover actually depends on: the
/// workspace and enrollment binding, the physical archive resource identity, the
/// persisted archive-resource claim, the durable promotion state that binds the
/// immutable activation anchor, the engine's committed history, the SQLite
/// authority the recovery must reproduce, and the enrollment lifecycle itself.
#[test]
fn a_takeover_refuses_wrong_workspace_archive_anchor_history_and_sqlite_evidence() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-evidence",
            None,
            vec![("pages/evidence.md".into(), b"- evidence\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-evidence");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "evidence");
        let crashed = SessionId::new();
        let verification_digest = {
            let (mut authority, mut runtime) = promote(&mut fixture, &root, crashed, &paths);
            append_local_batch(&fixture, &mut authority, &mut runtime, 0xC400);
            authority.verification_digest()
        };
        let unsafe_owner = LocalActiveHandoff::Unsafe {
            session_id: crashed,
        };

        let refused = |label: &str, prepare: &mut dyn FnMut() -> Box<dyn FnOnce()>| {
            let restore = prepare();
            let before = authoritative_world(&fixture, &root);
            let error = takeover_error(
                &root,
                &binding,
                SessionId::new(),
                &paths.open(&fixture),
                &format!("{label} must not authorize a takeover"),
            );
            assert_eq!(
                authoritative_world(&fixture, &root),
                before,
                "{label} wrote authoritative bytes before failing: {error}"
            );
            assert_eq!(
                committed_handoff(&root, &binding, verification_digest),
                unsafe_owner,
                "{label} moved the crashed owner's record"
            );
            restore();
            // A refused takeover releases the archive-rooted lease it took, so the
            // next attempt is not blocked by the last failure.
            drop(
                WorkspaceRuntimeLease::acquire(&fixture.archive(), fixture.workspace)
                    .unwrap_or_else(|error| panic!("{label} kept the workspace lease: {error}")),
            );
        };

        // A different device: the enrollment binding no longer names this journal.
        let foreign_binding = EnrollmentBindingV1::new(
            binding.workspace_id(),
            binding.lineage_digest(),
            binding.catalog_document_id(),
            binding.endpoint_id(),
            DeviceId::from_uuid(Uuid::from_u128(0xDEAD_BEEF)),
            binding.graph_resource_id(),
            binding.receipt_store_id(),
            binding.archive_resource_id(),
            fixture.graph.graph_text_scope_binding().unwrap(),
        )
        .unwrap();
        takeover_error(
            &root,
            &foreign_binding,
            SessionId::new(),
            &paths.open(&fixture),
            "a foreign device binding must not authorize a takeover",
        );

        // A byte-identical copy of the archive is a different physical resource.
        let copied_root = fixture.root.path().join("copied-archive");
        copy_tree(&fixture.archive_root, &copied_root);
        let copied_paths = PromotedPaths::new(&fixture, "evidence-copy");
        let copied_open = PromotedRuntimeOpen {
            graph: &fixture.graph,
            receipts: &fixture.receipts,
            archive_root: &copied_root,
            database_path: &copied_paths.database_path,
            application_runtime_root: &copied_paths.runtime_root,
            graph_root: &fixture.graph_root,
            migration_backup_root: fixture.roots.canonical_root(),
        };
        let before = authoritative_world(&fixture, &root);
        takeover_error(
            &root,
            &binding,
            SessionId::new(),
            &copied_open,
            "a look-alike archive must never receive this enrollment's takeover",
        );
        assert_eq!(authoritative_world(&fixture, &root), before);

        // The persisted canonical archive-resource claim.
        let claim_path = fixture.archive_root.join(ARCHIVE_INSTANCE_CLAIM_FILE);
        refused("a divergent archive instance claim", &mut || {
            let original = fs::read(&claim_path).unwrap();
            fs::write(&claim_path, divergent_archive_instance_claim(&original)).unwrap();
            let path = claim_path.clone();
            Box::new(move || fs::write(&path, &original).unwrap())
        });

        // The durable promotion state that binds the immutable activation anchor.
        let state_path = promotion_state_path(&fixture);
        refused("a truncated promotion state", &mut || {
            let original = fs::read(&state_path).unwrap();
            fs::write(&state_path, &original[..original.len() / 2]).unwrap();
            let path = state_path.clone();
            Box::new(move || fs::write(&path, &original).unwrap())
        });
        refused("an absent promotion state", &mut || {
            let original = fs::read(&state_path).unwrap();
            fs::remove_file(&state_path).unwrap();
            let path = state_path.clone();
            Box::new(move || fs::write(&path, &original).unwrap())
        });

        // The engine's own committed history: one truncated manifest and the
        // recovery this takeover depends on can no longer be authenticated.
        let manifest_path = find_file_with_prefix(&fixture.archive_root.join("batches"), "");
        refused("a truncated committed manifest", &mut || {
            let original = fs::read(&manifest_path).unwrap();
            fs::write(&manifest_path, &original[..original.len() / 2]).unwrap();
            let path = manifest_path.clone();
            Box::new(move || fs::write(&path, &original).unwrap())
        });

        // The SQLite authority the recovery must reproduce before it may swap the
        // handoff. The device-local database is deleted, so recovery must rebuild
        // it from the authoritative oplog, and that rebuild is interrupted inside
        // the materialization transaction: an unreproducible SQLite authority is a
        // failed open, not a takeover.
        refused("an interrupted SQLite rebuild", &mut || {
            remove_device_local_database(&paths.database_path);
            crate::oplog::sqlite::fail_next_apply_during_materialization_for_harness();
            Box::new(|| ())
        });

        // With every piece of evidence restored, the takeover commits.
        let successor = SessionId::new();
        assert_eq!(
            takeover_recovery(&root, &binding, successor, &paths.open(&fixture)),
            RuntimeRecoveryState::TookOverCrashedUnsafe {
                previous_session: crashed
            }
        );

        // A blocked enrollment is the last row: it is the one tampering that is
        // deliberately irreversible.
        let head = enrollment_head(&root, &binding);
        crate::oplog::enrollment::block_current_for_test(
            &root,
            &binding,
            head,
            "takeover-evidence-blocked".into(),
        )
        .unwrap();
        let blocked = authoritative_world(&fixture, &root);
        takeover_error(
            &root,
            &binding,
            SessionId::new(),
            &paths.open(&fixture),
            "a blocked enrollment must not authorize a takeover",
        );
        assert_eq!(authoritative_world(&fixture, &root), blocked);
        fixture.assert_graph_unchanged();
    });
}

/// A crash at every durability cut of the takeover publication recovers to
/// exactly the old or exactly the new `Unsafe` owner — never `Safe`, never a
/// split owner, and never an unrecoverable state. Each interrupted attempt
/// stays retryable, so the chain of eleven cuts ends in one exact owner.
#[test]
fn takeover_at_every_durability_cut_resumes_exactly_one_unsafe_owner() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-cuts",
            None,
            vec![("pages/cuts.md".into(), b"- cuts\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-cuts");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "cuts");
        let mut owner = SessionId::new();
        let (verification_digest, frontier) = {
            let (authority, runtime) = promote(&mut fixture, &root, owner, &paths);
            (
                authority.verification_digest(),
                runtime.engine().accepted_frontier_root().unwrap(),
            )
        };

        for cut in [
            CommitCut::AfterRecordTempCreate,
            CommitCut::AfterRecordWrite,
            CommitCut::AfterRecordFileSync,
            CommitCut::AfterRecordLink,
            CommitCut::AfterRecordInsert,
            CommitCut::AfterRecordsDirectorySync,
            CommitCut::AfterHeadTempCreate,
            CommitCut::AfterHeadWrite,
            CommitCut::AfterHeadFileSync,
            CommitCut::AfterHeadReplace,
            CommitCut::AfterEnrollmentDirectorySync,
        ] {
            let successor = SessionId::new();
            assert!(
                super::take_over_promoted_local_runtime_at_cut_for_test(
                    &root,
                    &binding,
                    successor,
                    &paths.open(&fixture),
                    cut,
                )
                .is_err(),
                "{cut:?} must not return a runtime"
            );

            // Whatever the cut left behind, the committed record is exactly one of
            // the two unsafe owners, still Idle, and never a synthesized Safe.
            match committed_handoff(&root, &binding, verification_digest) {
                LocalActiveHandoff::Unsafe { session_id } if session_id == owner => {}
                LocalActiveHandoff::Unsafe { session_id } if session_id == successor => {
                    owner = successor;
                }
                other => panic!("{cut:?} left an unexpected handoff: {other:?}"),
            }

            // And the interrupted transition is retryable: the runtime reopens for
            // whichever owner the cut actually left committed.
            let (_authority, resumed) =
                reopen_promoted_local_runtime(&root, &binding, owner, &paths.open(&fixture))
                    .unwrap();
            assert_eq!(resumed.engine().accepted_frontier_root().unwrap(), frontier);
            assert_eq!(resumed.database().frontier_root().unwrap(), frontier);
        }

        // One last uninterrupted takeover still commits from the surviving owner.
        let final_session = SessionId::new();
        assert_eq!(
            takeover_recovery(&root, &binding, final_session, &paths.open(&fixture)),
            RuntimeRecoveryState::TookOverCrashedUnsafe {
                previous_session: owner
            }
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: final_session
            }
        );
        fixture.assert_graph_unchanged();
    });
}

/// The bootstrap -> promoted database handoff runs under one retained
/// archive-rooted lease. A separate application-data profile is probed at every
/// step and stays blocked from the inactive bootstrap open until the promoted
/// runtime is finally dropped.
///
/// The handoff here is `InactiveBootstrapRuntimeSession::promote` — the crate's
/// only route from an inactive bootstrap database to a promoted runtime — not a
/// hand-assembled equivalent. That makes this a receipt for the construction the
/// activation wiring will call; it is not a claim that a running Tine binary
/// executes it today, because no part of this module is reachable from
/// application startup yet.
#[test]
fn the_bootstrap_to_promoted_database_handoff_never_releases_the_workspace_lease() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-handoff",
            None,
            vec![("pages/handoff.md".into(), b"- handoff\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-handoff");
        let paths = PromotedPaths::new(&fixture, "handoff");
        let session = SessionId::new();
        let world = HelperWorld::new(&fixture, &root, &paths, session);
        let mut profile = HelperProcess::spawn(
            "archive-lease",
            &world,
            Some(&fixture.root.path().join("profile-handoff")),
        );

        // The fixture already holds the lease: its inactive bootstrap database was
        // opened through that lease's single applier slot.
        assert_eq!(profile.ask("acquire"), "contended");

        let authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        // Sealing is idempotent, so two identical sealed promotions exist: one is
        // spent on the foreign-lease refusal below, the other on the real open.
        let refused_seal =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let sealed =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        assert_eq!(profile.ask("acquire"), "contended");

        // A lease for some *other* archive is not this archive's authority, even
        // though it is a genuine live workspace runtime lease.
        let foreign_archive = fixture.root.path().join("foreign-archive");
        let foreign_store = ObjectStore::open(&foreign_archive, fixture.workspace).unwrap();
        let foreign_lease =
            WorkspaceRuntimeLease::acquire(&foreign_store, fixture.workspace).unwrap();
        let before = PromotedRuntimeInstrumentation::capture();
        let (returned_foreign_lease, error) = open_promoted_local_runtime(
            refused_seal,
            &authority,
            &paths.open(&fixture),
            RetainedWorkspaceLease::new(foreign_lease),
        )
        .err()
        .expect("a foreign archive's lease must not authorize this promotion")
        .into_parts();
        let refused = before.since();
        assert!(
            matches!(
                error,
                RuntimePromotionError::Sqlite(ProjectionError::UnsafePath(_))
            ),
            "unexpected foreign-lease error: {error}"
        );
        assert_eq!(
            refused.archive_identity_reads, 0,
            "a foreign retained lease must be refused before any archive work"
        );
        // The refusal handed the caller's lease back rather than releasing it: the
        // foreign archive is still this process's.
        assert!(matches!(
            WorkspaceRuntimeLease::acquire(&foreign_store, fixture.workspace),
            Err(ProjectionError::LeaseContended(_))
        ));
        drop(returned_foreign_lease);
        drop(WorkspaceRuntimeLease::acquire(&foreign_store, fixture.workspace).unwrap());

        // The bootstrap database closes and the promoted one opens under the exact
        // same lease. The workspace lock does not move.
        //
        // The helper probes below can only observe the archive at the instants they
        // are asked, so they cannot by themselves rule out an infinitesimal
        // release-and-reacquire inside the handoff. The acquisition counter can, and
        // does: zero new archive-rooted leases across the whole handoff.
        let across_handoff = PromotedRuntimeInstrumentation::capture();
        let bootstrap = fixture.take_bootstrap_session();
        let runtime = bootstrap
            .promote(sealed, &authority, &paths.open(&fixture))
            .map_err(|refusal| refusal.into_parts().1)
            .unwrap();
        assert_eq!(
            across_handoff.since().workspace_lease_acquisitions,
            0,
            "the bootstrap -> promoted handoff must reuse the lease, not reacquire it"
        );
        assert_eq!(profile.ask("acquire"), "contended");
        assert_eq!(runtime.recovery(), RuntimeRecoveryState::FirstPromotion);
        assert!(matches!(
            runtime.automatic_external_import(),
            ExternalImportAdmission::Blocked(_)
        ));
        drop(runtime);
        drop(authority);

        // Only now is the archive free.
        assert_eq!(profile.ask("acquire"), "acquired");
        assert_eq!(profile.ask("release"), "released");
        profile.finish();
        fixture.assert_graph_unchanged();
    });
}

/// Every failure boundary of a retained promotion hands the caller's exact
/// archive-rooted lease back instead of releasing it.
///
/// `seal_local_runtime_promotion` has already published the durable promotion
/// state by the time phase two runs, so a refusal that quietly released the
/// archive would offer it to another process at the one moment this one must
/// keep holding it. The three boundaries below are, in order: before the lease
/// is inspected at all, after the device-local database has already been opened
/// under it, and the retry that then succeeds — all under one lease that is
/// never reacquired.
#[test]
fn a_refused_retained_promotion_returns_the_exact_lease_at_every_failure_boundary() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "retained-refusal",
            None,
            vec![("pages/refuse.md".into(), b"- refuse\n".to_vec())],
        );
        let root = fixture.enrollment_root("retained-refusal");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "retained-refusal");
        let session = SessionId::new();
        let archive = ObjectStore::open(&fixture.archive_root, fixture.workspace).unwrap();

        let authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        // Sealing is idempotent, so one sealed promotion is spent per boundary.
        let enrollment_boundary =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let post_open_boundary =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let sealed =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();

        let bootstrap = fixture.take_bootstrap_session();
        let before = PromotedRuntimeInstrumentation::capture();

        // Boundary 1: the very first step, before the lease is even looked at. A
        // second live enrollment session owns the device-local journal lease.
        let blocker =
            RetainedEnrollmentSession::open(&root, &binding, authority.verification_digest())
                .unwrap();
        let (lease, error) = bootstrap
            .promote(enrollment_boundary, &authority, &paths.open(&fixture))
            .err()
            .expect("a contended enrollment lease must refuse the promotion")
            .into_parts();
        assert!(
            matches!(error, RuntimePromotionError::Enrollment(_)),
            "unexpected pre-lease error: {error}"
        );
        assert!(
            matches!(
                WorkspaceRuntimeLease::acquire(&archive, fixture.workspace),
                Err(ProjectionError::LeaseContended(_))
            ),
            "a pre-lease refusal must hand the archive back, not release it"
        );
        drop(blocker);

        // Boundary 2: after the device-local database is open, where the lease can
        // only be recovered by closing the database it now lives inside.
        let bootstrap = fixture.reopen_bootstrap_session(lease);
        fail_next_promotion_after_the_database_opens_for_test();
        let (lease, error) = bootstrap
            .promote(post_open_boundary, &authority, &paths.open(&fixture))
            .err()
            .expect("the injected post-open fault must refuse the promotion")
            .into_parts();
        assert!(
            matches!(
                error,
                RuntimePromotionError::Anchor(
                    "injected failure after the promoted database opened"
                )
            ),
            "unexpected post-open error: {error}"
        );
        // The promoted database really was closed...
        let promoted_database_lock = paths.database_path.with_file_name(format!(
            ".{}.database-applier.lock",
            paths.database_path.file_name().unwrap().to_str().unwrap()
        ));
        assert!(
            !crate::oplog::sqlite::workspace_lock_is_contended(&promoted_database_lock),
            "the refused promotion must have closed its database"
        );
        // ...and the archive really did not move.
        assert!(
            matches!(
                WorkspaceRuntimeLease::acquire(&archive, fixture.workspace),
                Err(ProjectionError::LeaseContended(_))
            ),
            "a post-open refusal must hand the archive back, not release it"
        );

        // Boundary 3: the retry, on the same lease, succeeds.
        let bootstrap = fixture.reopen_bootstrap_session(lease);
        let runtime = bootstrap
            .promote(sealed, &authority, &paths.open(&fixture))
            .map_err(|refusal| refusal.into_parts().1)
            .unwrap();
        assert_eq!(runtime.recovery(), RuntimeRecoveryState::FirstPromotion);
        assert_eq!(
            runtime
                .engine()
                .bootstrap_recovery_instrumentation()
                .bootstrap_part_reads,
            0,
            "retry must safely re-adopt the already migrated candidate rather than replay"
        );
        assert_eq!(
            before.since().workspace_lease_acquisitions,
            0,
            "two refusals and a retry must not reacquire the archive even once"
        );

        drop(runtime);
        drop(authority);
        drop(WorkspaceRuntimeLease::acquire(&archive, fixture.workspace).unwrap());
        fixture.assert_graph_unchanged();
    });
}

/// `AcquireWorkspaceLease` is the branch a promotion takes when it retains no
/// inactive bootstrap session, and its documented contract is that the
/// archive-rooted lease *enforces* the bootstrap release rather than trusting
/// it. While the bootstrap session is still open the acquiring promotion is
/// refused as contended; once it is dropped, the same shape opens.
#[test]
fn an_acquiring_first_promotion_is_contended_until_the_bootstrap_session_is_released() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "acquire-promotion",
            None,
            vec![("pages/acquire.md".into(), b"- acquire\n".to_vec())],
        );
        let root = fixture.enrollment_root("acquire-promotion");
        let paths = PromotedPaths::new(&fixture, "acquire-promotion");
        let session = SessionId::new();

        let authority = activate_verified_local(
            &root,
            fixture.compose(&root),
            session,
            &fixture.proofs(),
            &fixture.runtime(),
        )
        .unwrap();
        let contended =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();
        let sealed =
            seal_local_runtime_promotion(&authority, &fixture.proofs(), &fixture.runtime())
                .unwrap();

        let error = open_promoted_local_runtime(
            contended,
            &authority,
            &paths.open(&fixture),
            AcquireWorkspaceLease,
        )
        .err()
        .expect("the retained bootstrap session still owns this archive");
        assert!(
            matches!(
                error,
                RuntimePromotionError::Sqlite(ProjectionError::LeaseContended(_))
            ),
            "unexpected acquiring-promotion error: {error}"
        );

        fixture.release_bootstrap_projection();
        let before = PromotedRuntimeInstrumentation::capture();
        let runtime = open_promoted_local_runtime(
            sealed,
            &authority,
            &paths.open(&fixture),
            AcquireWorkspaceLease,
        )
        .unwrap();
        assert_eq!(
            before.since().workspace_lease_acquisitions,
            1,
            "an acquiring promotion takes exactly one archive-rooted lease"
        );
        assert_eq!(runtime.recovery(), RuntimeRecoveryState::FirstPromotion);

        drop(runtime);
        drop(authority);
        fixture.assert_graph_unchanged();
    });
}

/// The archive-rooted workspace lock file, as a pathname.
fn workspace_lease_path(archive_root: &Path, workspace: WorkspaceId) -> PathBuf {
    archive_root
        .join(".tine-runtime")
        .join("sqlite-workspaces")
        .join(workspace.to_string())
        .join("sqlite-applier.lock")
}

/// Byte identity of one archive, excluding the device-local runtime lease
/// namespace whose file this test deliberately replaces.
fn archive_digests_outside_the_lease_namespace(
    archive_root: &Path,
) -> BTreeMap<String, ContentDigest> {
    snapshot_file_digests_matching(archive_root, |path, _| {
        !in_top_level_namespace(path, &[".tine-runtime"])
    })
}

/// The workspace lock file lives *inside* the replicated archive, so an
/// out-of-band replacement of its path — a Syncthing receive-only revert, a
/// folder reset/re-add, a delete-then-restore, a `.stversions` restore, or a
/// user deleting `.tine-runtime` by hand — leaves this runtime holding a lock
/// on a file no pathname reaches, while another process legitimately locks the
/// file that is now at the name.
///
/// A promoted runtime must stop being authority at that moment. This drives
/// every boundary in the module documentation's "Workspace lease identity"
/// census that a live promoted runtime can reach, and asserts at each one that
/// the refusal happens *before* the graph, the archive, the durable enrollment
/// chain, the engine's durable history, or the device-local SQLite projection
/// moves.
#[test]
fn a_replaced_workspace_lease_path_fails_promoted_authority_closed_without_mutating_anything() {
    let mut fixture = Fixture::new(
        "lease-replacement",
        None,
        vec![("pages/lease.md".into(), b"- lease\n".to_vec())],
    );
    let root = fixture.enrollment_root("lease-replacement");
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, "lease-replacement");
    let session = SessionId::new();
    let (mut authority, mut runtime) = promote(&mut fixture, &root, session, &paths);
    // Real post-bootstrap history, so every boundary below has something it
    // could get wrong.
    append_local_batch(&fixture, &mut authority, &mut runtime, 0xD100);

    let verification_digest = authority.verification_digest();
    let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
    let archive_before = archive_digests_outside_the_lease_namespace(&fixture.archive_root);
    let sqlite_before = promoted_projection_digests(&paths.database_path);
    let graph_before = snapshot_files(&fixture.graph_root);
    let head_before = enrollment_head(&root, &binding);
    let generation_before = enrollment_generation(&root, &binding);
    let handoff_before = committed_handoff(&root, &binding, verification_digest);
    let history_before = runtime.engine().durable_history_authority().unwrap();
    let frontier_before = runtime.engine().accepted_frontier_root().unwrap();

    // The replacement itself: a byte-identical file renamed over the exact
    // lease pathname. Nothing an observer of the file's contents, length, or
    // name could notice — only its identity changes.
    let incoming = lease_path.with_extension("lock.incoming");
    fs::write(&incoming, b"").unwrap();
    fs::rename(&incoming, &lease_path).unwrap();

    // A per-mutation admission window still opens *before* any boundary has
    // observed the loss. That is deliberate and documented: lease identity is a
    // stable session fact carried exactly like the archive control-directory
    // identity, so an unchanged-head admission performs no filesystem work for
    // it. Everything the lease actually gates is below — and the first boundary
    // that observes the loss revokes this runtime for good, which
    // `a_boundary_that_loses_the_workspace_lease_revokes_the_runtime_terminally`
    // drives on its own.
    let mut window = runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .unwrap();

    // Boundary: the SQLite advance.
    let error = window
        .drain_projection(16)
        .expect_err("a drain under a replaced lease path must fail closed");
    assert!(
        matches!(
            &error,
            RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                if refusal.demanded_at() == WorkspaceAuthorityBoundary::ProjectionTailDrain
                    && matches!(
                        refusal.cause(),
                        ProjectionError::LeaseIdentityReplaced(_)
                    )
        ),
        "unexpected drain error: {error}"
    );
    drop(window);

    // Boundary: the unabridged binding proof, which is the one every promoted
    // open, handoff, and recovery boundary runs. It is now refused from the
    // latch the drain set, which is the whole point: it must never re-derive
    // authority from facts carried before the replacement.
    let Err(error) =
        runtime.admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
    else {
        panic!("the boundary binding proof must fail closed under a replaced lease path");
    };
    assert!(
        matches!(
            &error,
            RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                if refusal
                    .revocation()
                    .expect("terminal refusal must retain its revocation")
                    .boundary()
                    == WorkspaceAuthorityBoundary::ProjectionTailDrain
                    && matches!(
                        refusal.cause(),
                        ProjectionError::LeaseIdentityReplaced(_)
                    )
        ),
        "unexpected boundary-proof error: {error}"
    );

    // Boundary: the workspace proof itself — what the bootstrap -> promoted
    // lease handover and the crash-takeover compare-and-swap consume.
    let archive = ObjectStore::open(&fixture.archive_root, fixture.workspace).unwrap();
    assert!(
        matches!(
            runtime
                .projection
                .workspace_proof()
                .authorize_archive(&archive, fixture.workspace),
            Err(ProjectionError::LeaseIdentityReplaced(_))
        ),
        "the takeover compare-and-swap's own gate must refuse a replaced lease path"
    );

    // Boundary: the promoted `Safe` handoff, which is a durable claim that this
    // process drained everything it owned.
    let Err(error) = runtime
        .quiesce_and_mark_safe_without_watcher_dependency_for_test(&mut authority, &fixture.graph)
    else {
        panic!("a Safe handoff must not be published under a replaced lease path");
    };
    assert!(
        matches!(error, SafeHandoffUnavailable::Runtime(_)),
        "unexpected Safe-handoff error: {error}"
    );

    // Nothing moved: not the graph, not the authoritative archive, not the
    // enrollment chain, not the engine's durable history, not SQLite.
    assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
    fixture.assert_graph_unchanged();
    assert_eq!(
        archive_digests_outside_the_lease_namespace(&fixture.archive_root),
        archive_before
    );
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        sqlite_before
    );
    assert_eq!(enrollment_head(&root, &binding), head_before);
    assert_eq!(enrollment_generation(&root, &binding), generation_before);
    assert_eq!(
        committed_handoff(&root, &binding, verification_digest),
        handoff_before
    );
    assert_eq!(
        runtime.engine().durable_history_authority().unwrap(),
        history_before
    );
    assert_eq!(
        runtime.engine().accepted_frontier_root().unwrap(),
        frontier_before
    );

    // And the other half of the invariant, at the runtime level: the process
    // that legitimately owns the replacement file is authority, and this one is
    // not. They are never both.
    let newcomer = WorkspaceRuntimeLease::acquire(&archive, fixture.workspace)
        .expect("the replacement file is unlocked, so another runtime may take it");
    newcomer
        .proof()
        .authorize_archive(&archive, fixture.workspace)
        .unwrap();
    assert!(runtime
        .projection
        .workspace_proof()
        .authorize_archive(&archive, fixture.workspace)
        .is_err());
}

/// Failure to perform the identity check is a one-operation refusal, not proof
/// that the lease pathname was replaced. A later exact proof on the same
/// runtime succeeds; no reopen is required and no terminal latch is installed.
#[test]
fn a_transient_lease_identity_check_failure_is_retryable_without_self_healing_replacement() {
    let mut fixture = Fixture::new(
        "lease-transient-check",
        None,
        vec![("pages/transient.md".into(), b"- transient\n".to_vec())],
    );
    let root = fixture.enrollment_root("lease-transient-check");
    let paths = PromotedPaths::new(&fixture, "lease-transient-check");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);

    fail_next_workspace_lease_identity_check();
    let error = match runtime
        .admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
    {
        Ok(_) => panic!("the injected check failure must refuse this operation"),
        Err(error) => error,
    };
    assert!(
        matches!(
            &error,
            RuntimePromotionError::WorkspaceAuthorityCheckUnavailable(refusal)
                if refusal.demanded_at() == WorkspaceAuthorityBoundary::BindingProof
                    && !refusal.is_terminal()
                    && matches!(
                        refusal.cause(),
                        ProjectionError::LeaseIdentityUnavailable(_)
                    )
        ),
        "unexpected transient refusal: {error}"
    );
    assert_eq!(
        runtime.workspace_authority_revocation(),
        None,
        "an unavailable check must not permanently poison the runtime"
    );

    runtime
        .admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
        .expect("a later exact proof on the same lease must succeed");
    assert_eq!(runtime.workspace_authority_revocation(), None);

    fail_next_workspace_lease_identity_check();
    let error = runtime
        .quiesce_and_mark_safe(&mut authority, &fixture.graph)
        .expect_err("the Safe operation must fail closed when its proof is unavailable");
    assert!(
        matches!(
            &error,
            SafeHandoffUnavailable::WorkspaceAuthorityCheckUnavailable(refusal)
                if !refusal.is_terminal()
                    && refusal.demanded_at() == WorkspaceAuthorityBoundary::SafeHandoff
        ),
        "unexpected transient Safe refusal: {error}"
    );
    assert_eq!(runtime.workspace_authority_revocation(), None);
    runtime
        .quiesce_and_mark_safe(&mut authority, &fixture.graph)
        .expect("a later valid Safe proof succeeds without reopening");
    fixture.assert_graph_unchanged();
}

#[test]
fn a_missing_workspace_lease_path_latches_terminally_after_a_retryable_check_failure() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "lease-missing-sticky",
            None,
            vec![("pages/missing.md".into(), b"- missing\n".to_vec())],
        );
        let root = fixture.enrollment_root("lease-missing-sticky");
        let paths = PromotedPaths::new(&fixture, "lease-missing-sticky");
        let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);

        fail_next_workspace_lease_identity_check();
        let Err(unavailable) =
            runtime.admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
        else {
            panic!("the injected inconclusive check must refuse once");
        };
        assert!(matches!(
            unavailable,
            RuntimePromotionError::WorkspaceAuthorityCheckUnavailable(_)
        ));
        assert_eq!(runtime.workspace_authority_revocation(), None);

        let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
        fs::remove_file(&lease_path).unwrap();
        let Err(terminal) =
            runtime.admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
        else {
            panic!("a missing final lease entry positively proves authority loss");
        };
        assert!(matches!(
            &terminal,
            RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                if matches!(
                    refusal.cause(),
                    ProjectionError::LeaseIdentityReplaced(_)
                )
        ));
        let revocation = runtime
            .workspace_authority_revocation()
            .expect("the missing pathname must latch terminal revocation");

        // Neither recreating the final entry nor injecting another transient check
        // can revive authority carried from before the deletion.
        fs::write(&lease_path, b"").unwrap();
        fail_next_workspace_lease_identity_check();
        for attempt in 0..4 {
            let Err(error) = runtime.admit_promoted_mutation(&mut authority, &fixture.graph) else {
                panic!("terminal lease loss must reject every later admission");
            };
            assert!(matches!(
                error,
                RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                    if refusal.revocation() == Some(&revocation)
            ));
            assert_eq!(
                runtime.workspace_authority_revocation(),
                Some(revocation.clone()),
                "attempt {attempt} changed the sticky cause"
            );
        }
        fixture.assert_graph_unchanged();
    });
}

/// The immutable published surface of one archive: exactly the batch manifests
/// and content-addressed objects `publish_prepared` writes.
///
/// Deliberately narrower than the whole archive tree, because authoring a draft
/// legitimately writes engine scratch and index namespaces that also live under
/// the archive root. What must not move when a publication boundary refuses is
/// the immutable batch itself.
fn published_immutable_digests(archive_root: &Path) -> BTreeMap<String, ContentDigest> {
    snapshot_file_digests_matching(archive_root, |path, _| {
        in_top_level_namespace(path, &["batches", "objects"])
    })
}

/// Replace the exact lease pathname with a byte-identical file of a different
/// identity. Nothing an observer of contents, length, or name could notice.
fn replace_workspace_lease_file(lease_path: &Path) {
    let incoming = lease_path.with_extension("lock.incoming");
    fs::write(&incoming, b"").unwrap();
    fs::rename(&incoming, lease_path).unwrap();
}

/// Losing the workspace lease at *one* boundary must kill the runtime at
/// *every* boundary, forever.
///
/// The hole this closes is specific. The unabridged binding proof rereads the
/// lease identity; an ordinary admission does not, because the archive identity
/// facts authenticated at the current enrollment binding generation are carried
/// (`ArchiveAuthentication::Carried`). A failed boundary proof does not advance
/// that generation, so without a terminal latch the very next unchanged-head
/// admission would take the carried branch, issue no filesystem call for the
/// lease at all, and succeed — handing out a mutation window over a workspace
/// another process now legitimately owns.
///
/// So: prove a boundary *successfully* first, replace the lease path, watch the
/// next boundary fail, and then require every later admission, window
/// authorization, mutable-part handout, drain, and `Safe` handoff to refuse.
#[test]
fn a_boundary_that_loses_the_workspace_lease_revokes_the_runtime_terminally() {
    let mut fixture = Fixture::new(
        "sticky-revocation",
        None,
        vec![("pages/sticky.md".into(), b"- sticky\n".to_vec())],
    );
    let root = fixture.enrollment_root("sticky-revocation");
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, "sticky-revocation");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    append_local_batch(&fixture, &mut authority, &mut runtime, 0xD200);

    // A *successful* boundary proof first, so the failure below is a genuine
    // transition rather than a runtime that was never authority.
    runtime
        .admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
        .expect("the boundary proof must succeed while this runtime owns the lease");
    assert_eq!(runtime.workspace_authority_revocation(), None);

    let verification_digest = authority.verification_digest();
    let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
    let archive_before = archive_digests_outside_the_lease_namespace(&fixture.archive_root);
    let sqlite_before = promoted_projection_digests(&paths.database_path);
    let graph_before = snapshot_files(&fixture.graph_root);
    let head_before = enrollment_head(&root, &binding);
    let generation_before = enrollment_generation(&root, &binding);
    let handoff_before = committed_handoff(&root, &binding, verification_digest);
    let history_before = runtime.engine().durable_history_authority().unwrap();
    let frontier_before = runtime.engine().accepted_frontier_root().unwrap();

    replace_workspace_lease_file(&lease_path);

    // The next boundary proof observes the loss and latches it.
    let Err(error) =
        runtime.admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
    else {
        panic!("the boundary proof must fail closed under a replaced lease path");
    };
    assert!(
        matches!(
            &error,
            RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                if refusal.demanded_at() == WorkspaceAuthorityBoundary::BindingProof
                    && matches!(
                        refusal.cause(),
                        ProjectionError::LeaseIdentityReplaced(_)
                    )
        ),
        "unexpected boundary-proof error: {error}"
    );
    let revocation = runtime
        .workspace_authority_revocation()
        .expect("a failed boundary proof must latch terminal revocation");
    assert_eq!(
        revocation.boundary(),
        WorkspaceAuthorityBoundary::BindingProof
    );

    // The load-bearing assertion: several *ordinary* admissions, which are
    // exactly the ones that would otherwise carry the pre-replacement archive
    // facts and perform no filesystem work at all. None of them may open.
    for attempt in 0..8 {
        let Err(error) = runtime.admit_promoted_mutation(&mut authority, &fixture.graph) else {
            panic!("ordinary admission {attempt} opened a window over a revoked runtime");
        };
        assert!(
            matches!(
                &error,
                RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                    if refusal.demanded_at() == WorkspaceAuthorityBoundary::Admission
                        && refusal.revocation() == Some(&revocation)
            ),
            "admission {attempt} refused for the wrong reason: {error}"
        );
    }

    // The promoted `Safe` handoff, which is a durable claim that this process
    // drained everything it owned.
    let Err(error) = runtime
        .quiesce_and_mark_safe_without_watcher_dependency_for_test(&mut authority, &fixture.graph)
    else {
        panic!("a revoked runtime must not publish a Safe handoff");
    };
    assert!(
        matches!(error, SafeHandoffUnavailable::Runtime(_)),
        "unexpected Safe-handoff error: {error}"
    );

    // Nothing moved: not the graph, not the authoritative archive, not the
    // enrollment chain, not the engine's durable history, not SQLite.
    assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
    fixture.assert_graph_unchanged();
    assert_eq!(
        archive_digests_outside_the_lease_namespace(&fixture.archive_root),
        archive_before
    );
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        sqlite_before
    );
    assert_eq!(enrollment_head(&root, &binding), head_before);
    assert_eq!(enrollment_generation(&root, &binding), generation_before);
    assert_eq!(
        committed_handoff(&root, &binding, verification_digest),
        handoff_before
    );
    assert_eq!(
        runtime.engine().durable_history_authority().unwrap(),
        history_before
    );
    assert_eq!(
        runtime.engine().accepted_frontier_root().unwrap(),
        frontier_before
    );
}

/// Revocation is terminal, not a transient observation that heals when the
/// filesystem looks right again.
///
/// This is the case the "sticky" requirement exists for: the provider restores
/// the original file at the original pathname a moment later, so the identity
/// check would pass again. It must not matter. Between the loss and the
/// restore another process could legitimately have taken the archive, so a
/// runtime that healed itself here would be the second applier. Recovery is a
/// fresh reopen or crash takeover — which the end of this test performs, and
/// which does work.
#[test]
fn restoring_the_original_lease_file_never_un_revokes_the_runtime() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "sticky-restore",
            None,
            vec![("pages/restore.md".into(), b"- restore\n".to_vec())],
        );
        let root = fixture.enrollment_root("sticky-restore");
        let paths = PromotedPaths::new(&fixture, "sticky-restore");
        let session = SessionId::new();
        let (mut authority, mut runtime) = promote(&mut fixture, &root, session, &paths);
        append_local_batch(&fixture, &mut authority, &mut runtime, 0xD300);

        let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
        // A hard link preserves the *inode* the running lease is locked on, so the
        // restore below puts back the exact file identity — not merely a file with
        // the same name and bytes. This is the strongest form of the restore.
        let preserved = lease_path.with_extension("lock.preserved");
        fs::hard_link(&lease_path, &preserved).unwrap();

        replace_workspace_lease_file(&lease_path);

        // Fail exactly once.
        let mut window = runtime
            .admit_promoted_mutation(&mut authority, &fixture.graph)
            .expect("the carried admission still opens before any boundary observes the loss");
        assert!(window.drain_projection(16).is_err());
        drop(window);
        let revocation = runtime
            .workspace_authority_revocation()
            .expect("the drain must latch terminal revocation");

        // Put the original identity back at the original pathname.
        fs::remove_file(&lease_path).unwrap();
        fs::rename(&preserved, &lease_path).unwrap();
        // The raw identity check itself now passes again, which is exactly what
        // makes this test meaningful: the refusals below are the latch, not the
        // filesystem.
        runtime
            .projection
            .revalidate_workspace_lease_identity()
            .expect("the restored file is the exact identity this lease is locked on");

        // Every boundary still refuses, and still names the original loss.
        for attempt in 0..4 {
            let Err(error) = runtime.admit_promoted_mutation(&mut authority, &fixture.graph) else {
                panic!("admission {attempt} self-healed after a restored lease file");
            };
            assert!(
                matches!(
                    &error,
                    RuntimePromotionError::WorkspaceAuthorityRevoked(refusal)
                        if refusal.revocation() == Some(&revocation)
                ),
                "admission {attempt} refused for the wrong reason: {error}"
            );
        }
        assert!(runtime
            .admit_promoted_mutation_at_full_depth_for_test(&mut authority, &fixture.graph)
            .is_err());
        assert!(runtime
            .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                &mut authority,
                &fixture.graph
            )
            .is_err());
        assert_eq!(
            runtime.workspace_authority_revocation().as_ref(),
            Some(&revocation),
            "the latched revocation must never be replaced or cleared"
        );

        // And the documented recovery: drop this runtime and reopen. A fresh
        // process contends for the lease honestly and becomes authority again.
        drop(runtime);
        drop(authority);
        let binding = fixture.enrollment_binding();
        let (mut reopened_authority, mut reopened) =
            reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture))
                .expect("a fresh reopen is the recovery path, and it must work");
        assert_eq!(reopened.workspace_authority_revocation(), None);
        reopened
            .admit_promoted_mutation(&mut reopened_authority, &fixture.graph)
            .expect("the reopened runtime is authority again");
        fixture.assert_graph_unchanged();
    });
}

/// `parts()` is the shape both real consumers take — the operational
/// coordinator and the reconciliation session — so handing out
/// `&mut SqliteFrontier` and `&mut TailOverlay` *is* handing out the
/// one-applier-per-workspace write. It must therefore be an authority boundary,
/// not an accessor.
///
/// The window here opened legitimately, before the replacement, which is
/// precisely the dangerous case: without this gate the caller would receive the
/// applier handle on the strength of a proof taken before another process could
/// have claimed the archive.
#[test]
fn parts_refuses_to_vend_the_sqlite_applier_and_tail_after_a_lease_replacement() {
    let mut fixture = Fixture::new(
        "parts-authority",
        None,
        vec![("pages/parts.md".into(), b"- parts\n".to_vec())],
    );
    let root = fixture.enrollment_root("parts-authority");
    let paths = PromotedPaths::new(&fixture, "parts-authority");
    let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);
    append_local_batch(&fixture, &mut authority, &mut runtime, 0xD400);

    let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
    let sqlite_before = promoted_projection_digests(&paths.database_path);

    let mut window = runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .unwrap();
    // The control: while this runtime owns the lease, the same call vends.
    window.parts().expect("an owned workspace vends its parts");

    replace_workspace_lease_file(&lease_path);

    let refusal = window
        .parts()
        .err()
        .expect("parts must not vend the applier under a replaced lease path");
    assert_eq!(
        refusal.demanded_at(),
        WorkspaceAuthorityBoundary::MutableParts
    );
    assert!(matches!(
        refusal.cause(),
        ProjectionError::LeaseIdentityReplaced(_)
    ));
    // Repeated attempts stay refused from the latch, not from a fresh stat that
    // could one day pass again.
    for attempt in 0..4 {
        let refusal = window
            .parts()
            .err()
            .unwrap_or_else(|| panic!("parts attempt {attempt} vended a revoked runtime"));
        assert_eq!(
            refusal
                .revocation()
                .expect("terminal refusal must retain its revocation")
                .boundary(),
            WorkspaceAuthorityBoundary::MutableParts
        );
    }
    // The engine alone is memory rather than the applier, but a revoked runtime
    // must not author into it either.
    assert!(window.engine().is_err());
    drop(window);

    assert!(runtime
        .admit_promoted_mutation(&mut authority, &fixture.graph)
        .is_err());
    assert_eq!(
        promoted_projection_digests(&paths.database_path),
        sqlite_before,
        "no SQLite byte may move once the applier handout is refused"
    );
    fixture.assert_graph_unchanged();
}

/// A real promoted runtime whose graph carries a projected page and one
/// external edit to it, so the production operational coordinator has a genuine
/// external reconciliation to execute.
struct PromotedCoordinatorWorld {
    fixture: Fixture,
    paths: PromotedPaths,
    authority: LocalActiveAuthority,
    runtime: PromotedLocalRuntime,
    page_path: String,
}

impl PromotedCoordinatorWorld {
    fn new(label: &str) -> Self {
        let mut fixture = Fixture::new(
            label,
            None,
            vec![("pages/seed.md".into(), b"- seed\n".to_vec())],
        );
        let root = fixture.enrollment_root(label);
        let paths = PromotedPaths::new(&fixture, label);
        let (mut authority, mut runtime) = promote(&mut fixture, &root, SessionId::new(), &paths);

        const SEED: u128 = 0xC0FFEE;
        append_local_batch(&fixture, &mut authority, &mut runtime, SEED);
        let page_path = format!("pages/promoted-{SEED}.md");
        // Project the authored page through the production projection writer,
        // so the receipt namespace records an expected exact head. Without it
        // an external edit is an unknown path rather than a reconcile.
        write_projection_exact(
            &fixture.graph,
            &fixture.receipts,
            runtime.engine(),
            PageId::from_uuid(Uuid::from_u128(SEED)),
            None,
        )
        .expect("the promoted runtime projects its own authored page");
        // The external edit an honest user makes in another editor, or that a
        // provider delivers.
        fs::write(
            fixture.graph_root.join(&page_path),
            b"- externally edited\n",
        )
        .unwrap();

        Self {
            fixture,
            paths,
            authority,
            runtime,
            page_path,
        }
    }

    fn lease_path(&self) -> PathBuf {
        workspace_lease_path(&self.fixture.archive_root, self.fixture.workspace)
    }

    fn page_bytes(&self) -> Vec<u8> {
        fs::read(self.fixture.graph_root.join(&self.page_path)).unwrap()
    }

    /// One complete coordinator journey through the promoted runtime's own
    /// admitted window, exactly as an activation caller would drive it.
    ///
    /// The coordinator's per-pass operation budget is deliberately small, so an
    /// honest journey over a real graph continues through
    /// `ExternalPublishedContinuation::retry`. Those bounded-slice
    /// continuations are resumed here; a refusal that names the lost workspace
    /// authority is terminal and is returned to the caller.
    fn reconcile(&mut self) -> Result<OperationalCoordinatorState, OperationalCoordinatorError> {
        let Self {
            fixture,
            authority,
            runtime,
            page_path,
            ..
        } = self;
        let requested = [page_path.as_str()];
        let mut window = runtime
            .admit_promoted_mutation(authority, &fixture.graph)
            .expect("the window opens while this runtime owns the workspace");
        let (admission, engine, database, tail) = window
            .parts()
            .expect("the parts handout is authorized while the lease is owned");
        let mut state = OperationalCoordinator::execute(
            &admission,
            &fixture.graph,
            &fixture.receipts,
            engine,
            database,
            tail,
            &requested,
        )?;
        for _ in 0..16 {
            match state {
                OperationalCoordinatorState::FailedClosed(failed)
                    if !failed.failure().detail().contains("workspace authority") =>
                {
                    state = failed.retry(
                        &admission,
                        &fixture.graph,
                        &fixture.receipts,
                        engine,
                        database,
                        tail,
                    );
                }
                terminal => return Ok(terminal),
            }
        }
        panic!("the bounded coordinator journey never converged");
    }
}

/// The coordinator's authority-changing boundaries each re-prove archive-rooted
/// workspace ownership immediately before their own side effect.
///
/// `execute` authorizes the admission once at entry and then runs the whole
/// journey — publication, tail admission, the SQLite advance, and manifested
/// Markdown projection — so a lease replacement anywhere inside it would
/// otherwise be invisible until the next journey. Every one of those four is a
/// write another process may now legitimately own.
///
/// Each case below moves the lease at the durability boundary *just before* a
/// phase and requires that exact phase to refuse. The phase is asserted
/// specifically, not merely "some failure", because a gate that collapsed every
/// loss into one generic error would make the journey undiagnosable.
#[test]
fn moving_the_lease_between_coordinator_phases_refuses_the_next_phase() {
    // Control: with the lease held throughout, the identical journey completes.
    let mut control = PromotedCoordinatorWorld::new("coordinator-control");
    let completion = match control.reconcile() {
        Ok(OperationalCoordinatorState::Complete(completion)) => completion,
        other => panic!(
            "the control journey must complete: {}",
            describe_coordinator_outcome(&other)
        ),
    };
    assert_eq!(completion.batch_id(), completion.import_id().batch_id());
    assert_eq!(
        control.page_bytes(),
        b"- externally edited\n",
        "a completed reconciliation keeps the user's external bytes"
    );
    assert_eq!(control.runtime.workspace_authority_revocation(), None);

    // Phase: publication. The lease moves at the tail-reservation boundary,
    // which is the last step before the first irreversible one, so the
    // publication gate must refuse before an immutable batch is written into
    // the shared archive. (A loss that happens *before* the journey is caught
    // earlier still, by `parts()`.)
    let mut world = PromotedCoordinatorWorld::new("coordinator-publication");
    let lease_path = world.lease_path();
    let archive_before = published_immutable_digests(&world.fixture.archive_root);
    act_once_at(OperationalFaultPoint::AfterReservation, move || {
        replace_workspace_lease_file(&lease_path);
    });
    let failed = expect_coordinator_failure(world.reconcile());
    assert_eq!(failed, OperationalPhase::Publication);
    assert_eq!(
        published_immutable_digests(&world.fixture.archive_root),
        archive_before,
        "a refused publication boundary must publish nothing"
    );

    // Phase: accepted-history archive staging. The immutable manifest is
    // already published, but no scratch root, accepted frontier, history head,
    // accepted tail event, SQLite row, or graph byte may move after the lease
    // is replaced. The already-published continuation retains its pre-stage
    // capacity reservation.
    let mut world = PromotedCoordinatorWorld::new("coordinator-archive-stage");
    let lease_path = world.lease_path();
    let accepted_before = world.runtime.engine().accepted_frontier_root().unwrap();
    let history_before = world.runtime.engine().durable_history_authority().unwrap();
    let snapshot_before = world.runtime.engine().canonical_snapshot().unwrap();
    let sqlite_before = world.runtime.database().frontier_root().unwrap();
    let graph_before = snapshot_files(&world.fixture.graph_root);
    act_once_at(OperationalFaultPoint::AfterManifest, move || {
        replace_workspace_lease_file(&lease_path);
    });
    let outcome = world.reconcile().unwrap();
    let OperationalCoordinatorState::FailedClosed(failed) = outcome else {
        panic!("ArchiveStage authority loss must retain the external continuation");
    };
    assert_eq!(failed.phase(), OperationalPhase::ArchiveStage);
    let revocation = failed
        .failure()
        .revocation()
        .expect("lease replacement must be terminal");
    assert_eq!(
        revocation.boundary(),
        WorkspaceAuthorityBoundary::ArchiveStage
    );
    assert_eq!(
        world.runtime.engine().accepted_frontier_root().unwrap(),
        accepted_before
    );
    assert_eq!(
        world.runtime.engine().durable_history_authority().unwrap(),
        history_before
    );
    assert_eq!(
        world.runtime.engine().canonical_snapshot().unwrap(),
        snapshot_before
    );
    assert!(
        !world
            .runtime
            .database()
            .contains_batch(failed.batch_id())
            .unwrap(),
        "the retained capacity reservation is not an admitted/applied event"
    );
    assert_eq!(
        world.runtime.database().frontier_root().unwrap(),
        sqlite_before
    );
    assert_eq!(snapshot_files(&world.fixture.graph_root), graph_before);

    // Phase: tail admission. The lease moves at the post-stage boundary, so
    // publication and archive staging have happened and the next durable step
    // is the device-local tail admission.
    let mut world = PromotedCoordinatorWorld::new("coordinator-tail-admission");
    let lease_path = world.lease_path();
    let sqlite_before = promoted_projection_digests(&world.paths.database_path);
    act_once_at(OperationalFaultPoint::BeforeTailAdmission, move || {
        replace_workspace_lease_file(&lease_path);
    });
    let failed = expect_coordinator_failure(world.reconcile());
    assert_eq!(failed, OperationalPhase::TailAdmission);
    assert_eq!(
        promoted_projection_digests(&world.paths.database_path),
        sqlite_before,
        "a refused tail-admission boundary must not touch the applier database"
    );

    // Phase: the SQLite advance.
    let mut world = PromotedCoordinatorWorld::new("coordinator-sqlite-drain");
    let lease_path = world.lease_path();
    let frontier_before = world.runtime.database().frontier_root().unwrap();
    act_once_at(OperationalFaultPoint::AfterTailAdmission, move || {
        replace_workspace_lease_file(&lease_path);
    });
    let failed = expect_coordinator_failure(world.reconcile());
    assert_eq!(failed, OperationalPhase::SqliteDrain);
    assert_eq!(
        world.runtime.database().frontier_root().unwrap(),
        frontier_before,
        "a refused SQLite boundary must not advance the accepted frontier"
    );

    // Phase: manifested Markdown projection, which writes the user's own graph.
    let mut world = PromotedCoordinatorWorld::new("coordinator-projection");
    let lease_path = world.lease_path();
    let graph_before = snapshot_files(&world.fixture.graph_root);
    act_once_at(OperationalFaultPoint::AfterSqliteApply, move || {
        replace_workspace_lease_file(&lease_path);
    });
    let failed = expect_coordinator_failure(world.reconcile());
    assert_eq!(failed, OperationalPhase::ProjectionDrain);
    assert_eq!(
        snapshot_files(&world.fixture.graph_root),
        graph_before,
        "a refused projection boundary must not write graph text"
    );

    // And every one of them revoked the runtime terminally, so no later journey
    // can even open its window: repeated attempts publish nothing, touch no
    // SQLite byte, and write no graph text.
    assert!(world.runtime.workspace_authority_revocation().is_some());
    let published_before = published_immutable_digests(&world.fixture.archive_root);
    let sqlite_before = promoted_projection_digests(&world.paths.database_path);
    for attempt in 0..8 {
        let Err(error) = world
            .runtime
            .admit_promoted_mutation(&mut world.authority, &world.fixture.graph)
        else {
            panic!("coordinator attempt {attempt} opened a window over a revoked runtime");
        };
        assert!(
            matches!(error, RuntimePromotionError::WorkspaceAuthorityRevoked(_)),
            "coordinator attempt {attempt} refused for the wrong reason: {error}"
        );
    }
    assert_eq!(
        published_immutable_digests(&world.fixture.archive_root),
        published_before
    );
    assert_eq!(
        promoted_projection_digests(&world.paths.database_path),
        sqlite_before
    );
    assert_eq!(snapshot_files(&world.fixture.graph_root), graph_before);
}

#[test]
fn inconclusive_archive_stage_reproof_is_retryable_without_revocation() {
    let mut world = PromotedCoordinatorWorld::new("coordinator-archive-stage-transient");
    let accepted_before = world.runtime.engine().accepted_frontier_root().unwrap();
    let history_before = world.runtime.engine().durable_history_authority().unwrap();
    let sqlite_before = world.runtime.database().frontier_root().unwrap();
    act_once_at(OperationalFaultPoint::AfterManifest, || {
        fail_next_workspace_lease_identity_check();
    });
    let outcome = world.reconcile().unwrap();
    let OperationalCoordinatorState::FailedClosed(mut failed) = outcome else {
        panic!("an inconclusive ArchiveStage proof must retain retryable continuation");
    };
    assert_eq!(failed.phase(), OperationalPhase::ArchiveStage);
    assert_eq!(failed.failure().revocation(), None);
    assert_eq!(world.runtime.workspace_authority_revocation(), None);
    assert_eq!(
        world.runtime.engine().accepted_frontier_root().unwrap(),
        accepted_before
    );
    assert_eq!(
        world.runtime.engine().durable_history_authority().unwrap(),
        history_before
    );
    assert_eq!(
        world.runtime.database().frontier_root().unwrap(),
        sqlite_before
    );

    let mut window = world
        .runtime
        .admit_promoted_mutation(&mut world.authority, &world.fixture.graph)
        .unwrap();
    let (admission, engine, database, tail) = window.parts().unwrap();
    let completion = loop {
        match failed.retry(
            &admission,
            &world.fixture.graph,
            &world.fixture.receipts,
            engine,
            database,
            tail,
        ) {
            OperationalCoordinatorState::Complete(completion) => break completion,
            OperationalCoordinatorState::FailedClosed(next) => failed = next,
            other => panic!(
                "retryable ArchiveStage proof changed outcome class: {}",
                describe_coordinator_outcome(&Ok(other))
            ),
        }
    };
    assert_eq!(completion.batch_id(), completion.import_id().batch_id());
    drop(window);
    assert_eq!(world.runtime.workspace_authority_revocation(), None);
}

fn describe_coordinator_outcome(
    outcome: &Result<OperationalCoordinatorState, OperationalCoordinatorError>,
) -> String {
    match outcome {
        Err(error) => format!("errored: {error}"),
        Ok(OperationalCoordinatorState::Blocked(_)) => "blocked".into(),
        Ok(OperationalCoordinatorState::Noop) => "no-op".into(),
        Ok(OperationalCoordinatorState::Complete(_)) => "complete".into(),
        Ok(OperationalCoordinatorState::FailedClosed(failed)) => {
            format!("failed closed: {}", failed.failure())
        }
    }
}

/// The phase a lost-workspace refusal named, from either shape the coordinator
/// can report it in: a pre-publication error, or a post-publication
/// failed-closed continuation.
fn expect_coordinator_failure(
    outcome: Result<OperationalCoordinatorState, OperationalCoordinatorError>,
) -> OperationalPhase {
    let (phase, detail) = match &outcome {
        Err(error) => (error.phase(), error.detail().to_owned()),
        Ok(OperationalCoordinatorState::FailedClosed(failed)) => {
            (failed.phase(), failed.failure().detail().to_owned())
        }
        other => panic!(
            "the coordinator must fail closed at its authority boundary, got {}",
            describe_coordinator_outcome(other)
        ),
    };
    assert!(
        detail.contains("workspace authority"),
        "the refusal must name the lost workspace authority: {detail}"
    );
    phase
}

/// The SQLite applier handle and the bounded tail must stay unreachable from
/// the promoted window except through the one call that proves the lease.
///
/// This is a self-policing source assertion in the same class as
/// `resume_point`'s one-production-mint grep and `page_name_index`'s own
/// `#[cfg(test)]` assertion: **if it fails, the invariant moved, not the test.**
/// Re-adding a `database()`/`tail()` accessor on the promoted window — in any
/// form, `#[cfg(test)]` included, because that is one deletion away from
/// production — restores exactly the gap this packet closed: a caller receiving
/// the one-applier-per-workspace write on the strength of a proof taken before
/// the archive could have been claimed by another process.
#[test]
fn the_promoted_window_vends_no_infallible_applier_handle() {
    let source = include_str!("../local_active.rs");
    let window = source
        .split_once("impl PromotedRuntimeSession<'_> {")
        .expect("the promoted window impl block must still exist")
        .1
        .split_once("\n}\n")
        .expect("the promoted window impl block must still close")
        .0;

    for forbidden in ["fn database(", "fn tail(", "fn parts(&mut self) -> ("] {
        assert!(
            !window.contains(forbidden),
            "`{forbidden}` reintroduces an unproved applier handout on the promoted window"
        );
    }
    assert!(
        window.contains("pub(crate) fn parts(\n        &mut self,\n    ) -> Result<"),
        "the production handout must stay fallible"
    );
    assert!(
        window.contains("reprove(WorkspaceAuthorityBoundary::MutableParts)?"),
        "the production handout must re-derive the lease identity before it splits"
    );
}

/// The two defence-in-depth guards around the crash-takeover swap, driven
/// directly.
///
/// Neither is reachable from an ordinary run — that is what makes them
/// defence in depth — so each is exercised at its own boundary instead of
/// through a whole takeover:
///
/// * the compare-and-swap refuses an authorization whose archive-rooted lease
///   belongs to a *different workspace* than this enrollment's binding, even
///   though that lease is genuine, live, and self-consistent with its own
///   archive;
/// * `require_unchanged_bootstrap_anchor` refuses a runtime whose recorded
///   immutable activation anchor is not the one the enrollment chain still
///   proves, rereading the chain rather than trusting memory.
#[test]
fn the_takeover_workspace_and_anchor_guards_each_refuse_directly() {
    let mut fixture = Fixture::new(
        "takeover-guards",
        None,
        vec![("pages/guards.md".into(), b"- guards\n".to_vec())],
    );
    let root = fixture.enrollment_root("takeover-guards");
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, "takeover-guards");
    let session = SessionId::new();
    let (authority, mut runtime) = promote(&mut fixture, &root, session, &paths);

    // A freshly promoted runtime commits `Unsafe { session }`, so its own
    // record is a well-formed takeover predecessor.
    let anchor = reopen_promoted_bootstrap_anchor(&root, &binding).unwrap();
    let predecessor =
        UnsafeHandoffPredecessor::observed_in(&anchor).expect("a promoted runtime commits Unsafe");
    assert_eq!(predecessor.session_id(), session);
    let head_before = runtime.enrollment.committed().enrollment_head();

    // A genuine, live workspace lease over a *different* workspace's archive.
    // It authorizes its own archive perfectly well, which is exactly why the
    // compare-and-swap has to check it against this enrollment's binding.
    let other_workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x91FF));
    let other_store = ObjectStore::open(
        &fixture.root.path().join("other-workspace-archive"),
        other_workspace,
    )
    .unwrap();
    let other_lease = WorkspaceRuntimeLease::acquire(&other_store, other_workspace).unwrap();
    let other_proof = other_lease.proof();
    let authorization = runtime
        .enrollment
        .authenticate_unsafe_predecessor(&other_proof, &other_store, predecessor)
        .expect("the lease is self-consistent with its own archive");
    let successor = SessionId::new();
    let error = runtime
        .enrollment
        .take_over_unsafe_handoff(authorization, successor)
        .map(|_| ())
        .expect_err("a lease for another workspace must not swap this enrollment's record");
    assert!(
        matches!(
            error,
            VerifiedLocalCompositionError::ProofMismatch(
                "the workspace runtime lease is not this enrollment's workspace"
            )
        ),
        "unexpected workspace-proof error: {error}"
    );
    assert_eq!(
        runtime.enrollment.committed().enrollment_head(),
        head_before,
        "a refused takeover must write nothing"
    );
    assert_eq!(
        runtime.enrollment.committed().handoff(),
        LocalActiveHandoff::Unsafe {
            session_id: session
        }
    );

    // A lease rooted at another archive directory does not even mint an
    // authorization, whatever workspace id it carries.
    let look_alike = ObjectStore::open(
        &fixture.root.path().join("look-alike-archive"),
        fixture.workspace,
    )
    .unwrap();
    let look_alike_lease = WorkspaceRuntimeLease::acquire(&look_alike, fixture.workspace).unwrap();
    let look_alike_proof = look_alike_lease.proof();
    let archive = ObjectStore::open(&fixture.archive_root, fixture.workspace).unwrap();
    let error = runtime
        .enrollment
        .authenticate_unsafe_predecessor(&look_alike_proof, &archive, predecessor)
        .expect_err("a lease rooted at another archive must not authorize this takeover");
    assert!(
        matches!(error, VerifiedLocalCompositionError::ProofBinding(_)),
        "unexpected archive-binding error: {error}"
    );

    // The post-swap anchor guard, driven directly. The honest pairing passes.
    let anchor = reopen_promoted_bootstrap_anchor(&root, &binding).unwrap();
    let (evidence, _committed) = anchor.into_predecessor_evidence();
    require_unchanged_bootstrap_anchor(&root, &binding, &evidence, &runtime).unwrap();

    // A runtime whose recorded anchor root is not the one the chain proves is
    // refused, and so is one whose generation moved.
    let honest_anchor = runtime.anchor;
    runtime.anchor.index_root = ContentDigest::of(b"not the bootstrap history root");
    let error = require_unchanged_bootstrap_anchor(&root, &binding, &evidence, &runtime)
        .expect_err("a moved anchor root must be refused");
    assert!(
        matches!(
            error,
            RuntimePromotionError::Enrollment(VerifiedLocalCompositionError::StaleEvidence(
                "bootstrap anchor changed during the promoted handoff transition"
            ))
        ),
        "unexpected anchor error: {error}"
    );
    runtime.anchor = honest_anchor;
    runtime.anchor.generation = honest_anchor.generation.saturating_add(1);
    assert!(require_unchanged_bootstrap_anchor(&root, &binding, &evidence, &runtime).is_err());
    runtime.anchor = honest_anchor;
    require_unchanged_bootstrap_anchor(&root, &binding, &evidence, &runtime).unwrap();

    drop(runtime);
    drop(authority);
    fixture.assert_graph_unchanged();
}

/// A clean `Safe` handoff is written only after the complete device-local drain
/// proof, on the promoted runtime's own retained enrollment session. The
/// restart then adopts exactly one requested new session under exactly one
/// archive-rooted lease, and is the only recovery that unblocks automatic
/// external import. An undrained runtime is still refused, and dropping a
/// writable runtime still leaves the new session `Unsafe`.
#[test]
fn a_clean_safe_restart_adopts_a_new_session_under_one_retained_lease() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-safe",
            None,
            vec![("pages/safe.md".into(), b"- safe\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-safe");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "safe");
        let first = SessionId::new();
        let (verification_digest, frontier) = {
            let (mut authority, mut runtime) = promote(&mut fixture, &root, first, &paths);
            let frontier = runtime.engine().accepted_frontier_root().unwrap();
            let permit = runtime
                .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                    &mut authority,
                    &fixture.graph,
                )
                .expect("a fully drained promoted runtime records a clean handoff");
            assert_eq!(permit.session_id(), first);
            (authority.verification_digest(), frontier)
        };
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Safe,
            "a proved drain records a clean handoff"
        );

        // A crash takeover is not what a clean restart needs, and it must not be
        // what a clean restart silently performs either.
        let second = SessionId::new();
        let before = PromotedRuntimeInstrumentation::capture();
        {
            let (mut authority, mut runtime) =
                reopen_promoted_local_runtime(&root, &binding, second, &paths.open(&fixture))
                    .unwrap();
            let opened = before.since();
            assert_eq!(
                opened.workspace_lease_acquisitions, 1,
                "one restart takes exactly one archive-rooted workspace lease"
            );
            assert_eq!(runtime.recovery(), RuntimeRecoveryState::AdoptedSafeHandoff);
            assert_eq!(
                runtime.automatic_external_import(),
                ExternalImportAdmission::Allowed,
                "only a proved clean handoff may unblock automatic external import"
            );
            assert_eq!(runtime.engine().accepted_frontier_root().unwrap(), frontier);
            assert_eq!(
                committed_handoff(&root, &binding, verification_digest),
                LocalActiveHandoff::Unsafe { session_id: second },
                "the runtime is unsafe again the moment it is writable"
            );

            // Fail-before for the drain proof itself: one accepted local batch
            // leaves projection work outstanding, and `Safe` is refused for that
            // exact named drain rather than synthesized.
            append_local_batch(&fixture, &mut authority, &mut runtime, 0xC700);
            let error = runtime
                .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                    &mut authority,
                    &fixture.graph,
                )
                .expect_err("an undrained runtime must never record a clean handoff");
            assert!(
                matches!(
                    error,
                    SafeHandoffUnavailable::DrainIncomplete {
                        drain: "projection work",
                        ..
                    }
                ),
                "unexpected drain failure: {error}"
            );
        }

        // Dropping a writable runtime leaves the new session unsafe.
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe { session_id: second }
        );
        fixture.assert_graph_unchanged();
    });
}

/// A promoted open that fails after it has taken the workspace lease and the
/// applier slot returns both: the slot goes back to the lease, the lease is
/// released, the enrollment lease is released, nothing authoritative moved, and
/// the very next attempt succeeds.
#[test]
fn a_failed_promoted_open_releases_every_authority_and_stays_retryable() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "takeover-unwind",
            None,
            vec![("pages/unwind.md".into(), b"- unwind\n".to_vec())],
        );
        let root = fixture.enrollment_root("takeover-unwind");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "unwind");
        let crashed = SessionId::new();
        let verification_digest = {
            let (mut authority, mut runtime) = promote(&mut fixture, &root, crashed, &paths);
            append_local_batch(&fixture, &mut authority, &mut runtime, 0xC600);
            authority.verification_digest()
        };
        let world = HelperWorld::new(&fixture, &root, &paths, SessionId::new());
        let mut profile = HelperProcess::spawn(
            "archive-lease",
            &world,
            Some(&fixture.root.path().join("profile-unwind")),
        );

        // Fail the SQLite rebuild, which happens after the workspace lease and its
        // applier slot have been taken.
        let before = authoritative_world(&fixture, &root);
        remove_device_local_database(&paths.database_path);
        crate::oplog::sqlite::fail_next_apply_during_materialization_for_harness();
        takeover_error(
            &root,
            &binding,
            SessionId::new(),
            &paths.open(&fixture),
            "an interrupted materialization must not authorize a takeover",
        );
        assert_eq!(authoritative_world(&fixture, &root), before);
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: crashed
            }
        );

        // Every authority came back: another profile can take the archive lease...
        assert_eq!(profile.ask("acquire"), "acquired");
        assert_eq!(profile.ask("release"), "released");
        profile.finish();

        // ...and this process can retry immediately, including its enrollment lease
        // and the same applier slot.
        let successor = SessionId::new();
        assert_eq!(
            takeover_recovery(&root, &binding, successor, &paths.open(&fixture)),
            RuntimeRecoveryState::TookOverCrashedUnsafe {
                previous_session: crashed
            }
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: successor
            }
        );
        fixture.assert_graph_unchanged();
    });
}

// ---------------------------------------------------------------------------
// P2N10 retained-resume lifecycle.
// ---------------------------------------------------------------------------

/// This endpoint's durable resume-point directory inside one archive.
///
/// Resolved by reading the single durable engine-history endpoint rather than
/// by hard-coding an id, so the test observes exactly the directory production
/// publishes into.
fn resume_point_directory(archive_root: &Path) -> PathBuf {
    let history = archive_root.join("engine-history");
    let mut endpoints: Vec<PathBuf> = fs::read_dir(&history)
        .unwrap_or_else(|error| panic!("a promoted archive has durable history: {error}"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .collect();
    endpoints.sort();
    assert_eq!(
        endpoints.len(),
        1,
        "a promoted archive has exactly one durable engine-history endpoint"
    );
    endpoints.pop().unwrap().join("resume-points")
}

/// Every entry name in the resume-point directory, recognized or not.
fn resume_point_entries(archive_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(resume_point_directory(archive_root)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Exact bytes of the resume-point directory, for the "not one candidate byte
/// changed" assertions.
fn resume_point_bytes(archive_root: &Path) -> BTreeMap<String, Vec<u8>> {
    let directory = resume_point_directory(archive_root);
    if directory.is_dir() {
        snapshot_files(&directory)
    } else {
        BTreeMap::new()
    }
}

fn remove_every_resume_point(archive_root: &Path) {
    for name in resume_point_entries(archive_root) {
        fs::remove_file(resume_point_directory(archive_root).join(name)).unwrap();
    }
}

fn restore_resume_point_bytes(archive_root: &Path, points: &BTreeMap<String, Vec<u8>>) {
    remove_every_resume_point(archive_root);
    let directory = resume_point_directory(archive_root);
    fs::create_dir_all(&directory).unwrap();
    for (name, bytes) in points {
        fs::write(directory.join(name), bytes).unwrap();
    }
}

/// The engine scratch namespace of one archive.
fn scratch_namespace(archive_root: &Path) -> PathBuf {
    archive_root.join("engine-scratch-v2")
}

/// Every scratch run directory currently on disk, sorted.
///
/// Ephemeral runs are removed when their owner drops, so once a runtime has
/// been released this is exactly the retained population.
fn retained_run_directories(archive_root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(scratch_namespace(archive_root)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("run-"))
        .collect();
    names.sort();
    names
}

/// Exact bytes of one scratch run directory.
fn run_directory_bytes(archive_root: &Path, run: &str) -> BTreeMap<String, Vec<u8>> {
    snapshot_files(&scratch_namespace(archive_root).join(run))
}

/// Take one retained run's own exclusive lease, exactly as a live owner holds
/// it. The returned file must stay alive for as long as the lease is wanted.
fn hold_retained_run_lease(archive_root: &Path, run: &str) -> fs::File {
    let path = scratch_namespace(archive_root).join(run).join("lease");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|error| panic!("a retained run has its own lease file: {error}"));
    fs2::FileExt::try_lock_exclusive(&held).expect("the run's exclusive lease is free");
    held
}

/// Everything a caller can observe about one promoted runtime through its
/// public surface.
///
/// Deliberately *not* private-struct equality: the canonical semantic snapshot
/// is the versioned convergence observable, the accepted frontier and durable
/// history authority are what every write path authorizes against, the SQLite
/// projection's applied semantic effects are the user data every query reads,
/// and the ready projection-work queue is what the graph-text writer acts on.
#[derive(Debug)]
struct PublicRuntimeObservation {
    snapshot: crate::oplog::CanonicalSnapshot,
    accepted: AcceptedFrontierRoot,
    history: String,
    sqlite_accepted: AcceptedFrontierRoot,
    sqlite_effects: Vec<String>,
    projection_work: Vec<String>,
    projected_documents: BTreeMap<String, Vec<u8>>,
}

fn public_runtime_observation(runtime: &PromotedLocalRuntime) -> PublicRuntimeObservation {
    let snapshot = runtime.engine().canonical_snapshot().unwrap();
    let projected_documents = snapshot
        .pages
        .iter()
        .map(|(page_id, _)| {
            let page = runtime.engine().materialize_page(*page_id).unwrap();
            let path = page.path.as_str().to_owned();
            let bytes =
                crate::oplog::projection::render_requested_page_document(&page, None).unwrap();
            (path, bytes)
        })
        .collect();
    PublicRuntimeObservation {
        snapshot,
        accepted: runtime.engine().accepted_frontier_root().unwrap(),
        history: format!(
            "{:?}",
            runtime.engine().durable_history_authority().unwrap()
        ),
        sqlite_accepted: runtime.database().frontier_root().unwrap(),
        sqlite_effects: runtime
            .database()
            .applied_semantic_effects_for_test()
            .unwrap()
            .iter()
            .map(|effect| format!("{effect:?}"))
            .collect(),
        projection_work: projection_work_fingerprint(runtime),
        projected_documents,
    }
}

/// Two runtimes are publicly indistinguishable.
///
/// The two accepted-frontier roots are compared with the engine's own
/// `same_accepted_authority`, not with `==`. `AcceptedFrontierRoot::scratch_root`
/// locates the run-local scratch LSM page holding that frontier's point index,
/// and its *file offset* legitimately differs between a run that appended to an
/// adopted file and a run that replayed into a fresh one. Every authenticated
/// field — including `state_digest` and the reference-catalog root — is
/// compared, which is exactly the distinction that makes adoption an
/// accelerator rather than a second truth.
fn assert_publicly_indistinguishable(
    adopted: &PublicRuntimeObservation,
    replayed: &PublicRuntimeObservation,
    what: &str,
) {
    assert_eq!(
        adopted.snapshot, replayed.snapshot,
        "{what}: canonical snapshot"
    );
    assert!(
        adopted.accepted.same_accepted_authority(&replayed.accepted),
        "{what}: accepted authority\n adopted: {:?}\nreplayed: {:?}",
        adopted.accepted,
        replayed.accepted
    );
    assert_eq!(
        adopted.accepted.acceptance_sequence(),
        replayed.accepted.acceptance_sequence(),
        "{what}: acceptance sequence"
    );
    assert_eq!(adopted.history, replayed.history, "{what}: durable history");
    assert!(
        adopted
            .sqlite_accepted
            .same_accepted_authority(&replayed.sqlite_accepted),
        "{what}: SQLite accepted authority"
    );
    assert_eq!(
        adopted.sqlite_effects, replayed.sqlite_effects,
        "{what}: applied SQLite semantic effects"
    );
    assert_eq!(
        adopted.projection_work, replayed.projection_work,
        "{what}: ready projection work"
    );
    assert_eq!(
        adopted.projected_documents, replayed.projected_documents,
        "{what}: projected Markdown/Org bytes"
    );
}

/// Run one lifecycle test body on a thread with a deliberately deep stack.
///
/// A promoted open is a single very large frame by design (see
/// `mint_promoted_runtime`), and these tests open several runtimes in sequence.
/// libtest's worker threads are much smaller than the process main thread every
/// production open actually runs on, so the harness — not the production path —
/// is the constraint here. No assertion changes; only where the body runs does.
fn on_a_deep_stack(body: impl FnOnce() + Send + 'static) {
    crate::test_support::run_on_deep_stack(body);
}

/// Clears the resume-lifecycle cut on entry *and* on drop, including during a
/// panic unwind, so a hook a failing test armed cannot leak into the next test
/// on the same libtest worker thread.
struct ResumeLifecycleCutGuard;

impl ResumeLifecycleCutGuard {
    fn new() -> Self {
        clear_resume_lifecycle_cut_for_test();
        Self
    }
}

impl Drop for ResumeLifecycleCutGuard {
    fn drop(&mut self) {
        clear_resume_lifecycle_cut_for_test();
    }
}

// A promoted runtime is enormous, so every one of the three openers below lives
// and dies in its own frame and returns only a compact value. Holding two of
// them in one test frame overflows the libtest worker thread's stack — the same
// reason `mint_promoted_runtime` and `takeover_error` are written the way they
// are.

fn with_promoted_runtime<T>(
    fixture: &mut Fixture,
    root: &EnrollmentApplicationRoot,
    paths: &PromotedPaths,
    session: SessionId,
    body: impl FnOnce(&Fixture, &mut LocalActiveAuthority, &mut PromotedLocalRuntime) -> T,
) -> T {
    let (mut authority, mut runtime) = promote(fixture, root, session, paths);
    let value = body(&*fixture, &mut authority, &mut runtime);
    drop(runtime);
    drop(authority);
    value
}

fn with_reopened_runtime<T>(
    fixture: &Fixture,
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    paths: &PromotedPaths,
    session: SessionId,
    body: impl FnOnce(&Fixture, &mut LocalActiveAuthority, &mut PromotedLocalRuntime) -> T,
) -> T {
    let (mut authority, mut runtime) =
        reopen_promoted_local_runtime(root, binding, session, &paths.open(fixture))
            .expect("the reopen must succeed");
    let value = body(fixture, &mut authority, &mut runtime);
    drop(runtime);
    drop(authority);
    value
}

fn with_taken_over_runtime<T>(
    fixture: &Fixture,
    root: &EnrollmentApplicationRoot,
    binding: &EnrollmentBindingV1,
    paths: &PromotedPaths,
    session: SessionId,
    body: impl FnOnce(&Fixture, &mut LocalActiveAuthority, &mut PromotedLocalRuntime) -> T,
) -> T {
    let (mut authority, mut runtime) =
        take_over_promoted_local_runtime(root, binding, session, &paths.open(fixture))
            .expect("the takeover must commit");
    let value = body(fixture, &mut authority, &mut runtime);
    drop(runtime);
    drop(authority);
    value
}

/// Return this runtime's automatic post-open publication when it succeeded;
/// otherwise retry through the ordinary later-quiescence publication surface.
fn publish_expecting_success(
    fixture: &Fixture,
    authority: &LocalActiveAuthority,
    runtime: &mut PromotedLocalRuntime,
) -> (u64, RetainedRunMaintenanceReport) {
    let status = match runtime.resume_publication_status().cloned() {
        Some(status @ ResumePublicationStatus::Published { .. }) => status,
        _ => runtime.publish_quiescent_resume_point(authority, &fixture.graph),
    };
    match status {
        ResumePublicationStatus::Published {
            resume_sequence,
            maintenance,
            ..
        } => (
            resume_sequence,
            maintenance.expect("a successful publication authorizes the maintenance pass"),
        ),
        other => panic!("a quiescent promoted runtime must publish, got {other:?}"),
    }
}

#[test]
fn ordinary_startup_read_and_write_never_attempt_packed_patricia_reclamation() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "packed-maintenance-normal-paths",
            None,
            vec![("pages/normal.md".into(), b"- normal\n".to_vec())],
        );
        let root = fixture.enrollment_root("packed-maintenance-normal-paths");
        let paths = PromotedPaths::new(&fixture, "packed-maintenance-normal-paths");

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::Published {
                        packed_maintenance: None,
                        ..
                    })
                ));
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                runtime.engine().accepted_frontier_root().unwrap();
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                append_local_batch(fixture, authority, runtime, 0xE080);
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);
            },
        );
        fixture.assert_graph_unchanged();
    });
}

#[test]
fn packed_patricia_maintenance_is_post_commit_complete_and_best_effort() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "packed-maintenance-best-effort",
            None,
            vec![("pages/maintenance.md".into(), b"- maintenance\n".to_vec())],
        );
        let root = fixture.enrollment_root("packed-maintenance-best-effort");
        let paths = PromotedPaths::new(&fixture, "packed-maintenance-best-effort");

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                fail_next_patricia_reclamation_for_test(PatriciaReclamationFailureForTest::Busy);
                let busy = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                let packed = match busy {
                    ResumePublicationStatus::Published {
                        maintenance: Some(_),
                        packed_maintenance: Some(packed),
                        ..
                    } => packed,
                    other => panic!("Busy maintenance must not fail publication: {other:?}"),
                };
                assert_eq!(
                    packed.indexes.each_ref().map(|index| index.kind),
                    [
                        AuthenticatedPatriciaIndexKind::LogseqUuidClaims,
                        AuthenticatedPatriciaIndexKind::PortablePaths,
                        AuthenticatedPatriciaIndexKind::PageNames,
                        AuthenticatedPatriciaIndexKind::ReferenceCatalog,
                    ]
                );
                assert_eq!(
                    packed.indexes[0].outcome,
                    PackedPatriciaMaintenanceOutcome::Busy
                );
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 4);

                fail_next_patricia_reclamation_for_test(
                    PatriciaReclamationFailureForTest::MalformedAuthority,
                );
                let receipt = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect("maintenance failure must not block a valid Safe handoff");
                let packed = match receipt.publication() {
                    ResumePublicationStatus::Published {
                        packed_maintenance: Some(packed),
                        ..
                    } => packed,
                    other => panic!("post-Safe maintenance must remain report-only: {other:?}"),
                };
                assert!(matches!(
                    packed.indexes[0].outcome,
                    PackedPatriciaMaintenanceOutcome::Unavailable(_)
                ));
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 8);
            },
        );
        fixture.assert_graph_unchanged();
    });
}

#[test]
fn packed_patricia_maintenance_requires_a_successful_durable_replacement() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "packed-maintenance-ordering",
            None,
            vec![("pages/ordering.md".into(), b"- ordering\n".to_vec())],
        );
        let root = fixture.enrollment_root("packed-maintenance-ordering");
        let paths = PromotedPaths::new(&fixture, "packed-maintenance-ordering");

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                remove_every_resume_point(&fixture.archive_root);
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                fail_next_resume_publication_at(ResumePublishBoundary::AfterPrePrune);
                assert!(matches!(
                    runtime.publish_quiescent_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(_)
                ));
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                fail_next_resume_publication_at(ResumePublishBoundary::AfterCommit);
                assert!(matches!(
                    runtime.publish_quiescent_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(_)
                ));
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 0);

                assert!(matches!(
                    runtime.publish_quiescent_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::Published {
                        packed_maintenance: Some(_),
                        ..
                    }
                ));
                assert_eq!(runtime.engine().packed_patricia_reclamation_attempts(), 4);
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// The complete retained-resume lifecycle, in the exact order production runs
/// it, and the equivalence that makes it safe.
///
/// 1. Nothing published: uninterrupted promotion migrates the authenticated
///    bootstrap candidate into a retained run without replay.
/// 2. The quiescent publication — the only place a resume point is ever minted
///    — plus the bounded reclamation its witness authorizes.
/// 3. A restart adopts the published point and replays only the durable tail
///    the point does not already cover, on the *same* retained run.
/// 4. Everything a caller can observe about the adopted runtime equals a fresh
///    full replay of the identical bytes at the identical paths.
#[test]
fn the_resume_accelerator_publishes_adopts_and_reclaims_across_restarts() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-lifecycle",
            None,
            vec![("pages/resume.md".into(), b"- resume\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-lifecycle");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-lifecycle");
        let session = SessionId::new();

        assert!(
            resume_point_entries(&fixture.archive_root).is_empty(),
            "the initial promotion has no published resume point to adopt"
        );

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                // (1) The uninterrupted initial promotion has no predecessor
                //     point. It adopts the already-authenticated same-process
                //     bootstrap candidate instead of replaying its parts.
                let opened = runtime.resume_open_status().clone();
                assert!(
                    opened.retained(),
                    "the provable retention plan authorizes a retained run: {opened:?}"
                );
                assert_eq!(runtime.recovery(), RuntimeRecoveryState::FirstPromotion);
                assert!(opened.adopted());
                assert_eq!(opened.unavailable(), None);
                let initial = opened.observation();
                assert!(!initial.refused);
                assert!(
                    initial.live_history_generation > 0,
                    "the nonempty bootstrap must have durable history: {initial:?}"
                );
                assert_eq!(
                    initial.replay_base_generation, initial.live_history_generation,
                    "same-process promotion adopts the exact bootstrap head"
                );
                assert_eq!(initial.replayed_generations, 0);
                // (2) Before returning the writable runtime, the constructor
                //     made the report-only quiescent publication unavoidable.
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::Published { .. })
                ));
                assert_eq!(resume_point_entries(&fixture.archive_root).len(), 1);
                let (sequence, maintenance) =
                    publish_expecting_success(fixture, authority, runtime);
                assert_eq!(sequence, 1);
                assert_eq!(
                    maintenance.outcome,
                    RetainedRunMaintenanceOutcome::Reclaimed
                );
                assert_eq!(
                    maintenance.reclaimed, 0,
                    "the only retained run is the live one, which the new point reaches"
                );
                assert_eq!(maintenance.retained_runs_remaining, 1);
                assert!(maintenance.within_retained_run_bound);
                assert!(maintenance.preserved_resume_residue.is_empty());
                assert_eq!(resume_point_entries(&fixture.archive_root).len(), 1);

                // Real durable work *after* the point, so the next open genuinely
                // replays a tail rather than nothing at all.
                append_local_batch(fixture, authority, runtime, 0xE100);
                append_local_batch(fixture, authority, runtime, 0xE200);

                // And this is no longer a quiescent cut: an accepted batch
                // leaves ready projection work, so the run-local roots a point
                // would record are still moving. The publication says so rather
                // than recording a state the engine cannot honour.
                let refused = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                assert!(
                    matches!(
                        refused,
                        ResumePublicationStatus::NotPublished(
                            ResumePublicationRefusal::DrainIncomplete(_)
                        )
                    ),
                    "a moving engine must not publish: {refused:?}"
                );
                assert_eq!(
                    resume_point_entries(&fixture.archive_root).len(),
                    1,
                    "the refused publication left the previous point exactly as it was"
                );
            },
        );
        let first_run = retained_run_directories(&fixture.archive_root);
        assert_eq!(first_run.len(), 1, "one session mints exactly one run");

        // (3) Restart: adopt, and replay only the tail.
        let adopted_observation = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, _, runtime| {
                let opened = runtime.resume_open_status().clone();
                assert!(
                    opened.adopted(),
                    "a valid point must be adopted: {opened:?}"
                );
                assert_eq!(opened.unavailable(), None);
                let observation = opened.observation();
                assert!(!observation.refused);
                assert!(
                    observation.replay_base_generation > 0
                        && observation.replay_base_generation < observation.live_history_generation,
                    "the adopted base must be a real earlier generation: {observation:?}"
                );
                assert_eq!(
                    observation.replayed_generations,
                    observation.live_history_generation - observation.replay_base_generation,
                    "an adopted restart replays exactly the durable tail"
                );
                assert_eq!(
                    retained_run_directories(&fixture.archive_root),
                    first_run,
                    "adoption reuses the published run instead of adding one"
                );
                public_runtime_observation(runtime)
            },
        );

        // (4) Exact observable equivalence with a fresh detached bootstrap
        //     reconstruction of the same bytes at the same paths.
        remove_every_resume_point(&fixture.archive_root);
        let replayed_observation = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, _, runtime| {
                let opened = runtime.resume_open_status().clone();
                assert!(opened.adopted(), "{opened:?}");
                assert_eq!(
                    opened.unavailable(),
                    Some(&ResumeAcceleratorUnavailable::NeverPublished)
                );
                assert_eq!(
                    opened.observation().replay_base_generation,
                    u64::from(fixture.verified.part_count())
                );
                assert_eq!(
                    opened.observation().replayed_generations,
                    opened.observation().live_history_generation
                        - opened.observation().replay_base_generation
                );
                public_runtime_observation(runtime)
            },
        );
        assert_publicly_indistinguishable(
            &adopted_observation,
            &replayed_observation,
            "an adopted restart must be publicly indistinguishable from detached reconstruction",
        );
        fixture.assert_graph_unchanged();
    });
}

/// Build one published world, damage exactly one thing about it, and require
/// the next open to be an ordinary full replay that changes no byte.
fn assert_damaged_candidate_replays_in_full(
    label: &str,
    expect_engine_refusal: bool,
    damage: impl FnOnce(&Fixture, &str) -> Option<fs::File>,
) {
    let mut fixture = Fixture::new(
        label,
        None,
        vec![("pages/candidate.md".into(), b"- candidate\n".to_vec())],
    );
    let root = fixture.enrollment_root(label);
    let binding = fixture.enrollment_binding();
    let paths = PromotedPaths::new(&fixture, label);
    let session = SessionId::new();

    with_promoted_runtime(
        &mut fixture,
        &root,
        &paths,
        session,
        |fixture, authority, runtime| {
            publish_expecting_success(fixture, authority, runtime);
            append_local_batch(fixture, authority, runtime, 0xE300);
        },
    );
    let runs = retained_run_directories(&fixture.archive_root);
    assert_eq!(runs.len(), 1);
    let run = runs.into_iter().next().unwrap();

    let held = damage(&fixture, &run);
    let points_before = resume_point_bytes(&fixture.archive_root);
    let run_before = run_directory_bytes(&fixture.archive_root, &run);
    let graph_before = snapshot_files(&fixture.graph_root);

    with_reopened_runtime(
        &fixture,
        &root,
        &binding,
        &paths,
        session,
        |fixture, _, runtime| {
            let opened = runtime.resume_open_status().clone();
            if expect_engine_refusal {
                assert!(!opened.adopted(), "{label}: {opened:?}");
                assert_eq!(opened.observation().replay_base_generation, 0);
            } else {
                assert!(opened.adopted(), "{label}: {opened:?}");
                assert_eq!(
                    opened.observation().replay_base_generation,
                    u64::from(fixture.verified.part_count())
                );
                assert!(opened.unavailable().is_some());
            }
            assert_eq!(
                opened.observation().refused,
                expect_engine_refusal,
                "{label}: unexpected engine-side refusal signal: {opened:?}"
            );
            // The runtime is genuinely usable, not merely constructed.
            runtime.engine().accepted_frontier_root().unwrap();
            runtime.database().frontier_root().unwrap();
            assert!(
                runtime
                    .engine()
                    .instrumentation()
                    .recovery_history_record_reads
                    >= opened.observation().live_history_generation as usize,
                "{label}: refused adoption must retain complete per-record replay validation"
            );
        },
    );
    drop(held);

    assert_eq!(
        resume_point_bytes(&fixture.archive_root),
        points_before,
        "{label}: a refusal must not change one candidate byte"
    );
    assert_eq!(
        run_directory_bytes(&fixture.archive_root, &run),
        run_before,
        "{label}: the refused run must be left byte-for-byte intact"
    );
    assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
    fixture.assert_graph_unchanged();
}

/// A torn point, a provider conflict copy beside it, and a run whose exclusive
/// lease is still held all open as ordinary full replays: none fails startup,
/// and none changes a byte.
#[test]
fn a_torn_conflicted_or_leased_candidate_replays_in_full_without_changing_a_byte() {
    on_a_deep_stack(|| {
        // Torn: the point file itself is truncated. It stops being recognizable, so
        // the strict complete-set proof is denied.
        assert_damaged_candidate_replays_in_full("candidate-torn", false, |fixture, _run| {
            let directory = resume_point_directory(&fixture.archive_root);
            let name = resume_point_entries(&fixture.archive_root)
                .into_iter()
                .next()
                .unwrap();
            let path = directory.join(name);
            let bytes = fs::read(&path).unwrap();
            fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
            None
        });

        // Conflicted: a Syncthing conflict copy carrying *valid* point bytes — the
        // shape most tempting to promote. It is unrecognizable residue, so the whole
        // proof is denied rather than the copy being quietly ignored.
        assert_damaged_candidate_replays_in_full("candidate-conflict", false, |fixture, _run| {
            let directory = resume_point_directory(&fixture.archive_root);
            let name = resume_point_entries(&fixture.archive_root)
                .into_iter()
                .next()
                .unwrap();
            let bytes = fs::read(directory.join(&name)).unwrap();
            fs::write(
                directory.join(format!("{name}.sync-conflict-20260728-120000-ÜBER")),
                bytes,
            )
            .unwrap();
            None
        });

        // Leased: the run the point names is still exclusively held, so the archive
        // boundary refuses the adoption before the engine exists.
        assert_damaged_candidate_replays_in_full("candidate-leased", true, |fixture, run| {
            Some(hold_retained_run_lease(&fixture.archive_root, run))
        });
    });
}

/// The production Safe transaction clears its Unsafe point, commits and
/// freshly reopens Safe, publishes an exactly Safe-bound successor, and a
/// fresh-session clean restart adopts it with observations equal to full
/// replay.
#[test]
fn a_clean_safe_restart_adopts_the_exact_safe_bound_point() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-safe-superseded",
            None,
            vec![("pages/safe.md".into(), b"- safe\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-superseded");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-safe-superseded");
        let session = SessionId::new();

        let verification_digest = with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                let receipt = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect("the production transaction records a clean handoff");
                assert_eq!(
                    receipt.cleared().removed,
                    1,
                    "the recognized Unsafe point is cleared before Safe"
                );
                assert!(matches!(
                    receipt.publication(),
                    ResumePublicationStatus::Published { .. }
                ));
                authority.verification_digest()
            },
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Safe
        );
        let points_before = resume_point_bytes(&fixture.archive_root);
        assert_eq!(points_before.len(), 1);

        let fresh_session = SessionId::new();
        let adopted = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            fresh_session,
            |_, _, runtime| {
                assert_eq!(runtime.recovery(), RuntimeRecoveryState::AdoptedSafeHandoff);
                let opened = runtime.resume_open_status().clone();
                assert!(opened.adopted(), "{opened:?}");
                assert_eq!(opened.unavailable(), None);
                assert!(
                    runtime
                        .engine()
                        .instrumentation()
                        .recovery_history_record_reads
                        <= 1,
                    "an exact-current adopted open may read only the current head for its automatic successor publication, never historical bootstrap records"
                );
                assert_eq!(
                    runtime
                        .engine()
                        .bootstrap_recovery_instrumentation()
                        .bootstrap_part_reads,
                    0,
                    "the aggregate-prefix proof must avoid bootstrap payload replay"
                );
                public_runtime_observation(runtime)
            },
        );

        remove_every_resume_point(&fixture.archive_root);
        let replayed = with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| {
                assert!(runtime.resume_open_status().adopted());
                public_runtime_observation(runtime)
            },
        );
        assert_publicly_indistinguishable(
            &adopted,
            &replayed,
            "Safe-bound adoption must equal detached bootstrap reconstruction",
        );
        assert_eq!(
            points_before.len(),
            1,
            "the Safe transaction leaves exactly its Safe successor"
        );
        fixture.assert_graph_unchanged();
    });
}

/// Clear is the last fallible deletion step before the durable Safe commit.
/// Its failure removes nothing and leaves the exact committed enrollment
/// Unsafe, so a later retry or crash takeover still has honest evidence.
#[test]
fn a_clear_failure_leaves_enrollment_unsafe_and_preserves_the_point_bytes() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-safe-clear-failure",
            None,
            vec![("pages/clear.md".into(), b"- clear\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-clear-failure");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-safe-clear-failure");
        let session = SessionId::new();

        let verification_digest = with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                let before = resume_point_bytes(&fixture.archive_root);
                fail_next_resume_clear();
                let error = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect_err("the injected clear failure must abort before Safe");
                assert!(
                    matches!(error, SafeHandoffUnavailable::Store(_)),
                    "unexpected clear refusal: {error}"
                );
                assert_eq!(
                    resume_point_bytes(&fixture.archive_root),
                    before,
                    "a failed clear must preserve every candidate byte"
                );
                assert_eq!(
                    authority.handoff(),
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    }
                );
                authority.verification_digest()
            },
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: session
            }
        );
        fixture.assert_graph_unchanged();
    });
}

#[test]
fn lease_replacement_between_initial_safe_validation_and_clear_deletes_nothing() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut fixture = Fixture::new(
            "resume-safe-lease-before-clear",
            None,
            vec![("pages/before-clear.md".into(), b"- before clear\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-lease-before-clear");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-safe-lease-before-clear");
        let session = SessionId::new();
        let verification_digest = with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                let points_before = resume_point_bytes(&fixture.archive_root);
                assert_eq!(points_before.len(), 1);
                let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
                act_once_at_resume_lifecycle_cut_for_test(
                    ResumeLifecycleCut::BeforeSafeClear,
                    Box::new(move || replace_workspace_lease_file(&lease_path)),
                );
                let error = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect_err("the clear boundary must revalidate the live lease");
                assert!(matches!(
                    &error,
                    SafeHandoffUnavailable::WorkspaceAuthorityRevoked(revocation)
                        if matches!(
                            revocation.cause(),
                            ProjectionError::LeaseIdentityReplaced(_)
                        )
                ));
                assert_eq!(
                    resume_point_bytes(&fixture.archive_root),
                    points_before,
                    "authority loss before clear must delete no point"
                );
                assert_eq!(
                    authority.handoff(),
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    }
                );
                for _ in 0..3 {
                    let Err(error) = runtime.admit_promoted_mutation(authority, &fixture.graph)
                    else {
                        panic!("terminal Safe-boundary loss must reject later admission");
                    };
                    assert!(matches!(
                        error,
                        RuntimePromotionError::WorkspaceAuthorityRevoked(_)
                    ));
                }
                authority.verification_digest()
            },
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: session
            }
        );
        fixture.assert_graph_unchanged();
    });
}

#[test]
fn lease_deletion_after_clear_but_before_safe_stays_unsafe_and_full_replays() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut fixture = Fixture::new(
            "resume-safe-lease-after-clear",
            None,
            vec![("pages/after-clear.md".into(), b"- after clear\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-lease-after-clear");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-safe-lease-after-clear");
        let session = SessionId::new();
        let verification_digest = with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                assert_eq!(resume_point_entries(&fixture.archive_root).len(), 1);
                let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);
                act_once_at_resume_lifecycle_cut_for_test(
                    ResumeLifecycleCut::AfterSafeClear,
                    Box::new(move || fs::remove_file(&lease_path).unwrap()),
                );
                let error = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect_err("the durable Safe boundary must revalidate after clear");
                assert!(matches!(
                    &error,
                    SafeHandoffUnavailable::WorkspaceAuthorityRevoked(revocation)
                        if matches!(
                            revocation.cause(),
                            ProjectionError::LeaseIdentityReplaced(_)
                        )
                ));
                assert!(
                    resume_point_entries(&fixture.archive_root).is_empty(),
                    "only reconstructible Unsafe points may have been cleared"
                );
                assert_eq!(
                    authority.handoff(),
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    }
                );
                for _ in 0..3 {
                    let Err(error) = runtime.admit_promoted_mutation(authority, &fixture.graph)
                    else {
                        panic!("terminal post-clear loss must reject later admission");
                    };
                    assert!(matches!(
                        error,
                        RuntimePromotionError::WorkspaceAuthorityRevoked(_)
                    ));
                }
                authority.verification_digest()
            },
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Unsafe {
                session_id: session
            }
        );

        with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| {
                assert!(runtime.resume_open_status().adopted());
                assert!(runtime.resume_open_status().unavailable().is_some());
                runtime.engine().accepted_frontier_root().unwrap();
                runtime.database().frontier_root().unwrap();
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// Failures before a Safe point's commit are report-only: the durable Safe
/// record remains valid, no publication witness exists to authorize
/// reclamation, and the next clean restart performs an ordinary full replay.
#[test]
fn post_safe_mint_and_precommit_publication_failures_full_replay_without_reclamation() {
    for (label, residue) in [("mint", true), ("precommit", false)] {
        on_a_deep_stack(move || {
            let mut fixture = Fixture::new(
                &format!("resume-safe-publish-{label}"),
                None,
                vec![("pages/publish.md".into(), b"- publish\n".to_vec())],
            );
            let root = fixture.enrollment_root(&format!("resume-safe-publish-{label}"));
            let binding = fixture.enrollment_binding();
            let paths = PromotedPaths::new(&fixture, &format!("resume-safe-publish-{label}"));
            let verification_digest = with_promoted_runtime(
                &mut fixture,
                &root,
                &paths,
                SessionId::new(),
                |fixture, authority, runtime| {
                    if residue {
                        fs::create_dir_all(resume_point_directory(&fixture.archive_root)).unwrap();
                        fs::write(
                            resume_point_directory(&fixture.archive_root).join(".DS_Store"),
                            b"preserved residue",
                        )
                        .unwrap();
                    } else {
                        fail_next_resume_publication_at(ResumePublishBoundary::AfterPrePrune);
                    }
                    let runs_before = retained_run_directories(&fixture.archive_root);
                    let receipt = runtime
                        .quiesce_and_mark_safe(authority, &fixture.graph)
                        .expect("post-Safe accelerator failure cannot fail the handoff");
                    assert!(
                        matches!(
                            receipt.publication(),
                            ResumePublicationStatus::NotPublished(ResumePublicationRefusal::Store(
                                _
                            ))
                        ),
                        "{label}: unexpected publication status: {:?}",
                        receipt.publication()
                    );
                    assert_eq!(
                        retained_run_directories(&fixture.archive_root),
                        runs_before,
                        "{label}: no publication witness may authorize reclamation"
                    );
                    authority.verification_digest()
                },
            );
            assert_eq!(
                committed_handoff(&root, &binding, verification_digest),
                LocalActiveHandoff::Safe,
                "{label}"
            );

            with_reopened_runtime(
                &fixture,
                &root,
                &binding,
                &paths,
                SessionId::new(),
                |_, _, runtime| {
                    assert!(runtime.resume_open_status().unavailable().is_some());
                },
            );
            fixture.assert_graph_unchanged();
        });
    }
}

/// Intake that lands after the watcher quiesce begins invalidates the durable
/// Safe proof. Releasing the failed barrier promotes that deferred event into
/// ordinary later work; after the facade drains and acknowledges it, the exact
/// same production transaction succeeds.
#[test]
fn watcher_intake_racing_quiesce_prevents_safe_and_becomes_later_work() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut fixture = Fixture::new(
            "resume-safe-watcher-race",
            None,
            vec![("pages/watcher.md".into(), b"- watcher\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-watcher-race");
        let paths = PromotedPaths::new(&fixture, "resume-safe-watcher-race");

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                let handle = runtime.watcher_handle();
                let binding = handle.binding();
                act_once_at_resume_lifecycle_cut_for_test(
                    ResumeLifecycleCut::AfterWatcherQuiesce,
                    Box::new(move || {
                        let intake = handle
                            .enqueue(binding, [WatcherObservation::NotifyError])
                            .expect("intake during the soft quiesce is retained");
                        assert!(intake.deferred_by_quiesce);
                    }),
                );
                let error = runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect_err("racing watcher intake must defeat Safe");
                assert!(
                    matches!(
                        error,
                        SafeHandoffUnavailable::Watcher(
                            WatcherQuiesceError::ArrivedDuringQuiesce { .. }
                        )
                    ),
                    "unexpected watcher race outcome: {error}"
                );
                assert!(runtime.watcher_status().pending);
                assert!(!runtime.watcher_status().deferred);

                let drain = runtime
                    .begin_watcher_drain()
                    .unwrap()
                    .expect("the deferred event becomes later drainable work");
                runtime.acknowledge_watcher_drain(drain.epoch()).unwrap();
                assert!(!runtime.watcher_status().pending);
                runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect("Safe succeeds after the later watcher work is settled");
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// The graph reservation is still live after the enrollment record has become
/// durably Safe and before Safe-point publication starts. A writer admission
/// at that exact cut is refused by the real graph gate.
#[test]
fn graph_writer_cannot_cross_handoff_safe_before_safe_publication() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut fixture = Fixture::new(
            "resume-safe-graph-barrier",
            None,
            vec![("pages/barrier.md".into(), b"- barrier\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-safe-graph-barrier");
        let paths = PromotedPaths::new(&fixture, "resume-safe-graph-barrier");
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                arm_safe_graph_writer_probe_for_test();
                runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect("the production Safe transaction succeeds");
                assert_eq!(
                    take_safe_graph_writer_probe_for_test(),
                    Some(std::io::ErrorKind::WouldBlock),
                    "the graph writer gate must remain closed after Safe commit"
                );
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// Durable crash cuts around clear and Safe preserve the one legal prefix at
/// each boundary: before clear the Unsafe point survives; after clear the
/// enrollment is still Unsafe with no point; after the Safe commit the record
/// is Safe and point publication remains optional.
#[test]
fn safe_transaction_crash_cuts_reopen_only_legal_durable_prefixes() {
    for (label, cut, expected_handoff, expected_points) in [
        (
            "before-clear",
            ResumeLifecycleCut::BeforeSafeClear,
            0_u8,
            1_usize,
        ),
        (
            "after-clear",
            ResumeLifecycleCut::AfterSafeClear,
            0_u8,
            0_usize,
        ),
        (
            "after-safe",
            ResumeLifecycleCut::AfterSafeCommit,
            1_u8,
            0_usize,
        ),
    ] {
        on_a_deep_stack(move || {
            let _guard = ResumeLifecycleCutGuard::new();
            let mut fixture = Fixture::new(
                &format!("resume-safe-crash-{label}"),
                None,
                vec![("pages/crash.md".into(), b"- crash\n".to_vec())],
            );
            let root = fixture.enrollment_root(&format!("resume-safe-crash-{label}"));
            let binding = fixture.enrollment_binding();
            let paths = PromotedPaths::new(&fixture, &format!("resume-safe-crash-{label}"));
            let session = SessionId::new();
            let verification_digest = with_promoted_runtime(
                &mut fixture,
                &root,
                &paths,
                session,
                |fixture, authority, runtime| {
                    publish_expecting_success(fixture, authority, runtime);
                    act_once_at_resume_lifecycle_cut_for_test(
                        cut,
                        Box::new(|| panic!("injected Safe transaction crash cut")),
                    );
                    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = runtime.quiesce_and_mark_safe(authority, &fixture.graph);
                    }));
                    assert!(crashed.is_err(), "{label}: the cut did not fire");
                    authority.verification_digest()
                },
            );
            let durable = committed_handoff(&root, &binding, verification_digest);
            if expected_handoff == 0 {
                assert_eq!(
                    durable,
                    LocalActiveHandoff::Unsafe {
                        session_id: session
                    },
                    "{label}"
                );
            } else {
                assert_eq!(durable, LocalActiveHandoff::Safe, "{label}");
            }
            assert_eq!(
                resume_point_entries(&fixture.archive_root).len(),
                expected_points,
                "{label}"
            );
            fixture.assert_graph_unchanged();
        });
    }
}

/// Every writable-open constructor publishes before it returns, so no caller
/// can omit the post-open/pre-first-mutation attempt. Each early crash takeover
/// therefore replaces the predecessor point instead of retaining another run
/// forever; the sealed one-shot cut cannot be replayed even after a refusal.
#[test]
fn post_open_publication_bounds_crash_takeovers_and_reclaims_the_first_unreachable_run() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-post-open-bounded",
            None,
            vec![("pages/post-open.md".into(), b"- post open\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-post-open-bounded");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-post-open-bounded");

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::Published { .. })
                ));
                assert_eq!(
                    runtime.publish_post_open_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::EngineNotPublishable
                    ),
                    "the constructor must close the one-shot window before returning"
                );
            },
        );

        for attempt in 0..4 {
            with_taken_over_runtime(
                &fixture,
                &root,
                &binding,
                &paths,
                SessionId::new(),
                |fixture, authority, runtime| {
                    assert!(
                        runtime.resume_open_status().adopted(),
                        "takeover {attempt} must adopt before its CAS"
                    );
                    assert!(matches!(
                        runtime.resume_publication_status(),
                        Some(ResumePublicationStatus::Published { .. })
                    ));
                    assert_eq!(
                        runtime.publish_post_open_resume_point(authority, &fixture.graph),
                        ResumePublicationStatus::NotPublished(
                            ResumePublicationRefusal::EngineNotPublishable
                        )
                    );
                },
            );
            assert_eq!(
                retained_run_directories(&fixture.archive_root).len(),
                1,
                "post-open publication must bound takeover {attempt}"
            );
            assert_eq!(resume_point_entries(&fixture.archive_root).len(), 1);
        }

        let predecessor = retained_run_directories(&fixture.archive_root)
            .into_iter()
            .next()
            .unwrap();
        let point_path = {
            let name = resume_point_entries(&fixture.archive_root)
                .into_iter()
                .next()
                .unwrap();
            resume_point_directory(&fixture.archive_root).join(name)
        };
        let bytes = fs::read(&point_path).unwrap();
        fs::write(&point_path, &bytes[..bytes.len() / 2]).unwrap();

        with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                assert!(runtime.resume_open_status().adopted());
                assert!(runtime.resume_open_status().unavailable().is_some());
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::Store(_)
                    ))
                ));
                remove_every_resume_point(&fixture.archive_root);
                assert_eq!(
                    runtime.publish_post_open_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::EngineNotPublishable
                    ),
                    "even a failed automatic attempt permanently closes the one-shot window"
                );
                let status = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                assert!(
                    matches!(
                        status,
                        ResumePublicationStatus::Published {
                            maintenance: Some(_),
                            ..
                        }
                    ),
                    "the first clean post-open publication must carry maintenance: {status:?}"
                );
            },
        );
        assert!(
            !retained_run_directories(&fixture.archive_root).contains(&predecessor),
            "the successfully witnessed publication must reclaim the unreachable predecessor"
        );
        assert_eq!(retained_run_directories(&fixture.archive_root).len(), 1);
        fixture.assert_graph_unchanged();
    });
}

/// One promoted world whose archive holds an *unreachable* retained predecessor
/// run.
///
/// Session one published a point naming run A and then crashed. The takeover
/// could not adopt A — its exclusive run lease was held at the instant of the
/// open, exactly as a live predecessor or a torn run would present — so it
/// replayed in full into a fresh run B. A now exists, is retained, and is named
/// only by a point the next publication supersedes.
struct SupersededPredecessorWorld {
    fixture: Fixture,
    root: EnrollmentApplicationRoot,
    binding: EnrollmentBindingV1,
    paths: PromotedPaths,
    authority: LocalActiveAuthority,
    runtime: PromotedLocalRuntime,
    predecessor_run: String,
}

impl SupersededPredecessorWorld {
    fn new(label: &str) -> Self {
        let mut fixture = Fixture::new(
            label,
            None,
            vec![("pages/superseded.md".into(), b"- superseded\n".to_vec())],
        );
        let root = fixture.enrollment_root(label);
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, label);
        let crashed = SessionId::new();

        // Deliberately no local batch after the publication: this world exists
        // to be *published from* after the takeover, and an accepted batch
        // leaves ready projection work, which is not a quiescent cut.
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            crashed,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
            },
        );
        let runs = retained_run_directories(&fixture.archive_root);
        assert_eq!(runs.len(), 1);
        let predecessor_run = runs.into_iter().next().unwrap();

        // Hold the predecessor run's exclusive lease across the takeover open
        // only. Releasing it afterwards is what makes the run reclaimable once
        // a replacement point exists.
        let held = hold_retained_run_lease(&fixture.archive_root, &predecessor_run);
        let (authority, runtime) = take_over_promoted_local_runtime(
            &root,
            &binding,
            SessionId::new(),
            &paths.open(&fixture),
        )
        .expect("a crash takeover succeeds whether or not the accelerator is usable");
        fs2::FileExt::unlock(&held).unwrap();
        drop(held);

        assert!(matches!(
            runtime.recovery(),
            RuntimeRecoveryState::TookOverCrashedUnsafe { .. }
        ));
        let opened = runtime.resume_open_status().clone();
        assert!(
            !opened.adopted(),
            "the leased run must be refused: {opened:?}"
        );
        assert!(opened.observation().refused);
        let runs = retained_run_directories(&fixture.archive_root);
        assert_eq!(
            runs.len(),
            2,
            "a refused adoption replays into a fresh retained run"
        );
        assert!(runs.contains(&predecessor_run));

        Self {
            fixture,
            root,
            binding,
            paths,
            authority,
            runtime,
            predecessor_run,
        }
    }
}

/// A crash takeover's own quiescent publication supersedes the predecessor's
/// point and immediately reclaims the run that point was the only reference to.
///
/// This is the self-healing property: a crash-heavy sequence of refused
/// adoptions cannot accumulate archive directories forever, because the first
/// clean quiescence both replaces the evidence and collects what the
/// replacement no longer reaches.
#[test]
fn a_takeover_publication_supersedes_the_predecessor_point_and_reclaims_its_run() {
    on_a_deep_stack(|| {
        let mut world = SupersededPredecessorWorld::new("resume-supersede");
        let predecessor = world.predecessor_run.clone();
        let (sequence, maintenance) = match world
            .runtime
            .publish_quiescent_resume_point(&world.authority, &world.fixture.graph)
        {
            ResumePublicationStatus::Published {
                resume_sequence,
                maintenance: Some(maintenance),
                ..
            } => (resume_sequence, maintenance),
            other => panic!("later quiescence must publish and reclaim: {other:?}"),
        };
        assert_eq!(
            sequence, 3,
            "the later publication follows both automatic post-open attempts"
        );
        assert_eq!(
            maintenance.outcome,
            RetainedRunMaintenanceOutcome::Reclaimed
        );
        assert_eq!(
            maintenance.reclaimed, 1,
            "exactly the unreachable predecessor"
        );
        assert_eq!(maintenance.retained_runs_remaining, 1);
        assert!(maintenance.within_retained_run_bound);
        assert_eq!(maintenance.unclassified_preserved, 0);

        let runs = retained_run_directories(&world.fixture.archive_root);
        assert_eq!(runs.len(), 1);
        assert!(
            !runs.contains(&predecessor),
            "the reclaimed predecessor must be gone: {runs:?}"
        );
        assert_eq!(
            resume_point_entries(&world.fixture.archive_root).len(),
            1,
            "publication keeps the durable point set bounded"
        );
        world.fixture.assert_graph_unchanged();
    });
}

/// Losing the workspace between a committed publication and the maintenance
/// pass it authorized deletes nothing and latches terminal revocation.
///
/// Reclamation is the only boundary in this module that can delete archive
/// bytes, so it reproves ownership on its own rather than inheriting the
/// publication's proof.
#[test]
fn losing_the_lease_before_reclamation_deletes_nothing_and_latches() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut world = SupersededPredecessorWorld::new("resume-lease-reclaim");
        let predecessor = world.predecessor_run.clone();
        let lease_path = workspace_lease_path(&world.fixture.archive_root, world.fixture.workspace);

        let replaced = lease_path.clone();
        act_once_at_resume_lifecycle_cut_for_test(
            ResumeLifecycleCut::BeforeReclamation,
            Box::new(move || replace_workspace_lease_file(&replaced)),
        );
        let published = world
            .runtime
            .publish_quiescent_resume_point(&world.authority, &world.fixture.graph);
        assert_eq!(
            published,
            ResumePublicationStatus::Published {
                resume_sequence: 3,
                maintenance: None,
                packed_maintenance: None,
            },
            "a lost workspace must skip the maintenance pass, not run it"
        );
        let revocation = world
            .runtime
            .workspace_authority_revocation()
            .expect("the reclamation boundary must latch terminal revocation");
        assert_eq!(
            revocation.boundary(),
            WorkspaceAuthorityBoundary::ResumeReclamation
        );
        let runs = retained_run_directories(&world.fixture.archive_root);
        assert_eq!(runs.len(), 2, "nothing may be deleted without the proof");
        assert!(runs.contains(&predecessor));

        // The latch is terminal: a second attempt refuses before it reads anything.
        assert_eq!(
            world
                .runtime
                .publish_quiescent_resume_point(&world.authority, &world.fixture.graph),
            ResumePublicationStatus::NotPublished(
                ResumePublicationRefusal::WorkspaceAuthorityRevoked(revocation)
            )
        );
        assert_eq!(retained_run_directories(&world.fixture.archive_root), runs);
        world.fixture.assert_graph_unchanged();
    });
}

/// Losing the workspace immediately before the snapshot/mint/publication
/// refuses the publication, latches terminal revocation, and writes nothing.
#[test]
fn losing_the_lease_before_publication_publishes_nothing_and_latches() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-lease-publication",
            None,
            vec![("pages/publish.md".into(), b"- publish\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-lease-publication");
        let paths = PromotedPaths::new(&fixture, "resume-lease-publication");
        let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                let points_before = resume_point_bytes(&fixture.archive_root);
                let runs_before = retained_run_directories(&fixture.archive_root);
                let archive_before =
                    archive_digests_outside_the_lease_namespace(&fixture.archive_root);
                let sqlite_before = promoted_projection_digests(&paths.database_path);
                let graph_before = snapshot_files(&fixture.graph_root);

                replace_workspace_lease_file(&lease_path);
                let refused = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                let ResumePublicationStatus::NotPublished(
                    ResumePublicationRefusal::WorkspaceAuthorityRevoked(revocation),
                ) = &refused
                else {
                    panic!("a replaced lease must refuse the publication: {refused:?}");
                };
                assert_eq!(
                    revocation.boundary(),
                    WorkspaceAuthorityBoundary::ResumePublication
                );

                assert_eq!(resume_point_bytes(&fixture.archive_root), points_before);
                assert_eq!(retained_run_directories(&fixture.archive_root), runs_before);
                assert_eq!(
                    archive_digests_outside_the_lease_namespace(&fixture.archive_root),
                    archive_before
                );
                assert_eq!(
                    promoted_projection_digests(&paths.database_path),
                    sqlite_before
                );
                assert_eq!(snapshot_files(&fixture.graph_root), graph_before);
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// Losing the workspace immediately before a published candidate is read fails
/// the whole open closed.
///
/// There is no runtime to latch yet — the lease is this open's one-applier
/// proof, so the honest outcome is that the open does not happen at all, and
/// every authoritative byte stays where it was.
#[test]
fn losing_the_lease_before_the_candidate_read_fails_the_open_closed() {
    on_a_deep_stack(|| {
        let _guard = ResumeLifecycleCutGuard::new();
        let mut fixture = Fixture::new(
            "resume-lease-candidate",
            None,
            vec![("pages/candidate.md".into(), b"- candidate\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-lease-candidate");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-lease-candidate");
        let session = SessionId::new();
        let lease_path = workspace_lease_path(&fixture.archive_root, fixture.workspace);

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                append_local_batch(fixture, authority, runtime, 0xE700);
            },
        );

        let world_before = authoritative_world(&fixture, &root);
        let points_before = resume_point_bytes(&fixture.archive_root);
        let runs_before = retained_run_directories(&fixture.archive_root);
        // The exact bytes of the run the surviving point names. This is the
        // load-bearing half: a process that no longer owns the workspace must
        // not reach the point at all, let alone take the run it names and
        // append a replayed tail into it.
        let run_before = run_directory_bytes(&fixture.archive_root, &runs_before[0]);
        let graph_before = snapshot_files(&fixture.graph_root);

        let replaced = lease_path.clone();
        act_once_at_resume_lifecycle_cut_for_test(
            ResumeLifecycleCut::BeforeCandidateRead,
            Box::new(move || replace_workspace_lease_file(&replaced)),
        );
        let error = reopen_promoted_local_runtime(&root, &binding, session, &paths.open(&fixture))
            .err()
            .expect("a workspace lost before the candidate read must fail the open closed");
        assert!(
            error.to_string().contains("workspace"),
            "unexpected refusal: {error}"
        );

        assert_eq!(authoritative_world(&fixture, &root), world_before);
        assert_eq!(resume_point_bytes(&fixture.archive_root), points_before);
        assert_eq!(retained_run_directories(&fixture.archive_root), runs_before);
        assert_eq!(
            run_directory_bytes(&fixture.archive_root, &runs_before[0]),
            run_before,
            "the candidate's retained run must not be opened, adopted, or appended to"
        );
        assert_eq!(snapshot_files(&fixture.graph_root), graph_before);

        // And the very next honest open still works.
        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                runtime.engine().accepted_frontier_root().unwrap();
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// Reaching the retained-run bound beside residue nothing can classify makes
/// the next open ephemeral: a full replay, and **no** new retained run.
///
/// Without this the flip to retained runs would leak one permanently
/// uncollectable archive directory per restart, because a single conflict copy
/// in the resume-point directory denies the reachability proof forever.
#[test]
fn the_retained_run_bound_beside_residue_opens_ephemeral_and_adds_no_run() {
    on_a_deep_stack(|| {
        // The regression is specifically the expensive multipart bootstrap
        // fallback, not merely selection of an ephemeral run.
        force_next_bootstrap_part_operation_limit(1);
        let mut fixture = Fixture::new(
            "resume-bound",
            None,
            vec![
                (
                    "pages/bound.md".into(),
                    b"- bound one\n- bound two\n".to_vec(),
                ),
                ("pages/bound-other.md".into(), b"- bound three\n".to_vec()),
            ],
        );
        let bootstrap_parts = fixture.verified.part_count() as u64;
        assert!(
            bootstrap_parts > 1,
            "the ephemeral recovery regression needs a multipart bootstrap"
        );
        let root = fixture.enrollment_root("resume-bound");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-bound");
        let session = SessionId::new();

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                append_local_batch(fixture, authority, runtime, 0xE500);
            },
        );
        // A provider removes the point — a receive-only revert, a `.stversions`
        // restore of an older tree. The next open has nothing to adopt and mints a
        // second retained run.
        remove_every_resume_point(&fixture.archive_root);
        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                assert!(runtime.resume_open_status().retained());
            },
        );
        let at_bound = retained_run_directories(&fixture.archive_root);
        assert_eq!(at_bound.len(), 2, "the population is now at the bound");

        // Now the directory becomes permanently unprovable.
        fs::create_dir_all(resume_point_directory(&fixture.archive_root)).unwrap();
        fs::write(
            resume_point_directory(&fixture.archive_root).join(".DS_Store"),
            b"desktop residue",
        )
        .unwrap();
        let points_before = resume_point_bytes(&fixture.archive_root);

        let observation = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, authority, runtime| {
                let opened = runtime.resume_open_status().clone();
                assert!(
                    matches!(opened.plan(), EngineScratchRetentionPlan::Ephemeral { .. }),
                    "an unprovable directory at the bound must not authorize growth: {opened:?}"
                );
                assert!(!opened.retained());
                assert!(!opened.adopted());
                assert!(
                    !opened.observation().adopted,
                    "an ephemeral bootstrap predecessor is not a publishable runtime resume"
                );
                assert_eq!(
                    opened.observation().replay_base_generation,
                    bootstrap_parts,
                    "the enrolled engine must replay only the post-bootstrap tail"
                );
                assert!(
                    opened.observation().replayed_generations
                        < opened.observation().live_history_generation,
                    "the bootstrap prefix must not be replayed a second time"
                );
                let timing = take_promoted_runtime_open_instrumentation(fixture.workspace);
                assert!(
                    timing.reconstructed_ephemeral_bootstrap,
                    "fail-before: the old ephemeral fallback performed a complete enrolled bootstrap replay"
                );
                assert!(
                    !timing.reconstructed_bootstrap_resume,
                    "the ephemeral path must not create a retained migrated bootstrap"
                );
                assert_eq!(
                    runtime
                        .engine()
                        .bootstrap_recovery_instrumentation()
                        .bootstrap_part_reads,
                    0,
                    "the enrolled engine consumes the same reconstructed scratch rather than rereading parts"
                );
                // An ephemeral engine has nothing a resume point could name, and
                // says so rather than attempting a publication.
                assert_eq!(
                    runtime.publish_quiescent_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::EngineNotPublishable
                    )
                );
                public_runtime_observation(runtime)
            },
        );

        assert_eq!(
            retained_run_directories(&fixture.archive_root),
            at_bound,
            "an ephemeral open must add no retained run and remove none"
        );
        assert_eq!(
            resume_point_bytes(&fixture.archive_root),
            points_before,
            "the ephemeral predecessor must not publish or rewrite a resume point"
        );
        assert!(
            resume_point_entries(&fixture.archive_root).contains(&".DS_Store".to_owned()),
            "residue is reported, never deleted"
        );
        // And the ephemeral engine remains an ordinary operational runtime
        // after the authenticated bootstrap-prefix reconstruction.
        assert_eq!(observation.projection_work.len(), 1);
        fixture.assert_graph_unchanged();
    });
}

/// If the one-process ephemeral predecessor cannot be restored, recovery must
/// discard that disposable scratch and replay the complete immutable history
/// into another ephemeral run. It must never turn that refusal into permission
/// to mint/publish a retained resume artifact beside unclassifiable residue.
#[test]
fn an_ephemeral_bootstrap_predecessor_refusal_falls_back_without_retained_mutation() {
    on_a_deep_stack(|| {
        force_next_bootstrap_part_operation_limit(1);
        let mut fixture = Fixture::new(
            "ephemeral-predecessor-refusal",
            None,
            vec![
                (
                    "pages/refusal.md".into(),
                    b"- refusal one\n- refusal two\n".to_vec(),
                ),
                (
                    "pages/refusal-other.md".into(),
                    b"- refusal three\n".to_vec(),
                ),
            ],
        );
        let root = fixture.enrollment_root("ephemeral-predecessor-refusal");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "ephemeral-predecessor-refusal");
        let session = SessionId::new();

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                append_local_batch(fixture, authority, runtime, 0xE501);
            },
        );
        remove_every_resume_point(&fixture.archive_root);
        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                assert!(runtime.resume_open_status().retained());
            },
        );
        let retained_before = retained_run_directories(&fixture.archive_root);
        assert_eq!(retained_before.len(), 2);
        fs::create_dir_all(resume_point_directory(&fixture.archive_root)).unwrap();
        fs::write(
            resume_point_directory(&fixture.archive_root).join(".DS_Store"),
            b"desktop residue",
        )
        .unwrap();
        let points_before = resume_point_bytes(&fixture.archive_root);

        fail_next_ephemeral_bootstrap_predecessor_restore_for_test();
        let observation = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, authority, runtime| {
                let opened = runtime.resume_open_status().clone();
                assert!(matches!(
                    opened.plan(),
                    EngineScratchRetentionPlan::Ephemeral { .. }
                ));
                assert!(!opened.retained());
                assert!(!opened.adopted());
                assert!(!opened.observation().adopted);
                assert_eq!(
                    opened.observation().replay_base_generation,
                    0,
                    "an ephemeral predecessor refusal must replay from the durable origin"
                );
                assert_eq!(
                    opened.observation().replayed_generations,
                    opened.observation().live_history_generation,
                    "the fallback must replay every bootstrap and operational generation"
                );
                let timing = take_promoted_runtime_open_instrumentation(fixture.workspace);
                assert!(
                    timing.reconstructed_ephemeral_bootstrap,
                    "the injected refusal must happen after direct ephemeral reconstruction"
                );
                assert_eq!(
                    runtime
                        .engine()
                        .bootstrap_recovery_instrumentation()
                        .bootstrap_part_reads,
                    fixture.verified.part_count() as usize,
                    "the fallback must reread each immutable bootstrap part"
                );
                assert_eq!(
                    runtime.publish_quiescent_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::EngineNotPublishable
                    )
                );
                public_runtime_observation(runtime)
            },
        );

        assert_eq!(
            retained_run_directories(&fixture.archive_root),
            retained_before
        );
        assert_eq!(resume_point_bytes(&fixture.archive_root), points_before);
        assert!(resume_point_entries(&fixture.archive_root).contains(&".DS_Store".to_owned()));
        assert_eq!(observation.projection_work.len(), 1);
        fixture.assert_graph_unchanged();
    });
}

/// A publication that fails before, at, or after its commit point never blocks
/// an otherwise valid `Unsafe -> Safe` handoff, and never reclaims a byte.
#[test]
fn an_injected_publication_failure_never_blocks_the_safe_handoff_and_reclaims_nothing() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-publication-faults",
            None,
            vec![("pages/faults.md".into(), b"- faults\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-publication-faults");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-publication-faults");
        let session = SessionId::new();

        let verification_digest = with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                let runs_before = retained_run_directories(&fixture.archive_root);
                assert_eq!(runs_before.len(), 1);
                assert!(
                    matches!(
                        runtime.resume_publication_status(),
                        Some(ResumePublicationStatus::Published { .. })
                    ),
                    "the constructor must already have made its report-only attempt"
                );
                remove_every_resume_point(&fixture.archive_root);

                // (a) Before the mint: unclassifiable residue makes the endpoint
                //     binding fail closed. Nothing is published; nothing is removed.
                fs::create_dir_all(resume_point_directory(&fixture.archive_root)).unwrap();
                let residue = resume_point_directory(&fixture.archive_root).join(".DS_Store");
                fs::write(&residue, b"desktop residue").unwrap();
                let refused = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                assert!(
                    matches!(
                        refused,
                        ResumePublicationStatus::NotPublished(ResumePublicationRefusal::Store(_))
                    ),
                    "residue must fail the mint closed rather than publish beside it: {refused:?}"
                );
                assert_eq!(
                    resume_point_entries(&fixture.archive_root),
                    vec![".DS_Store".to_owned()]
                );
                fs::remove_file(&residue).unwrap();

                // (b) At the pre-prune cut, before the commit point.
                fail_next_resume_publication_at(ResumePublishBoundary::AfterPrePrune);
                let refused = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                assert!(
                    matches!(
                        refused,
                        ResumePublicationStatus::NotPublished(ResumePublicationRefusal::Store(_))
                    ),
                    "{refused:?}"
                );
                assert!(
                    resume_point_entries(&fixture.archive_root).is_empty(),
                    "an interrupted publication before its commit point leaves nothing"
                );
                assert_eq!(retained_run_directories(&fixture.archive_root), runs_before);

                // (c) After the commit point: the point is durable, but the call
                //     reports a failure, so the reclamation it would otherwise have
                //     authorized never runs.
                fail_next_resume_publication_at(ResumePublishBoundary::AfterCommit);
                let refused = runtime.publish_quiescent_resume_point(authority, &fixture.graph);
                assert!(
                    matches!(
                        refused,
                        ResumePublicationStatus::NotPublished(ResumePublicationRefusal::Store(_))
                    ),
                    "{refused:?}"
                );
                assert_eq!(
                    resume_point_entries(&fixture.archive_root).len(),
                    1,
                    "the commit point is durable even though the call reported a failure"
                );
                assert_eq!(
                    retained_run_directories(&fixture.archive_root),
                    runs_before,
                    "no maintenance pass may run without a successful publication witness"
                );

                // The handoff is untouched by every one of those.
                let permit = runtime
                    .quiesce_and_mark_safe_without_watcher_dependency_for_test(
                        authority,
                        &fixture.graph,
                    )
                    .expect("a publication failure must never block a valid Safe handoff");
                assert_eq!(permit.session_id(), session);
                authority.verification_digest()
            },
        );
        assert_eq!(
            committed_handoff(&root, &binding, verification_digest),
            LocalActiveHandoff::Safe
        );

        // And the durable cut the interrupted publication left behind reopens.
        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| {
                runtime.engine().accepted_frontier_root().unwrap();
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// The whole lifecycle over nested, non-ASCII graph, archive, and
/// application-runtime paths.
///
/// No resume-point payload member is path-derived, so this proves the binding,
/// the adoption and the reclamation are all independent of how the user's
/// directories are spelled.
#[test]
fn the_resume_lifecycle_works_over_nested_and_utf8_paths() {
    on_a_deep_stack(|| {
        let label = "résumé-日本語-a b-🗂️";
        let mut fixture = Fixture::new(
            label,
            None,
            vec![(
                "pages/journaux/日本語/a b/c-d/emoji-🗂️/nested résumé.md".into(),
                "- nested résumé 日本語\n".as_bytes().to_vec(),
            )],
        );
        let root = fixture.enrollment_root(label);
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, label);
        let session = SessionId::new();

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                let (sequence, maintenance) =
                    publish_expecting_success(fixture, authority, runtime);
                assert_eq!(sequence, 1);
                assert_eq!(
                    maintenance.outcome,
                    RetainedRunMaintenanceOutcome::Reclaimed
                );
                append_local_batch_at(fixture, authority, runtime, 0xE600, "pages/journaux/日本語");
            },
        );
        let adopted = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                let opened = runtime.resume_open_status().clone();
                assert!(opened.adopted(), "{opened:?}");
                public_runtime_observation(runtime)
            },
        );
        remove_every_resume_point(&fixture.archive_root);
        let replayed = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                assert!(runtime.resume_open_status().adopted());
                public_runtime_observation(runtime)
            },
        );
        assert_publicly_indistinguishable(
            &adopted,
            &replayed,
            "nested/UTF-8 paths must not change what an adopted restart observes",
        );
        fixture.assert_graph_unchanged();
    });
}

/// The resume lifecycle is absent from the keystroke path.
///
/// An adopted runtime admitting ordinary mutation windows performs zero archive
/// identity reads, zero workspace-lease revalidations, and zero SQLite
/// statements, and neither the resume observation nor the durable resume-point
/// and retained-run populations move.
#[test]
fn ordinary_admissions_do_no_resume_lifecycle_work() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-admission-cost",
            None,
            vec![("pages/cost.md".into(), b"- cost\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-admission-cost");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-admission-cost");
        let session = SessionId::new();
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
            },
        );

        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, authority, runtime| {
                assert!(runtime.resume_open_status().adopted());
                let opened_before = runtime.resume_open_status().clone();
                let publication_before = runtime
                    .resume_publication_status()
                    .expect("every writable open attempts publication before return")
                    .clone();
                let points_before = resume_point_bytes(&fixture.archive_root);
                let runs_before = retained_run_directories(&fixture.archive_root);
                let watcher_before = runtime.watcher_status();

                for count in [1_usize, 1_000] {
                    let before = PromotedRuntimeInstrumentation::capture();
                    for _ in 0..count {
                        let window = runtime
                            .admit_promoted_mutation(authority, &fixture.graph)
                            .unwrap();
                        window
                            .admission()
                            .authorize(&fixture.graph, window.engine)
                            .unwrap();
                    }
                    let measured = before.since();
                    assert_eq!(measured.archive_identity_reads, 0, "{count} admissions");
                    assert_eq!(
                        measured.workspace_lease_identity_revalidations, 0,
                        "{count} admissions"
                    );
                    assert_eq!(measured.sqlite_frontier_reads, 0, "{count} admissions");
                    assert_eq!(measured.workspace_lease_acquisitions, 0);
                    assert_eq!(measured.enrollment.namespace_scans, 0);
                }

                assert_eq!(
                    runtime.resume_open_status(),
                    &opened_before,
                    "an admission never re-derives the resume observation"
                );
                assert_eq!(
                    runtime.resume_publication_status(),
                    Some(&publication_before),
                    "an admission never republishes"
                );
                assert_eq!(
                    runtime.watcher_status(),
                    watcher_before,
                    "an admission never touches watcher queue state"
                );
                assert_eq!(resume_point_bytes(&fixture.archive_root), points_before);
                assert_eq!(retained_run_directories(&fixture.archive_root), runs_before);
                assert_eq!(
                    runtime.publish_post_open_resume_point(authority, &fixture.graph),
                    ResumePublicationStatus::NotPublished(
                        ResumePublicationRefusal::EngineNotPublishable
                    ),
                    "the first admitted mutation permanently seals the post-open cut"
                );
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// An exact-current adopted open may serve projected reads while the engine
/// catalog stays cold, but it must authenticate the retained catalog bytes
/// before admitting the first mutation. If that authentication refuses, the
/// damaged accelerator is replaced only after an ordinary immutable-history
/// replay, so it cannot trap every later restart in the same write refusal.
#[test]
fn first_mutation_refuses_when_deferred_catalog_bytes_are_missing() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-deferred-catalog-corruption",
            None,
            vec![("pages/corrupt.md".into(), b"- corrupt\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-deferred-catalog-corruption");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-deferred-catalog-corruption");
        let session = SessionId::new();
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            session,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
            },
        );

        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, authority, runtime| {
                assert!(runtime.resume_open_status().adopted());
                assert_eq!(
                    runtime.engine().instrumentation().catalog_hot_state_loads,
                    0
                );
                let runs = retained_run_directories(&fixture.archive_root);
                assert_eq!(runs.len(), 1);
                fs::OpenOptions::new()
                    .write(true)
                    .open(
                        scratch_namespace(&fixture.archive_root)
                            .join(&runs[0])
                            .join("pages.index"),
                    )
                    .unwrap()
                    .set_len(0)
                    .unwrap();

                let Err(error) = runtime.admit_promoted_mutation(authority, &fixture.graph) else {
                    panic!("missing deferred catalog bytes must refuse the first mutation");
                };
                assert!(matches!(
                    error,
                    RuntimePromotionError::Engine(EngineError::Archive(_))
                ));
                assert!(
                    runtime.engine().instrumentation().catalog_hot_state_loads > 1,
                    "the refused deferred read must be followed by ordinary replay"
                );
                assert!(
                    !retained_run_directories(&fixture.archive_root).contains(&runs[0]),
                    "the refused adopted run must be durably retired after full replay"
                );
            },
        );

        let recovered = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |fixture, authority, runtime| {
                append_local_batch(fixture, authority, runtime, 0xE7A0);
                public_runtime_observation(runtime)
            },
        );
        remove_every_resume_point(&fixture.archive_root);
        let replayed = with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            session,
            |_, _, runtime| {
                assert!(runtime.resume_open_status().adopted());
                public_runtime_observation(runtime)
            },
        );
        assert_publicly_indistinguishable(
            &recovered,
            &replayed,
            "deferred-catalog refusal recovery must preserve clean replay equivalence",
        );
        fixture.assert_graph_unchanged();
    });
}

/// A crash takeover adopts the crashed session's published point.
///
/// This is the common crash-recovery shape, and it is the reason the enrollment
/// admission is derived from the *committed record* rather than from the
/// recovery classification: at the instant the candidate is read the takeover's
/// compare-and-swap has not run, so the live durable record is still exactly the
/// record the crashed session published under.
#[test]
fn a_crash_takeover_adopts_the_crashed_sessions_published_point() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-takeover-adopt",
            None,
            vec![("pages/takeover.md".into(), b"- takeover\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-takeover-adopt");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-takeover-adopt");
        let crashed = SessionId::new();

        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            crashed,
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
                // Durable work after the point, then the process dies without a
                // drain: the classic `Unsafe` crash.
                append_local_batch(fixture, authority, runtime, 0xE800);
            },
        );
        let runs = retained_run_directories(&fixture.archive_root);
        assert_eq!(runs.len(), 1);

        with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |fixture, _, runtime| {
                assert_eq!(
                    runtime.recovery(),
                    RuntimeRecoveryState::TookOverCrashedUnsafe {
                        previous_session: crashed
                    }
                );
                let opened = runtime.resume_open_status().clone();
                assert!(
                    opened.adopted(),
                    "a takeover must be able to adopt its predecessor's point: {opened:?}"
                );
                assert!(!opened.observation().refused);
                assert!(opened.observation().replay_base_generation > 0);
                assert_eq!(
                    retained_run_directories(&fixture.archive_root),
                    runs,
                    "adoption reuses the crashed session's run"
                );
                // Automatic external import stays blocked: adopting a
                // predecessor's *scratch state* says nothing about its drain.
                assert_eq!(
                    runtime.automatic_external_import(),
                    ExternalImportAdmission::Blocked(
                        "this runtime took over a crashed session's Unsafe handoff, whose drain \
                         was never proved"
                    )
                );
            },
        );
        fixture.assert_graph_unchanged();
    });
}

/// Candidate selection happens before each takeover CAS, so only the exact
/// currently crashed predecessor is admissible. Restoring the older recognized
/// point after each automatic successor publication proves that later
/// takeovers refuse it at every distance; the unavoidable post-open attempt
/// then supersedes it only after that refusal and keeps the point set bounded.
#[test]
fn a_point_older_than_one_or_more_takeovers_refuses_before_automatic_supersession() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-old-takeover-point",
            None,
            vec![("pages/old.md".into(), b"- old\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-old-takeover-point");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-old-takeover-point");
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                publish_expecting_success(fixture, authority, runtime);
            },
        );
        let original = resume_point_bytes(&fixture.archive_root);

        with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| {
                assert!(
                    runtime.resume_open_status().adopted(),
                    "the exact crashed predecessor remains admissible before the first CAS"
                );
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::Published { .. })
                ));
            },
        );
        assert_ne!(
            resume_point_bytes(&fixture.archive_root),
            original,
            "the mandatory attempt publishes the new Unsafe session after adoption"
        );

        for distance in 1..=2 {
            restore_resume_point_bytes(&fixture.archive_root, &original);
            with_taken_over_runtime(
                &fixture,
                &root,
                &binding,
                &paths,
                SessionId::new(),
                |_, _, runtime| {
                    assert!(runtime.resume_open_status().adopted());
                    assert!(runtime.resume_open_status().unavailable().is_some());
                    assert!(
                        matches!(
                            runtime.resume_publication_status(),
                            Some(ResumePublicationStatus::Published { .. })
                        ),
                        "the refused full replay must still make its automatic report-only attempt"
                    );
                },
            );
            let successor = resume_point_bytes(&fixture.archive_root);
            assert_eq!(
                successor.len(),
                1,
                "automatic publication keeps the point set bounded at distance {distance}"
            );
            assert_ne!(
                successor, original,
                "only the later automatic publication may supersede the refused evidence"
            );
        }
        fixture.assert_graph_unchanged();
    });
}

/// A Safe-bound point is exact for one clean restart only. After that restart
/// durably opens and automatically publishes its new Unsafe session, restoring
/// the older Safe generation proves it cannot be reinterpreted as crash
/// evidence. The later automatic attempt may supersede it only after refusal.
#[test]
fn a_safe_point_older_than_the_clean_restart_refuses_before_automatic_supersession() {
    on_a_deep_stack(|| {
        let mut fixture = Fixture::new(
            "resume-old-safe-point",
            None,
            vec![("pages/old-safe.md".into(), b"- old safe\n".to_vec())],
        );
        let root = fixture.enrollment_root("resume-old-safe-point");
        let binding = fixture.enrollment_binding();
        let paths = PromotedPaths::new(&fixture, "resume-old-safe-point");
        with_promoted_runtime(
            &mut fixture,
            &root,
            &paths,
            SessionId::new(),
            |fixture, authority, runtime| {
                runtime
                    .quiesce_and_mark_safe(authority, &fixture.graph)
                    .expect("Safe publication succeeds");
            },
        );
        let original = resume_point_bytes(&fixture.archive_root);
        assert_eq!(original.len(), 1);

        with_reopened_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| assert!(runtime.resume_open_status().adopted()),
        );
        restore_resume_point_bytes(&fixture.archive_root, &original);
        with_taken_over_runtime(
            &fixture,
            &root,
            &binding,
            &paths,
            SessionId::new(),
            |_, _, runtime| {
                assert!(runtime.resume_open_status().adopted());
                assert!(runtime.resume_open_status().unavailable().is_some());
                assert!(matches!(
                    runtime.resume_publication_status(),
                    Some(ResumePublicationStatus::Published { .. })
                ));
            },
        );
        let successor = resume_point_bytes(&fixture.archive_root);
        assert_eq!(successor.len(), 1);
        assert_ne!(
            successor, original,
            "the mandatory attempt publishes only after refusing the stale Safe point"
        );
        fixture.assert_graph_unchanged();
    });
}

/// Terminal SQLite construction differentials.
///
/// Every test below builds the same authority twice — once from the retained
/// terminal accepted state and once by forced clean archive replay — and
/// compares what a reader can actually observe. Physical bytes, page layout,
/// and the construction-only `materialization_batches`/`materialization_stamp`
/// provenance are deliberately not required to match; everything a query,
/// frontier, digest, or complete row observation can see is.
mod terminal_construction {
    use super::*;
    use crate::oplog::import::TerminalBootstrapConstructionMaterial;
    use crate::oplog::sqlite_materialization::{
        MaterializedBlockRow, MaterializedPageRow, MaterializedPropertyRow,
        MaterializedReferrerRow, MaterializedTaskRow,
    };
    use crate::oplog::{
        ContentDigest, MaterializedEntityId, ProjectionRecovery, RebuildInstrumentation,
    };

    const QUERY_LIMIT: usize = 4_096;

    /// Construction-only provenance. Both databases authenticate independently
    /// and rebuild from archive history; neither is required to record the same
    /// per-event derivation evidence for rows it never replayed.
    const CONSTRUCTION_PROVENANCE_TABLES: [&str; 2] =
        ["materialization_batches", "materialization_stamp"];

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct BlockObservation {
        row: Option<MaterializedBlockRow>,
        properties: Vec<MaterializedPropertyRow>,
        referrers: Vec<MaterializedReferrerRow>,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct PageObservation {
        row: Option<MaterializedPageRow>,
        by_path: Vec<MaterializedPageRow>,
        by_name: Vec<MaterializedPageRow>,
        by_name_key: Vec<MaterializedPageRow>,
        by_name_key_and_kind: Vec<MaterializedPageRow>,
        blocks: Vec<MaterializedBlockRow>,
        properties: Vec<MaterializedPropertyRow>,
        referrers: Vec<MaterializedReferrerRow>,
        blocks_detail: Vec<BlockObservation>,
        search_hits: Vec<(String, Uuid, Uuid)>,
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(super) struct ProjectionObservation {
        frontier_root: AcceptedFrontierRoot,
        pub(super) accepted_batch_count: usize,
        semantic_projection_digest: ContentDigest,
        semantic_effects: Vec<Vec<u8>>,
        reference_catalog_root: Option<crate::oplog::ReferenceCatalogRootV2>,
        table_rows: Vec<(&'static str, ContentDigest)>,
        pages: Vec<MaterializedPageRow>,
        page_kind_pages: Vec<MaterializedPageRow>,
        journal_kind_pages: Vec<MaterializedPageRow>,
        tasks: Vec<MaterializedTaskRow>,
        page_details: Vec<PageObservation>,
    }

    fn observe(database: &SqliteFrontier) -> ProjectionObservation {
        let frontier_root = database.frontier_root().unwrap();
        let accepted_batch_count = database.applied_batch_count().unwrap();
        let read = database.materialized_read().unwrap();
        let pages = read.pages(None, QUERY_LIMIT).unwrap();
        let mut page_details = Vec::new();
        for page in &pages {
            let blocks = read.blocks_on_page(page.page_id, QUERY_LIMIT).unwrap();
            let blocks_detail = blocks
                .iter()
                .map(|block| BlockObservation {
                    row: read.block(block.block_id).unwrap(),
                    properties: read
                        .properties(MaterializedEntityId::Block(block.block_id), QUERY_LIMIT)
                        .unwrap(),
                    referrers: read
                        .referrers_to(MaterializedEntityId::Block(block.block_id), QUERY_LIMIT)
                        .unwrap(),
                })
                .collect();
            // The page's own logical name is a term every page contributes to
            // the search index, so this exercises FTS ownership per page.
            let mut search_hits = read
                .search(&fts_query_for(&page.name), QUERY_LIMIT)
                .unwrap()
                .into_iter()
                .map(|hit| {
                    (
                        hit.text,
                        match hit.entity {
                            MaterializedEntityId::Page(id) => id.as_uuid(),
                            MaterializedEntityId::Block(id) => id.as_uuid(),
                        },
                        hit.page_id.as_uuid(),
                    )
                })
                .collect::<Vec<_>>();
            search_hits.sort();
            page_details.push(PageObservation {
                row: read.page(page.page_id).unwrap(),
                by_path: read.pages_by_path(&page.path, QUERY_LIMIT).unwrap(),
                by_name: read.pages_by_name(&page.name, QUERY_LIMIT).unwrap(),
                by_name_key: read.pages_by_name_key(&page.name_key, QUERY_LIMIT).unwrap(),
                by_name_key_and_kind: read
                    .pages_by_name_key_and_kind(&page.name_key, page.kind, QUERY_LIMIT)
                    .unwrap(),
                properties: read
                    .properties(MaterializedEntityId::Page(page.page_id), QUERY_LIMIT)
                    .unwrap(),
                referrers: read
                    .referrers_to(MaterializedEntityId::Page(page.page_id), QUERY_LIMIT)
                    .unwrap(),
                blocks,
                blocks_detail,
                search_hits,
            });
        }
        ProjectionObservation {
            reference_catalog_root: (accepted_batch_count != 0)
                .then(|| database.authenticated_reference_catalog_root().unwrap()),
            frontier_root,
            accepted_batch_count,
            semantic_projection_digest: database.semantic_projection_digest().unwrap(),
            semantic_effects: database
                .applied_semantic_effects_for_test()
                .unwrap()
                .iter()
                .map(|effect| effect.encode().unwrap())
                .collect(),
            table_rows: database
                .materialized_row_digests_by_table_for_test()
                .unwrap()
                .into_iter()
                .filter(|(table, _)| !CONSTRUCTION_PROVENANCE_TABLES.contains(table))
                .collect(),
            pages,
            page_kind_pages: read
                .pages(Some(ManagedTextKind::Page), QUERY_LIMIT)
                .unwrap(),
            journal_kind_pages: read
                .pages(Some(ManagedTextKind::Journal), QUERY_LIMIT)
                .unwrap(),
            tasks: read.tasks(None, QUERY_LIMIT).unwrap(),
            page_details,
        }
    }

    /// FTS5 treats most punctuation as a separator; quote the whole name so a
    /// nonstandard or Unicode page title stays one legal query term.
    fn fts_query_for(name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    pub(super) struct BuiltProjection {
        observation: ProjectionObservation,
        recovery: ProjectionRecovery,
        rebuild: RebuildInstrumentation,
        bootstrap: crate::oplog::sqlite::BootstrapSqliteRebuildInstrumentation,
        proof_frontier: AcceptedFrontierRoot,
        proof_semantic_digest: ContentDigest,
        proof_accepted_count: u64,
    }

    impl BuiltProjection {
        pub(super) const fn observation(&self) -> &ProjectionObservation {
            &self.observation
        }

        pub(super) const fn recovery(&self) -> &ProjectionRecovery {
            &self.recovery
        }

        pub(super) const fn bootstrap(
            &self,
        ) -> &crate::oplog::sqlite::BootstrapSqliteRebuildInstrumentation {
            &self.bootstrap
        }
    }

    pub(super) fn build_projection(
        fixture: &Fixture,
        label: &str,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> BuiltProjection {
        let path = fixture.root.path().join(format!("{label}.sqlite"));
        build_projection_at(fixture, &path, label, terminal)
    }

    pub(super) fn build_projection_at(
        fixture: &Fixture,
        path: &Path,
        label: &str,
        terminal: Option<&TerminalBootstrapConstructionMaterial>,
    ) -> BuiltProjection {
        let runtime =
            ApplicationRuntimeRoot::open_for_test(&fixture.root.path().join(format!("rt-{label}")))
                .unwrap();
        let (opened, proof) = match terminal {
            Some(material) => SqliteFrontier::open_or_rebuild_inactive_bootstrap_terminally(
                path,
                &runtime,
                &fixture.authority,
                material,
            ),
            None => SqliteFrontier::open_or_rebuild_inactive_bootstrap(
                path,
                &runtime,
                &fixture.authority,
            ),
        }
        .unwrap_or_else(|error| panic!("{label} bootstrap projection: {error}"));
        // Every current terminal proof must still hold on the built database.
        opened.database.diagnose_full_integrity().unwrap();
        opened
            .database
            .freshly_verify_inactive_bootstrap(&fixture.authority, &proof)
            .unwrap();
        let observation = observe(&opened.database);
        BuiltProjection {
            observation,
            recovery: opened.recovery.clone(),
            rebuild: opened.rebuild,
            bootstrap: proof.bootstrap_rebuild(),
            proof_frontier: proof.frontier_root().clone(),
            proof_semantic_digest: proof.semantic_projection_digest(),
            proof_accepted_count: proof.accepted_batch_count(),
        }
    }

    /// Build the same authority both ways and prove a reader cannot tell them
    /// apart.
    fn assert_terminal_equals_replay(fixture: &mut Fixture) {
        let material = fixture
            .prepared
            .take_terminal_construction_material()
            .expect("uninterrupted preparation retains terminal construction material");
        assert!(
            fixture
                .prepared
                .take_terminal_construction_material()
                .is_none(),
            "the one-shot construction capability must not be reusable"
        );
        // Release the fixture's own bootstrap session so each build below takes
        // and releases the workspace lease itself.
        fixture.release_bootstrap_projection();

        let terminal = build_projection(fixture, "terminal", Some(&material));
        let replay = build_projection(fixture, "replay", None);

        assert_eq!(terminal.bootstrap.terminal_constructions, 1);
        assert_eq!(terminal.bootstrap.terminal_archive_replays, 0);
        assert_eq!(terminal.bootstrap.terminal_construction_refusals, 0);
        assert_eq!(terminal.bootstrap.bootstrap_part_reads, 0);
        assert_eq!(terminal.bootstrap.bootstrap_object_reads, 0);
        assert_eq!(terminal.bootstrap.intermediate_page_materializations, 0);
        assert_eq!(terminal.bootstrap.terminal_materializations, 1);
        assert_eq!(
            terminal.bootstrap.terminal_pages_materialized,
            terminal.observation.pages.len()
        );
        assert!(
            terminal.bootstrap.peak_terminal_bulk_pages
                <= crate::oplog::hot_engine::BOOTSTRAP_MATERIALIZATION_CHUNK_PAGES
        );
        terminal
            .bootstrap
            .assert_catalog_authority_is_window_bounded();

        // Durable replay authenticates every part but materializes only the
        // exact terminal profile, just like the same-process construction.
        assert_eq!(replay.bootstrap.terminal_constructions, 0);
        assert_eq!(replay.bootstrap.terminal_archive_replays, 1);
        assert_eq!(
            replay.bootstrap.bootstrap_part_reads,
            replay.observation.accepted_batch_count
        );
        assert_eq!(replay.bootstrap.intermediate_page_materializations, 0);
        assert_eq!(replay.bootstrap.terminal_materializations, 1);
        assert_eq!(
            replay.bootstrap.terminal_pages_materialized,
            replay.observation.pages.len()
        );
        assert_eq!(replay.bootstrap.terminal_frontier_bulk_seeds, 1);
        assert_eq!(
            replay.bootstrap.max_live_bootstrap_parts,
            usize::from(replay.bootstrap.bootstrap_part_reads != 0)
        );
        replay
            .bootstrap
            .assert_catalog_authority_is_window_bounded();

        // One candidate transaction and one durability barrier on both paths.
        for built in [&terminal, &replay] {
            assert_eq!(
                built.rebuild.accepted_frontier_session_hits,
                built.bootstrap.terminal_accepted_frontier_session_hits
            );
            assert_eq!(
                built.rebuild.accepted_frontier_session_misses,
                built.bootstrap.terminal_accepted_frontier_session_misses
            );
            assert_eq!(
                built.rebuild.accepted_frontier_session_evictions,
                built.bootstrap.terminal_accepted_frontier_session_evictions
            );
            assert_eq!(
                built.rebuild.accepted_frontier_session_oversize,
                built.bootstrap.terminal_accepted_frontier_session_oversize
            );
            assert_eq!(
                built.rebuild.accepted_frontier_session_peak_resident_bytes,
                built
                    .bootstrap
                    .terminal_accepted_frontier_session_peak_resident_bytes
            );
            assert_eq!(
                built.rebuild.external_exact_session_hits,
                built.bootstrap.terminal_external_exact_session_hits
            );
            assert_eq!(
                built.rebuild.external_exact_session_misses,
                built.bootstrap.terminal_external_exact_session_misses
            );
            assert_eq!(
                built.rebuild.external_exact_session_evictions,
                built.bootstrap.terminal_external_exact_session_evictions
            );
            assert_eq!(
                built.rebuild.external_exact_session_oversize,
                built.bootstrap.terminal_external_exact_session_oversize
            );
            assert_eq!(
                built.rebuild.external_exact_session_peak_resident_bytes,
                built
                    .bootstrap
                    .terminal_external_exact_session_peak_resident_bytes
            );
            assert!(
                built.rebuild.exact_document_loads >= built.observation.pages.len(),
                "terminal materialization did not report its exact document loads"
            );
            assert_eq!(built.rebuild.physical_candidate_transactions, 1);
            assert_eq!(built.rebuild.physical_candidate_durability_barriers, 1);
            assert_eq!(built.rebuild.physical_ordinary_transactions, 0);
            assert_eq!(built.rebuild.final_semantic_equivalence_proofs, 1);
            assert_eq!(built.rebuild.final_row_digest_equivalence_proofs, 1);
        }

        assert_eq!(terminal.proof_frontier, replay.proof_frontier);
        assert_eq!(terminal.proof_accepted_count, replay.proof_accepted_count);
        assert_eq!(
            terminal.proof_semantic_digest, replay.proof_semantic_digest,
            "terminal and replayed accepted-prefix semantics diverged"
        );
        assert_eq!(
            terminal.observation, replay.observation,
            "terminal construction and clean archive replay are observably different"
        );
    }

    #[test]
    fn terminal_construction_matches_clean_replay_for_zero_one_and_multipart_shapes() {
        let mut cases = local_active_shape_fixtures("terminal-shapes");
        assert_local_active_fixture_shapes(&cases);
        for fixture in &mut cases {
            assert_terminal_equals_replay(fixture);
            fixture.assert_graph_unchanged();
        }
    }

    #[test]
    fn terminal_construction_matches_clean_replay_for_a_huge_page_split() {
        let mut blocks = String::new();
        for ordinal in 0..3_000 {
            blocks.push_str(&format!("- split {ordinal:04} [[Target {ordinal:04}]]\n"));
        }
        force_next_bootstrap_part_operation_limit(2_048);
        let mut fixture = Fixture::new(
            "terminal-huge-page-split",
            None,
            vec![("pages/huge.md".into(), blocks.into_bytes())],
        );
        assert!(
            fixture.verified.part_count() >= 2,
            "the huge page must genuinely split across parts"
        );
        assert_terminal_equals_replay(&mut fixture);
        fixture.assert_graph_unchanged();
    }

    #[test]
    fn terminal_construction_matches_clean_replay_for_rich_semantic_layout() {
        let mut fixture = Fixture::new(
            "terminal-rich-semantics",
            Some(
                br#"{:pages-directory "notes"
                    :journals-directory "diary"
                    :file/name-format :triple-lowbar
                    :journal/file-name-format "dd-MM-yyyy"
                    :journal/page-title-format "yyyy-MM-dd"}"#,
            ),
            vec![
                (
                    "notes/Hub.md".into(),
                    concat!(
                        "title:: Hub logical\n",
                        "alias:: Hub Alias, Second Alias\n",
                        "\n",
                        "- TODO [#A] plan the [[Spoke]] work\n",
                        "  SCHEDULED: <2026-08-02 Sun>\n",
                        "  id:: 00000000-0000-0000-0000-0000000cafe1\n",
                        "  kind:: planning\n",
                        "- DONE tagged #project work\n",
                        "- embed ((00000000-0000-0000-0000-0000000cafe1))\n",
                    )
                    .as_bytes()
                    .to_vec(),
                ),
                (
                    "notes/Spoke.org".into(),
                    concat!(
                        "#+title: Spoke logical\n",
                        "* TODO org task referencing [[Hub Alias]]\n",
                        "  :PROPERTIES:\n",
                        "  :owner: spoke\n",
                        "  :END:\n",
                        "* another #org-tag line\n",
                    )
                    .as_bytes()
                    .to_vec(),
                ),
                (
                    "notes/Crlf___Bom.markdown".into(),
                    "\u{feff}title:: Crlf/Bom\r\n\r\n- caf\u{e9} \u{4e2d}\u{6587} [[Hub]]\r\n"
                        .as_bytes()
                        .to_vec(),
                ),
                ("notes/Empty.md".into(), Vec::new()),
                (
                    "diary/nested/02-08-2026.org".into(),
                    b"* journal entry [[Spoke]]\n".to_vec(),
                ),
            ],
        );
        assert!(fixture.verified.part_count() > 0);
        assert_terminal_equals_replay(&mut fixture);
        fixture.assert_graph_unchanged();
    }

    #[test]
    fn terminal_construction_matches_clean_replay_for_duplicate_uuid_collapse() {
        let duplicate = "00000000-0000-0000-0000-0000000dup01";
        let mut fixture = Fixture::new(
            "terminal-duplicate-uuid",
            None,
            vec![
                (
                    "pages/first.md".into(),
                    format!("- first claim\n  id:: {duplicate}\n").into_bytes(),
                ),
                (
                    "pages/second.md".into(),
                    format!("- second claim\n  id:: {duplicate}\n").into_bytes(),
                ),
                (
                    "pages/third.org".into(),
                    format!("* third claim\n  :PROPERTIES:\n  :id: {duplicate}\n  :END:\n")
                        .into_bytes(),
                ),
            ],
        );
        assert_terminal_equals_replay(&mut fixture);
        fixture.assert_graph_unchanged();
    }
}

/// Terminal SQLite construction interruption and fallback.
mod terminal_construction_interruption {
    use super::terminal_construction::*;
    use super::*;
    use crate::oplog::sqlite::{fail_next_terminal_construction_at, TerminalConstructionCut};
    use crate::oplog::ProjectionRecovery;

    fn candidate_residue(root: &Path) -> Vec<String> {
        fs::read_dir(root)
            .unwrap()
            .map(Result::unwrap)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("candidate"))
            .collect()
    }

    /// Every private-candidate interruption discards the candidate and lets the
    /// unchanged archive replay path finish the same activation.
    #[test]
    fn interrupted_terminal_candidate_falls_back_to_archive_replay() {
        for cut in [
            TerminalConstructionCut::BeforeCandidateCommit,
            TerminalConstructionCut::AfterCandidateCommitBeforePublication,
        ] {
            let mut fixture = Fixture::new(
                &format!("terminal-cut-{cut:?}"),
                None,
                vec![
                    (
                        "pages/interrupted.md".into(),
                        b"- interrupted [[Other]]\n".to_vec(),
                    ),
                    ("pages/other.md".into(), b"- other\n".to_vec()),
                ],
            );
            let material = fixture
                .prepared
                .take_terminal_construction_material()
                .unwrap();
            fixture.release_bootstrap_projection();

            fail_next_terminal_construction_at(cut);
            let interrupted = build_projection(&fixture, "interrupted", Some(&material));
            assert_eq!(
                interrupted.bootstrap().terminal_construction_refusals,
                1,
                "{cut:?} must be recorded as a discarded private candidate"
            );
            assert_eq!(interrupted.bootstrap().terminal_constructions, 0);
            assert_eq!(interrupted.bootstrap().terminal_archive_replays, 1);
            assert_eq!(
                interrupted.bootstrap().bootstrap_part_reads,
                interrupted.observation().accepted_batch_count,
                "{cut:?} must fall back to the physical archive replay"
            );

            let replay = build_projection(&fixture, "replay", None);
            assert_eq!(
                interrupted.observation(),
                replay.observation(),
                "{cut:?} fallback is observably different from a clean archive replay"
            );
            assert!(
                candidate_residue(fixture.root.path()).is_empty(),
                "{cut:?} left private candidate residue"
            );
            fixture.assert_graph_unchanged();
        }
    }

    /// An interruption after the atomic file publication but before the
    /// checkpoint proof refuses outright: the unproved database authorizes
    /// nothing, and a restart rebuilds it while preserving the evidence.
    #[test]
    fn interruption_after_publication_refuses_and_a_restart_rebuilds() {
        let mut fixture = Fixture::new(
            "terminal-cut-after-publication",
            None,
            vec![(
                "pages/published.md".into(),
                b"- published [[Other]]\n".to_vec(),
            )],
        );
        let material = fixture
            .prepared
            .take_terminal_construction_material()
            .unwrap();
        fixture.release_bootstrap_projection();

        let runtime =
            ApplicationRuntimeRoot::open_for_test(&fixture.root.path().join("rt-cut")).unwrap();
        let path = fixture.root.path().join("cut.sqlite");
        fail_next_terminal_construction_at(
            TerminalConstructionCut::AfterPublicationBeforeCheckpointProof,
        );
        let refused = SqliteFrontier::open_or_rebuild_inactive_bootstrap_terminally(
            &path,
            &runtime,
            &fixture.authority,
            &material,
        );
        assert!(
            matches!(refused, Err(ProjectionError::InjectedFailure)),
            "an unproved published database must not authorize anything: {:?}",
            refused.map(|(_opened, proof)| proof)
        );

        // Restart without the one-shot process artifact: the existing archive
        // replay path rebuilds the published file and preserves its evidence.
        drop(material);
        let restarted = build_projection_at(&fixture, &path, "restart", None);
        assert!(
            matches!(
                restarted.recovery(),
                ProjectionRecovery::RebuiltPreservingEvidence { .. }
            ),
            "a restart over an interrupted publication must preserve evidence: {:?}",
            restarted.recovery()
        );
        let clean = build_projection(&fixture, "clean", None);
        assert_eq!(restarted.observation(), clean.observation());
        fixture.assert_graph_unchanged();
    }

    /// Retained material from another preparation is not authority: it refuses
    /// to bind and the build falls back to the archive.
    #[test]
    fn substituted_terminal_material_refuses_and_falls_back() {
        let mut donor = Fixture::new(
            "terminal-substitute-donor",
            None,
            vec![("pages/donor.md".into(), b"- donor\n".to_vec())],
        );
        let foreign = donor
            .prepared
            .take_terminal_construction_material()
            .unwrap();
        donor.release_bootstrap_projection();

        let mut fixture = Fixture::new(
            "terminal-substitute-target",
            None,
            vec![("pages/target.md".into(), b"- target [[Donor]]\n".to_vec())],
        );
        fixture.release_bootstrap_projection();

        let substituted = build_projection(&fixture, "substituted", Some(&foreign));
        assert_eq!(substituted.bootstrap().terminal_construction_refusals, 1);
        assert_eq!(substituted.bootstrap().terminal_constructions, 0);
        assert_eq!(substituted.bootstrap().terminal_archive_replays, 1);
        let clean = build_projection(&fixture, "clean", None);
        assert_eq!(substituted.observation(), clean.observation());
        assert!(candidate_residue(fixture.root.path()).is_empty());
        fixture.assert_graph_unchanged();
        donor.assert_graph_unchanged();
    }

    /// A forced rebuild over an already-published terminal database produces the
    /// same observations from the archive alone.
    #[test]
    fn forced_rebuild_over_a_terminal_database_replays_the_archive() {
        let mut fixture = Fixture::new(
            "terminal-forced-rebuild",
            None,
            vec![
                (
                    "pages/rebuilt.md".into(),
                    b"- rebuilt #tagged [[Other]]\n".to_vec(),
                ),
                ("pages/other.md".into(), b"- other\n".to_vec()),
            ],
        );
        let material = fixture
            .prepared
            .take_terminal_construction_material()
            .unwrap();
        fixture.release_bootstrap_projection();

        let path = fixture.root.path().join("rebuilt.sqlite");
        let terminal = build_projection_at(&fixture, &path, "terminal", Some(&material));
        assert_eq!(terminal.bootstrap().terminal_constructions, 1);
        drop(material);

        let rebuilt = build_projection_at(&fixture, &path, "rebuild", None);
        assert_eq!(rebuilt.bootstrap().terminal_constructions, 0);
        assert_eq!(rebuilt.bootstrap().terminal_archive_replays, 1);
        assert_eq!(
            rebuilt.bootstrap().bootstrap_part_reads,
            rebuilt.observation().accepted_batch_count
        );
        assert_eq!(terminal.observation(), rebuilt.observation());
        assert!(candidate_residue(fixture.root.path()).is_empty());
        fixture.assert_graph_unchanged();
    }
}
