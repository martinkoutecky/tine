use std::collections::BTreeMap;
#[cfg(unix)]
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind};
#[cfg(unix)]
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
#[cfg(windows)]
use cap_std::fs::OpenOptions as CapOpenOptions;
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::object_store::{
    StoreError, ensure_directory_nofollow, is_temp_name, open_dir_nofollow,
    publish_immutable_exact, read_optional_regular, require_regular_entry, sync_dir_required,
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
const MUTATION_AUTHORITY_SCHEMA_VERSION: u32 = 1;
const INTENT_NAMESPACE_RESERVATION_SUFFIX: &str = ".namespace-reservation";
const INTENT_NAMESPACE_AUTHORITY_SUFFIX: &str = ".namespace-authority";
const MUTATION_AUTHORITY_SUFFIX: &str = ".mutation-authority";
const MUTATION_AUTHORITY_LEASE_SUFFIX: &str = ".mutation-authority.lock";
const MAX_MUTATION_ATTEMPTS: usize = 1_000_000;
const MAX_MUTATION_AUTHORITY_BYTES: usize = 64 * 1024 * 1024;

type DirectoryIdentity = [u8; 32];

#[cfg(test)]
thread_local! {
    static MUTATION_AUTHORITY_CAPTURED_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static MUTATION_AUTHORITY_ACT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static MUTATION_AUTHORITY_LEASED_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static MUTATION_AUTHORITY_DROP_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static COMPLETION_PUBLICATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
    static COMPLETION_PUBLICATION_ACT_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn mutation_authority_captured_hook() {
    MUTATION_AUTHORITY_CAPTURED_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn mutation_authority_captured_hook() {}

#[cfg(test)]
fn mutation_authority_act_hook() {
    MUTATION_AUTHORITY_ACT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn mutation_authority_act_hook() {}

#[cfg(test)]
fn mutation_authority_leased_hook() {
    MUTATION_AUTHORITY_LEASED_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn mutation_authority_leased_hook() {}

#[cfg(test)]
fn mutation_authority_drop_hook() {
    MUTATION_AUTHORITY_DROP_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn mutation_authority_drop_hook() {}

#[cfg(test)]
fn completion_publication_hook() {
    COMPLETION_PUBLICATION_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn completion_publication_hook() {}

#[cfg(test)]
fn completion_publication_act_hook() {
    COMPLETION_PUBLICATION_ACT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn completion_publication_act_hook() {}

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
struct DurableProjectionMutationAuthority {
    schema_version: u32,
    authority_id: Uuid,
    store_id: ProjectionReceiptStoreId,
    store_claim_digest: [u8; 32],
    workspace_id: WorkspaceId,
    endpoint_binding: Option<Vec<u8>>,
    intent_id: ProjectionIntentId,
    intent_digest: [u8; 32],
    base: Option<BlobDescription>,
    namespace_identities: [DirectoryIdentity; 5],
    attempts_identity: DirectoryIdentity,
    forensics_identity: DirectoryIdentity,
    active_attempt_id: Option<Uuid>,
    reservation_bytes: Vec<Vec<u8>>,
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

/// Private one-shot authority spanning one exact graph operation and its
/// completion publication. The durable record at the receipt-store root is a
/// recovery-stable witness even if a validated child namespace is moved after
/// the graph operation starts.
pub(crate) struct ProjectionMutationAuthority {
    durable: DurableProjectionMutationAuthority,
    durable_bytes: Vec<u8>,
    durable_name: String,
    _lease: File,
    root: Dir,
    bases: Dir,
    intents: Dir,
    attempts_parent: Dir,
    attempts: Dir,
    forensics_parent: Dir,
    forensics: Dir,
    completions: Dir,
    reservations: Vec<ProjectionAttemptReservation>,
    active: Option<ProjectionAttemptReservation>,
    created_durable_record: bool,
    graph_operation_consumed: bool,
    completion_published: bool,
}

impl fmt::Debug for ProjectionMutationAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectionMutationAuthority")
            .field("store_id", &self.durable.store_id)
            .field("intent_id", &self.durable.intent_id)
            .field("authority_id", &self.durable.authority_id)
            .field("graph_operation_consumed", &self.graph_operation_consumed)
            .finish_non_exhaustive()
    }
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
        let _lease = self.acquire_mutation_lease(intent_id)?;
        mutation_authority_leased_hook();
        self.reserve_attempt_under_lease(intent, intent_id)
    }

    fn reserve_attempt_under_lease(
        &self,
        intent: &ProjectionIntent,
        intent_id: ProjectionIntentId,
    ) -> Result<ProjectionAttemptReservation, ProjectionStoreError> {
        let durable_name = mutation_authority_filename(intent_id);
        if read_optional_regular(
            &self.capability,
            &durable_name,
            MAX_MUTATION_AUTHORITY_BYTES as u64,
            None,
        )?
        .is_some()
        {
            return Err(ProjectionStoreError::MutationAuthorityPending);
        }
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
        self.load_attempt_reservations_from(intent, &attempts)
    }

    fn load_attempt_reservations_from(
        &self,
        intent: &ProjectionIntent,
        attempts: &Dir,
    ) -> Result<Vec<ProjectionAttemptReservation>, ProjectionStoreError> {
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
            if reservations.len() == MAX_MUTATION_ATTEMPTS {
                return Err(ProjectionStoreError::MutationAuthorityTooLarge {
                    attempts: reservations.len() + 1,
                    bytes: 0,
                });
            }
            reservations.push(reservation);
        }
        reservations.sort_unstable_by_key(ProjectionAttemptReservation::attempt_id);
        Ok(reservations)
    }

    /// Seal the exact receipt capabilities and canonical attempt bytes that one
    /// graph operation may consume. The root-level immutable record is written
    /// before this authority can cross into Graph.
    pub(crate) fn begin_mutation(
        &self,
        intent: &ProjectionIntent,
        active: Option<&ProjectionAttemptReservation>,
    ) -> Result<ProjectionMutationAuthority, ProjectionStoreError> {
        let intent_id = self.require_published_intent(intent)?;
        let lease = self.acquire_mutation_lease(intent_id)?;
        mutation_authority_leased_hook();
        let durable_name = mutation_authority_filename(intent_id);
        let existing_durable_bytes = read_optional_regular(
            &self.capability,
            &durable_name,
            MAX_MUTATION_AUTHORITY_BYTES as u64,
            None,
        )?;
        // A newly established recovery slot carries one fresh current attempt
        // in addition to all retained evidence. If proof-only recovery finds
        // that the target was retired before publication, the same immutable
        // slot can authorize the guarded writer without appending on retries.
        let recovery_active = if existing_durable_bytes.is_none() && active.is_none() {
            Some(self.reserve_attempt_under_lease(intent, intent_id)?)
        } else {
            None
        };
        let requested_active = active.cloned().or(recovery_active);
        let store_claim = read_optional_regular(
            &self.capability,
            STORE_CLAIM_FILE,
            STORE_CLAIM_LEN as u64,
            Some(STORE_CLAIM_LEN as u64),
        )?
        .ok_or(ProjectionStoreError::MalformedStoreClaim)?;
        let bases = self.namespace(BASES_DIR)?;
        let intents = self.namespace(INTENTS_DIR)?;
        let attempts_parent = self.namespace(ATTEMPTS_DIR)?;
        let attempts = self.required_intent_namespace(ATTEMPTS_DIR, intent_id)?;
        let forensics_parent = self.namespace(FORENSICS_DIR)?;
        let forensics = self.required_intent_namespace(FORENSICS_DIR, intent_id)?;
        let completions = self.namespace(COMPLETIONS_DIR)?;
        let reservations = self.load_attempt_reservations_from(intent, &attempts)?;
        if let Some(active) = active {
            active.validate(intent)?;
            if !reservations.iter().any(|reservation| reservation == active) {
                return Err(ProjectionStoreError::AttemptBindingMismatch);
            }
        }
        let reservation_bytes = reservations
            .iter()
            .map(|reservation| {
                serde_json::to_vec(reservation)
                    .map_err(|error| ProjectionStoreError::Encode(error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let intent_bytes = intent.encode()?;
        let base = match intent.precondition() {
            ProjectionPrecondition::Absent => None,
            ProjectionPrecondition::Base(description) => {
                let bytes = read_optional_regular(
                    &bases,
                    &base_filename(*description),
                    MAX_PROJECTION_EVIDENCE_BYTES,
                    Some(description.byte_length()),
                )?
                .ok_or(ProjectionStoreError::MissingBase(*description))?;
                if BlobDescription::of(&bytes) != *description {
                    return Err(ProjectionStoreError::BaseEvidenceMismatch(*description));
                }
                Some(*description)
            }
        };
        let store_claim_digest = Sha256::digest(&store_claim).into();
        let endpoint_binding = self.endpoint.map(endpoint_binding_bytes);
        let intent_digest = Sha256::digest(&intent_bytes).into();
        let namespace_identities = self.namespaces.identities();
        let attempts_identity = canonical_directory_identity(&attempts)?;
        let forensics_identity = canonical_directory_identity(&forensics)?;
        let (durable, durable_bytes, reservations, authority_active, created_durable_record) =
            if let Some(durable_bytes) = existing_durable_bytes {
                let durable = decode_durable_mutation_authority(
                    &durable_bytes,
                    intent,
                    self.store_id,
                    store_claim_digest,
                    self.workspace_id,
                    endpoint_binding.as_deref(),
                    intent_id,
                    intent_digest,
                    base,
                    namespace_identities,
                    attempts_identity,
                    forensics_identity,
                    &reservations,
                )?;
                let reservations = decode_mutation_reservations(&durable, intent)?;
                let durable_active = durable.active_attempt_id.and_then(|active_attempt_id| {
                    reservations
                        .iter()
                        .find(|reservation| reservation.attempt_id() == active_attempt_id)
                        .cloned()
                });
                if requested_active.is_some() && requested_active != durable_active {
                    return Err(ProjectionStoreError::MutationAuthorityPending);
                }
                (
                    durable,
                    durable_bytes,
                    reservations,
                    requested_active.or(durable_active),
                    false,
                )
            } else {
                let authority_active = requested_active;
                let durable = DurableProjectionMutationAuthority {
                    schema_version: MUTATION_AUTHORITY_SCHEMA_VERSION,
                    authority_id: Uuid::new_v4(),
                    store_id: self.store_id,
                    store_claim_digest,
                    workspace_id: self.workspace_id,
                    endpoint_binding,
                    intent_id,
                    intent_digest,
                    base,
                    namespace_identities,
                    attempts_identity,
                    forensics_identity,
                    active_attempt_id: authority_active
                        .as_ref()
                        .map(ProjectionAttemptReservation::attempt_id),
                    reservation_bytes,
                };
                let durable_bytes = serde_json::to_vec(&durable)
                    .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
                if durable_bytes.len() > MAX_MUTATION_AUTHORITY_BYTES {
                    return Err(ProjectionStoreError::MutationAuthorityTooLarge {
                        attempts: reservations.len(),
                        bytes: durable_bytes.len(),
                    });
                }
                publish_immutable_exact(
                    &self.capability,
                    &durable_name,
                    &durable_bytes,
                    "projection graph mutation authority",
                )?;
                (
                    durable,
                    durable_bytes,
                    reservations,
                    authority_active,
                    true,
                )
            };
        let authority = ProjectionMutationAuthority {
            durable,
            durable_bytes,
            durable_name,
            _lease: lease,
            root: self.capability.try_clone()?,
            bases,
            intents,
            attempts_parent,
            attempts,
            forensics_parent,
            forensics,
            completions,
            reservations,
            active: authority_active,
            created_durable_record,
            graph_operation_consumed: false,
            completion_published: false,
        };
        authority.validate_live_names()?;
        mutation_authority_captured_hook();
        Ok(authority)
    }

    /// Publish completion only through the same one-shot capability session
    /// that Graph consumed for the exact mutation or recovery operation.
    pub(crate) fn publish_completion(
        &self,
        mut authority: ProjectionMutationAuthority,
        intent: &ProjectionIntent,
        proof: &ProjectionWriteProof,
    ) -> Result<ProjectionCompletion, ProjectionStoreError> {
        completion_publication_hook();
        completion_publication_act_hook();
        authority.consume_completion_publication(self, intent, |authority| {
            self.require_write_proof(intent, proof)?;
            let intent_id = authority.durable.intent_id;
            for evidence in proof.recovery_evidence() {
                let reservation = authority
                    .reservations
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
                self.validate_forensic_record_with_reservation(intent, &record, reservation)?;
                let record_bytes = serde_json::to_vec(&record)
                    .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?;
                let digest: [u8; 32] = Sha256::digest(&record_bytes).into();
                publish_immutable_exact(
                    &authority.forensics,
                    &format!("{}.evidence", hex(&digest)),
                    &record_bytes,
                    "local projection forensic evidence",
                )?;
            }
            let completion = ProjectionCompletion::for_intent(intent, proof.bytes())?;
            let bytes = completion.encode()?;
            require_evidence_length(
                "projection completion",
                bytes.len() as u64,
                MAX_PROJECTION_EVIDENCE_BYTES,
            )?;
            publish_immutable_exact(
                &authority.completions,
                &completion_filename(intent_id),
                &bytes,
                "projection completion",
            )?;
            Ok(completion)
        })
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
        authority: ProjectionMutationAuthority,
        intent: &ProjectionIntent,
        replayed_target: &[u8],
        proof: &ProjectionWriteProof,
    ) -> Result<ProjectionCompletion, ProjectionStoreError> {
        if BlobDescription::of(replayed_target) != intent.target() {
            return Err(ProjectionStoreError::RecoveryTargetMismatch);
        }
        self.require_write_proof(intent, proof)?;
        self.publish_completion(authority, intent, proof)
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
        self.validate_forensic_record_with_reservation(intent, record, &reservation)
    }

    fn validate_forensic_record_with_reservation(
        &self,
        intent: &ProjectionIntent,
        record: &LocalProjectionEvidenceRecord,
        reservation: &ProjectionAttemptReservation,
    ) -> Result<(), ProjectionStoreError> {
        if record.schema_version != LOCAL_FORENSIC_SCHEMA_VERSION
            || record.intent_id != intent.id()?
            || record.target_path != *intent.path()
            || reservation.attempt_id() != record.attempt_id
        {
            return Err(ProjectionStoreError::ForensicBindingMismatch);
        }
        require_evidence_length(
            "local projection forensic evidence",
            record.observed.byte_length(),
            MAX_PROJECTION_EVIDENCE_BYTES,
        )?;
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

    fn acquire_mutation_lease(
        &self,
        intent_id: ProjectionIntentId,
    ) -> Result<File, ProjectionStoreError> {
        let name = mutation_authority_lease_filename(intent_id);
        let file = open_mutation_authority_lease_file(&self.capability, &name)?;
        if let Err(error) = file.try_lock_exclusive() {
            if matches!(
                error.kind(),
                ErrorKind::WouldBlock | ErrorKind::PermissionDenied
            ) {
                return Err(ProjectionStoreError::MutationAuthorityPending);
            }
            return Err(error.into());
        }
        validate_mutation_authority_lease_file(&file, &name)?;
        file.set_len(0)?;
        Ok(file)
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

#[allow(clippy::too_many_arguments)]
fn decode_durable_mutation_authority(
    durable_bytes: &[u8],
    intent: &ProjectionIntent,
    store_id: ProjectionReceiptStoreId,
    store_claim_digest: [u8; 32],
    workspace_id: WorkspaceId,
    endpoint_binding: Option<&[u8]>,
    intent_id: ProjectionIntentId,
    intent_digest: [u8; 32],
    base: Option<BlobDescription>,
    namespace_identities: [DirectoryIdentity; 5],
    attempts_identity: DirectoryIdentity,
    forensics_identity: DirectoryIdentity,
    live_reservations: &[ProjectionAttemptReservation],
) -> Result<DurableProjectionMutationAuthority, ProjectionStoreError> {
    let durable: DurableProjectionMutationAuthority = serde_json::from_slice(durable_bytes)
        .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
    if serde_json::to_vec(&durable)
        .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?
        != durable_bytes
        || durable.schema_version != MUTATION_AUTHORITY_SCHEMA_VERSION
        || durable.store_id != store_id
        || durable.store_claim_digest != store_claim_digest
        || durable.workspace_id != workspace_id
        || durable.endpoint_binding.as_deref() != endpoint_binding
        || durable.intent_id != intent_id
        || durable.intent_digest != intent_digest
        || durable.base != base
        || durable.namespace_identities != namespace_identities
        || durable.attempts_identity != attempts_identity
        || durable.forensics_identity != forensics_identity
    {
        return Err(ProjectionStoreError::MutationAuthorityMismatch);
    }
    let durable_reservations = decode_mutation_reservations(&durable, intent)?;
    if durable_reservations != live_reservations {
        return Err(ProjectionStoreError::AttemptBindingMismatch);
    }
    Ok(durable)
}

fn decode_mutation_reservations(
    durable: &DurableProjectionMutationAuthority,
    intent: &ProjectionIntent,
) -> Result<Vec<ProjectionAttemptReservation>, ProjectionStoreError> {
    if durable.reservation_bytes.len() > MAX_MUTATION_ATTEMPTS {
        return Err(ProjectionStoreError::MutationAuthorityTooLarge {
            attempts: durable.reservation_bytes.len(),
            bytes: 0,
        });
    }
    let mut reservations = Vec::with_capacity(durable.reservation_bytes.len());
    for bytes in &durable.reservation_bytes {
        let reservation: ProjectionAttemptReservation = serde_json::from_slice(bytes)
            .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
        reservation.validate(intent)?;
        if serde_json::to_vec(&reservation)
            .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?
            != *bytes
        {
            return Err(ProjectionStoreError::AttemptBindingMismatch);
        }
        if reservations
            .last()
            .is_some_and(|prior: &ProjectionAttemptReservation| {
                prior.attempt_id() >= reservation.attempt_id()
            })
        {
            return Err(ProjectionStoreError::AttemptBindingMismatch);
        }
        reservations.push(reservation);
    }
    if durable.active_attempt_id.is_some_and(|active_attempt_id| {
        !reservations
            .iter()
            .any(|reservation| reservation.attempt_id() == active_attempt_id)
    }) {
        return Err(ProjectionStoreError::AttemptBindingMismatch);
    }
    Ok(reservations)
}

impl ProjectionMutationAuthority {
    pub(crate) fn consume_write_evidence<T>(
        &mut self,
        relative_path: &str,
        operation: impl FnOnce(
            &ProjectionAttemptReservation,
            &[ProjectionAttemptReservation],
        ) -> io::Result<T>,
    ) -> io::Result<T> {
        mutation_authority_act_hook();
        self.consume_graph_operation(relative_path)?;
        let active = self.active.as_ref().ok_or_else(|| {
            io::Error::new(
                ErrorKind::PermissionDenied,
                "projection mutation authority has no active reserved attempt",
            )
        })?;
        operation(active, &self.reservations)
    }

    pub(crate) fn consume_recovery_evidence<T>(
        &mut self,
        relative_path: &str,
        operation: impl FnOnce(&[ProjectionAttemptReservation]) -> io::Result<T>,
    ) -> io::Result<T> {
        mutation_authority_act_hook();
        self.consume_graph_operation(relative_path)?;
        if self.reservations.is_empty() {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "projection recovery authority has no durable attempts",
            ));
        }
        operation(&self.reservations)
    }

    fn consume_completion_publication<T>(
        &mut self,
        store: &ProjectionReceiptStore,
        intent: &ProjectionIntent,
        operation: impl FnOnce(&Self) -> Result<T, ProjectionStoreError>,
    ) -> Result<T, ProjectionStoreError> {
        self.require_store_and_intent(store, intent)?;
        self.require_consumed()?;
        self.validate_live_names()?;
        let result = operation(self)?;
        self.validate_live_names()?;
        self.completion_published = true;
        self.retire_durable_record()?;
        Ok(result)
    }

    fn retire_durable_record(&self) -> Result<(), ProjectionStoreError> {
        self.remove_durable_record_if_exact()
    }

    fn remove_durable_record_if_exact(&self) -> Result<(), ProjectionStoreError> {
        let Some(bytes) = read_optional_regular(
            &self.root,
            &self.durable_name,
            MAX_MUTATION_AUTHORITY_BYTES as u64,
            Some(self.durable_bytes.len() as u64),
        )?
        else {
            return Ok(());
        };
        if bytes != self.durable_bytes {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        self.root.remove_file(&self.durable_name)?;
        sync_dir_required(&self.root)?;
        Ok(())
    }

    fn consume_graph_operation(&mut self, relative_path: &str) -> io::Result<()> {
        if self.graph_operation_consumed {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "projection mutation authority was already consumed",
            ));
        }
        self.validate_live_names().map_err(|error| {
            io::Error::new(
                ErrorKind::PermissionDenied,
                format!("projection mutation authority is no longer live: {error}"),
            )
        })?;
        if self
            .reservations
            .iter()
            .any(|reservation| reservation.target_path().as_str() != relative_path)
            || self
                .active
                .as_ref()
                .is_some_and(|reservation| reservation.target_path().as_str() != relative_path)
        {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "projection mutation authority target path mismatch",
            ));
        }
        self.graph_operation_consumed = true;
        Ok(())
    }

    fn require_store_and_intent(
        &self,
        store: &ProjectionReceiptStore,
        intent: &ProjectionIntent,
    ) -> Result<(), ProjectionStoreError> {
        let intent_bytes = intent.encode()?;
        if self.durable.store_id != store.store_id
            || self.durable.workspace_id != store.workspace_id
            || self.durable.endpoint_binding != store.endpoint.map(endpoint_binding_bytes)
            || self.durable.intent_id != intent.id()?
            || self.durable.intent_digest != <[u8; 32]>::from(Sha256::digest(&intent_bytes))
            || self.durable.namespace_identities != store.namespaces.identities()
        {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        Ok(())
    }

    fn require_consumed(&self) -> Result<(), ProjectionStoreError> {
        if !self.graph_operation_consumed {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        Ok(())
    }

    fn validate_live_names(&self) -> Result<(), ProjectionStoreError> {
        if canonical_receipt_store_id(&self.root)? != self.durable.store_id {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        let store_claim = read_optional_regular(
            &self.root,
            STORE_CLAIM_FILE,
            STORE_CLAIM_LEN as u64,
            Some(STORE_CLAIM_LEN as u64),
        )?
        .ok_or(ProjectionStoreError::MalformedStoreClaim)?;
        if <[u8; 32]>::from(Sha256::digest(&store_claim)) != self.durable.store_claim_digest {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        let stored = read_optional_regular(
            &self.root,
            &self.durable_name,
            MAX_MUTATION_AUTHORITY_BYTES as u64,
            Some(self.durable_bytes.len() as u64),
        )?
        .ok_or_else(|| {
            ProjectionStoreError::NamespaceSubstitution(
                "projection mutation authority disappeared".into(),
            )
        })?;
        if stored != self.durable_bytes {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        for (index, name) in [
            BASES_DIR,
            INTENTS_DIR,
            COMPLETIONS_DIR,
            ATTEMPTS_DIR,
            FORENSICS_DIR,
        ]
        .into_iter()
        .enumerate()
        {
            let live = open_dir_nofollow(&self.root, name).map_err(|error| {
                ProjectionStoreError::NamespaceSubstitution(format!("{name}: {error}"))
            })?;
            if canonical_directory_identity(&live)? != self.durable.namespace_identities[index] {
                return Err(ProjectionStoreError::NamespaceSubstitution(name.into()));
            }
        }
        if canonical_directory_identity(&self.bases)? != self.durable.namespace_identities[0]
            || canonical_directory_identity(&self.completions)?
                != self.durable.namespace_identities[2]
            || canonical_directory_identity(&self.intents)? != self.durable.namespace_identities[1]
            || canonical_directory_identity(&self.attempts_parent)?
                != self.durable.namespace_identities[3]
            || canonical_directory_identity(&self.forensics_parent)?
                != self.durable.namespace_identities[4]
            || canonical_directory_identity(&self.attempts)? != self.durable.attempts_identity
            || canonical_directory_identity(&self.forensics)? != self.durable.forensics_identity
        {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        let intent_bytes = read_optional_regular(
            &self.intents,
            &intent_filename(self.durable.intent_id),
            MAX_PROJECTION_EVIDENCE_BYTES,
            None,
        )?
        .ok_or(ProjectionStoreError::MissingIntent(self.durable.intent_id))?;
        if <[u8; 32]>::from(Sha256::digest(&intent_bytes)) != self.durable.intent_digest {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        if let Some(description) = self.durable.base {
            let bytes = read_optional_regular(
                &self.bases,
                &base_filename(description),
                MAX_PROJECTION_EVIDENCE_BYTES,
                Some(description.byte_length()),
            )?
            .ok_or(ProjectionStoreError::MissingBase(description))?;
            if BlobDescription::of(&bytes) != description {
                return Err(ProjectionStoreError::BaseEvidenceMismatch(description));
            }
        }
        if self.reservations.len() != self.durable.reservation_bytes.len() {
            return Err(ProjectionStoreError::MutationAuthorityMismatch);
        }
        let expected_attempts = self
            .reservations
            .iter()
            .zip(&self.durable.reservation_bytes)
            .map(|(reservation, bytes)| (attempt_filename(reservation.attempt_id()), bytes))
            .collect::<BTreeMap<_, _>>();
        let mut live_attempts = BTreeMap::new();
        for entry in self.attempts.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                ProjectionStoreError::UnsafeEntry(
                    "non-UTF-8 projection attempt entry".into(),
                )
            })?;
            require_regular_entry(&entry.file_type()?, name)?;
            if is_temp_name(name) {
                continue;
            }
            parse_attempt_filename(name)?;
            let expected_bytes = expected_attempts
                .get(name)
                .ok_or(ProjectionStoreError::AttemptBindingMismatch)?;
            let bytes = read_optional_regular(
                &self.attempts,
                name,
                MAX_PROJECTION_EVIDENCE_BYTES,
                Some(expected_bytes.len() as u64),
            )?
            .ok_or(ProjectionStoreError::AttemptBindingMismatch)?;
            if bytes.as_slice() != expected_bytes.as_slice() {
                return Err(ProjectionStoreError::AttemptBindingMismatch);
            }
            live_attempts.insert(name.to_owned(), bytes);
        }
        if live_attempts.len() != expected_attempts.len() {
            return Err(ProjectionStoreError::AttemptBindingMismatch);
        }
        validate_live_intent_namespace(
            &self.attempts_parent,
            ATTEMPTS_DIR,
            self.durable.store_id,
            self.durable.intent_id,
            self.durable.attempts_identity,
        )?;
        validate_live_intent_namespace(
            &self.forensics_parent,
            FORENSICS_DIR,
            self.durable.store_id,
            self.durable.intent_id,
            self.durable.forensics_identity,
        )
    }
}

impl Drop for ProjectionMutationAuthority {
    fn drop(&mut self) {
        mutation_authority_drop_hook();
        if self.completion_published
            || (self.created_durable_record && !self.graph_operation_consumed)
        {
            let _ = self.remove_durable_record_if_exact();
        }
    }
}

fn validate_live_intent_namespace(
    parent: &Dir,
    namespace: &str,
    store_id: ProjectionReceiptStoreId,
    intent_id: ProjectionIntentId,
    expected_identity: DirectoryIdentity,
) -> Result<(), ProjectionStoreError> {
    let name = hex(intent_id.as_bytes());
    let authority_name = format!("{name}{INTENT_NAMESPACE_AUTHORITY_SUFFIX}");
    let bytes = read_optional_regular(parent, &authority_name, 1024, None)?.ok_or_else(|| {
        ProjectionStoreError::NamespaceSubstitution(format!(
            "missing established {namespace}/{name} authority"
        ))
    })?;
    let authority: IntentNamespaceAuthority = serde_json::from_slice(&bytes)
        .map_err(|error| ProjectionStoreError::Decode(error.to_string()))?;
    if serde_json::to_vec(&authority)
        .map_err(|error| ProjectionStoreError::Encode(error.to_string()))?
        != bytes
        || authority.schema_version != INTENT_NAMESPACE_SCHEMA_VERSION
        || authority.store_id != store_id
        || authority.namespace != namespace
        || authority.intent_id != intent_id
        || authority.directory_identity != expected_identity
    {
        return Err(ProjectionStoreError::NamespaceSubstitution(format!(
            "{namespace}/{name}"
        )));
    }
    let live = open_dir_nofollow(parent, &name).map_err(|error| {
        ProjectionStoreError::NamespaceSubstitution(format!("{namespace}/{name}: {error}"))
    })?;
    if canonical_directory_identity(&live)? != expected_identity {
        return Err(ProjectionStoreError::NamespaceSubstitution(format!(
            "{namespace}/{name}"
        )));
    }
    Ok(())
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
    MutationAuthorityMismatch,
    MutationAuthorityPending,
    MutationAuthorityTooLarge {
        attempts: usize,
        bytes: usize,
    },
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
            Self::MutationAuthorityMismatch => {
                f.write_str("projection mutation authority does not match the durable operation")
            }
            Self::MutationAuthorityPending => {
                f.write_str("projection mutation authority is pending recovery")
            }
            Self::MutationAuthorityTooLarge { attempts, bytes } => write!(
                f,
                "projection mutation authority exceeds its bound: {attempts} attempts, {bytes} bytes"
            ),
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

fn mutation_authority_filename(intent_id: ProjectionIntentId) -> String {
    format!(
        "{}{}",
        hex(intent_id.as_bytes()),
        MUTATION_AUTHORITY_SUFFIX
    )
}

fn mutation_authority_lease_filename(intent_id: ProjectionIntentId) -> String {
    format!(
        "{}{}",
        hex(intent_id.as_bytes()),
        MUTATION_AUTHORITY_LEASE_SUFFIX
    )
}

#[cfg(unix)]
fn open_mutation_authority_lease_file(directory: &Dir, name: &str) -> io::Result<File> {
    let name = CString::new(name)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid lease file name"))?;
    // SAFETY: `name` is a live NUL-terminated relative name and `directory`
    // retains the authoritative receipt-store capability. O_NOFOLLOW rejects
    // a final-component symlink in the same open that produces the locked
    // handle.
    let fd = unsafe {
        libc::openat(
            directory.as_fd().as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(windows)]
fn open_mutation_authority_lease_file(directory: &Dir, name: &str) -> io::Result<File> {
    let mut options = CapOpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .follow(FollowSymlinks::No);
    Ok(directory.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_mutation_authority_lease_file(_directory: &Dir, _name: &str) -> io::Result<File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-follow projection mutation leases are unsupported on this target",
    ))
}

fn validate_mutation_authority_lease_file(
    file: &File,
    name: &str,
) -> Result<(), ProjectionStoreError> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ProjectionStoreError::UnsafeEntry(format!(
            "projection mutation lease is not a regular file: {name}"
        )));
    }
    #[cfg(unix)]
    if metadata.uid() !=
        // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
        unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(ProjectionStoreError::UnsafeEntry(format!(
            "projection mutation lease has unsafe ownership or links: {name}"
        )));
    }
    #[cfg(windows)]
    if metadata.file_attributes()
        & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err(ProjectionStoreError::UnsafeEntry(format!(
            "projection mutation lease is a reparse point: {name}"
        )));
    }
    Ok(())
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

fn endpoint_binding_bytes(binding: ProjectionEndpointBinding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(64);
    bytes.extend_from_slice(binding.endpoint_id().as_uuid().as_bytes());
    bytes.extend_from_slice(binding.device_id().as_uuid().as_bytes());
    bytes.extend_from_slice(binding.graph_resource_id().as_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use crate::oplog::{FrontierV2, PageId};

    use super::*;

    struct Fixture {
        root: PathBuf,
        graph_root: PathBuf,
        store: ProjectionReceiptStore,
        graph: Graph,
        intent: ProjectionIntent,
        target: Vec<u8>,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            Self::new_at(label, "pages/authority.md")
        }

        fn new_at(label: &str, target_path: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("tine-receipt-authority-{label}-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let graph_root = root.join("graph");
            fs::create_dir(&graph_root).unwrap();
            fs::create_dir(graph_root.join("pages")).unwrap();
            let graph = Graph::open(&graph_root);
            let store = ProjectionReceiptStore::open(
                &root.join("receipts"),
                WorkspaceId::from_uuid(Uuid::from_u128(1)),
            )
            .unwrap();
            let target = b"- target\n".to_vec();
            let intent = ProjectionIntent::new(
                store.workspace_id(),
                PageId::from_uuid(Uuid::from_u128(2)),
                ManagedPath::parse(target_path).unwrap(),
                FrontierV2::default(),
                Vec::new(),
                ProjectionPrecondition::Absent,
                BlobDescription::of(&target),
                Vec::new(),
            )
            .unwrap();
            store.publish_intent(&intent, None).unwrap();
            Self {
                root,
                graph_root,
                store,
                graph,
                intent,
                target,
            }
        }

        fn snapshot_graph(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
            let mut snapshot = BTreeMap::new();
            let mut pending = vec![self.graph_root.clone()];
            while let Some(path) = pending.pop() {
                let relative = path.strip_prefix(&self.graph_root).unwrap().to_path_buf();
                if path.is_dir() {
                    snapshot.insert(relative, None);
                    for entry in fs::read_dir(path).unwrap() {
                        pending.push(entry.unwrap().path());
                    }
                } else {
                    snapshot.insert(relative, Some(fs::read(path).unwrap()));
                }
            }
            snapshot
        }

        fn authority_path(&self, intent: &ProjectionIntent) -> PathBuf {
            self.store
                .root_path()
                .join(mutation_authority_filename(intent.id().unwrap()))
        }

        fn reopen_store(&self) -> ProjectionReceiptStore {
            ProjectionReceiptStore::open(self.store.root_path(), self.store.workspace_id()).unwrap()
        }

        fn authority_stats(&self) -> (usize, u64) {
            fs::read_dir(self.store.root_path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(MUTATION_AUTHORITY_SUFFIX)
                })
                .fold((0, 0), |(count, bytes), entry| {
                    (count + 1, bytes + entry.metadata().unwrap().len())
                })
        }

        fn attempt_stats(&self, intent: &ProjectionIntent) -> (usize, u64) {
            fs::read_dir(
                self.store
                    .root_path()
                    .join(ATTEMPTS_DIR)
                    .join(hex(intent.id().unwrap().as_bytes())),
            )
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".attempt")
            })
            .fold((0, 0), |(count, bytes), entry| {
                (count + 1, bytes + entry.metadata().unwrap().len())
            })
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn attempt_namespace_delete_or_substitute_after_capture_denies_before_graph_mutation() {
        #[derive(Clone, Copy)]
        enum Attack {
            DeleteFile,
            SubstituteFile,
            DeleteDirectory,
            SubstituteDirectory,
        }

        for (label, attack) in [
            ("delete-file-before-mutation", Attack::DeleteFile),
            ("substitute-file-before-mutation", Attack::SubstituteFile),
            ("delete-dir-before-mutation", Attack::DeleteDirectory),
            ("substitute-dir-before-mutation", Attack::SubstituteDirectory),
        ] {
            let fixture = Fixture::new(label);
            let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
            let before = fixture.snapshot_graph();
            let intent_name = hex(fixture.intent.id().unwrap().as_bytes());
            let attempt_dir = fixture
                .store
                .root_path()
                .join(ATTEMPTS_DIR)
                .join(&intent_name);
            let attempt_file = attempt_dir.join(attempt_filename(reservation.attempt_id()));
            MUTATION_AUTHORITY_CAPTURED_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    match attack {
                        Attack::DeleteFile => fs::remove_file(attempt_file).unwrap(),
                        Attack::SubstituteFile => {
                            fs::write(attempt_file, b"substituted reservation").unwrap()
                        }
                        Attack::DeleteDirectory | Attack::SubstituteDirectory => {
                            fs::remove_file(attempt_file).unwrap();
                            fs::remove_dir(&attempt_dir).unwrap();
                            if matches!(attack, Attack::SubstituteDirectory) {
                                fs::create_dir(&attempt_dir).unwrap();
                            }
                        }
                    }
                }));
            });
            let mut authority = fixture
                .store
                .begin_mutation(&fixture.intent, Some(&reservation))
                .unwrap();

            assert!(fixture
                .graph
                .write_page_projection(
                    fixture.intent.path().as_str(),
                    None,
                    &fixture.target,
                    &mut authority,
                )
                .is_err());
            assert_eq!(fixture.snapshot_graph(), before);
            drop(authority);
            assert!(!fs::read_dir(fixture.store.root_path())
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(MUTATION_AUTHORITY_SUFFIX)));
        }
    }

    #[test]
    fn authority_or_attempt_change_after_validation_denies_before_graph_mutation() {
        enum Attack {
            RemoveRootAuthority,
            RemoveActiveAttempt,
            SubstituteAttemptNamespace,
            InsertCanonicalAttempt,
        }

        for (label, attack) in [
            ("remove-root-authority-at-act", Attack::RemoveRootAuthority),
            ("remove-active-attempt-at-act", Attack::RemoveActiveAttempt),
            (
                "substitute-attempt-namespace-at-act",
                Attack::SubstituteAttemptNamespace,
            ),
            ("insert-canonical-attempt-at-act", Attack::InsertCanonicalAttempt),
        ] {
            let fixture = Fixture::new(label);
            let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
            let before = fixture.snapshot_graph();
            let intent_name = hex(fixture.intent.id().unwrap().as_bytes());
            let attempt_dir = fixture
                .store
                .root_path()
                .join(ATTEMPTS_DIR)
                .join(&intent_name);
            let attempt_file = attempt_dir.join(attempt_filename(reservation.attempt_id()));
            let root = fixture.store.root_path().to_path_buf();
            let reservation_for_hook = reservation.clone();
            MUTATION_AUTHORITY_ACT_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || match attack {
                    Attack::RemoveRootAuthority => {
                        let authority = fs::read_dir(&root)
                            .unwrap()
                            .map(Result::unwrap)
                            .find(|entry| {
                                entry
                                    .file_name()
                                    .to_string_lossy()
                                    .ends_with(MUTATION_AUTHORITY_SUFFIX)
                            })
                            .expect("published mutation authority");
                        fs::remove_file(authority.path()).unwrap();
                    }
                    Attack::RemoveActiveAttempt => fs::remove_file(attempt_file).unwrap(),
                    Attack::SubstituteAttemptNamespace => {
                        fs::remove_file(attempt_file).unwrap();
                        fs::remove_dir(&attempt_dir).unwrap();
                        fs::create_dir(&attempt_dir).unwrap();
                    }
                    Attack::InsertCanonicalAttempt => {
                        let mut extra = reservation_for_hook;
                        extra.attempt_id = Uuid::new_v4();
                        extra.recovery_filename = format!(
                            ".authority.md.{}.projection.recovery",
                            extra.attempt_id.simple()
                        );
                        fs::write(
                            attempt_dir.join(attempt_filename(extra.attempt_id)),
                            serde_json::to_vec(&extra).unwrap(),
                        )
                        .unwrap();
                    }
                }));
            });
            let mut authority = fixture
                .store
                .begin_mutation(&fixture.intent, Some(&reservation))
                .unwrap();

            assert!(
                fixture
                    .graph
                    .write_page_projection(
                        fixture.intent.path().as_str(),
                        None,
                        &fixture.target,
                        &mut authority,
                    )
                    .is_err(),
                "attack {label} reached the graph act"
            );
            assert_eq!(
                fixture.snapshot_graph(),
                before,
                "attack {label} mutated the graph"
            );
        }
    }

    #[test]
    fn store_claim_or_intent_delete_or_substitute_after_capture_denies_before_mutation() {
        #[derive(Clone, Copy)]
        enum Attack {
            DeleteClaim,
            SubstituteClaim,
            DeleteIntent,
            SubstituteIntent,
        }

        for (label, attack) in [
            ("delete-claim-before-mutation", Attack::DeleteClaim),
            ("substitute-claim-before-mutation", Attack::SubstituteClaim),
            ("delete-intent-before-mutation", Attack::DeleteIntent),
            ("substitute-intent-before-mutation", Attack::SubstituteIntent),
        ] {
            let fixture = Fixture::new(label);
            let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
            let before = fixture.snapshot_graph();
            let target = match attack {
                Attack::DeleteClaim | Attack::SubstituteClaim => {
                    fixture.store.root_path().join(STORE_CLAIM_FILE)
                }
                Attack::DeleteIntent | Attack::SubstituteIntent => fixture
                    .store
                    .root_path()
                    .join(INTENTS_DIR)
                    .join(intent_filename(fixture.intent.id().unwrap())),
            };
            MUTATION_AUTHORITY_CAPTURED_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || match attack {
                    Attack::DeleteClaim | Attack::DeleteIntent => {
                        fs::remove_file(target).unwrap()
                    }
                    Attack::SubstituteClaim | Attack::SubstituteIntent => {
                        fs::remove_file(&target).unwrap();
                        fs::write(target, b"substituted durable authority").unwrap()
                    }
                }));
            });
            let mut authority = fixture
                .store
                .begin_mutation(&fixture.intent, Some(&reservation))
                .unwrap();

            assert!(fixture
                .graph
                .write_page_projection(
                    fixture.intent.path().as_str(),
                    None,
                    &fixture.target,
                    &mut authority,
                )
                .is_err());
            assert_eq!(fixture.snapshot_graph(), before);
        }
    }

    #[test]
    fn completion_substitution_after_graph_mutation_keeps_recovery_authority_and_resumes() {
        let fixture = Fixture::new("completion-after-mutation");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        let proof = fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        let completions = fixture.store.root_path().join(COMPLETIONS_DIR);
        let moved = fixture.store.root_path().join("completions-moved-for-test");
        let completions_hook = completions.clone();
        let replacement_hook = completions.clone();
        let moved_hook = moved.clone();
        COMPLETION_PUBLICATION_ACT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(completions_hook, moved_hook).unwrap();
                fs::create_dir(replacement_hook).unwrap();
            }));
        });

        assert!(fixture
            .store
            .publish_completion(authority, &fixture.intent, &proof)
            .is_err());
        assert_eq!(
            fs::read(fixture.graph_root.join("pages/authority.md")).unwrap(),
            fixture.target
        );
        assert!(ProjectionReceiptStore::open(
            fixture.store.root_path(),
            fixture.store.workspace_id()
        )
        .is_err());

        fs::remove_dir(&completions).unwrap();
        fs::rename(&moved, &completions).unwrap();
        let reopened = ProjectionReceiptStore::open(
            fixture.store.root_path(),
            fixture.store.workspace_id(),
        )
        .unwrap();
        let mut recovery = reopened.begin_mutation(&fixture.intent, None).unwrap();
        let proof = fixture
            .graph
            .recover_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut recovery,
            )
            .unwrap();
        reopened
            .publish_completion(recovery, &fixture.intent, &proof)
            .unwrap();
        assert!(reopened.load_completion(&fixture.intent).unwrap().is_some());
    }

    #[test]
    fn root_authority_removal_before_completion_publication_preserves_recovery() {
        let fixture = Fixture::new("completion-root-authority");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        let proof = fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        let root = fixture.store.root_path().to_path_buf();
        COMPLETION_PUBLICATION_ACT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let authority = fs::read_dir(root)
                    .unwrap()
                    .map(Result::unwrap)
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .ends_with(MUTATION_AUTHORITY_SUFFIX)
                    })
                    .expect("live mutation authority");
                fs::remove_file(authority.path()).unwrap();
            }));
        });
        assert!(
            fixture
                .store
                .publish_completion(authority, &fixture.intent, &proof)
                .is_err()
        );
        assert_eq!(
            fs::read(fixture.graph_root.join("pages/authority.md")).unwrap(),
            fixture.target
        );

        let mut recovery = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        let proof = fixture
            .graph
            .recover_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut recovery,
            )
            .unwrap();
        fixture
            .store
            .publish_completion(recovery, &fixture.intent, &proof)
            .unwrap();
    }

    #[test]
    fn interrupted_recoveries_reuse_one_exact_authority_slot() {
        let fixture = Fixture::new("bounded-interrupted-recovery");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        drop(authority);

        let authority_path = fixture.authority_path(&fixture.intent);
        let witness = fs::read(&authority_path).unwrap();
        let stable_stats = fixture.authority_stats();
        let stable_attempt_stats = fixture.attempt_stats(&fixture.intent);
        assert_eq!(stable_stats, (1, witness.len() as u64));

        let reopened = ProjectionReceiptStore::open(
            fixture.store.root_path(),
            fixture.store.workspace_id(),
        )
        .unwrap();
        let recovery = reopened.begin_mutation(&fixture.intent, None).unwrap();
        drop(recovery);
        assert_eq!(
            fs::read(&authority_path).unwrap(),
            witness,
            "dropping a reopened pre-graph recovery must retain the sole witness"
        );

        for _ in 0..3 {
            let reopened = ProjectionReceiptStore::open(
                fixture.store.root_path(),
                fixture.store.workspace_id(),
            )
            .unwrap();
            let mut recovery = reopened.begin_mutation(&fixture.intent, None).unwrap();
            fixture
                .graph
                .recover_page_projection(
                    fixture.intent.path().as_str(),
                    None,
                    &fixture.target,
                    &mut recovery,
                )
                .unwrap();
            drop(recovery);
            assert_eq!(fixture.authority_stats(), stable_stats);
            assert_eq!(
                fixture.attempt_stats(&fixture.intent),
                stable_attempt_stats
            );
            assert_eq!(fs::read(&authority_path).unwrap(), witness);
        }
    }

    #[test]
    fn pending_recovery_blocks_new_active_mutation_and_attempt_reservation() {
        let fixture = Fixture::new("pending-recovery-blocks-active");
        let active = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let blocked = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&active))
            .unwrap();
        fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        drop(authority);

        let authority_path = fixture.authority_path(&fixture.intent);
        let witness = fs::read(&authority_path).unwrap();
        let attempts_path = fixture
            .store
            .root_path()
            .join(ATTEMPTS_DIR)
            .join(hex(fixture.intent.id().unwrap().as_bytes()));
        let attempt_count = fs::read_dir(&attempts_path).unwrap().count();
        let error = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&blocked))
            .unwrap_err();
        assert!(matches!(
            error,
            ProjectionStoreError::MutationAuthorityPending
        ));
        assert!(matches!(
            fixture.store.reserve_attempt(&fixture.intent),
            Err(ProjectionStoreError::MutationAuthorityPending)
        ));
        assert_eq!(fs::read_dir(attempts_path).unwrap().count(), attempt_count);
        assert_eq!(fs::read(authority_path).unwrap(), witness);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn begin_lease_blocks_competing_reservation_until_release() {
        let fixture = Fixture::new("begin-lease-blocks-reservation");
        let competing_root = fixture.store.root_path().to_path_buf();
        let workspace_id = fixture.store.workspace_id();
        let competing_intent = fixture.intent.clone();
        MUTATION_AUTHORITY_LEASED_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let competing =
                    ProjectionReceiptStore::open(&competing_root, workspace_id).unwrap();
                assert!(matches!(
                    competing.reserve_attempt(&competing_intent),
                    Err(ProjectionStoreError::MutationAuthorityPending)
                ));
            }));
        });

        let authority = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        drop(authority);

        fixture.store.reserve_attempt(&fixture.intent).unwrap();
        assert_eq!(
            fs::read(
                fixture
                    .store
                    .root_path()
                    .join(mutation_authority_lease_filename(
                        fixture.intent.id().unwrap()
                    ))
            )
            .unwrap(),
            Vec::<u8>::new()
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn reservation_lease_blocks_competing_begin_until_release() {
        let fixture = Fixture::new("reservation-lease-blocks-begin");
        let competing_root = fixture.store.root_path().to_path_buf();
        let workspace_id = fixture.store.workspace_id();
        let competing_intent = fixture.intent.clone();
        MUTATION_AUTHORITY_LEASED_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let competing =
                    ProjectionReceiptStore::open(&competing_root, workspace_id).unwrap();
                assert!(matches!(
                    competing.begin_mutation(&competing_intent, None),
                    Err(ProjectionStoreError::MutationAuthorityPending)
                ));
            }));
        });

        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let retry = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        drop(retry);
    }

    #[cfg(unix)]
    #[test]
    fn mutation_lease_rejects_a_final_component_symlink() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new("mutation-lease-symlink");
        let target = fixture.root.join("lease-target");
        fs::write(&target, b"sentinel").unwrap();
        symlink(
            &target,
            fixture
                .store
                .root_path()
                .join(mutation_authority_lease_filename(
                    fixture.intent.id().unwrap(),
                )),
        )
        .unwrap();

        assert!(fixture.store.reserve_attempt(&fixture.intent).is_err());
        assert_eq!(fs::read(target).unwrap(), b"sentinel");
        assert_eq!(fixture.attempt_stats(&fixture.intent), (0, 0));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn recovery_reopen_contends_across_handles_and_retries_after_release() {
        let fixture = Fixture::new("recovery-reopen-lease");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut creator = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut creator,
            )
            .unwrap();
        drop(creator);

        let reopened = fixture.reopen_store();
        let recovery = reopened.begin_mutation(&fixture.intent, None).unwrap();
        assert!(matches!(
            fixture.store.begin_mutation(&fixture.intent, None),
            Err(ProjectionStoreError::MutationAuthorityPending)
        ));
        assert!(matches!(
            fixture.store.reserve_attempt(&fixture.intent),
            Err(ProjectionStoreError::MutationAuthorityPending)
        ));

        drop(recovery);
        let retry = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        drop(retry);
        assert!(fixture.authority_path(&fixture.intent).exists());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn creator_drop_serializes_slot_removal_before_reopen() {
        let fixture = Fixture::new("creator-drop-lease");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let creator = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        let authority_path = fixture.authority_path(&fixture.intent);
        assert!(authority_path.exists());

        let competing_root = fixture.store.root_path().to_path_buf();
        let workspace_id = fixture.store.workspace_id();
        let competing_intent = fixture.intent.clone();
        let competing_reservation = reservation.clone();
        MUTATION_AUTHORITY_DROP_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let competing =
                    ProjectionReceiptStore::open(&competing_root, workspace_id).unwrap();
                assert!(matches!(
                    competing.begin_mutation(
                        &competing_intent,
                        Some(&competing_reservation)
                    ),
                    Err(ProjectionStoreError::MutationAuthorityPending)
                ));
            }));
        });

        drop(creator);
        assert!(!authority_path.exists());
        let reopened = fixture.reopen_store();
        let second = reopened
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        assert!(authority_path.exists());
        drop(second);
        assert!(!authority_path.exists());
    }

    #[test]
    fn completion_removes_only_its_matching_authority_slot() {
        let fixture = Fixture::new("matching-authority-retirement");
        let second_target = b"- second target\n".to_vec();
        let second_intent = ProjectionIntent::new(
            fixture.store.workspace_id(),
            PageId::from_uuid(Uuid::from_u128(3)),
            ManagedPath::parse("pages/second-authority.md").unwrap(),
            FrontierV2::default(),
            Vec::new(),
            ProjectionPrecondition::Absent,
            BlobDescription::of(&second_target),
            Vec::new(),
        )
        .unwrap();
        fixture.store.publish_intent(&second_intent, None).unwrap();

        for (intent, target) in [
            (&fixture.intent, fixture.target.as_slice()),
            (&second_intent, second_target.as_slice()),
        ] {
            let reservation = fixture.store.reserve_attempt(intent).unwrap();
            let mut authority = fixture
                .store
                .begin_mutation(intent, Some(&reservation))
                .unwrap();
            fixture
                .graph
                .write_page_projection(
                    intent.path().as_str(),
                    None,
                    target,
                    &mut authority,
                )
                .unwrap();
            drop(authority);
        }

        let first_path = fixture.authority_path(&fixture.intent);
        let second_path = fixture.authority_path(&second_intent);
        let second_witness = fs::read(&second_path).unwrap();
        let mut recovery = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        let proof = fixture
            .graph
            .recover_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut recovery,
            )
            .unwrap();
        fixture
            .store
            .publish_completion(recovery, &fixture.intent, &proof)
            .unwrap();

        assert!(!first_path.exists());
        assert_eq!(fs::read(second_path).unwrap(), second_witness);
        assert_eq!(fixture.authority_stats(), (1, second_witness.len() as u64));
    }

    #[test]
    fn pre_graph_drop_frees_only_a_new_authority_slot() {
        let fixture = Fixture::new("pre-graph-authority-drop");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let authority_path = fixture.authority_path(&fixture.intent);

        let authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        assert!(authority_path.exists());
        drop(authority);
        assert!(!authority_path.exists());

        let recovery = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        assert!(authority_path.exists());
        drop(recovery);
        assert!(!authority_path.exists());
    }

    #[test]
    fn completed_mutation_authorities_do_not_accumulate_at_store_root() {
        let fixture = Fixture::new("completed-authority-lifecycle");
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        let proof = fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        fixture
            .store
            .publish_completion(authority, &fixture.intent, &proof)
            .unwrap();

        let mut interrupted = fixture
            .store
            .begin_mutation(&fixture.intent, None)
            .unwrap();
        fixture
            .graph
            .recover_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut interrupted,
            )
            .unwrap();
        drop(interrupted);
        assert_eq!(
            fs::read_dir(fixture.store.root_path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .ends_with(MUTATION_AUTHORITY_SUFFIX))
                .count(),
            1,
            "post-graph interruption must retain recovery authority"
        );

        for _ in 0..3 {
            let mut recovery = fixture
                .store
                .begin_mutation(&fixture.intent, None)
                .unwrap();
            let proof = fixture
                .graph
                .recover_page_projection(
                    fixture.intent.path().as_str(),
                    None,
                    &fixture.target,
                    &mut recovery,
                )
                .unwrap();
            fixture
                .store
                .publish_completion(recovery, &fixture.intent, &proof)
                .unwrap();
            assert_eq!(
                fs::read_dir(fixture.store.root_path())
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter(|entry| entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(MUTATION_AUTHORITY_SUFFIX))
                    .count(),
                0
            );
        }
    }

    #[test]
    fn mutation_authority_preserves_deeply_nested_projection_layouts() {
        let fixture = Fixture::new_at(
            "nested-authority-layout",
            "pages/topic/subtopic/archive/a.md",
        );
        let reservation = fixture.store.reserve_attempt(&fixture.intent).unwrap();
        let mut authority = fixture
            .store
            .begin_mutation(&fixture.intent, Some(&reservation))
            .unwrap();
        let proof = fixture
            .graph
            .write_page_projection(
                fixture.intent.path().as_str(),
                None,
                &fixture.target,
                &mut authority,
            )
            .unwrap();
        fixture
            .store
            .publish_completion(authority, &fixture.intent, &proof)
            .unwrap();
        assert_eq!(
            fs::read(
                fixture
                    .graph_root
                    .join("pages/topic/subtopic/archive/a.md")
            )
            .unwrap(),
            fixture.target
        );
    }

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
