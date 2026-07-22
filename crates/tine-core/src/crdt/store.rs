use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use loro::VersionVector;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AffectedPage, CrdtError, ManagedSyncStoreState, ProjectionPrecondition};

pub(crate) const SCHEMA_VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"TINESYNC";
const CHECKSUM_LEN: usize = 32;
const FIXED_PREFIX_LEN: usize = MAGIC.len() + 4 + 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChunkKind {
    Genesis,
    Update,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EncryptionMode {
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ChunkHeader {
    pub schema_version: u32,
    pub workspace_id: Uuid,
    encryption: EncryptionMode,
    pub kind: ChunkKind,
    pub author_device_id: Uuid,
    pub author_session_id: Uuid,
    pub affected_pages: Vec<AffectedPage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    projection_intent_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    projection_preconditions: Vec<ProjectionExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    projection_frontier: Option<VersionVector>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenesisClaim {
    schema_version: u32,
    workspace_id: Uuid,
    device_id: Uuid,
    session_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectionReceipt {
    schema_version: u32,
    workspace_id: Uuid,
    encryption: EncryptionMode,
    path: String,
    content_sha256: String,
    frontier: VersionVector,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectionIntent {
    schema_version: u32,
    workspace_id: Uuid,
    encryption: EncryptionMode,
    path: String,
    frontier: VersionVector,
    #[serde(default)]
    update_chunk_id: String,
    #[serde(default)]
    precondition: Option<ProjectionState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ProjectionExpectation {
    path: String,
    precondition: ProjectionState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum ProjectionState {
    Absent,
    Present { content_sha256: String },
}

impl ProjectionState {
    pub(crate) fn from_content(content: Option<&str>) -> Self {
        match content {
            Some(content) => Self::Present {
                content_sha256: digest_hex(content.as_bytes()),
            },
            None => Self::Absent,
        }
    }

    fn matches(&self, content: Option<&str>) -> bool {
        match (self, content) {
            (Self::Absent, None) => true,
            (Self::Present { content_sha256 }, Some(content)) => {
                *content_sha256 == digest_hex(content.as_bytes())
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Chunk {
    pub id: String,
    pub header: ChunkHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct Store {
    pub root: PathBuf,
    pub workspace_id: Uuid,
    pub device_id: Uuid,
    pub session_id: Uuid,
    capability: Dir,
    session_dir: PathBuf,
}

impl Store {
    pub fn state(sync_root: &Path) -> Result<ManagedSyncStoreState, CrdtError> {
        let graph = Dir::open_ambient_dir(sync_root, ambient_authority())?;
        match graph.symlink_metadata(".tine-sync") {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(ManagedSyncStoreState::Absent)
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(unsafe_store_entry(".tine-sync is not a real directory"))
            }
            Ok(_) => {}
        }
        let sync = graph.open_dir(".tine-sync")?;
        match sync.symlink_metadata("v1") {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(ManagedSyncStoreState::Unclaimed)
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(unsafe_store_entry(".tine-sync/v1 is not a real directory"))
            }
            Ok(_) => {}
        }
        let root = sync.open_dir("v1")?;
        let chunks = load_chunks_from(&root)?;
        if chunks
            .iter()
            .any(|chunk| chunk.header.kind == ChunkKind::Genesis)
        {
            Ok(ManagedSyncStoreState::Initialized)
        } else if !chunks.is_empty() || store_has_files(&root, true)? {
            Err(CrdtError::InvalidChunk(
                "managed-sync residue contains artifacts without genesis".into(),
            ))
        } else if cap_file_exists(&root, "genesis.claim")? {
            Ok(ManagedSyncStoreState::Claimed)
        } else {
            Ok(ManagedSyncStoreState::Unclaimed)
        }
    }

    pub fn validate_resume_device(sync_root: &Path, device_id: Uuid) -> Result<(), CrdtError> {
        if Self::state(sync_root)? != ManagedSyncStoreState::Claimed {
            return Ok(());
        }
        let (_, root) = open_store_capability(sync_root, false)?;
        let metadata = root.symlink_metadata("genesis.claim")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(unsafe_store_entry(
                "genesis claim is not a regular no-follow file",
            ));
        }
        let claim: GenesisClaim = serde_json::from_reader(root.open("genesis.claim")?)
            .map_err(|error| CrdtError::InvalidChunk(format!("invalid genesis claim: {error}")))?;
        if claim.schema_version != SCHEMA_VERSION {
            return Err(CrdtError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                found: claim.schema_version,
            });
        }
        if claim.device_id != device_id {
            return Err(CrdtError::StoreNotInitialized);
        }
        Ok(())
    }

    pub fn initialize(
        sync_root: &Path,
        device_id: Uuid,
        session_id: Uuid,
    ) -> Result<Self, CrdtError> {
        let (root_path, root) = open_store_capability(sync_root, true)?;
        ensure_cap_dir(&root, Path::new("genesis"), true)?;

        let existing = load_chunks_from(&root)?;
        let genesis_count = existing
            .iter()
            .filter(|chunk| chunk.header.kind == ChunkKind::Genesis)
            .count();
        if genesis_count > 0 {
            return Err(if genesis_count > 1 {
                CrdtError::MultipleGenesis(genesis_count)
            } else {
                CrdtError::StoreNotInitialized
            });
        }
        if !existing.is_empty() || store_has_files(&root, true)? {
            return Err(CrdtError::InvalidChunk(
                "cannot initialize a store with artifacts but no genesis".into(),
            ));
        }

        let (workspace_id, resumed) = create_or_resume_genesis_claim(&root, device_id, session_id)?;
        let session_dir = create_session_dir(&root, device_id, session_id, resumed)?;
        Ok(Self {
            root: root_path,
            workspace_id,
            device_id,
            session_id,
            capability: root,
            session_dir,
        })
    }

    pub fn open(
        sync_root: &Path,
        device_id: Uuid,
        session_id: Uuid,
    ) -> Result<(Self, Vec<Chunk>), CrdtError> {
        let (root_path, root) = open_store_capability(sync_root, false)?;

        let chunks = load_chunks_from(&root)?;
        let workspace_id = validate_chunk_set(&chunks)?;
        let session_dir = create_session_dir(&root, device_id, session_id, false)?;
        Ok((
            Self {
                root: root_path,
                workspace_id,
                device_id,
                session_id,
                capability: root,
                session_dir,
            },
            chunks,
        ))
    }

    pub fn load_chunks(&self) -> Result<Vec<Chunk>, CrdtError> {
        let chunks = load_chunks_from(&self.capability)?;
        let workspace_id = validate_chunk_set(&chunks)?;
        if workspace_id != self.workspace_id {
            return Err(CrdtError::WorkspaceMismatch {
                expected: self.workspace_id,
                found: workspace_id,
            });
        }
        Ok(chunks)
    }

    /// Load and validate only content IDs this process has not imported. Known
    /// immutable filenames are skipped without reopening their payloads; a full
    /// replay on process start remains the integrity check for the whole store.
    pub fn load_new_chunks(&self, imported: &HashSet<String>) -> Result<Vec<Chunk>, CrdtError> {
        let mut paths = Vec::new();
        collect_chunk_paths(&self.capability, Path::new(""), &mut paths)?;
        paths.sort();
        let mut chunks = BTreeMap::new();
        for path in paths {
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    CrdtError::InvalidChunk(format!(
                        "non-UTF-8 chunk filename at {}",
                        path.display()
                    ))
                })?;
            if imported.contains(file_id) {
                continue;
            }
            let chunk = read_chunk(&self.capability, &path)?;
            if chunk.header.workspace_id != self.workspace_id {
                return Err(CrdtError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: chunk.header.workspace_id,
                });
            }
            if chunk.header.kind == ChunkKind::Genesis {
                return Err(CrdtError::MultipleGenesis(2));
            }
            chunks.entry(chunk.id.clone()).or_insert(chunk);
        }
        Ok(chunks.into_values().collect())
    }

    pub fn publish(
        &self,
        kind: ChunkKind,
        affected_pages: Vec<AffectedPage>,
        payload: Vec<u8>,
    ) -> Result<String, CrdtError> {
        self.publish_with_authorization(kind, affected_pages, payload, Vec::new(), None)
    }

    pub fn publish_authorized_update(
        &self,
        affected_pages: Vec<AffectedPage>,
        payload: Vec<u8>,
        projection_preconditions: Vec<ProjectionPrecondition>,
        projection_frontier: VersionVector,
    ) -> Result<String, CrdtError> {
        let projection_preconditions = projection_preconditions
            .into_iter()
            .map(|precondition| ProjectionExpectation {
                path: precondition.path,
                precondition: ProjectionState::from_content(
                    precondition.expected_content.as_deref(),
                ),
            })
            .collect();
        self.publish_with_authorization(
            ChunkKind::Update,
            affected_pages,
            payload,
            projection_preconditions,
            Some(projection_frontier),
        )
    }

    fn publish_with_authorization(
        &self,
        kind: ChunkKind,
        affected_pages: Vec<AffectedPage>,
        payload: Vec<u8>,
        projection_preconditions: Vec<ProjectionExpectation>,
        projection_frontier: Option<VersionVector>,
    ) -> Result<String, CrdtError> {
        let header = ChunkHeader {
            schema_version: SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            encryption: EncryptionMode::None,
            kind,
            author_device_id: self.device_id,
            author_session_id: self.session_id,
            affected_pages,
            projection_intent_paths: Vec::new(),
            projection_preconditions,
            projection_frontier,
        };
        let bytes = encode_envelope(&header, &payload)?;
        let id = digest_hex(&bytes);
        let target_dir = match kind {
            ChunkKind::Genesis => self.capability.open_dir("genesis")?,
            ChunkKind::Update => self.capability.open_dir(&self.session_dir)?,
        };
        publish_immutable(&target_dir, &id, &bytes)?;
        Ok(id)
    }

    pub fn publish_projection_receipt(
        &self,
        path: &str,
        content: &str,
        frontier: VersionVector,
    ) -> Result<String, CrdtError> {
        let content_sha256 = digest_hex(content.as_bytes());
        let receipt = ProjectionReceipt {
            schema_version: SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            encryption: EncryptionMode::None,
            path: path.to_string(),
            content_sha256: content_sha256.clone(),
            frontier,
        };
        let bytes = serde_json::to_vec(&receipt)
            .map_err(|error| CrdtError::Serialization(error.to_string()))?;
        let id = digest_hex(&bytes);
        let dir = Path::new("projections")
            .join(digest_hex(path.as_bytes()))
            .join(content_sha256);
        ensure_cap_dir(&self.capability, &dir, true)?;
        let dir = self.capability.open_dir(&dir)?;
        publish_immutable_named(&dir, &format!("{id}.receipt"), &bytes)?;
        Ok(id)
    }

    pub fn is_known_projection(
        &self,
        path: &str,
        content: &str,
        current: &VersionVector,
    ) -> Result<bool, CrdtError> {
        let content_sha256 = digest_hex(content.as_bytes());
        let dir = Path::new("projections")
            .join(digest_hex(path.as_bytes()))
            .join(&content_sha256);
        if !ensure_optional_cap_dir(&self.capability, &dir)? {
            return Ok(false);
        }
        let dir = self.capability.open_dir(&dir)?;
        for entry in dir.entries()? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(CrdtError::Io(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "projection receipt is a symlink",
                )));
            }
            if !file_type.is_file()
                || Path::new(&entry.file_name())
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("receipt")
            {
                continue;
            }
            let mut bytes = Vec::new();
            dir.open(entry.file_name())?.read_to_end(&mut bytes)?;
            let filename = Path::new(&entry.file_name())
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| CrdtError::InvalidChunk("invalid projection receipt name".into()))?
                .to_string();
            if filename != digest_hex(&bytes) {
                return Err(CrdtError::ChecksumMismatch);
            }
            let receipt: ProjectionReceipt = serde_json::from_slice(&bytes).map_err(|error| {
                CrdtError::InvalidChunk(format!("invalid projection receipt: {error}"))
            })?;
            if receipt.schema_version != SCHEMA_VERSION {
                return Err(CrdtError::SchemaMismatch {
                    expected: SCHEMA_VERSION,
                    found: receipt.schema_version,
                });
            }
            if receipt.workspace_id != self.workspace_id {
                return Err(CrdtError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: receipt.workspace_id,
                });
            }
            if receipt.path != path || receipt.content_sha256 != content_sha256 {
                return Err(CrdtError::InvalidChunk(
                    "projection receipt does not match its directory".into(),
                ));
            }
            if current.includes_vv(&receipt.frontier) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Persist an explicit user-authorized projection overwrite at exactly this
    /// operation frontier. Unlike a receipt, an intent does not claim that bytes
    /// have already reached disk; it lets crash recovery finish a backup restore
    /// even when unexplained projection bytes were present beforehand.
    pub fn publish_projection_intent(
        &self,
        path: &str,
        precondition: ProjectionState,
        frontier: VersionVector,
        update_chunk_id: &str,
    ) -> Result<String, CrdtError> {
        let intent = ProjectionIntent {
            schema_version: SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            encryption: EncryptionMode::None,
            path: path.to_string(),
            frontier,
            update_chunk_id: update_chunk_id.to_string(),
            precondition: Some(precondition),
        };
        let bytes = serde_json::to_vec(&intent)
            .map_err(|error| CrdtError::Serialization(error.to_string()))?;
        let id = digest_hex(&bytes);
        let dir = Path::new("projection-intents").join(digest_hex(path.as_bytes()));
        ensure_cap_dir(&self.capability, &dir, true)?;
        let dir = self.capability.open_dir(&dir)?;
        publish_immutable_named(&dir, &format!("{id}.intent"), &bytes)?;
        Ok(id)
    }

    pub fn is_projection_authorized(
        &self,
        path: &str,
        content: Option<&str>,
        current: &VersionVector,
    ) -> Result<bool, CrdtError> {
        let chunks = self.load_chunks()?;
        self.recover_projection_intents(&chunks)?;
        let chunks_by_id: std::collections::HashMap<&str, &Chunk> = chunks
            .iter()
            .map(|chunk| (chunk.id.as_str(), chunk))
            .collect();
        let dir = Path::new("projection-intents").join(digest_hex(path.as_bytes()));
        if !ensure_optional_cap_dir(&self.capability, &dir)? {
            return Ok(false);
        }
        let dir = self.capability.open_dir(&dir)?;
        for entry in dir.entries()? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(CrdtError::Io(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "projection intent is a symlink",
                )));
            }
            if !file_type.is_file()
                || Path::new(&entry.file_name())
                    .extension()
                    .and_then(|value| value.to_str())
                    != Some("intent")
            {
                continue;
            }
            let mut bytes = Vec::new();
            dir.open(entry.file_name())?.read_to_end(&mut bytes)?;
            let filename = Path::new(&entry.file_name())
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| CrdtError::InvalidChunk("invalid projection intent name".into()))?
                .to_string();
            if filename != digest_hex(&bytes) {
                return Err(CrdtError::ChecksumMismatch);
            }
            let intent: ProjectionIntent = serde_json::from_slice(&bytes).map_err(|error| {
                CrdtError::InvalidChunk(format!("invalid projection intent: {error}"))
            })?;
            if intent.schema_version != SCHEMA_VERSION {
                return Err(CrdtError::SchemaMismatch {
                    expected: SCHEMA_VERSION,
                    found: intent.schema_version,
                });
            }
            if intent.workspace_id != self.workspace_id {
                return Err(CrdtError::WorkspaceMismatch {
                    expected: self.workspace_id,
                    found: intent.workspace_id,
                });
            }
            if intent.path != path {
                return Err(CrdtError::InvalidChunk(
                    "projection intent does not match its directory".into(),
                ));
            }
            let Some(precondition) = intent.precondition.as_ref() else {
                // v1 prototypes without an exact pre-state are intentionally not
                // projection authority: path/frontier alone is unbounded.
                continue;
            };
            let Some(chunk) = chunks_by_id.get(intent.update_chunk_id.as_str()) else {
                continue;
            };
            if chunk.header.kind != ChunkKind::Update
                || !chunk
                    .header
                    .projection_preconditions
                    .iter()
                    .any(|expected| expected.path == path && expected.precondition == *precondition)
                || chunk.header.projection_frontier.as_ref() != Some(&intent.frontier)
            {
                continue;
            }
            if current.includes_vv(&intent.frontier) && precondition.matches(content) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn recover_projection_intents(&self, chunks: &[Chunk]) -> Result<(), CrdtError> {
        for chunk in chunks {
            let Some(frontier) = chunk.header.projection_frontier.as_ref() else {
                continue;
            };
            for expectation in &chunk.header.projection_preconditions {
                self.publish_projection_intent(
                    &expectation.path,
                    expectation.precondition.clone(),
                    frontier.clone(),
                    &chunk.id,
                )?;
            }
        }
        Ok(())
    }
}

fn store_root(sync_root: &Path) -> PathBuf {
    sync_root.join(".tine-sync").join("v1")
}

fn unsafe_store_entry(message: impl Into<String>) -> CrdtError {
    CrdtError::Io(std::io::Error::new(
        ErrorKind::PermissionDenied,
        message.into(),
    ))
}

fn open_store_capability(sync_root: &Path, create: bool) -> Result<(PathBuf, Dir), CrdtError> {
    let graph_path = fs::canonicalize(sync_root)?;
    let graph = Dir::open_ambient_dir(&graph_path, ambient_authority())?;
    ensure_cap_dir(&graph, Path::new(".tine-sync/v1"), create)?;
    let root = graph.open_dir(".tine-sync/v1")?;
    Ok((store_root(&graph_path), root))
}

fn validate_relative(path: &Path) -> Result<(), CrdtError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(unsafe_store_entry("invalid managed-sync path component"))
    }
}

fn ensure_cap_dir(root: &Dir, target: &Path, create: bool) -> Result<(), CrdtError> {
    validate_relative(target)?;
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component.as_os_str());
        match root.symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(unsafe_store_entry(format!(
                    "managed-sync path is a symlink: {}",
                    current.display()
                )))
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(CrdtError::Io(std::io::Error::new(
                    ErrorKind::NotADirectory,
                    format!(
                        "managed-sync path is not a directory: {}",
                        current.display()
                    ),
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound && create => {
                root.create_dir(&current)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(CrdtError::StoreNotInitialized)
            }
            Err(error) => return Err(error.into()),
        }
        root.open_dir(&current)?;
    }
    Ok(())
}

fn ensure_optional_cap_dir(root: &Dir, target: &Path) -> Result<bool, CrdtError> {
    match ensure_cap_dir(root, target, false) {
        Ok(()) => Ok(true),
        Err(CrdtError::StoreNotInitialized) => Ok(false),
        Err(error) => Err(error),
    }
}

fn cap_file_exists(root: &Dir, path: impl AsRef<Path>) -> Result<bool, CrdtError> {
    match root.symlink_metadata(path.as_ref()) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(unsafe_store_entry(format!(
                "managed-sync artifact is not a regular file: {}",
                path.as_ref().display()
            )))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn store_has_files(root: &Dir, ignore_genesis_claim: bool) -> Result<bool, CrdtError> {
    for entry in root.entries()? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(unsafe_store_entry("managed-sync store contains a symlink"));
        }
        if file_type.is_file() {
            if ignore_genesis_claim && entry.file_name() == "genesis.claim" {
                continue;
            }
            return Ok(true);
        }
        if file_type.is_dir() {
            let child = root.open_dir(entry.file_name())?;
            if store_has_files(&child, false)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn create_session_dir(
    root: &Dir,
    device_id: Uuid,
    session_id: Uuid,
    allow_existing: bool,
) -> Result<PathBuf, CrdtError> {
    let sessions = Path::new("devices")
        .join(device_id.to_string())
        .join("sessions");
    ensure_cap_dir(root, &sessions, true)?;
    let session_dir = sessions.join(session_id.to_string());
    match root.create_dir(&session_dir) {
        Ok(()) => {
            sync_cap_dir_best_effort(&root.open_dir(&sessions)?)?;
            Ok(session_dir)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists && allow_existing => {
            ensure_cap_dir(root, &session_dir, false)?;
            Ok(session_dir)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            Err(CrdtError::SessionAlreadyExists(session_id))
        }
        Err(error) => Err(error.into()),
    }
}

fn create_or_resume_genesis_claim(
    root: &Dir,
    device_id: Uuid,
    session_id: Uuid,
) -> Result<(Uuid, bool), CrdtError> {
    let path = Path::new("genesis.claim");
    let claim = GenesisClaim {
        schema_version: SCHEMA_VERSION,
        workspace_id: Uuid::new_v4(),
        device_id,
        session_id,
    };
    let bytes =
        serde_json::to_vec(&claim).map_err(|error| CrdtError::Serialization(error.to_string()))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    match root.open_with(path, &options) {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            sync_cap_dir_best_effort(root)?;
            Ok((claim.workspace_id, false))
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = root.symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(CrdtError::Io(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "genesis claim is not a regular no-follow file",
                )));
            }
            let existing: GenesisClaim =
                serde_json::from_reader(root.open(path)?).map_err(|error| {
                    CrdtError::InvalidChunk(format!("invalid genesis claim: {error}"))
                })?;
            if existing.schema_version != SCHEMA_VERSION {
                return Err(CrdtError::SchemaMismatch {
                    expected: SCHEMA_VERSION,
                    found: existing.schema_version,
                });
            }
            // A process crash necessarily changes `session_id` on restart. The
            // stable device is the initialization owner and may resume with a
            // fresh Loro actor; a different device must wait for genesis rather
            // than creating a split-brain workspace.
            if existing.device_id != device_id {
                return Err(CrdtError::StoreNotInitialized);
            }
            Ok((existing.workspace_id, true))
        }
        Err(error) => Err(error.into()),
    }
}

fn validate_chunk_set(chunks: &[Chunk]) -> Result<Uuid, CrdtError> {
    let genesis: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| chunk.header.kind == ChunkKind::Genesis)
        .collect();
    match genesis.len() {
        0 => return Err(CrdtError::StoreNotInitialized),
        1 => {}
        count => return Err(CrdtError::MultipleGenesis(count)),
    }

    let workspace_id = genesis[0].header.workspace_id;
    for chunk in chunks {
        if chunk.header.schema_version != SCHEMA_VERSION {
            return Err(CrdtError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                found: chunk.header.schema_version,
            });
        }
        if chunk.header.workspace_id != workspace_id {
            return Err(CrdtError::WorkspaceMismatch {
                expected: workspace_id,
                found: chunk.header.workspace_id,
            });
        }
        if chunk.header.encryption != EncryptionMode::None {
            return Err(CrdtError::InvalidChunk(
                "encrypted chunks are not supported by this build".into(),
            ));
        }
    }
    Ok(workspace_id)
}

fn load_chunks_from(root: &Dir) -> Result<Vec<Chunk>, CrdtError> {
    let mut paths = Vec::new();
    collect_chunk_paths(root, Path::new(""), &mut paths)?;
    paths.sort();

    // The same immutable chunk may be delivered more than once in different
    // incoming directories. Collapse it by content ID after validating each copy.
    let mut chunks = BTreeMap::new();
    for path in paths {
        let chunk = read_chunk(root, &path)?;
        chunks.entry(chunk.id.clone()).or_insert(chunk);
    }
    Ok(chunks.into_values().collect())
}

fn collect_chunk_paths(
    dir: &Dir,
    prefix: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), CrdtError> {
    for entry in dir.entries()? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(unsafe_store_entry(format!(
                "managed-sync store contains a symlink: {}",
                prefix.join(entry.file_name()).display()
            )));
        }
        let relative = prefix.join(entry.file_name());
        if file_type.is_dir() {
            collect_chunk_paths(&dir.open_dir(entry.file_name())?, &relative, output)?;
        } else if file_type.is_file()
            && relative.extension().and_then(|value| value.to_str()) == Some("chunk")
        {
            output.push(relative);
        }
    }
    Ok(())
}

fn read_chunk(root: &Dir, path: &Path) -> Result<Chunk, CrdtError> {
    let mut bytes = Vec::new();
    root.open(path)?.read_to_end(&mut bytes)?;
    let id = digest_hex(&bytes);
    let file_id = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            CrdtError::InvalidChunk(format!("non-UTF-8 chunk filename at {}", path.display()))
        })?;
    if file_id != id {
        return Err(CrdtError::InvalidChunk(format!(
            "chunk filename does not match content at {}",
            path.display()
        )));
    }
    let (header, payload) = decode_envelope(&bytes)?;
    Ok(Chunk {
        id,
        header,
        payload,
    })
}

fn encode_envelope(header: &ChunkHeader, payload: &[u8]) -> Result<Vec<u8>, CrdtError> {
    let header_bytes =
        serde_json::to_vec(header).map_err(|error| CrdtError::Serialization(error.to_string()))?;
    let header_len = u32::try_from(header_bytes.len())
        .map_err(|_| CrdtError::Serialization("chunk header is too large".into()))?;
    let payload_len = u64::try_from(payload.len())
        .map_err(|_| CrdtError::Serialization("chunk payload is too large".into()))?;

    let mut bytes =
        Vec::with_capacity(FIXED_PREFIX_LEN + header_bytes.len() + payload.len() + CHECKSUM_LEN);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&header_len.to_be_bytes());
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(&header_bytes);
    bytes.extend_from_slice(payload);
    let checksum = Sha256::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    Ok(bytes)
}

fn decode_envelope(bytes: &[u8]) -> Result<(ChunkHeader, Vec<u8>), CrdtError> {
    if bytes.len() < FIXED_PREFIX_LEN + CHECKSUM_LEN {
        return Err(CrdtError::InvalidChunk("truncated envelope".into()));
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(CrdtError::InvalidChunk("invalid envelope magic".into()));
    }

    let header_len = u32::from_be_bytes(
        bytes[MAGIC.len()..MAGIC.len() + 4]
            .try_into()
            .expect("fixed-width header length"),
    ) as usize;
    let payload_len = u64::from_be_bytes(
        bytes[MAGIC.len() + 4..FIXED_PREFIX_LEN]
            .try_into()
            .expect("fixed-width payload length"),
    );
    let payload_len = usize::try_from(payload_len)
        .map_err(|_| CrdtError::InvalidChunk("payload length exceeds this platform".into()))?;
    let body_len = FIXED_PREFIX_LEN
        .checked_add(header_len)
        .and_then(|length| length.checked_add(payload_len))
        .ok_or_else(|| CrdtError::InvalidChunk("envelope length overflow".into()))?;
    let expected_len = body_len
        .checked_add(CHECKSUM_LEN)
        .ok_or_else(|| CrdtError::InvalidChunk("envelope length overflow".into()))?;
    if bytes.len() != expected_len {
        return Err(CrdtError::InvalidChunk(format!(
            "envelope length mismatch: expected {expected_len}, found {}",
            bytes.len()
        )));
    }

    let expected_checksum = Sha256::digest(&bytes[..body_len]);
    if bytes[body_len..] != expected_checksum[..] {
        return Err(CrdtError::ChecksumMismatch);
    }
    let header: ChunkHeader = serde_json::from_slice(&bytes[FIXED_PREFIX_LEN..][..header_len])
        .map_err(|error| CrdtError::InvalidChunk(format!("invalid header JSON: {error}")))?;
    if header.schema_version != SCHEMA_VERSION {
        return Err(CrdtError::SchemaMismatch {
            expected: SCHEMA_VERSION,
            found: header.schema_version,
        });
    }
    let payload_start = FIXED_PREFIX_LEN + header_len;
    Ok((header, bytes[payload_start..body_len].to_vec()))
}

fn publish_immutable(dir: &Dir, id: &str, bytes: &[u8]) -> Result<(), CrdtError> {
    publish_immutable_named(dir, &format!("{id}.chunk"), bytes)
}

fn publish_immutable_named(dir: &Dir, filename: &str, bytes: &[u8]) -> Result<(), CrdtError> {
    match dir.symlink_metadata(filename) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(unsafe_store_entry(format!(
                "managed-sync target is a symlink: {filename}"
            )))
        }
        Ok(metadata) if metadata.is_file() => return verify_existing(dir, filename, bytes),
        Ok(_) => {
            return Err(CrdtError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("managed-sync target is not a file: {filename}"),
            )))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let temp_path = format!(".tmp-{}", Uuid::new_v4());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut temp = dir.open_with(&temp_path, &options)?;

    let publish_result = (|| {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        drop(temp);

        // SHA-256 is the integrity boundary: an existing target for this hash
        // must contain the same bytes. Recheck after fsync to narrow the race
        // before the provider-friendly atomic rename.
        match dir.symlink_metadata(filename) {
            Ok(_) => verify_existing(dir, filename, bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                dir.rename(&temp_path, dir, filename)?;
                sync_cap_dir_best_effort(dir)
            }
            Err(error) => Err(error.into()),
        }
    })();

    let remove_result = dir.remove_file(&temp_path);
    if let Err(error) = publish_result {
        let _ = remove_result;
        return Err(error);
    }
    if remove_result
        .as_ref()
        .is_err_and(|error| error.kind() != ErrorKind::NotFound)
    {
        remove_result?;
    }
    Ok(())
}

fn verify_existing(dir: &Dir, path: &str, expected: &[u8]) -> Result<(), CrdtError> {
    let metadata = dir.symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CrdtError::Io(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("managed-sync target is not a regular no-follow file: {path}"),
        )));
    }
    let mut existing = Vec::new();
    dir.open(path)?.read_to_end(&mut existing)?;
    if existing == expected {
        Ok(())
    } else {
        Err(CrdtError::InvalidChunk(format!(
            "content-address collision at {path}"
        )))
    }
}

fn sync_cap_dir_best_effort(dir: &Dir) -> Result<(), CrdtError> {
    let result = dir.try_clone()?.into_std_file().sync_all();
    match result {
        Ok(()) => Ok(()),
        Err(error)
            if unsupported_dir_sync(error.kind()) || error.raw_os_error() == Some(libc::EBADF) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn unsupported_dir_sync(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Unsupported | ErrorKind::InvalidInput | ErrorKind::PermissionDenied
    )
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

pub(crate) fn chunk_ids(chunks: &[Chunk]) -> HashSet<String> {
    chunks.iter().map(|chunk| chunk.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::unsupported_dir_sync;
    use std::io::ErrorKind;

    #[test]
    fn provider_directory_sync_limitations_are_nonfatal() {
        assert!(unsupported_dir_sync(ErrorKind::Unsupported));
        assert!(unsupported_dir_sync(ErrorKind::InvalidInput));
        assert!(unsupported_dir_sync(ErrorKind::PermissionDenied));
        assert!(!unsupported_dir_sync(ErrorKind::NotFound));
        assert!(!unsupported_dir_sync(ErrorKind::WriteZero));
    }
}
