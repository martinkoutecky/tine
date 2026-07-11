use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use loro::VersionVector;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AffectedPage, CrdtError};

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
    session_dir: PathBuf,
}

impl Store {
    pub fn initialize(
        sync_root: &Path,
        device_id: Uuid,
        session_id: Uuid,
    ) -> Result<Self, CrdtError> {
        let root = store_root(sync_root);
        fs::create_dir_all(&root)?;
        let genesis_dir = root.join("genesis");
        fs::create_dir_all(&genesis_dir)?;

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

        let (workspace_id, resumed) = create_or_resume_genesis_claim(&root, device_id, session_id)?;
        let session_dir = create_session_dir(&root, device_id, session_id, resumed)?;
        Ok(Self {
            root,
            workspace_id,
            device_id,
            session_id,
            session_dir,
        })
    }

    pub fn open(
        sync_root: &Path,
        device_id: Uuid,
        session_id: Uuid,
    ) -> Result<(Self, Vec<Chunk>), CrdtError> {
        let root = store_root(sync_root);
        if !root.is_dir() {
            return Err(CrdtError::StoreNotInitialized);
        }

        let chunks = load_chunks_from(&root)?;
        let workspace_id = validate_chunk_set(&chunks)?;
        let session_dir = create_session_dir(&root, device_id, session_id, false)?;
        Ok((
            Self {
                root,
                workspace_id,
                device_id,
                session_id,
                session_dir,
            },
            chunks,
        ))
    }

    pub fn load_chunks(&self) -> Result<Vec<Chunk>, CrdtError> {
        let chunks = load_chunks_from(&self.root)?;
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
        collect_chunk_paths(&self.root, &mut paths)?;
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
            let chunk = read_chunk(&path)?;
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
        let header = ChunkHeader {
            schema_version: SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            encryption: EncryptionMode::None,
            kind,
            author_device_id: self.device_id,
            author_session_id: self.session_id,
            affected_pages,
        };
        let bytes = encode_envelope(&header, &payload)?;
        let id = digest_hex(&bytes);
        let target_dir = match kind {
            ChunkKind::Genesis => self.root.join("genesis"),
            ChunkKind::Update => self.session_dir.clone(),
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
        let dir = self
            .root
            .join("projections")
            .join(digest_hex(path.as_bytes()))
            .join(content_sha256);
        fs::create_dir_all(&dir)?;
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
        let dir = self
            .root
            .join("projections")
            .join(digest_hex(path.as_bytes()))
            .join(&content_sha256);
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("receipt")
            {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let filename = entry
                .path()
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
        frontier: VersionVector,
    ) -> Result<String, CrdtError> {
        let intent = ProjectionIntent {
            schema_version: SCHEMA_VERSION,
            workspace_id: self.workspace_id,
            encryption: EncryptionMode::None,
            path: path.to_string(),
            frontier,
        };
        let bytes = serde_json::to_vec(&intent)
            .map_err(|error| CrdtError::Serialization(error.to_string()))?;
        let id = digest_hex(&bytes);
        let dir = self
            .root
            .join("projection-intents")
            .join(digest_hex(path.as_bytes()));
        fs::create_dir_all(&dir)?;
        publish_immutable_named(&dir, &format!("{id}.intent"), &bytes)?;
        Ok(id)
    }

    pub fn is_projection_authorized(
        &self,
        path: &str,
        current: &VersionVector,
    ) -> Result<bool, CrdtError> {
        let dir = self
            .root
            .join("projection-intents")
            .join(digest_hex(path.as_bytes()));
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("intent")
            {
                continue;
            }
            let bytes = fs::read(entry.path())?;
            let filename = entry
                .path()
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
            if &intent.frontier == current {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn store_root(sync_root: &Path) -> PathBuf {
    sync_root.join(".tine-sync").join("v1")
}

fn create_session_dir(
    root: &Path,
    device_id: Uuid,
    session_id: Uuid,
    allow_existing: bool,
) -> Result<PathBuf, CrdtError> {
    let sessions = root
        .join("devices")
        .join(device_id.to_string())
        .join("sessions");
    fs::create_dir_all(&sessions)?;
    let session_dir = sessions.join(session_id.to_string());
    match fs::create_dir(&session_dir) {
        Ok(()) => {
            sync_dir_best_effort(&sessions)?;
            Ok(session_dir)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists && allow_existing => Ok(session_dir),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            Err(CrdtError::SessionAlreadyExists(session_id))
        }
        Err(error) => Err(error.into()),
    }
}

fn create_or_resume_genesis_claim(
    root: &Path,
    device_id: Uuid,
    session_id: Uuid,
) -> Result<(Uuid, bool), CrdtError> {
    let path = root.join("genesis.claim");
    let claim = GenesisClaim {
        schema_version: SCHEMA_VERSION,
        workspace_id: Uuid::new_v4(),
        device_id,
        session_id,
    };
    let bytes =
        serde_json::to_vec(&claim).map_err(|error| CrdtError::Serialization(error.to_string()))?;
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => {
            file.write_all(&bytes)?;
            file.sync_all()?;
            sync_dir_best_effort(root)?;
            Ok((claim.workspace_id, false))
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing: GenesisClaim =
                serde_json::from_reader(File::open(&path)?).map_err(|error| {
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

fn load_chunks_from(root: &Path) -> Result<Vec<Chunk>, CrdtError> {
    let mut paths = Vec::new();
    collect_chunk_paths(root, &mut paths)?;
    paths.sort();

    // The same immutable chunk may be delivered more than once in different
    // incoming directories. Collapse it by content ID after validating each copy.
    let mut chunks = BTreeMap::new();
    for path in paths {
        let chunk = read_chunk(&path)?;
        chunks.entry(chunk.id.clone()).or_insert(chunk);
    }
    Ok(chunks.into_values().collect())
}

fn collect_chunk_paths(dir: &Path, output: &mut Vec<PathBuf>) -> Result<(), CrdtError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_chunk_paths(&entry.path(), output)?;
        } else if file_type.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("chunk")
        {
            output.push(entry.path());
        }
    }
    Ok(())
}

fn read_chunk(path: &Path) -> Result<Chunk, CrdtError> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
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

fn publish_immutable(dir: &Path, id: &str, bytes: &[u8]) -> Result<(), CrdtError> {
    publish_immutable_named(dir, &format!("{id}.chunk"), bytes)
}

fn publish_immutable_named(dir: &Path, filename: &str, bytes: &[u8]) -> Result<(), CrdtError> {
    let final_path = dir.join(filename);
    if final_path.exists() {
        return verify_existing(&final_path, bytes);
    }

    let temp_path = dir.join(format!(".tmp-{}", Uuid::new_v4()));
    let mut temp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;

    let publish_result = (|| {
        temp.write_all(bytes)?;
        temp.sync_all()?;
        drop(temp);

        // SHA-256 is the integrity boundary: an existing target for this hash
        // must contain the same bytes. Recheck after fsync to narrow the race
        // before the provider-friendly atomic rename.
        if final_path.exists() {
            verify_existing(&final_path, bytes)
        } else {
            fs::rename(&temp_path, &final_path)?;
            sync_dir_best_effort(dir)
        }
    })();

    let remove_result = fs::remove_file(&temp_path);
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

fn verify_existing(path: &Path, expected: &[u8]) -> Result<(), CrdtError> {
    let mut existing = Vec::new();
    File::open(path)?.read_to_end(&mut existing)?;
    if existing == expected {
        Ok(())
    } else {
        Err(CrdtError::InvalidChunk(format!(
            "content-address collision at {}",
            path.display()
        )))
    }
}

fn sync_dir_best_effort(path: &Path) -> Result<(), CrdtError> {
    let result = File::open(path).and_then(|directory| directory.sync_all());
    match result {
        Ok(()) => Ok(()),
        Err(error) if unsupported_dir_sync(error.kind()) => Ok(()),
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
