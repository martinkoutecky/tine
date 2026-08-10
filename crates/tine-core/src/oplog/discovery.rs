//! Inert read-only discovery of an explicitly selected sparse-oplog profile.
//!
//! Discovery is advisory only. It never activates a profile, constructs writer
//! authority, repairs residue, or transfers an unsafe session. Every later
//! actor must independently reopen and authenticate the state and acquire the
//! enrollment/archive writer leases.

use std::path::Path;

use super::enrollment::{
    inspect_existing_enrollment_at, EnrollmentBindingField, EnrollmentDiscoveryExclusion,
    EnrollmentDiscoveryHandoff, EnrollmentDiscoveryInspection, EnrollmentDiscoveryLifecycle,
    EnrollmentError,
};
use super::object_store::{inspect_existing_archive_at, ArchiveDiscoveryInspection, StoreError};
use super::{CanonicalGraphResourceId, ContentDigest, ImportId, SessionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StartupStorageProfile {
    LegacyDefault,
    ExperimentalSparse,
}

pub(crate) struct DiscoveryRequest<'a> {
    pub(crate) profile: StartupStorageProfile,
    pub(crate) graph_resource_id: CanonicalGraphResourceId,
    pub(crate) runtime_root: &'a Path,
    pub(crate) archive_root: &'a Path,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryClassification {
    LegacyDefault,
    Absent,
    ExistingLocalActive(LocalActiveAdvisory),
    ExistingNonActive(NonActiveAdvisory),
    Blocked(BlockedAdvisory),
    UnsupportedOrIncompatible(DiscoveryComponent),
    CorruptOrUnreadable(DiscoveryComponent),
    AmbiguousOrForeignResidue(AmbiguousEvidence),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryComponent {
    Enrollment,
    Archive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NonActiveStage {
    ShadowImport,
    VerifiedLocal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NonActiveAdvisory {
    pub(crate) stage: NonActiveStage,
    pub(crate) binding: super::enrollment::EnrollmentBindingV1,
    pub(crate) enrollment_head: ContentDigest,
    pub(crate) enrollment_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlockedAdvisory {
    pub(crate) binding: super::enrollment::EnrollmentBindingV1,
    pub(crate) enrollment_head: ContentDigest,
    pub(crate) enrollment_generation: u64,
    pub(crate) reason_code: String,
    pub(crate) evidence_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalActiveAdvisory {
    pub(crate) binding: super::enrollment::EnrollmentBindingV1,
    pub(crate) enrollment_head: ContentDigest,
    pub(crate) enrollment_generation: u64,
    pub(crate) handoff: EnrollmentDiscoveryHandoff,
    pub(crate) exclusion: EnrollmentDiscoveryExclusion,
    pub(crate) promotion_session_id: SessionId,
    pub(crate) promoted_state_digest: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AmbiguousEvidence {
    EnrollmentResidue,
    EnrollmentNamespace,
    EnrollmentGraphBinding,
    ArchiveResidue,
    ArchiveNamespace,
    ArchiveBinding,
    ActiveArchiveMismatch,
}

/// Classify explicitly supplied graph/runtime/archive identities without
/// changing any filesystem state.
pub(crate) fn discover_startup(request: &DiscoveryRequest<'_>) -> DiscoveryClassification {
    // The legacy/default path is intentionally decided before either
    // experimental pathname is observed.
    if request.profile == StartupStorageProfile::LegacyDefault {
        return DiscoveryClassification::LegacyDefault;
    }

    let enrollment =
        match inspect_existing_enrollment_at(request.runtime_root, request.graph_resource_id) {
            Ok(inspection) => inspection,
            Err(error) => return classify_enrollment_error(error),
        };
    let evidence = match enrollment {
        EnrollmentDiscoveryInspection::Absent => {
            return match inspect_existing_archive_at(request.archive_root, None) {
                Ok(ArchiveDiscoveryInspection::Absent) => DiscoveryClassification::Absent,
                Ok(
                    ArchiveDiscoveryInspection::Residue | ArchiveDiscoveryInspection::Present(_),
                ) => DiscoveryClassification::AmbiguousOrForeignResidue(
                    AmbiguousEvidence::ArchiveResidue,
                ),
                Err(error) => classify_archive_error(error),
            };
        }
        EnrollmentDiscoveryInspection::Residue => {
            return DiscoveryClassification::AmbiguousOrForeignResidue(
                AmbiguousEvidence::EnrollmentResidue,
            );
        }
        EnrollmentDiscoveryInspection::Present(evidence) => evidence,
    };

    match &evidence.lifecycle {
        EnrollmentDiscoveryLifecycle::ShadowImport => {
            DiscoveryClassification::ExistingNonActive(NonActiveAdvisory {
                stage: NonActiveStage::ShadowImport,
                binding: evidence.binding,
                enrollment_head: evidence.head_digest,
                enrollment_generation: evidence.generation,
            })
        }
        EnrollmentDiscoveryLifecycle::VerifiedLocal => {
            DiscoveryClassification::ExistingNonActive(NonActiveAdvisory {
                stage: NonActiveStage::VerifiedLocal,
                binding: evidence.binding,
                enrollment_head: evidence.head_digest,
                enrollment_generation: evidence.generation,
            })
        }
        EnrollmentDiscoveryLifecycle::Blocked {
            reason_code,
            evidence_digest,
        } => DiscoveryClassification::Blocked(BlockedAdvisory {
            binding: evidence.binding,
            enrollment_head: evidence.head_digest,
            enrollment_generation: evidence.generation,
            reason_code: reason_code.clone(),
            evidence_digest: *evidence_digest,
        }),
        EnrollmentDiscoveryLifecycle::LocalActive(active) => {
            let archive =
                match inspect_existing_archive_at(request.archive_root, Some(&evidence.binding)) {
                    Ok(inspection) => inspection,
                    Err(error) => return classify_archive_error(error),
                };
            let ArchiveDiscoveryInspection::Present(archive) = archive else {
                return DiscoveryClassification::AmbiguousOrForeignResidue(
                    AmbiguousEvidence::ArchiveResidue,
                );
            };
            if archive.bootstrap_import_id
                != ImportId::from_digest(*active.bootstrap_import_id.as_bytes())
                || archive.anchor_history_generation != active.anchor_history_generation
                || archive.anchor_history_index_root != active.anchor_history_index_root
                || archive.anchor_acceptance_sequence != active.anchor_acceptance_sequence
                || archive.anchor_accepted_frontier_state_digest
                    != active.anchor_accepted_frontier_state_digest
                || archive.enrollment_verification_digest != active.verification_digest
            {
                return DiscoveryClassification::AmbiguousOrForeignResidue(
                    AmbiguousEvidence::ActiveArchiveMismatch,
                );
            }
            DiscoveryClassification::ExistingLocalActive(LocalActiveAdvisory {
                binding: evidence.binding,
                enrollment_head: evidence.head_digest,
                enrollment_generation: evidence.generation,
                handoff: active.handoff,
                exclusion: active.exclusion,
                promotion_session_id: archive.promotion_session_id,
                promoted_state_digest: archive.state_digest,
            })
        }
    }
}

fn classify_enrollment_error(error: EnrollmentError) -> DiscoveryClassification {
    match error {
        EnrollmentError::UnsupportedAuthoritySchema(_)
        | EnrollmentError::UnsupportedCheckpointSchema(_)
        | EnrollmentError::UnsupportedRecordSchema(_)
        | EnrollmentError::UnsupportedPacketSchema(_)
        | EnrollmentError::UnsupportedCompatibility { .. }
        | EnrollmentError::FutureUnsupportedLifecycle(_) => {
            DiscoveryClassification::UnsupportedOrIncompatible(DiscoveryComponent::Enrollment)
        }
        EnrollmentError::BindingMismatch(EnrollmentBindingField::GraphResource) => {
            DiscoveryClassification::AmbiguousOrForeignResidue(
                AmbiguousEvidence::EnrollmentGraphBinding,
            )
        }
        EnrollmentError::BindingMismatch(_)
        | EnrollmentError::UnsafeNamespace(_)
        | EnrollmentError::UnsupportedArtifact(_)
        | EnrollmentError::NamespaceBoundExceeded
        | EnrollmentError::AmbiguousInitialCreation
        | EnrollmentError::AmbiguousRecordPublication
        | EnrollmentError::AmbiguousAuthorityProvisioning => {
            DiscoveryClassification::AmbiguousOrForeignResidue(
                AmbiguousEvidence::EnrollmentNamespace,
            )
        }
        _ => DiscoveryClassification::CorruptOrUnreadable(DiscoveryComponent::Enrollment),
    }
}

fn classify_archive_error(error: StoreError) -> DiscoveryClassification {
    match error {
        StoreError::UpgradeRequired { .. }
        | StoreError::UnsupportedStoreVersion { .. }
        | StoreError::UnsupportedPromotedRuntimeSchema(_) => {
            DiscoveryClassification::UnsupportedOrIncompatible(DiscoveryComponent::Archive)
        }
        StoreError::PromotedRuntimeStateMismatch(_)
        | StoreError::WorkspaceMismatch { .. }
        | StoreError::LineageMismatch { .. } => {
            DiscoveryClassification::AmbiguousOrForeignResidue(AmbiguousEvidence::ArchiveBinding)
        }
        StoreError::UnsafeEntry(_) | StoreError::MalformedPath(_) => {
            DiscoveryClassification::AmbiguousOrForeignResidue(AmbiguousEvidence::ArchiveNamespace)
        }
        _ => DiscoveryClassification::CorruptOrUnreadable(DiscoveryComponent::Archive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Graph;
    use crate::oplog::enrollment::{
        create_discovery_enrollment_for_test, EnrollmentBindingV1,
        EnrollmentDiscoveryFixtureLifecycle, EnrollmentDiscoveryLifecycle,
        EnrollmentDiscoveryLocalActive,
    };
    use crate::oplog::object_store::{
        create_discovery_promoted_archive_for_test, PromotedRuntimeStateV1,
    };
    use crate::oplog::{
        CanonicalArchiveResourceId, DeviceId, DocumentId, LineageDigest, ObjectStore,
        ProjectionEndpointId, ProjectionReceiptStoreId, WorkspaceId,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;
    use uuid::Uuid;

    struct TestWorld {
        root: PathBuf,
        graph_root: PathBuf,
        runtime_root: PathBuf,
        archive_root: PathBuf,
    }

    impl TestWorld {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tine-readonly-discovery-{label}-{}",
                Uuid::new_v4()
            ));
            let graph_root = root.join("graph");
            fs::create_dir_all(&graph_root).unwrap();
            Self {
                graph_root,
                runtime_root: root.join("runtime"),
                archive_root: root.join("archive"),
                root,
            }
        }

        fn graph_resource(&self) -> CanonicalGraphResourceId {
            Graph::open_checked(&self.graph_root)
                .unwrap()
                .canonical_resource_id()
                .unwrap()
        }

        fn request(&self, profile: StartupStorageProfile) -> DiscoveryRequest<'_> {
            DiscoveryRequest {
                profile,
                graph_resource_id: self.graph_resource(),
                runtime_root: &self.runtime_root,
                archive_root: &self.archive_root,
            }
        }
    }

    impl Drop for TestWorld {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn binding_with_archive(world: &TestWorld, seed: u128) -> EnrollmentBindingV1 {
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(seed));
        let store = ObjectStore::open(&world.archive_root, workspace).unwrap();
        let archive_resource = store.provision_enrolled_archive_resource_id().unwrap();
        drop(store);
        binding(world, seed, workspace, archive_resource)
    }

    fn binding(
        world: &TestWorld,
        seed: u128,
        workspace: WorkspaceId,
        archive_resource: CanonicalArchiveResourceId,
    ) -> EnrollmentBindingV1 {
        let graph = Graph::open_checked(&world.graph_root).unwrap();
        let graph_resource = graph.canonical_resource_id().unwrap();
        EnrollmentBindingV1::new(
            workspace,
            LineageDigest::from_bytes(*ContentDigest::of(&seed.to_be_bytes()).as_bytes()),
            DocumentId::from_uuid(Uuid::from_u128(seed + 1)),
            ProjectionEndpointId::from_uuid(Uuid::from_u128(seed + 2)),
            DeviceId::from_uuid(Uuid::from_u128(seed + 3)),
            graph_resource,
            ProjectionReceiptStoreId::from_capability_identity(
                b"discovery-test",
                &seed.to_be_bytes(),
            ),
            archive_resource,
            graph.graph_text_scope_binding().unwrap(),
        )
        .unwrap()
    }

    fn create_active(
        world: &TestWorld,
        lifecycle: EnrollmentDiscoveryFixtureLifecycle,
        seed: u128,
    ) -> (EnrollmentBindingV1, EnrollmentDiscoveryLocalActive) {
        let binding = binding_with_archive(world, seed);
        let evidence =
            create_discovery_enrollment_for_test(&world.runtime_root, binding.clone(), lifecycle)
                .unwrap();
        let EnrollmentDiscoveryLifecycle::LocalActive(active) = evidence.lifecycle else {
            panic!("active fixture produced a non-active lifecycle");
        };
        create_discovery_promoted_archive_for_test(
            &world.archive_root,
            &binding,
            &active,
            SessionId::from_uuid(Uuid::from_u128(seed + 4)),
        )
        .unwrap();
        (binding, active)
    }

    #[test]
    fn legacy_default_returns_before_touching_experimental_paths() {
        let root = std::env::temp_dir().join(format!(
            "tine-discovery-legacy-parent-missing-{}",
            Uuid::new_v4()
        ));
        let runtime = root.join("runtime");
        let archive = root.join("archive");
        let result = discover_startup(&DiscoveryRequest {
            profile: StartupStorageProfile::LegacyDefault,
            graph_resource_id: CanonicalGraphResourceId::from_capability_identity(
                b"legacy-test",
                b"graph",
            ),
            runtime_root: &runtime,
            archive_root: &archive,
        });
        assert_eq!(result, DiscoveryClassification::LegacyDefault);
        assert!(!root.exists());
    }

    #[test]
    fn absent_explicit_roots_remain_absent() {
        let world = TestWorld::new("absent");
        assert!(!world.runtime_root.exists());
        assert!(!world.archive_root.exists());
        assert_eq!(
            discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse)),
            DiscoveryClassification::Absent
        );
        assert!(!world.runtime_root.exists());
        assert!(!world.archive_root.exists());
    }

    #[test]
    fn authenticated_non_active_states_need_explicit_activation() {
        for (label, lifecycle, expected) in [
            (
                "shadow",
                EnrollmentDiscoveryFixtureLifecycle::ShadowImport,
                NonActiveStage::ShadowImport,
            ),
            (
                "verified",
                EnrollmentDiscoveryFixtureLifecycle::VerifiedLocal,
                NonActiveStage::VerifiedLocal,
            ),
        ] {
            let world = TestWorld::new(label);
            let binding = binding_with_archive(&world, 0x1000);
            create_discovery_enrollment_for_test(&world.runtime_root, binding, lifecycle).unwrap();
            assert!(matches!(
                discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse)),
                DiscoveryClassification::ExistingNonActive(NonActiveAdvisory {
                    stage,
                    ..
                }) if stage == expected
            ));
        }
    }

    #[test]
    fn authenticated_local_active_safe_and_unsafe_are_advisory_only() {
        let unsafe_world = TestWorld::new("active-unsafe");
        let unsafe_session = SessionId::from_uuid(Uuid::from_u128(0x2005));
        create_active(
            &unsafe_world,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveUnsafe {
                session_id: unsafe_session,
            },
            0x2000,
        );
        assert!(matches!(
            discover_startup(
                &unsafe_world.request(StartupStorageProfile::ExperimentalSparse)
            ),
            DiscoveryClassification::ExistingLocalActive(LocalActiveAdvisory {
                handoff: EnrollmentDiscoveryHandoff::Unsafe { session_id },
                exclusion: EnrollmentDiscoveryExclusion::Idle,
                ..
            }) if session_id == unsafe_session
        ));

        let safe_world = TestWorld::new("active-safe");
        create_active(
            &safe_world,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
            0x2100,
        );
        assert!(matches!(
            discover_startup(&safe_world.request(StartupStorageProfile::ExperimentalSparse)),
            DiscoveryClassification::ExistingLocalActive(LocalActiveAdvisory {
                handoff: EnrollmentDiscoveryHandoff::Safe,
                exclusion: EnrollmentDiscoveryExclusion::Idle,
                ..
            })
        ));
    }

    #[test]
    fn blocked_and_unsupported_states_do_not_classify_active() {
        let blocked = TestWorld::new("blocked");
        let binding = binding_with_archive(&blocked, 0x3000);
        create_discovery_enrollment_for_test(
            &blocked.runtime_root,
            binding,
            EnrollmentDiscoveryFixtureLifecycle::Blocked,
        )
        .unwrap();
        assert!(matches!(
            discover_startup(&blocked.request(StartupStorageProfile::ExperimentalSparse)),
            DiscoveryClassification::Blocked(_)
        ));

        let unsupported_enrollment = TestWorld::new("unsupported-enrollment");
        let binding = binding_with_archive(&unsupported_enrollment, 0x3100);
        create_discovery_enrollment_for_test(
            &unsupported_enrollment.runtime_root,
            binding,
            EnrollmentDiscoveryFixtureLifecycle::ShadowImport,
        )
        .unwrap();
        let authority = find_named_file(&unsupported_enrollment.runtime_root, "authority-v1.claim");
        let claim = String::from_utf8(fs::read(&authority).unwrap())
            .unwrap()
            .replacen("\"schema_version\":2", "\"schema_version\":4294967295", 1);
        fs::write(&authority, claim).unwrap();
        assert_eq!(
            discover_startup(
                &unsupported_enrollment.request(StartupStorageProfile::ExperimentalSparse)
            ),
            DiscoveryClassification::UnsupportedOrIncompatible(DiscoveryComponent::Enrollment)
        );

        let unsupported_archive = TestWorld::new("unsupported-archive");
        let (binding, _) = create_active(
            &unsupported_archive,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
            0x3200,
        );
        let state_path = promoted_state_path(&unsupported_archive.archive_root, &binding);
        let mut state = PromotedRuntimeStateV1::decode(&fs::read(&state_path).unwrap()).unwrap();
        state.schema_version = u32::MAX;
        fs::write(&state_path, postcard::to_allocvec(&state).unwrap()).unwrap();
        assert_eq!(
            discover_startup(
                &unsupported_archive.request(StartupStorageProfile::ExperimentalSparse)
            ),
            DiscoveryClassification::UnsupportedOrIncompatible(DiscoveryComponent::Archive)
        );
    }

    #[test]
    fn malformed_truncated_and_corrupt_enrollment_is_never_active() {
        for (label, damage) in [
            ("truncated-head", "head"),
            ("truncated-record", "record"),
            ("record-corrupt", "record-integrity"),
        ] {
            let world = TestWorld::new(label);
            create_active(
                &world,
                EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
                0x4000,
            );
            match damage {
                "head" => {
                    let head = find_named_file(&world.runtime_root, "head");
                    fs::write(head, b"truncated").unwrap();
                }
                "record" => {
                    let head = find_named_file(&world.runtime_root, "head");
                    let digest = fs::read_to_string(head).unwrap();
                    let record = find_named_file(
                        &world.runtime_root,
                        &format!("{}.enrollment", digest.trim()),
                    );
                    fs::write(record, b"{").unwrap();
                }
                "record-integrity" => {
                    let record = find_file_containing(&world.runtime_root, b"\"integrity_tag\":");
                    mutate_record_integrity_tag(&record);
                    let error =
                        inspect_existing_enrollment_at(&world.runtime_root, world.graph_resource())
                            .unwrap_err();
                    assert!(
                        matches!(error, EnrollmentError::RecordDigestMismatch(_)),
                        "expected content-addressed record corruption failure, got {error:?}"
                    );
                }
                _ => unreachable!(),
            }
            assert_eq!(
                discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse)),
                DiscoveryClassification::CorruptOrUnreadable(DiscoveryComponent::Enrollment)
            );
        }
    }

    #[test]
    fn malformed_promoted_state_is_never_classified_active() {
        let world = TestWorld::new("malformed-promoted-state");
        let (binding, _) = create_active(
            &world,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
            0x4100,
        );
        fs::write(
            promoted_state_path(&world.archive_root, &binding),
            b"truncated",
        )
        .unwrap();
        assert_eq!(
            discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse)),
            DiscoveryClassification::CorruptOrUnreadable(DiscoveryComponent::Archive)
        );
    }

    #[test]
    fn copied_and_mismatched_archive_evidence_is_ambiguous_not_active() {
        let copied = TestWorld::new("copied-archive");
        let (binding, _) = create_active(
            &copied,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
            0x5000,
        );
        let copied_root = copied.root.join("archive-copy");
        copy_tree(&copied.archive_root, &copied_root);
        let copied_result = discover_startup(&DiscoveryRequest {
            profile: StartupStorageProfile::ExperimentalSparse,
            graph_resource_id: copied.graph_resource(),
            runtime_root: &copied.runtime_root,
            archive_root: &copied_root,
        });
        assert_eq!(
            copied_result,
            DiscoveryClassification::AmbiguousOrForeignResidue(AmbiguousEvidence::ArchiveBinding)
        );

        let foreign = TestWorld::new("foreign-archive");
        create_active(
            &foreign,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveSafe,
            0x5100,
        );
        let foreign_result = discover_startup(&DiscoveryRequest {
            profile: StartupStorageProfile::ExperimentalSparse,
            graph_resource_id: copied.graph_resource(),
            runtime_root: &copied.runtime_root,
            archive_root: &foreign.archive_root,
        });
        assert!(matches!(
            foreign_result,
            DiscoveryClassification::AmbiguousOrForeignResidue(AmbiguousEvidence::ArchiveBinding)
        ));
        assert_eq!(
            binding.graph_resource_id(),
            copied.graph_resource(),
            "the copied case used the intended graph binding"
        );
    }

    #[test]
    fn repeated_discovery_is_deterministic_and_changes_no_content_or_metadata() {
        let world = TestWorld::new("deterministic");
        create_active(
            &world,
            EnrollmentDiscoveryFixtureLifecycle::LocalActiveUnsafe {
                session_id: SessionId::from_uuid(Uuid::from_u128(0x6005)),
            },
            0x6000,
        );
        let before = snapshot_tree(&world.root);
        let first = discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse));
        let middle = snapshot_tree(&world.root);
        let second = discover_startup(&world.request(StartupStorageProfile::ExperimentalSparse));
        let after = snapshot_tree(&world.root);
        assert_eq!(first, second);
        assert_eq!(before, middle);
        assert_eq!(middle, after);
    }

    #[test]
    fn nested_nonstandard_unicode_and_space_graph_paths_work() {
        let root = std::env::temp_dir().join(format!("tine-discovery-paths-{}", Uuid::new_v4()));
        let graph = root.join("outer space").join("非標準").join("nested.graph");
        fs::create_dir_all(&graph).unwrap();
        let opened = Graph::open_checked(&graph).unwrap();
        let runtime = root.join("runtime roots").join("device α");
        let archive = root.join("archive roots").join("device β");
        let result = discover_startup(&DiscoveryRequest {
            profile: StartupStorageProfile::ExperimentalSparse,
            graph_resource_id: opened.canonical_resource_id().unwrap(),
            runtime_root: &runtime,
            archive_root: &archive,
        });
        assert_eq!(result, DiscoveryClassification::Absent);
        assert!(!runtime.exists());
        assert!(!archive.exists());
        crate::test_support::remove_dir_all(root);
    }

    fn find_named_file(root: &Path, expected: &str) -> PathBuf {
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else if entry.file_name() == expected {
                    return entry.path();
                }
            }
        }
        panic!("did not find {expected} below {}", root.display());
    }

    fn find_file_containing(root: &Path, expected: &[u8]) -> PathBuf {
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else if fs::read(entry.path())
                    .unwrap()
                    .windows(expected.len())
                    .any(|window| window == expected)
                {
                    return entry.path();
                }
            }
        }
        panic!(
            "did not find file containing {:?} below {}",
            String::from_utf8_lossy(expected),
            root.display()
        );
    }

    fn promoted_state_path(archive: &Path, binding: &EnrollmentBindingV1) -> PathBuf {
        archive
            .join("engine-history")
            .join(binding.endpoint_id().to_string())
            .join("promoted-runtime.state")
    }

    fn mutate_record_integrity_tag(path: &Path) {
        let mut record = String::from_utf8(fs::read(path).unwrap()).unwrap();
        let start = record.find("\"integrity_tag\":").unwrap() + "\"integrity_tag\":".len();
        let end = record[start..]
            .find('}')
            .map(|offset| start + offset)
            .unwrap();
        let old = record[start..end].parse::<u32>().unwrap();
        record.replace_range(start..end, &(old ^ 1).to_string());
        fs::write(path, record).unwrap();
    }

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct SnapshotEntry {
        is_dir: bool,
        len: u64,
        readonly: bool,
        modified: Option<SystemTime>,
        bytes: Option<Vec<u8>>,
    }

    fn snapshot_tree(root: &Path) -> BTreeMap<PathBuf, SnapshotEntry> {
        let mut snapshot = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let metadata = fs::symlink_metadata(&path).unwrap();
            let is_dir = metadata.is_dir();
            snapshot.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                SnapshotEntry {
                    is_dir,
                    len: metadata.len(),
                    readonly: metadata.permissions().readonly(),
                    modified: metadata.modified().ok(),
                    bytes: (!is_dir).then(|| fs::read(&path).unwrap()),
                },
            );
            if is_dir {
                let mut children = fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                pending.extend(children.into_iter().rev());
            }
        }
        snapshot
    }
}
