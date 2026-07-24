use std::collections::BTreeMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::object_store::{
    StoreError, ensure_directory_nofollow, is_temp_name, open_dir_nofollow,
    publish_immutable_exact, read_optional_regular, require_regular_entry,
};
use super::{
    BaseBlob, BlobDescription, CapabilityCapturedProjectionInput,
    CapabilityCapturedProjectionState, ManagedPath, ProjectionCompletion,
    ProjectionEndpointBinding, ProjectionIntent, ProjectionIntentId, ProjectionPrecondition,
    ProjectionReceiptStoreId, ProjectionWork, ProjectionWorkCompletionAuthority,
    ProjectionWorkTarget, ReceiptError, WorkspaceId,
};
use crate::model::{Graph, ProjectionWriteProof};

const STORE_CLAIM_FILE: &str = "projection-receipts.claim";
const BASES_DIR: &str = "bases";
const INTENTS_DIR: &str = "intents";
const COMPLETIONS_DIR: &str = "completions";
const ATTEMPTS_DIR: &str = "attempts";
const FORENSICS_DIR: &str = "forensics";
const STORE_INIT_FILE: &str = "projection-receipts.init";
const STORE_CLAIM_MAGIC: &[u8; 8] = b"TINEPR5\0";
const PRIOR_STORE_CLAIM_MAGICS: [&[u8; 8]; 2] = [b"TINEPR4\0", b"TINEPR3\0"];
const STORE_INIT_MAGIC: &[u8; 8] = b"TINEPI5\0";
const STORE_CLAIM_VERSION: u32 = 5;
const STORE_CLAIM_BASE_LEN: usize = STORE_CLAIM_MAGIC.len() + 4 + 32 + 16 + 1 + 16 + 16 + 32;
const STORE_CLAIM_LEN: usize = STORE_CLAIM_BASE_LEN + 5 * 32;
const STORE_INIT_LEN: usize = STORE_CLAIM_BASE_LEN;
pub(crate) const MAX_PROJECTION_EVIDENCE_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_PROJECTION_CATALOG_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PROJECTION_CATALOG_ROWS: usize = 2_000_000;
const MAX_PROJECTION_CATALOG_DIRECTORY_ENTRIES: usize = 4_000_000;
const LOCAL_ATTEMPT_SCHEMA_VERSION: u32 = 1;
const LOCAL_FORENSIC_SCHEMA_VERSION: u32 = 1;
const INTENT_NAMESPACE_SCHEMA_VERSION: u32 = 1;
const INTENT_NAMESPACE_RESERVATION_SUFFIX: &str = ".namespace-reservation";
const INTENT_NAMESPACE_AUTHORITY_SUFFIX: &str = ".namespace-authority";

type DirectoryIdentity = [u8; 32];

#[derive(Debug)]
struct BoundNamespace {
    capability: Dir,
    identity: DirectoryIdentity,
}

#[derive(Debug)]
struct ReceiptNamespaces {
    bases: BoundNamespace,
    intents: BoundNamespace,
    completions: BoundNamespace,
    attempts: BoundNamespace,
    forensics: BoundNamespace,
}

impl ReceiptNamespaces {
    fn get(&self, name: &str) -> Option<&BoundNamespace> {
        match name {
            BASES_DIR => Some(&self.bases),
            INTENTS_DIR => Some(&self.intents),
            COMPLETIONS_DIR => Some(&self.completions),
            ATTEMPTS_DIR => Some(&self.attempts),
            FORENSICS_DIR => Some(&self.forensics),
            _ => None,
        }
    }

    fn identities(&self) -> [DirectoryIdentity; 5] {
        [
            self.bases.identity,
            self.intents.identity,
            self.completions.identity,
            self.attempts.identity,
            self.forensics.identity,
        ]
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentNamespaceReservation {
    schema_version: u32,
    store_id: ProjectionReceiptStoreId,
    namespace: String,
    intent_id: ProjectionIntentId,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentNamespaceAuthority {
    schema_version: u32,
    store_id: ProjectionReceiptStoreId,
    namespace: String,
    intent_id: ProjectionIntentId,
    directory_identity: DirectoryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionAttemptReservation {
    schema_version: u32,
    intent_id: ProjectionIntentId,
    attempt_id: Uuid,
    target_path: ManagedPath,
    recovery_filename: String,
}

impl ProjectionAttemptReservation {
    pub const fn intent_id(&self) -> ProjectionIntentId {
        self.intent_id
    }

    pub const fn attempt_id(&self) -> Uuid {
        self.attempt_id
    }

    pub fn target_path(&self) -> &ManagedPath {
        &self.target_path
    }

    pub fn recovery_filename(&self) -> &str {
        &self.recovery_filename
    }

    #[cfg(test)]
    pub(crate) fn for_test(target_path: &str) -> Self {
        let target_path = ManagedPath::parse(target_path).expect("valid test projection path");
        let target_filename = target_path
            .as_str()
            .rsplit_once('/')
            .expect("managed paths contain a parent")
            .1
            .to_owned();
        let attempt_id = Uuid::new_v4();
        Self {
            schema_version: LOCAL_ATTEMPT_SCHEMA_VERSION,
            intent_id: ProjectionIntentId::test_only_zero(),
            attempt_id,
            target_path,
            recovery_filename: format!(
                ".{target_filename}.{}.projection.recovery",
                attempt_id.simple()
            ),
        }
    }

    fn new(intent: &ProjectionIntent, attempt_id: Uuid) -> Result<Self, ProjectionStoreError> {
        let target_filename = intent
            .path()
            .as_str()
            .rsplit_once('/')
            .expect("managed paths contain a parent")
            .1;
        let reservation = Self {
            schema_version: LOCAL_ATTEMPT_SCHEMA_VERSION,
            intent_id: intent.id()?,
            attempt_id,
            target_path: intent.path().clone(),
            recovery_filename: format!(
                ".{target_filename}.{}.projection.recovery",
                attempt_id.simple()
            ),
        };
        reservation.validate(intent)?;
        Ok(reservation)
    }

    fn validate(&self, intent: &ProjectionIntent) -> Result<(), ProjectionStoreError> {
        let expected = Self::new_unchecked(intent, self.attempt_id)?;
        if self.schema_version != LOCAL_ATTEMPT_SCHEMA_VERSION
            || self.intent_id != intent.id()?
            || self.target_path != *intent.path()
            || self.recovery_filename != expected.recovery_filename
        {
            return Err(ProjectionStoreError::AttemptBindingMismatch);
        }
        Ok(())
    }

    fn new_unchecked(
        intent: &ProjectionIntent,
        attempt_id: Uuid,
    ) -> Result<Self, ProjectionStoreError> {
        let target_filename = intent
            .path()
            .as_str()
            .rsplit_once('/')
            .expect("managed paths contain a parent")
            .1;
        Ok(Self {
            schema_version: LOCAL_ATTEMPT_SCHEMA_VERSION,
            intent_id: intent.id()?,
            attempt_id,
            target_path: intent.path().clone(),
            recovery_filename: format!(
                ".{target_filename}.{}.projection.recovery",
                attempt_id.simple()
            ),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalProjectionEvidenceRecord {
    schema_version: u32,
    intent_id: ProjectionIntentId,
    attempt_id: Uuid,
    target_path: ManagedPath,
    recovery_relative_path: String,
    recovery_filename: String,
    observed: BlobDescription,
}

impl LocalProjectionEvidenceRecord {
    pub const fn intent_id(&self) -> ProjectionIntentId {
        self.intent_id
    }

    pub const fn attempt_id(&self) -> Uuid {
        self.attempt_id
    }

    pub fn recovery_relative_path(&self) -> &str {
        &self.recovery_relative_path
    }

    pub fn recovery_filename(&self) -> &str {
        &self.recovery_filename
    }

    pub const fn observed(&self) -> BlobDescription {
        self.observed
    }
}

/// Disconnected immutable storage for projection bases, intents, and completions.
///
/// Opening this store is never performed by graph startup. Every path operation
/// remains relative to the retained no-follow directory capability.
#[derive(Debug)]
pub struct ProjectionReceiptStore {
    root_path: PathBuf,
    store_id: ProjectionReceiptStoreId,
    workspace_id: WorkspaceId,
    endpoint: Option<ProjectionEndpointBinding>,
    capability: Dir,
    namespaces: ReceiptNamespaces,
}

/// Canonical read-only catalog row used only by the combined import authority.
///
/// Fields stay crate-private so a downstream caller cannot manufacture a
/// durable intent/completion claim. Construction validates the entire intent
/// and completion namespaces, including exact base bytes and orphan entries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectionCatalogEntry {
    pub(crate) intent: ProjectionIntent,
    pub(crate) completion: Option<ProjectionCompletion>,
}

impl ProjectionReceiptStore {
    pub fn open(root: &Path, workspace_id: WorkspaceId) -> Result<Self, ProjectionStoreError> {
        Self::open_with_binding(root, workspace_id, None)
    }

    /// Open a receipt namespace durably enrolled to one endpoint and one exact
    /// graph-root filesystem resource.
    pub fn open_for_endpoint(
        root: &Path,
        workspace_id: WorkspaceId,
        endpoint: ProjectionEndpointBinding,
    ) -> Result<Self, ProjectionStoreError> {
        Self::open_with_binding(root, workspace_id, Some(endpoint))
    }

    fn open_with_binding(
        root: &Path,
        workspace_id: WorkspaceId,
        endpoint: Option<ProjectionEndpointBinding>,
    ) -> Result<Self, ProjectionStoreError> {
        let name = root
            .file_name()
            .ok_or_else(|| ProjectionStoreError::UnsafeEntry("store root has no name".into()))?;
        if !matches!(root.components().next_back(), Some(Component::Normal(_))) {
            return Err(ProjectionStoreError::UnsafeEntry(
                "store root must end in a normal path component".into(),
            ));
        }
        let name = name.to_str().ok_or_else(|| {
            ProjectionStoreError::UnsafeEntry("store root name is not UTF-8".into())
        })?;
        let parent = root.parent().ok_or_else(|| {
            ProjectionStoreError::UnsafeEntry("store root has no existing parent".into())
        })?;
        let canonical_parent = std::fs::canonicalize(parent)?;
        let parent_capability = Dir::open_ambient_dir(&canonical_parent, ambient_authority())?;
        ensure_directory_nofollow(&parent_capability, name)?;
        let capability = open_dir_nofollow(&parent_capability, name)?;
        let store_id = canonical_receipt_store_id(&capability)?;
        let namespaces = Self::initialize(&capability, store_id, workspace_id, endpoint)?;

        Ok(Self {
            root_path: canonical_parent.join(name),
            store_id,
            workspace_id,
            endpoint,
            capability,
            namespaces,
        })
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub const fn store_id(&self) -> ProjectionReceiptStoreId {
        self.store_id
    }

    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }

    pub const fn endpoint_binding(&self) -> Option<ProjectionEndpointBinding> {
        self.endpoint
    }

    /// Capture an exact authoring precondition through Graph's retained
    /// no-follow capability. A present input is accepted only with a completion
    /// reloaded from this enrolled store and bound to the exact current bytes.
    pub fn capture_projection_input(
        &self,
        graph: &Graph,
        endpoint: ProjectionEndpointBinding,
        path: ManagedPath,
        prior_intent: Option<&ProjectionIntent>,
    ) -> Result<CapabilityCapturedProjectionInput, ProjectionStoreError> {
        self.require_endpoint(endpoint)?;
        let graph_resource_id = graph
            .canonical_resource_id()
            .map_err(ProjectionStoreError::Io)?;
        if graph_resource_id != endpoint.graph_resource_id {
            return Err(ProjectionStoreError::GraphResourceMismatch);
        }
        let current = graph
            .read_projection_input(&path)
            .map_err(ProjectionStoreError::Io)?;
        let state = match (current, prior_intent) {
            (None, None) => CapabilityCapturedProjectionState::Absent,
            (None, Some(_)) => return Err(ProjectionStoreError::CapturedInputMismatch),
            (Some(_), None) => return Err(ProjectionStoreError::MissingPriorCompletion),
            (Some(bytes), Some(intent)) => {
                if intent.workspace_id() != self.workspace_id
                    || intent.path() != &path
                    || intent.target() != BlobDescription::of(&bytes)
                {
                    return Err(ProjectionStoreError::CapturedInputMismatch);
                }
                let prior_completion = self
                    .load_completion(intent)?
                    .ok_or(ProjectionStoreError::MissingPriorCompletion)?;
                CapabilityCapturedProjectionState::Present {
                    bytes,
                    prior_intent: intent.clone(),
                    prior_completion,
                }
            }
        };
        Ok(CapabilityCapturedProjectionInput::from_graph_capability(
            path,
            endpoint,
            self.store_id,
            state,
        ))
    }

    /// Publish immutable base bytes first and the canonical intent last.
    pub fn publish_intent(
        &self,
        intent: &ProjectionIntent,
        base_bytes: Option<&[u8]>,
    ) -> Result<ProjectionIntentId, ProjectionStoreError> {
        self.require_workspace(intent)?;
        let bytes = intent.encode()?;
        require_evidence_length(
            "projection target",
            intent.target().byte_length(),
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;
        require_evidence_length(
            "projection intent",
            bytes.len() as u64,
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;

        let intent_id = intent.id()?;
        match (intent.precondition(), base_bytes) {
            (ProjectionPrecondition::Absent, None) => {}
            (ProjectionPrecondition::Absent, Some(_)) => {
                return Err(ProjectionStoreError::UnexpectedBase);
            }
            (ProjectionPrecondition::Base(description), None) => {
                return Err(ProjectionStoreError::MissingBase(*description));
            }
            (ProjectionPrecondition::Base(description), Some(base_bytes)) => {
                require_evidence_length(
                    "projection base",
                    description.byte_length(),
                    MAX_PROJECTION_EVIDENCE_BYTES,
                )?;
                if BlobDescription::of(base_bytes) != *description {
                    return Err(ProjectionStoreError::BaseEvidenceMismatch(*description));
                }
                let bases = self.namespace(BASES_DIR)?;
                publish_immutable_exact(
                    &bases,
                    &base_filename(*description),
                    base_bytes,
                    "projection base",
                )?;
            }
        }

        let intents = self.namespace(INTENTS_DIR)?;
        let intent_name = intent_filename(intent_id);
        let already_published =
            read_optional_regular(&intents, &intent_name, MAX_PROJECTION_EVIDENCE_BYTES, None)?
                .is_some();
        // The intent is the commit marker for its local recovery namespaces.
        // Once it is visible, both per-intent directory identities must
        // already be durably bound and can never be recreated by name.
        if already_published {
            self.required_intent_namespace(ATTEMPTS_DIR, intent_id)?;
            self.required_intent_namespace(FORENSICS_DIR, intent_id)?;
        } else {
            self.intent_namespace(ATTEMPTS_DIR, intent_id)?;
            self.intent_namespace(FORENSICS_DIR, intent_id)?;
        }
        publish_immutable_exact(&intents, &intent_name, &bytes, "projection intent")?;
        Ok(intent_id)
    }

    /// Load and validate the intent and every base byte needed to authorize it.
    pub fn load_intent(
        &self,
        intent_id: ProjectionIntentId,
    ) -> Result<Option<ProjectionIntent>, ProjectionStoreError> {
        let intents = self.namespace(INTENTS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &intents,
            &intent_filename(intent_id),
            MAX_PROJECTION_EVIDENCE_BYTES,
            None,
        )?
        else {
            return Ok(None);
        };
        let intent = ProjectionIntent::decode(&bytes)?;
        self.require_workspace(&intent)?;
        if intent.id()? != intent_id {
            return Err(ProjectionStoreError::PathBindingMismatch(
                "projection intent",
            ));
        }
        if intent.encode()? != bytes {
            return Err(ProjectionStoreError::NonCanonical("projection intent"));
        }
        self.load_base(&intent)?;
        Ok(Some(intent))
    }

    /// Retrieve exact base bytes from the immutable base namespace.
    pub fn load_base(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<Option<BaseBlob>, ProjectionStoreError> {
        self.require_workspace(intent)?;
        let ProjectionPrecondition::Base(description) = intent.precondition() else {
            return Ok(None);
        };
        require_evidence_length(
            "projection base",
            description.byte_length(),
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;
        let bases = self.namespace(BASES_DIR)?;
        let filename = base_filename(*description);
        let bytes = read_optional_regular(
            &bases,
            &filename,
            MAX_PROJECTION_EVIDENCE_BYTES,
            Some(description.byte_length()),
        )?
        .ok_or(ProjectionStoreError::MissingBase(*description))?;
        if BlobDescription::of(&bytes) != *description {
            return Err(ProjectionStoreError::BaseEvidenceMismatch(*description));
        }
        Ok(Some(BaseBlob::from_parts(*description, bytes)?))
    }

    /// Durably reserve the exact recovery filename Graph must use before any
    /// live page name can be retired or published.
    pub fn reserve_attempt(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<ProjectionAttemptReservation, ProjectionStoreError> {
        let intent_id = self.require_published_intent(intent)?;
        let reservation = ProjectionAttemptReservation::new(intent, Uuid::new_v4())?;
        let bytes = serde_json::to_vec(&reservation)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
        let attempts = self.required_intent_namespace(ATTEMPTS_DIR, intent_id)?;
        publish_immutable_exact(
            &attempts,
            &attempt_filename(reservation.attempt_id),
            &bytes,
            "projection attempt reservation",
        )?;
        Ok(reservation)
    }

    /// Load only this intent's bounded attempt namespace. Recovery never scans
    /// graph page directories or other intents for generated-name patterns.
    pub fn load_attempt_reservations(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<Vec<ProjectionAttemptReservation>, ProjectionStoreError> {
        let intent_id = self.require_published_intent(intent)?;
        let attempts = self.required_intent_namespace(ATTEMPTS_DIR, intent_id)?;
        let mut reservations = Vec::new();
        for entry in attempts.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                ProjectionStoreError::UnsafeEntry("non-UTF-8 projection attempt entry".into())
            })?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            let attempt_id = parse_attempt_filename(name)?;
            let bytes =
                read_optional_regular(&attempts, name, MAX_PROJECTION_EVIDENCE_BYTES, None)?
                    .ok_or_else(|| {
                        ProjectionStoreError::UnsafeEntry(format!(
                            "projection attempt disappeared during enumeration: {name}"
                        ))
                    })?;
            let reservation: ProjectionAttemptReservation = serde_json::from_slice(&bytes)
                .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
            reservation.validate(intent)?;
            if reservation.attempt_id != attempt_id
                || serde_json::to_vec(&reservation)
                    .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?
                    != bytes
            {
                return Err(ProjectionStoreError::AttemptBindingMismatch);
            }
            reservations.push(reservation);
        }
        reservations.sort_unstable_by_key(ProjectionAttemptReservation::attempt_id);
        Ok(reservations)
    }

    /// Publish completion only from Graph's capability-issued exact-write proof.
    pub(crate) fn publish_completion(
        &self,
        intent: &ProjectionIntent,
        proof: &ProjectionWriteProof,
    ) -> Result<ProjectionCompletion, ProjectionStoreError> {
        self.require_write_proof(intent, proof)?;
        let intent_id = self.require_published_intent(intent)?;
        let reservations = self.load_attempt_reservations(intent)?;
        for evidence in proof.recovery_evidence() {
            let reservation = reservations
                .iter()
                .find(|reservation| reservation.recovery_filename() == evidence.filename())
                .ok_or(ProjectionStoreError::UnreservedRecoveryEvidence)?;
            let record = LocalProjectionEvidenceRecord {
                schema_version: LOCAL_FORENSIC_SCHEMA_VERSION,
                intent_id,
                attempt_id: reservation.attempt_id(),
                target_path: intent.path().clone(),
                recovery_relative_path: evidence.path().to_owned(),
                recovery_filename: evidence.filename().to_owned(),
                observed: BlobDescription::from_parts(*evidence.digest(), evidence.len()),
            };
            self.publish_forensic_record(intent, &record)?;
        }
        let completion = ProjectionCompletion::for_intent(intent, proof.bytes())?;
        let bytes = completion.encode()?;
        require_evidence_length(
            "projection completion",
            bytes.len() as u64,
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;
        let completions = self.namespace(COMPLETIONS_DIR)?;
        publish_immutable_exact(
            &completions,
            &completion_filename(intent_id),
            &bytes,
            "projection completion",
        )?;
        Ok(completion)
    }

    pub fn load_completion(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<Option<ProjectionCompletion>, ProjectionStoreError> {
        let intent_id = self.require_published_intent(intent)?;
        let completions = self.namespace(COMPLETIONS_DIR)?;
        let Some(bytes) = read_optional_regular(
            &completions,
            &completion_filename(intent_id),
            MAX_PROJECTION_EVIDENCE_BYTES,
            None,
        )?
        else {
            return Ok(None);
        };
        let completion = ProjectionCompletion::decode_bound(&bytes, intent)?;
        if completion.encode()? != bytes {
            return Err(ProjectionStoreError::NonCanonical("projection completion"));
        }
        Ok(Some(completion))
    }

    pub(crate) fn completed_work_authority(
        &self,
        work: &ProjectionWork,
        intent: &ProjectionIntent,
    ) -> Result<ProjectionWorkCompletionAuthority, ProjectionStoreError> {
        let endpoint = self
            .endpoint
            .ok_or(ProjectionStoreError::EndpointBindingMismatch)?;
        self.require_endpoint(endpoint)?;
        let target_matches = match work.target() {
            ProjectionWorkTarget::Absent => intent.target() == BlobDescription::of(&[]),
            ProjectionWorkTarget::Present(target) => intent.target() == target,
        };
        if work.workspace_id() != self.workspace_id
            || work.endpoint_id() != endpoint.endpoint_id
            || work.graph_resource_id() != endpoint.graph_resource_id
            || intent.workspace_id() != work.workspace_id()
            || intent.page_id() != work.page_id()
            || intent.path() != work.path()
            || intent.frontier() != work.post_frontier()
            || !target_matches
        {
            return Err(ProjectionStoreError::EndpointBindingMismatch);
        }
        let completion = self
            .load_completion(intent)?
            .ok_or(ProjectionStoreError::MissingPriorCompletion)?;
        completion.validate_against(intent)?;
        Ok(ProjectionWorkCompletionAuthority::from_durable_completion(
            work,
            self.store_id,
            completion.intent_id(),
            completion.logical_completion_id(),
        ))
    }

    pub fn local_forensic_evidence(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<Vec<LocalProjectionEvidenceRecord>, ProjectionStoreError> {
        let intent_id = self.require_published_intent(intent)?;
        let forensics = self.required_intent_namespace(FORENSICS_DIR, intent_id)?;
        let mut records = Vec::new();
        for entry in forensics.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                ProjectionStoreError::UnsafeEntry("non-UTF-8 forensic evidence entry".into())
            })?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            require_canonical_evidence_name(name, ".evidence")?;
            let bytes =
                read_optional_regular(&forensics, name, MAX_PROJECTION_EVIDENCE_BYTES, None)?
                    .ok_or_else(|| {
                        ProjectionStoreError::UnsafeEntry(format!(
                            "forensic evidence disappeared during enumeration: {name}"
                        ))
                    })?;
            let record: LocalProjectionEvidenceRecord = serde_json::from_slice(&bytes)
                .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if name != format!("{}.evidence", hex(&digest)) {
                return Err(ProjectionStoreError::ForensicBindingMismatch);
            }
            self.validate_forensic_record(intent, &record)?;
            records.push(record);
        }
        records.sort_unstable_by_key(LocalProjectionEvidenceRecord::attempt_id);
        Ok(records)
    }

    /// Enumerate every durably published intent that has no valid completion.
    ///
    /// Both namespaces are validated as a whole. Only exact immutable-publication
    /// temporary names are ignored; malformed names and non-regular entries fail
    /// closed instead of disappearing from recovery.
    pub fn incomplete_intents(&self) -> Result<Vec<ProjectionIntent>, ProjectionStoreError> {
        Ok(self
            .validated_catalog()?
            .into_iter()
            .filter_map(|entry| entry.completion.is_none().then_some(entry.intent))
            .collect())
    }

    /// Validate and load the complete durable intent/completion catalog.
    ///
    /// This is deliberately crate-private: import authority is minted only by
    /// the projection bridge after it also proves enrolled endpoint, accepted
    /// engine frontier, and immutable object readiness.
    pub(crate) fn validated_catalog(
        &self,
    ) -> Result<Vec<ProjectionCatalogEntry>, ProjectionStoreError> {
        let intents_dir = self.namespace(INTENTS_DIR)?;
        let mut intents = BTreeMap::new();
        let mut validated_bases = std::collections::BTreeSet::new();
        let mut catalog_bytes = 0_u64;
        let mut directory_entries = 0_usize;
        for entry in intents_dir.entries()? {
            charge_catalog_directory_entry(
                &mut directory_entries,
                MAX_PROJECTION_CATALOG_DIRECTORY_ENTRIES,
            )?;
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                ProjectionStoreError::UnsafeEntry("non-UTF-8 projection intent entry".into())
            })?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            if intents.len() == MAX_PROJECTION_CATALOG_ROWS {
                return Err(ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection catalog rows",
                    declared: intents.len().saturating_add(1) as u64,
                    limit: MAX_PROJECTION_CATALOG_ROWS as u64,
                });
            }
            require_canonical_evidence_name(name, ".intent")?;
            let bytes =
                read_optional_regular(&intents_dir, name, MAX_PROJECTION_EVIDENCE_BYTES, None)?
                    .ok_or_else(|| {
                        ProjectionStoreError::UnsafeEntry(format!(
                            "projection intent disappeared during enumeration: {name}"
                        ))
                    })?;
            let intent = ProjectionIntent::decode(&bytes)?;
            self.require_workspace(&intent)?;
            if intent.encode()? != bytes || intent_filename(intent.id()?) != name {
                return Err(ProjectionStoreError::PathBindingMismatch(
                    "projection intent",
                ));
            }
            catalog_bytes = catalog_bytes.checked_add(bytes.len() as u64).ok_or(
                ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection catalog",
                    declared: u64::MAX,
                    limit: MAX_PROJECTION_CATALOG_BYTES,
                },
            )?;
            if let ProjectionPrecondition::Base(description) = intent.precondition() {
                if validated_bases.insert(*description) {
                    catalog_bytes = catalog_bytes.checked_add(description.byte_length()).ok_or(
                        ProjectionStoreError::EvidenceTooLarge {
                            kind: "projection catalog",
                            declared: u64::MAX,
                            limit: MAX_PROJECTION_CATALOG_BYTES,
                        },
                    )?;
                    self.load_base(&intent)?;
                }
            }
            if catalog_bytes > MAX_PROJECTION_CATALOG_BYTES {
                return Err(ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection catalog",
                    declared: catalog_bytes,
                    limit: MAX_PROJECTION_CATALOG_BYTES,
                });
            }
            intents.insert(completion_filename(intent.id()?), intent);
        }

        let completions_dir = self.namespace(COMPLETIONS_DIR)?;
        let mut completed = BTreeMap::new();
        for entry in completions_dir.entries()? {
            charge_catalog_directory_entry(
                &mut directory_entries,
                MAX_PROJECTION_CATALOG_DIRECTORY_ENTRIES,
            )?;
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                ProjectionStoreError::UnsafeEntry("non-UTF-8 projection completion entry".into())
            })?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            if completed.len() == MAX_PROJECTION_CATALOG_ROWS {
                return Err(ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection completion rows",
                    declared: completed.len().saturating_add(1) as u64,
                    limit: MAX_PROJECTION_CATALOG_ROWS as u64,
                });
            }
            require_canonical_evidence_name(name, ".completion")?;
            let intent = intents
                .get(name)
                .ok_or_else(|| ProjectionStoreError::OrphanCompletion(name.into()))?;
            let bytes =
                read_optional_regular(&completions_dir, name, MAX_PROJECTION_EVIDENCE_BYTES, None)?
                    .ok_or_else(|| {
                        ProjectionStoreError::UnsafeEntry(format!(
                            "projection completion disappeared during enumeration: {name}"
                        ))
                    })?;
            let completion = ProjectionCompletion::decode_bound(&bytes, intent)?;
            if completion.encode()? != bytes {
                return Err(ProjectionStoreError::NonCanonical("projection completion"));
            }
            catalog_bytes = catalog_bytes.checked_add(bytes.len() as u64).ok_or(
                ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection catalog",
                    declared: u64::MAX,
                    limit: MAX_PROJECTION_CATALOG_BYTES,
                },
            )?;
            if catalog_bytes > MAX_PROJECTION_CATALOG_BYTES {
                return Err(ProjectionStoreError::EvidenceTooLarge {
                    kind: "projection catalog",
                    declared: catalog_bytes,
                    limit: MAX_PROJECTION_CATALOG_BYTES,
                });
            }
            completed.insert(name.to_owned(), completion);
        }

        Ok(intents
            .into_iter()
            .map(|(completion_name, intent)| ProjectionCatalogEntry {
                completion: completed.remove(&completion_name),
                intent,
            })
            .collect())
    }

    /// Reconstruct completion only from an authorized replay and Graph's fresh
    /// capability-bound durable-target proof.
    pub(crate) fn reconstruct_completion(
        &self,
        intent: &ProjectionIntent,
        replayed_target: &[u8],
        proof: &ProjectionWriteProof,
    ) -> Result<ProjectionCompletion, ProjectionStoreError> {
        if BlobDescription::of(replayed_target) != intent.target() {
            return Err(ProjectionStoreError::RecoveryTargetMismatch);
        }
        self.require_write_proof(intent, proof)?;
        self.publish_completion(intent, proof)
    }

    fn initialize(
        capability: &Dir,
        store_id: ProjectionReceiptStoreId,
        workspace_id: WorkspaceId,
        endpoint: Option<ProjectionEndpointBinding>,
    ) -> Result<ReceiptNamespaces, ProjectionStoreError> {
        let existing = read_optional_regular(capability, STORE_CLAIM_FILE, 512, None)?;
        if let Some(bytes) = existing {
            let expected = validate_claim(&bytes, store_id, workspace_id, endpoint)?;
            let namespaces = open_receipt_namespaces(capability)?;
            if namespaces.identities() != expected {
                return Err(ProjectionStoreError::NamespaceSubstitution(
                    "top-level receipt namespace".into(),
                ));
            }
            return Ok(namespaces);
        }

        let expected_init = init_claim_bytes(store_id, workspace_id, endpoint);
        match read_optional_regular(capability, STORE_INIT_FILE, 256, None)? {
            Some(bytes) => {
                if bytes != expected_init {
                    return Err(ProjectionStoreError::MalformedStoreClaim);
                }
            }
            None => {
                if capability.entries()?.next().transpose()?.is_some() {
                    return Err(ProjectionStoreError::ClaimlessNonemptyStore);
                }
                publish_immutable_exact(
                    capability,
                    STORE_INIT_FILE,
                    &expected_init,
                    "projection receipt store initialization claim",
                )?;
            }
        }

                for namespace in [
                    BASES_DIR,
                    INTENTS_DIR,
                    COMPLETIONS_DIR,
                    ATTEMPTS_DIR,
                    FORENSICS_DIR,
                ] {
            ensure_directory_nofollow(capability, namespace)?;
            }
        require_incomplete_store_is_empty(capability)?;
        let namespaces = open_receipt_namespaces(capability)?;
        let claim = claim_bytes(store_id, workspace_id, endpoint, &namespaces.identities());
        publish_immutable_exact(
            capability,
            STORE_CLAIM_FILE,
            &claim,
            "projection receipt store claim",
        )?;
        Ok(namespaces)
    }

    fn namespace(&self, name: &str) -> Result<Dir, ProjectionStoreError> {
        let retained = self.namespaces.get(name).ok_or_else(|| {
            ProjectionStoreError::UnsafeEntry(format!("unknown receipt namespace {name}"))
        })?;
        let live = open_dir_nofollow(&self.capability, name).map_err(|error| {
            ProjectionStoreError::NamespaceSubstitution(format!("{name}: {error}"))
        })?;
        if canonical_directory_identity(&live)? != retained.identity {
            return Err(ProjectionStoreError::NamespaceSubstitution(name.into()));
        }
        retained.capability.try_clone().map_err(Into::into)
    }

    fn intent_namespace(
        &self,
        namespace: &str,
        intent_id: ProjectionIntentId,
    ) -> Result<Dir, ProjectionStoreError> {
        self.open_intent_namespace(namespace, intent_id, true)?
            .ok_or_else(|| {
                ProjectionStoreError::NamespaceSubstitution(format!(
                    "{namespace}/{}",
                    hex(intent_id.as_bytes())
                ))
            })
    }

    fn existing_intent_namespace(
        &self,
        namespace: &str,
        intent_id: ProjectionIntentId,
    ) -> Result<Option<Dir>, ProjectionStoreError> {
        self.open_intent_namespace(namespace, intent_id, false)
    }

    fn required_intent_namespace(
        &self,
        namespace: &str,
        intent_id: ProjectionIntentId,
    ) -> Result<Dir, ProjectionStoreError> {
        self.existing_intent_namespace(namespace, intent_id)?
            .ok_or_else(|| {
                ProjectionStoreError::NamespaceSubstitution(format!(
                    "missing established {namespace}/{}",
                    hex(intent_id.as_bytes())
                ))
            })
    }

    fn open_intent_namespace(
        &self,
        namespace: &str,
        intent_id: ProjectionIntentId,
        create: bool,
    ) -> Result<Option<Dir>, ProjectionStoreError> {
        let parent = self.namespace(namespace)?;
        let name = hex(intent_id.as_bytes());
        let reservation_name = format!("{name}{INTENT_NAMESPACE_RESERVATION_SUFFIX}");
        let authority_name = format!("{name}{INTENT_NAMESPACE_AUTHORITY_SUFFIX}");
        let expected_reservation = IntentNamespaceReservation {
            schema_version: INTENT_NAMESPACE_SCHEMA_VERSION,
            store_id: self.store_id,
            namespace: namespace.to_owned(),
            intent_id,
        };
        let reservation_bytes = serde_json::to_vec(&expected_reservation)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;

        if let Some(bytes) = read_optional_regular(&parent, &authority_name, 1024, None)? {
            let authority: IntentNamespaceAuthority = serde_json::from_slice(&bytes)
                .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
            if serde_json::to_vec(&authority)
                .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?
                != bytes
                || authority.schema_version != INTENT_NAMESPACE_SCHEMA_VERSION
                || authority.store_id != self.store_id
                || authority.namespace != namespace
                || authority.intent_id != intent_id
            {
                return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                    "{namespace}/{name}"
                )));
            }
            let directory = open_dir_nofollow(&parent, &name).map_err(|error| {
                ProjectionStoreError::NamespaceSubstitution(format!("{namespace}/{name}: {error}"))
            })?;
            if canonical_directory_identity(&directory)? != authority.directory_identity {
                return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                    "{namespace}/{name}"
                )));
            }
            return Ok(Some(directory));
        }

        match read_optional_regular(&parent, &reservation_name, 1024, None)? {
            Some(bytes) => {
                if bytes != reservation_bytes {
                    return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                        "{namespace}/{name}"
                    )));
                }
                if !create {
                    return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                        "incomplete established {namespace}/{name}"
                    )));
                }
            }
            None => {
                match parent.symlink_metadata(&name) {
                    Ok(_) => {
                        return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                            "unbound {namespace}/{name}"
                        )));
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                if !create {
                    return Ok(None);
                }
                publish_immutable_exact(
                    &parent,
                    &reservation_name,
                    &reservation_bytes,
                    "per-intent namespace reservation",
                )?;
            }
        }

        ensure_directory_nofollow(&parent, &name)?;
        let directory = open_dir_nofollow(&parent, &name)?;
        if directory.entries()?.next().transpose()?.is_some() {
            return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                "unbound nonempty {namespace}/{name}"
            )));
        }
        let authority = IntentNamespaceAuthority {
            schema_version: INTENT_NAMESPACE_SCHEMA_VERSION,
            store_id: self.store_id,
            namespace: namespace.to_owned(),
            intent_id,
            directory_identity: canonical_directory_identity(&directory)?,
        };
        let authority_bytes = serde_json::to_vec(&authority)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
        publish_immutable_exact(
            &parent,
            &authority_name,
            &authority_bytes,
            "per-intent namespace authority",
        )?;
        let live = open_dir_nofollow(&parent, &name).map_err(|error| {
            ProjectionStoreError::NamespaceSubstitution(format!("{namespace}/{name}: {error}"))
        })?;
        if canonical_directory_identity(&live)? != authority.directory_identity {
            return Err(ProjectionStoreError::NamespaceSubstitution(format!(
                "{namespace}/{name}"
            )));
        }
        Ok(Some(directory))
    }

    fn publish_forensic_record(
        &self,
        intent: &ProjectionIntent,
        record: &LocalProjectionEvidenceRecord,
    ) -> Result<(), ProjectionStoreError> {
        self.validate_forensic_record(intent, record)?;
        let bytes = serde_json::to_vec(record)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let forensics = self.required_intent_namespace(FORENSICS_DIR, record.intent_id)?;
        publish_immutable_exact(
            &forensics,
            &format!("{}.evidence", hex(&digest)),
            &bytes,
            "local projection forensic evidence",
        )?;
        Ok(())
    }

    fn validate_forensic_record(
        &self,
        intent: &ProjectionIntent,
        record: &LocalProjectionEvidenceRecord,
    ) -> Result<(), ProjectionStoreError> {
        if record.schema_version != LOCAL_FORENSIC_SCHEMA_VERSION
            || record.intent_id != intent.id()?
            || record.target_path != *intent.path()
        {
            return Err(ProjectionStoreError::ForensicBindingMismatch);
        }
        require_evidence_length(
            "local projection forensic evidence",
            record.observed.byte_length(),
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;
        let reservation = self
            .load_attempt_reservations(intent)?
            .into_iter()
            .find(|reservation| reservation.attempt_id() == record.attempt_id)
            .ok_or(ProjectionStoreError::ForensicBindingMismatch)?;
        let parent = intent
            .path()
            .as_str()
            .rsplit_once('/')
            .expect("managed paths contain a parent")
            .0;
        if reservation.recovery_filename() != record.recovery_filename
            || record.recovery_relative_path != format!("{parent}/{}", record.recovery_filename)
        {
            return Err(ProjectionStoreError::ForensicBindingMismatch);
        }
        Ok(())
    }

    fn require_workspace(&self, intent: &ProjectionIntent) -> Result<(), ProjectionStoreError> {
        if intent.workspace_id() != self.workspace_id {
            return Err(ProjectionStoreError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: intent.workspace_id(),
            });
        }
        Ok(())
    }

    pub(crate) fn require_endpoint(
        &self,
        endpoint: ProjectionEndpointBinding,
    ) -> Result<(), ProjectionStoreError> {
        if self.endpoint != Some(endpoint) {
            return Err(ProjectionStoreError::EndpointBindingMismatch);
        }
        Ok(())
    }

    fn require_write_proof(
        &self,
        intent: &ProjectionIntent,
        proof: &ProjectionWriteProof,
    ) -> Result<(), ProjectionStoreError> {
        if proof.path() != intent.path().as_str()
            || proof.digest() != intent.target().sha256()
            || BlobDescription::of(proof.bytes()) != intent.target()
        {
            return Err(ProjectionStoreError::WriteProofMismatch);
        }
        Ok(())
    }

    fn require_published_intent(
        &self,
        intent: &ProjectionIntent,
    ) -> Result<ProjectionIntentId, ProjectionStoreError> {
        self.require_workspace(intent)?;
        let intent_id = intent.id()?;
        let stored = self
            .load_intent(intent_id)?
            .ok_or(ProjectionStoreError::MissingIntent(intent_id))?;
        if stored != *intent {
            return Err(ProjectionStoreError::IntentCollision(intent_id));
        }
        Ok(intent_id)
    }
}

#[derive(Debug)]
pub enum ProjectionStoreError {
    Io(std::io::Error),
    Store(Box<StoreError>),
    Receipt(ReceiptError),
    UnsafeEntry(String),
    UnknownStoreVersion(u32),
    UpgradeRequired {
        found: u32,
        current: u32,
    },
    MalformedStoreClaim,
    ClaimlessNonemptyStore,
    NamespaceSubstitution(String),
    EndpointBindingMismatch,
    GraphResourceMismatch,
    CapturedInputMismatch,
    MissingPriorCompletion,
    WorkspaceMismatch {
        expected: WorkspaceId,
        found: WorkspaceId,
    },
    MissingBase(BlobDescription),
    UnexpectedBase,
    MissingIntent(ProjectionIntentId),
    BaseEvidenceMismatch(BlobDescription),
    PathBindingMismatch(&'static str),
    NonCanonical(&'static str),
    IntentCollision(ProjectionIntentId),
    EvidenceTooLarge {
        kind: &'static str,
        declared: u64,
        limit: u64,
    },
    MalformedEvidenceName(String),
    OrphanCompletion(String),
    WriteProofMismatch,
    RecoveryTargetMismatch,
    AttemptBindingMismatch,
    ForensicBindingMismatch,
    UnreservedRecoveryEvidence,
    Decode(String),
    Encode(String),
}

impl fmt::Display for ProjectionStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Store(error) => error.fmt(f),
            Self::Receipt(error) => error.fmt(f),
            Self::UnsafeEntry(message) => write!(f, "unsafe projection store entry: {message}"),
            Self::UnknownStoreVersion(version) => {
                write!(f, "unknown projection store version {version}")
            }
            Self::UpgradeRequired { found, current } => write!(
                f,
                "projection receipt store version {found} requires upgrade to {current}"
            ),
            Self::MalformedStoreClaim => f.write_str("malformed projection store claim"),
            Self::ClaimlessNonemptyStore => {
                f.write_str("claimless nonempty projection receipt store cannot be initialized")
            }
            Self::NamespaceSubstitution(namespace) => {
                write!(
                    f,
                    "projection receipt namespace no longer denotes retained resource: {namespace}"
                )
            }
            Self::EndpointBindingMismatch => {
                f.write_str("projection receipt store endpoint enrollment mismatch")
            }
            Self::GraphResourceMismatch => {
                f.write_str("projection graph capability does not match endpoint enrollment")
            }
            Self::CapturedInputMismatch => {
                f.write_str("capability-captured projection input does not match its completion")
            }
            Self::MissingPriorCompletion => {
                f.write_str("present projection input has no durable prior completion")
            }
            Self::WorkspaceMismatch { expected, found } => {
                write!(f, "workspace mismatch: expected {expected}, found {found}")
            }
            Self::MissingBase(description) => {
                write!(f, "missing immutable projection base {description:?}")
            }
            Self::UnexpectedBase => {
                f.write_str("base bytes were supplied for an absent projection precondition")
            }
            Self::MissingIntent(intent_id) => {
                write!(f, "missing immutable projection intent {intent_id}")
            }
            Self::BaseEvidenceMismatch(description) => {
                write!(f, "projection base evidence mismatch for {description:?}")
            }
            Self::PathBindingMismatch(kind) => {
                write!(f, "{kind} bytes do not match their canonical path")
            }
            Self::NonCanonical(kind) => write!(f, "{kind} bytes are not canonical"),
            Self::IntentCollision(intent_id) => {
                write!(f, "stored projection intent differs at {intent_id}")
            }
            Self::EvidenceTooLarge {
                kind,
                declared,
                limit,
            } => {
                write!(
                    f,
                    "{kind} declares {declared} bytes, exceeding reload limit {limit}"
                )
            }
            Self::MalformedEvidenceName(name) => {
                write!(f, "malformed projection evidence name: {name}")
            }
            Self::OrphanCompletion(name) => {
                write!(f, "projection completion has no matching intent: {name}")
            }
            Self::WriteProofMismatch => {
                f.write_str("Graph write proof does not match the exact projection intent")
            }
            Self::RecoveryTargetMismatch => {
                f.write_str("current bytes do not equal the exact replayed projection target")
            }
            Self::AttemptBindingMismatch => {
                f.write_str("local projection attempt is not canonically bound to its intent")
            }
            Self::ForensicBindingMismatch => {
                f.write_str("local projection forensic evidence is not bound to its attempt")
            }
            Self::UnreservedRecoveryEvidence => {
                f.write_str("Graph returned recovery evidence without a durable reservation")
            }
            Self::Decode(error) => write!(f, "local projection evidence decode failed: {error}"),
            Self::Encode(error) => write!(f, "local projection evidence encode failed: {error}"),
        }
    }
}

impl std::error::Error for ProjectionStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Receipt(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ProjectionStoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<StoreError> for ProjectionStoreError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Io(error) if error.kind() == ErrorKind::InvalidData => Self::Io(error),
            other => Self::Store(Box::new(other)),
        }
    }
}

impl From<ReceiptError> for ProjectionStoreError {
    fn from(error: ReceiptError) -> Self {
        Self::Receipt(error)
    }
}

fn claim_bytes(
    store_id: ProjectionReceiptStoreId,
    workspace_id: WorkspaceId,
    endpoint: Option<ProjectionEndpointBinding>,
    namespace_identities: &[DirectoryIdentity; 5],
) -> Vec<u8> {
    let mut bytes = enrollment_claim_bytes(STORE_CLAIM_MAGIC, store_id, workspace_id, endpoint);
    bytes.reserve(5 * 32);
    for identity in namespace_identities {
        bytes.extend_from_slice(identity);
    }
    debug_assert_eq!(bytes.len(), STORE_CLAIM_LEN);
    bytes
}

fn init_claim_bytes(
    store_id: ProjectionReceiptStoreId,
    workspace_id: WorkspaceId,
    endpoint: Option<ProjectionEndpointBinding>,
) -> Vec<u8> {
    let bytes = enrollment_claim_bytes(STORE_INIT_MAGIC, store_id, workspace_id, endpoint);
    debug_assert_eq!(bytes.len(), STORE_INIT_LEN);
    bytes
}

fn enrollment_claim_bytes(
    magic: &[u8; 8],
    store_id: ProjectionReceiptStoreId,
    workspace_id: WorkspaceId,
    endpoint: Option<ProjectionEndpointBinding>,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(STORE_CLAIM_BASE_LEN);
    bytes.extend_from_slice(magic);
    bytes.extend_from_slice(&STORE_CLAIM_VERSION.to_be_bytes());
    bytes.extend_from_slice(store_id.as_bytes());
    bytes.extend_from_slice(workspace_id.as_uuid().as_bytes());
    match endpoint {
        Some(endpoint) => {
            bytes.push(1);
            bytes.extend_from_slice(endpoint.endpoint_id.as_uuid().as_bytes());
            bytes.extend_from_slice(endpoint.device_id.as_uuid().as_bytes());
            bytes.extend_from_slice(endpoint.graph_resource_id.as_bytes());
        }
        None => {
            bytes.push(0);
            bytes.extend_from_slice(&[0_u8; 16]);
            bytes.extend_from_slice(&[0_u8; 16]);
            bytes.extend_from_slice(&[0_u8; 32]);
        }
    }
    bytes
}

fn charge_catalog_directory_entry(
    count: &mut usize,
    limit: usize,
) -> Result<(), ProjectionStoreError> {
    *count = count.saturating_add(1);
    if *count > limit {
        Err(ProjectionStoreError::EvidenceTooLarge {
            kind: "projection catalog directory entries",
            declared: *count as u64,
            limit: limit as u64,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod catalog_limit_tests {
    use super::*;

    #[test]
    fn catalog_directory_budget_counts_temp_entries_before_loading() {
        let mut count = 0;
        charge_catalog_directory_entry(&mut count, 2).unwrap();
        charge_catalog_directory_entry(&mut count, 2).unwrap();
        assert!(matches!(
            charge_catalog_directory_entry(&mut count, 2),
            Err(ProjectionStoreError::EvidenceTooLarge {
                kind: "projection catalog directory entries",
                declared: 3,
                limit: 2
            })
        ));
    }
}

fn validate_claim(
    bytes: &[u8],
    expected_store_id: ProjectionReceiptStoreId,
    expected_workspace: WorkspaceId,
    expected_endpoint: Option<ProjectionEndpointBinding>,
) -> Result<[DirectoryIdentity; 5], ProjectionStoreError> {
    for magic in PRIOR_STORE_CLAIM_MAGICS {
        if bytes.len() >= magic.len() + 4 && &bytes[..magic.len()] == magic {
        let version = u32::from_be_bytes(
                bytes[magic.len()..magic.len() + 4]
                .try_into()
                .expect("prior claim version slice"),
        );
        return Err(ProjectionStoreError::UpgradeRequired {
            found: version,
            current: STORE_CLAIM_VERSION,
        });
    }
    }
    if bytes.len() < STORE_CLAIM_MAGIC.len() + 4
        || &bytes[..STORE_CLAIM_MAGIC.len()] != STORE_CLAIM_MAGIC
    {
        return Err(ProjectionStoreError::MalformedStoreClaim);
    }
    let version = u32::from_be_bytes(
        bytes[STORE_CLAIM_MAGIC.len()..STORE_CLAIM_MAGIC.len() + 4]
            .try_into()
            .expect("claim version slice"),
    );
    if version < STORE_CLAIM_VERSION {
        return Err(ProjectionStoreError::UpgradeRequired {
            found: version,
            current: STORE_CLAIM_VERSION,
        });
    }
    if version > STORE_CLAIM_VERSION {
        return Err(ProjectionStoreError::UnknownStoreVersion(version));
    }
    if bytes.len() != STORE_CLAIM_LEN {
        return Err(ProjectionStoreError::MalformedStoreClaim);
    }
    let store_offset = STORE_CLAIM_MAGIC.len() + 4;
    if bytes[store_offset..store_offset + 32] != *expected_store_id.as_bytes() {
        return Err(ProjectionStoreError::EndpointBindingMismatch);
    }
    let workspace_offset = store_offset + 32;
    let workspace = WorkspaceId::from_uuid(
        Uuid::from_slice(&bytes[workspace_offset..workspace_offset + 16])
            .map_err(|_| ProjectionStoreError::MalformedStoreClaim)?,
    );
    if workspace != expected_workspace {
        return Err(ProjectionStoreError::WorkspaceMismatch {
            expected: expected_workspace,
            found: workspace,
        });
    }
    let mut identities = [[0_u8; 32]; 5];
    for (index, identity) in identities.iter_mut().enumerate() {
        let offset = STORE_CLAIM_BASE_LEN + index * 32;
        identity.copy_from_slice(&bytes[offset..offset + 32]);
    }
    if bytes
        != claim_bytes(
            expected_store_id,
            expected_workspace,
            expected_endpoint,
            &identities,
        )
    {
        return Err(ProjectionStoreError::EndpointBindingMismatch);
    }
    Ok(identities)
}

fn open_receipt_namespaces(capability: &Dir) -> Result<ReceiptNamespaces, ProjectionStoreError> {
    Ok(ReceiptNamespaces {
        bases: open_bound_namespace(capability, BASES_DIR)?,
        intents: open_bound_namespace(capability, INTENTS_DIR)?,
        completions: open_bound_namespace(capability, COMPLETIONS_DIR)?,
        attempts: open_bound_namespace(capability, ATTEMPTS_DIR)?,
        forensics: open_bound_namespace(capability, FORENSICS_DIR)?,
    })
}

fn open_bound_namespace(
    capability: &Dir,
    name: &str,
) -> Result<BoundNamespace, ProjectionStoreError> {
    let directory = open_dir_nofollow(capability, name)
        .map_err(|error| ProjectionStoreError::NamespaceSubstitution(format!("{name}: {error}")))?;
    Ok(BoundNamespace {
        identity: canonical_directory_identity(&directory)?,
        capability: directory,
    })
}

fn require_incomplete_store_is_empty(capability: &Dir) -> Result<(), ProjectionStoreError> {
    for entry in capability.entries()? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            ProjectionStoreError::UnsafeEntry("non-UTF-8 entry in incomplete receipt store".into())
        })?;
        if name == STORE_INIT_FILE {
            require_regular_entry(&entry.file_type()?, name)?;
            continue;
        }
        if ![
            BASES_DIR,
            INTENTS_DIR,
            COMPLETIONS_DIR,
            ATTEMPTS_DIR,
            FORENSICS_DIR,
        ]
        .contains(&name)
            || !entry.file_type()?.is_dir()
        {
            return Err(ProjectionStoreError::ClaimlessNonemptyStore);
        }
        let directory = open_dir_nofollow(capability, name)?;
        if directory.entries()?.next().transpose()?.is_some() {
            return Err(ProjectionStoreError::ClaimlessNonemptyStore);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn canonical_receipt_store_id(dir: &Dir) -> Result<ProjectionReceiptStoreId, ProjectionStoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = dir.try_clone()?.into_std_file().metadata()?;
    let mut identity = [0_u8; 16];
    identity[..8].copy_from_slice(&metadata.dev().to_be_bytes());
    identity[8..].copy_from_slice(&metadata.ino().to_be_bytes());
    Ok(ProjectionReceiptStoreId::from_capability_identity(
        b"unix-dev-inode",
        &identity,
    ))
}

#[cfg(windows)]
fn canonical_receipt_store_id(dir: &Dir) -> Result<ProjectionReceiptStoreId, ProjectionStoreError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
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
        return Err(ProjectionStoreError::Io(std::io::Error::last_os_error()));
    }
    let mut identity = [0_u8; 24];
    identity[..8].copy_from_slice(&information.VolumeSerialNumber.to_be_bytes());
    identity[8..].copy_from_slice(&information.FileId.Identifier);
    Ok(ProjectionReceiptStoreId::from_capability_identity(
        b"windows-volume-file-id",
        &identity,
    ))
}

#[cfg(not(any(unix, windows)))]
fn canonical_receipt_store_id(
    _dir: &Dir,
) -> Result<ProjectionReceiptStoreId, ProjectionStoreError> {
    Err(ProjectionStoreError::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "projection receipt-store identity is unsupported on this platform",
    )))
}

#[cfg(unix)]
fn canonical_directory_identity(dir: &Dir) -> Result<DirectoryIdentity, ProjectionStoreError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = dir.try_clone()?.into_std_file().metadata()?;
    let mut hasher = Sha256::new();
    hasher.update(b"tine/projection-directory-identity/unix-v1\0");
    hasher.update(metadata.dev().to_be_bytes());
    hasher.update(metadata.ino().to_be_bytes());
    Ok(hasher.finalize().into())
}

#[cfg(windows)]
fn canonical_directory_identity(dir: &Dir) -> Result<DirectoryIdentity, ProjectionStoreError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
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
        return Err(ProjectionStoreError::Io(std::io::Error::last_os_error()));
    }
    let mut hasher = Sha256::new();
    hasher.update(b"tine/projection-directory-identity/windows-v1\0");
    hasher.update(information.VolumeSerialNumber.to_be_bytes());
    hasher.update(information.FileId.Identifier);
    Ok(hasher.finalize().into())
}

#[cfg(not(any(unix, windows)))]
fn canonical_directory_identity(_dir: &Dir) -> Result<DirectoryIdentity, ProjectionStoreError> {
    Err(ProjectionStoreError::Io(std::io::Error::new(
        ErrorKind::Unsupported,
        "projection directory identity is unsupported on this platform",
    )))
}

fn base_filename(description: BlobDescription) -> String {
    format!("{}.base", hex(description.sha256()))
}

fn intent_filename(intent_id: ProjectionIntentId) -> String {
    format!("{}.intent", hex(intent_id.as_bytes()))
}

fn completion_filename(intent_id: ProjectionIntentId) -> String {
    format!("{}.completion", hex(intent_id.as_bytes()))
}

fn attempt_filename(attempt_id: Uuid) -> String {
    format!("{}.attempt", attempt_id.simple())
}

fn parse_attempt_filename(name: &str) -> Result<Uuid, ProjectionStoreError> {
    let value = name
        .strip_suffix(".attempt")
        .ok_or_else(|| ProjectionStoreError::MalformedEvidenceName(name.into()))?;
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProjectionStoreError::MalformedEvidenceName(name.into()));
    }
    Uuid::parse_str(value).map_err(|_| ProjectionStoreError::MalformedEvidenceName(name.into()))
}

fn require_evidence_length(
    kind: &'static str,
    declared: u64,
    limit: u64,
) -> Result<(), ProjectionStoreError> {
    if declared > limit {
        return Err(ProjectionStoreError::EvidenceTooLarge {
            kind,
            declared,
            limit,
        });
    }
    Ok(())
}

fn require_canonical_evidence_name(
    name: &str,
    suffix: &'static str,
) -> Result<(), ProjectionStoreError> {
    let Some(digest) = name.strip_suffix(suffix) else {
        return Err(ProjectionStoreError::MalformedEvidenceName(name.into()));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProjectionStoreError::MalformedEvidenceName(name.into()));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_published_evidence_class_obeys_the_reload_limit() {
        for kind in [
            "projection base",
            "projection target",
            "projection intent",
            "projection completion",
        ] {
            assert!(
                require_evidence_length(
                kind,
                MAX_PROJECTION_EVIDENCE_BYTES,
                MAX_PROJECTION_EVIDENCE_BYTES
            )
                .is_ok()
            );
            assert!(matches!(
                require_evidence_length(
                    kind,
                    MAX_PROJECTION_EVIDENCE_BYTES + 1,
                    MAX_PROJECTION_EVIDENCE_BYTES
                ),
                Err(ProjectionStoreError::EvidenceTooLarge {
                    kind: found,
                    declared,
                    limit,
                }) if found == kind
                    && declared == MAX_PROJECTION_EVIDENCE_BYTES + 1
                    && limit == MAX_PROJECTION_EVIDENCE_BYTES
            ));
        }
    }
}
