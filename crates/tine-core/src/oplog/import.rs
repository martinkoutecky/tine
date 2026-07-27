//! Exact, read-only external inventory and conservative identity matching.
//!
//! This module plans reconciliation only. It does not publish semantic
//! operations, write a graph, consult SQLite, or activate managed sync.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::bootstrap_import::{
    ArchiveLocalFrontierBindingV1, BootstrapAggregateCommitV1, BootstrapAggregateManifestV1,
    BootstrapImportError, BootstrapImportPartEvidenceV1, BootstrapManifestFingerprintV1,
    BootstrapPartDescriptorV1, BootstrapPartSpanIndexV1, BootstrapPartitionProfileV1,
    FullObjectDescriptorV1, OperationDigestV1, OperationLeafV1, OperationRootV1,
    PayloadObjectDescriptorV1, PayloadObjectRootV1, SourceBlobChunkDescriptorV1,
    SourceBlobChunkDigestV1, SourceBlobChunkRootBuilderV1, SourceBlobChunkRootV1,
    SourceBlobIndexBuilderV1, SourceContentDigestV1, SourceInventoryIndexBuilderV1,
    SourceInventoryRootBuilderV1, SourceInventoryRootV1, SourceLeafDigestV1, SourceLeafV1,
    SourceSpanRootV1, SourceSpanV1, MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART, MAX_BOOTSTRAP_PARTS,
    MAX_OPERATIONS_PER_BOOTSTRAP_PART, MAX_PARSED_NODES_PER_SOURCE_FILE,
    MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART, MAX_SOURCE_FILE_BYTES, MAX_SOURCE_INDEX_PAGES,
    MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART, MAX_TOTAL_SOURCE_BYTES,
};
use super::external_import::{
    ExternalImportObservationEntry, ExternalImportObservationMaterial,
    ExternalImportObservationMaterialError, ExternalImportObservationState,
};
use super::hot_engine::{
    AcceptedFrontierRoot, AuthorBatch, DetachedBootstrapAcceptedEngineMaterial,
    DetachedBootstrapAuthoringSession, DetachedBootstrapCandidate, MAX_TRANSACTION_OPERATIONS,
};
use super::receipt::ImportIdDerivation;
use super::{
    plan_projection, AnnotatedIdentity, BatchId, BatchOrigin, BlobDescription, BlockId,
    BlockLocation, ContentDigest, CrdtPeerId, CurrentPageAtPath, DeviceId, DocumentId, ImportId,
    ImportInventoryEntry, ImportInventoryState, ImportLocator, LineageDigest, LogicalCompletionId,
    LogicalPageName, LogseqIdentityMutation, LogseqUuid, ManagedPath, ManagedTextKind, ObjectKind,
    OperationTransaction, PageId, ProjectionCompletedReceipt, ProjectionCompletion,
    ProjectionIntent, ProjectionReceiptStore, ProjectionStoreError, ReferenceCatalogPolicyV1,
    SemanticOperation, SessionId, ShardedHotEngine, StructuralLocator, StructuralSpan, WorkspaceId,
    DIFF_SCHEMA_VERSION,
};
use crate::model::{
    path_is_sync_conflict, BootstrapSourceCapture, BootstrapSourceCaptureInstrumentation,
    BootstrapSourceChunk, BootstrapSourceEntry, Graph, PageKind,
};

#[cfg(test)]
thread_local! {
    static SNAPSHOT_REVALIDATION_HOOK:
        std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
    static POST_FRONTIER_OVERRIDE:
        std::cell::RefCell<Option<AcceptedFrontierRoot>> = const { std::cell::RefCell::new(None) };
}

/// The 1M-block program target is expected to fit below these aggregate
/// ceilings for ordinary shallow documents. Inputs beyond them remain exact
/// raw evidence but are not parsed into an authoritative import plan.
pub const MAX_IMPORT_FILES: usize = 1_000_000;
pub const MAX_IMPORT_RAW_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_PARSED_NODES: usize = 2_000_000;
pub const MAX_IMPORT_DEPTH: usize = 256;
pub const MAX_IMPORT_LOCATOR_COMPONENTS: usize = 16_000_000;
pub const MAX_IMPORT_CATALOG_ENTRIES: usize = 2_000_000;
pub const MAX_IMPORT_PATH_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_REPLAY_ENTRIES: usize = 1_000_000;
pub const MAX_IMPORT_REPLAY_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_RENDERED_TARGET_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_IMPORT_STRUCTURAL_KEY_WORK: usize = 64_000_000;

const BOOTSTRAP_STREAM_SORT_BUFFER_BYTES: usize = 1024 * 1024;
const BOOTSTRAP_STREAM_SORT_FAN_IN: usize = 4;
const BOOTSTRAP_STREAM_MAX_SORT_RUNS: usize = 4096;
const BOOTSTRAP_STREAM_FRAME_BYTES: usize = 64 * 1024 * 1024 + 1024 * 1024;
const BOOTSTRAP_STREAM_DIRECTORY: &str = "inactive-bootstrap-publication-v1";
const BOOTSTRAP_STREAM_SEAL: &str = "sealed.commit";
const BOOTSTRAP_STREAM_AGGREGATE: &str = "aggregate.bin";
const BOOTSTRAP_STREAM_COMMIT: &str = "commit.bin";
const BOOTSTRAP_STREAM_INVENTORY_PAGES: &str = "source-inventory-pages";
const BOOTSTRAP_STREAM_BLOB_PAGES: &str = "source-blob-pages";
const BOOTSTRAP_STREAM_PARTS: &str = "parts";
const BOOTSTRAP_STREAM_PART_MANIFEST: &str = "manifest.bin";
const BOOTSTRAP_STREAM_PART_EVIDENCE: &str = "evidence.bin";
const BOOTSTRAP_STREAM_PART_SPANS: &str = "spans.bin";
const BOOTSTRAP_STREAM_PART_OBJECTS: &str = "objects.frames";
const BOOTSTRAP_STREAM_OPERATION_SPOOL: &str = "operations.sorted";
const BOOTSTRAP_STREAM_BOUNDARY_SPOOL: &str = "part-boundaries.frames";
const BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES: usize = 768 * 1024;
/// Fixed per-operation allowance for the before/after and membership fields
/// added by the existing semantic-effect encoder. The operation's own
/// canonical bytes are charged separately. Exact prepared bytes are still
/// checked before the authoritative detached pass.
const BOOTSTRAP_STREAM_SEMANTIC_EFFECT_OVERHEAD: u64 = 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct BootstrapStreamingImportInstrumentation {
    pub(crate) source_files: u64,
    pub(crate) source_chunks: u64,
    pub(crate) source_bytes: u64,
    pub(crate) source_bytes_read: u64,
    pub(crate) parser_nodes: u64,
    pub(crate) operations: u64,
    pub(crate) source_spans: u64,
    pub(crate) parts: u32,
    pub(crate) operation_spool_bytes: u64,
    pub(crate) prepared_bytes: u64,
    pub(crate) external_sort_runs: u64,
    pub(crate) capture_passes: u64,
    pub(crate) peak_owned_source_bytes: u64,
    pub(crate) peak_owned_parser_nodes: u64,
    pub(crate) peak_owned_part_operations: u64,
    pub(crate) peak_owned_part_bytes: u64,
    pub(crate) peak_owned_sort_buffer_bytes: u64,
}

#[derive(Debug)]
pub(crate) enum BootstrapStreamingImportError {
    Io(io::Error),
    Protocol(BootstrapImportError),
    InvalidSource(String),
    InvalidOperation(String),
    ResourceLimit {
        resource: &'static str,
        observed: u64,
        limit: u64,
    },
    SingletonOverLimit(&'static str),
    ConflictingSeal,
}

impl fmt::Display for BootstrapStreamingImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Protocol(error) => error.fmt(formatter),
            Self::InvalidSource(detail) | Self::InvalidOperation(detail) => {
                formatter.write_str(detail)
            }
            Self::ResourceLimit {
                resource,
                observed,
                limit,
            } => write!(
                formatter,
                "{resource} limit exceeded: observed {observed}, limit {limit}"
            ),
            Self::SingletonOverLimit(resource) => {
                write!(
                    formatter,
                    "one bootstrap operation cannot fit the {resource} limit"
                )
            }
            Self::ConflictingSeal => {
                formatter.write_str("conflicting sealed bootstrap preparation")
            }
        }
    }
}

impl std::error::Error for BootstrapStreamingImportError {}

impl From<io::Error> for BootstrapStreamingImportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<BootstrapImportError> for BootstrapStreamingImportError {
    fn from(error: BootstrapImportError) -> Self {
        Self::Protocol(error)
    }
}

/// Inactive ownership of a complete, sealed bootstrap preparation. It carries
/// no object-store, history, graph-writer, projection, SQLite, enrollment, or
/// runtime capability.
#[allow(dead_code)]
pub(crate) struct InactiveBootstrapPreparedPublication {
    source_capture: BootstrapSourceCapture,
    sealed_directory: PathBuf,
    aggregate: BootstrapAggregateManifestV1,
    commit: BootstrapAggregateCommitV1,
    candidate: Box<DetachedBootstrapCandidate>,
    engine_materials: Vec<DetachedBootstrapAcceptedEngineMaterial>,
    instrumentation: BootstrapStreamingImportInstrumentation,
}

#[allow(dead_code)]
impl InactiveBootstrapPreparedPublication {
    pub(crate) const fn aggregate(&self) -> &BootstrapAggregateManifestV1 {
        &self.aggregate
    }

    pub(crate) const fn commit(&self) -> BootstrapAggregateCommitV1 {
        self.commit
    }

    pub(crate) const fn candidate(&self) -> &DetachedBootstrapCandidate {
        &self.candidate
    }

    pub(crate) fn engine_materials(&self) -> &[DetachedBootstrapAcceptedEngineMaterial] {
        &self.engine_materials
    }

    pub(crate) const fn instrumentation(&self) -> &BootstrapStreamingImportInstrumentation {
        &self.instrumentation
    }

    pub(crate) const fn source_capture(&self) -> &BootstrapSourceCapture {
        &self.source_capture
    }

    pub(crate) fn aggregate_bytes(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        Ok(read_bounded_file(
            &self.sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
            super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES,
        )?)
    }

    pub(crate) fn commit_bytes(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        Ok(read_bounded_file(
            &self.sealed_directory.join(BOOTSTRAP_STREAM_COMMIT),
            super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES,
        )?)
    }

    pub(crate) fn source_inventory_page(
        &self,
        ordinal: u32,
    ) -> Result<super::bootstrap_import::SourceInventoryIndexPageV1, BootstrapStreamingImportError>
    {
        let bytes = read_bounded_file(
            &numbered_path(
                &self.sealed_directory.join(BOOTSTRAP_STREAM_INVENTORY_PAGES),
                ordinal,
            ),
            super::bootstrap_import::MAX_SOURCE_INDEX_PAGE_BYTES,
        )?;
        super::bootstrap_import::SourceInventoryIndexPageV1::decode(&bytes).map_err(Into::into)
    }

    pub(crate) fn source_blob_page(
        &self,
        ordinal: u32,
    ) -> Result<super::bootstrap_import::SourceBlobIndexPageV1, BootstrapStreamingImportError> {
        let bytes = read_bounded_file(
            &numbered_path(
                &self.sealed_directory.join(BOOTSTRAP_STREAM_BLOB_PAGES),
                ordinal,
            ),
            super::bootstrap_import::MAX_SOURCE_INDEX_PAGE_BYTES,
        )?;
        super::bootstrap_import::SourceBlobIndexPageV1::decode(&bytes).map_err(Into::into)
    }

    pub(crate) fn open_part(
        &self,
        ordinal: u32,
    ) -> Result<InactiveBootstrapPreparedPartCursor, BootstrapStreamingImportError> {
        if ordinal >= self.aggregate.parts().len() as u32 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "bootstrap part ordinal is outside the sealed aggregate".into(),
            ));
        }
        let directory = self
            .sealed_directory
            .join(BOOTSTRAP_STREAM_PARTS)
            .join(format!("{ordinal:08}"));
        let manifest = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_MANIFEST),
            BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES,
        )?;
        let evidence = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_EVIDENCE),
            super::bootstrap_import::MAX_BOOTSTRAP_PART_EVIDENCE_BYTES,
        )?;
        let spans = read_bounded_file(
            &directory.join(BOOTSTRAP_STREAM_PART_SPANS),
            super::bootstrap_import::MAX_PART_SPAN_INDEX_BYTES,
        )?;
        Ok(InactiveBootstrapPreparedPartCursor {
            ordinal,
            manifest,
            evidence,
            spans,
            objects: FrameReader::open(
                &directory.join(BOOTSTRAP_STREAM_PART_OBJECTS),
                super::batch::MAX_OBJECT_BYTES,
            )?,
        })
    }
}

#[allow(dead_code)]
pub(crate) struct InactiveBootstrapPreparedPartCursor {
    ordinal: u32,
    manifest: Vec<u8>,
    evidence: Vec<u8>,
    spans: Vec<u8>,
    objects: FrameReader,
}

#[allow(dead_code)]
impl InactiveBootstrapPreparedPartCursor {
    pub(crate) const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub(crate) fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }

    pub(crate) fn evidence(
        &self,
    ) -> Result<BootstrapImportPartEvidenceV1, BootstrapStreamingImportError> {
        BootstrapImportPartEvidenceV1::decode(&self.evidence).map_err(Into::into)
    }

    pub(crate) fn span_index(
        &self,
    ) -> Result<BootstrapPartSpanIndexV1, BootstrapStreamingImportError> {
        BootstrapPartSpanIndexV1::decode(&self.spans).map_err(Into::into)
    }

    pub(crate) fn next_object_bytes(
        &mut self,
    ) -> Result<Option<Vec<u8>>, BootstrapStreamingImportError> {
        self.objects.next().map_err(Into::into)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SortRecord {
    key: Vec<u8>,
    value: Vec<u8>,
}

struct ExternalSort {
    directory: PathBuf,
    label: String,
    buffer: Vec<SortRecord>,
    buffer_bytes: usize,
    runs: Vec<PathBuf>,
    next_run: usize,
    total_bytes: u64,
    total_runs: u64,
    peak_buffer_bytes: u64,
}

impl ExternalSort {
    fn new(directory: &Path, label: &str) -> Result<Self, BootstrapStreamingImportError> {
        let directory = directory.join(format!("{label}-sort"));
        create_private_directory(&directory)?;
        Ok(Self {
            directory,
            label: label.to_owned(),
            buffer: Vec::new(),
            buffer_bytes: 0,
            runs: Vec::new(),
            next_run: 0,
            total_bytes: 0,
            total_runs: 0,
            peak_buffer_bytes: 0,
        })
    }

    fn push(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), BootstrapStreamingImportError> {
        let bytes = key
            .len()
            .checked_add(value.len())
            .and_then(|bytes| bytes.checked_add(8))
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "external-sort record length overflow".into(),
                )
            })?;
        if bytes > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "external-sort record bytes",
                observed: bytes as u64,
                limit: BOOTSTRAP_STREAM_FRAME_BYTES as u64,
            });
        }
        if !self.buffer.is_empty()
            && self.buffer_bytes.saturating_add(bytes) > BOOTSTRAP_STREAM_SORT_BUFFER_BYTES
        {
            self.flush()?;
        }
        self.buffer.push(SortRecord { key, value });
        self.buffer_bytes = self.buffer_bytes.saturating_add(bytes);
        self.peak_buffer_bytes = self.peak_buffer_bytes.max(self.buffer_bytes as u64);
        if self.buffer_bytes >= BOOTSTRAP_STREAM_SORT_BUFFER_BYTES {
            self.flush()?;
        }
        Ok(())
    }

    fn finish(
        mut self,
        destination: &Path,
    ) -> Result<ExternalSortReceipt, BootstrapStreamingImportError> {
        self.flush()?;
        if self.runs.is_empty() {
            write_exact_new(destination, &[])?;
        } else {
            while self.runs.len() > 1 {
                let current = std::mem::take(&mut self.runs);
                for group in current.chunks(BOOTSTRAP_STREAM_SORT_FAN_IN) {
                    let output = self.next_run_path("merge");
                    merge_sort_runs(group, &output)?;
                    self.total_runs = self.total_runs.saturating_add(1);
                    if self.total_runs as usize > BOOTSTRAP_STREAM_MAX_SORT_RUNS {
                        return Err(BootstrapStreamingImportError::ResourceLimit {
                            resource: "external-sort runs",
                            observed: self.total_runs,
                            limit: BOOTSTRAP_STREAM_MAX_SORT_RUNS as u64,
                        });
                    }
                    self.runs.push(output);
                }
                for path in current {
                    fs::remove_file(path)?;
                }
            }
            fs::rename(&self.runs[0], destination)?;
            sync_parent(destination)?;
        }
        Ok(ExternalSortReceipt {
            bytes: self.total_bytes,
            runs: self.total_runs,
            peak_buffer_bytes: self.peak_buffer_bytes,
        })
    }

    fn flush(&mut self) -> Result<(), BootstrapStreamingImportError> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        self.buffer.sort_unstable_by(|left, right| {
            (&left.key, &left.value).cmp(&(&right.key, &right.value))
        });
        let path = self.next_run_path("run");
        let mut writer = BufWriter::new(create_new_file(&path)?);
        for record in self.buffer.drain(..) {
            self.total_bytes = self
                .total_bytes
                .checked_add(write_sort_record(&mut writer, &record)?)
                .ok_or_else(|| {
                    BootstrapStreamingImportError::InvalidOperation(
                        "external-sort byte count overflow".into(),
                    )
                })?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
        self.runs.push(path);
        self.total_runs = self.total_runs.saturating_add(1);
        if self.total_runs as usize > BOOTSTRAP_STREAM_MAX_SORT_RUNS {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "external-sort runs",
                observed: self.total_runs,
                limit: BOOTSTRAP_STREAM_MAX_SORT_RUNS as u64,
            });
        }
        self.buffer_bytes = 0;
        Ok(())
    }

    fn next_run_path(&mut self, kind: &str) -> PathBuf {
        let path = self
            .directory
            .join(format!("{}-{kind}-{:08}", self.label, self.next_run));
        self.next_run += 1;
        path
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ExternalSortReceipt {
    bytes: u64,
    runs: u64,
    peak_buffer_bytes: u64,
}

struct SortRecordReader {
    reader: BufReader<File>,
}

impl SortRecordReader {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, File::open(path)?),
        })
    }

    fn next(&mut self) -> io::Result<Option<SortRecord>> {
        let Some(key_length) = read_optional_u32(&mut self.reader)? else {
            return Ok(None);
        };
        let value_length = read_u32_io(&mut self.reader)?;
        let total = usize::try_from(key_length)
            .ok()
            .and_then(|key| {
                usize::try_from(value_length)
                    .ok()
                    .and_then(|value| key.checked_add(value))
            })
            .ok_or_else(|| invalid_bootstrap_data("external-sort record length overflow"))?;
        if total > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(invalid_bootstrap_data(
                "external-sort record exceeds its bounded frame",
            ));
        }
        let mut key = vec![0; key_length as usize];
        let mut value = vec![0; value_length as usize];
        self.reader.read_exact(&mut key)?;
        self.reader.read_exact(&mut value)?;
        Ok(Some(SortRecord { key, value }))
    }
}

fn merge_sort_runs(inputs: &[PathBuf], output: &Path) -> Result<(), BootstrapStreamingImportError> {
    let mut readers = inputs
        .iter()
        .map(|path| SortRecordReader::open(path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut heads = readers
        .iter_mut()
        .map(SortRecordReader::next)
        .collect::<Result<Vec<_>, _>>()?;
    let mut writer = BufWriter::new(create_new_file(output)?);
    loop {
        let next = heads
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.as_ref().map(|record| (index, record)))
            .min_by(|(_, left), (_, right)| {
                (&left.key, &left.value).cmp(&(&right.key, &right.value))
            })
            .map(|(index, _)| index);
        let Some(index) = next else {
            break;
        };
        let record = heads[index]
            .take()
            .expect("selected external-sort input has a record");
        write_sort_record(&mut writer, &record)?;
        heads[index] = readers[index].next()?;
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    Ok(())
}

fn write_sort_record(
    writer: &mut impl Write,
    record: &SortRecord,
) -> Result<u64, BootstrapStreamingImportError> {
    let key_length = u32::try_from(record.key.len()).map_err(|_| {
        BootstrapStreamingImportError::InvalidOperation(
            "external-sort key length cannot be represented".into(),
        )
    })?;
    let value_length = u32::try_from(record.value.len()).map_err(|_| {
        BootstrapStreamingImportError::InvalidOperation(
            "external-sort value length cannot be represented".into(),
        )
    })?;
    writer.write_all(&key_length.to_be_bytes())?;
    writer.write_all(&value_length.to_be_bytes())?;
    writer.write_all(&record.key)?;
    writer.write_all(&record.value)?;
    Ok(8 + u64::from(key_length) + u64::from(value_length))
}

pub(crate) struct FrameReader {
    reader: BufReader<File>,
    max_frame: usize,
}

impl FrameReader {
    fn open(path: &Path, max_frame: usize) -> io::Result<Self> {
        Ok(Self {
            reader: BufReader::with_capacity(64 * 1024, File::open(path)?),
            max_frame,
        })
    }

    fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        let Some(length) = read_optional_u32(&mut self.reader)? else {
            return Ok(None);
        };
        let length = length as usize;
        if length > self.max_frame {
            return Err(invalid_bootstrap_data(
                "sealed bootstrap frame exceeds its declared bound",
            ));
        }
        let mut bytes = vec![0; length];
        self.reader.read_exact(&mut bytes)?;
        Ok(Some(bytes))
    }
}

fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> io::Result<u64> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| invalid_bootstrap_data("bootstrap frame length cannot be represented"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(bytes)?;
    Ok(4 + u64::from(length))
}

fn read_optional_u32(reader: &mut impl Read) -> io::Result<Option<u32>> {
    let mut bytes = [0; 4];
    let mut read = 0;
    while read != bytes.len() {
        let current = reader.read(&mut bytes[read..])?;
        if current == 0 {
            return if read == 0 {
                Ok(None)
            } else {
                Err(invalid_bootstrap_data("truncated bootstrap frame length"))
            };
        }
        read += current;
    }
    Ok(Some(u32::from_be_bytes(bytes)))
}

fn read_u32_io(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn invalid_bootstrap_data(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(invalid_bootstrap_data(
            "bootstrap scratch path is not a real directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path)?;
            sync_parent(path)
        }
        Err(error) => Err(error),
    }
}

fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn write_exact_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_new_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    sync_parent(path)
}

fn publish_exact_file(path: &Path, bytes: &[u8]) -> Result<(), BootstrapStreamingImportError> {
    match create_new_file(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
            sync_parent(path)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if read_bounded_file(path, bytes.len())? == bytes {
                Ok(())
            } else {
                Err(BootstrapStreamingImportError::ConflictingSeal)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn read_bounded_file(path: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact is not a regular no-follow file",
        ));
    }
    let length = usize::try_from(metadata.len())
        .map_err(|_| invalid_bootstrap_data("bootstrap artifact length cannot be represented"))?;
    if length > max_bytes {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact exceeds its bounded length",
        ));
    }
    let mut bytes = Vec::with_capacity(length);
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.len() != length {
        return Err(invalid_bootstrap_data(
            "bootstrap artifact changed while it was read",
        ));
    }
    Ok(bytes)
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_bootstrap_data("bootstrap path has no parent"))?;
    File::open(parent)?.sync_all()
}

fn numbered_path(directory: &Path, ordinal: u32) -> PathBuf {
    directory.join(format!("{ordinal:08}.bin"))
}

struct BootstrapSourceReader<'a> {
    capture: &'a BootstrapSourceCapture,
    chunks: crate::model::BootstrapSourceChunkCursor,
    next_chunk: Option<BootstrapSourceChunk>,
}

impl<'a> BootstrapSourceReader<'a> {
    fn new(capture: &'a BootstrapSourceCapture) -> io::Result<Self> {
        let mut chunks = capture.chunks_cursor()?;
        let next_chunk = chunks.next()?;
        Ok(Self {
            capture,
            chunks,
            next_chunk,
        })
    }

    fn read_entry(
        &mut self,
        entry: &BootstrapSourceEntry,
        instrumentation: &mut BootstrapStreamingImportInstrumentation,
    ) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        let declared = entry.description().byte_length();
        if declared > MAX_SOURCE_FILE_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "source file bytes",
                observed: declared,
                limit: MAX_SOURCE_FILE_BYTES,
            });
        }
        let capacity = usize::try_from(declared).map_err(|_| {
            BootstrapStreamingImportError::InvalidSource(
                "source file length cannot be represented".into(),
            )
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        for ordinal in 0..entry.chunk_count() {
            let chunk = self.next_chunk.take().ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source {} is missing chunk {ordinal}",
                    entry.path()
                ))
            })?;
            if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
                return Err(BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source chunk order differs for {}",
                    entry.path()
                )));
            }
            let mut reader = self.capture.open_chunk(&chunk)?;
            reader.read_to_end(&mut bytes)?;
            reader.finish()?;
            self.next_chunk = self.chunks.next()?;
        }
        if bytes.len() as u64 != declared || BlobDescription::of(&bytes) != entry.description() {
            return Err(BootstrapStreamingImportError::InvalidSource(format!(
                "sealed source bytes differ for {}",
                entry.path()
            )));
        }
        instrumentation.source_bytes_read = instrumentation
            .source_bytes_read
            .checked_add(declared)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(
                    "source byte instrumentation overflow".into(),
                )
            })?;
        instrumentation.peak_owned_source_bytes =
            instrumentation.peak_owned_source_bytes.max(declared);
        Ok(bytes)
    }

    fn finish(self) -> Result<(), BootstrapStreamingImportError> {
        if self.next_chunk.is_some() {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "sealed source capture has trailing chunks".into(),
            ));
        }
        Ok(())
    }
}

struct BootstrapSourceProtocolPreparation {
    import_id: ImportId,
    inventory_root: SourceInventoryRootV1,
    inventory_page_count: u32,
    blob_root: SourceBlobChunkRootV1,
    blob_page_count: u32,
    source_count: u32,
}

fn prepare_bootstrap_source_protocol(
    workspace_id: WorkspaceId,
    capture: &BootstrapSourceCapture,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<BootstrapSourceProtocolPreparation, BootstrapStreamingImportError> {
    let source_count = u32::try_from(capture.source_file_count()).map_err(|_| {
        BootstrapStreamingImportError::ResourceLimit {
            resource: "source files",
            observed: capture.source_file_count(),
            limit: super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES as u64,
        }
    })?;
    if source_count > super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source files",
            observed: u64::from(source_count),
            limit: super::bootstrap_import::MAX_SOURCE_INVENTORY_LEAVES as u64,
        });
    }
    if capture.source_chunk_count() > u64::from(super::bootstrap_import::MAX_SOURCE_BLOB_CHUNKS) {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source chunks",
            observed: capture.source_chunk_count(),
            limit: super::bootstrap_import::MAX_SOURCE_BLOB_CHUNKS as u64,
        });
    }

    let blob_sorted = working.join("source-blob-records.sorted");
    let names_sorted = working.join("logical-page-names.sorted");
    let mut blob_sort = ExternalSort::new(working, "source-blobs")?;
    let mut names_sort = ExternalSort::new(working, "logical-names")?;
    let mut inventory_root = SourceInventoryRootBuilderV1::new();
    let mut derivation =
        ImportIdDerivation::new(workspace_id, 0, source_count as usize, DIFF_SCHEMA_VERSION)
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
    derivation
        .begin_inventory()
        .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;

    let mut entries = capture.entries_cursor()?;
    let mut chunks = capture.chunks_cursor()?;
    let mut next_chunk = chunks.next()?;
    let mut observed_sources = 0_u32;
    let mut total_source_bytes = 0_u64;
    while let Some(entry) = entries.next()? {
        observed_sources = observed_sources.checked_add(1).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidSource("source count overflow".into())
        })?;
        let description = entry.description();
        total_source_bytes = total_source_bytes
            .checked_add(description.byte_length())
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource("total source bytes overflow".into())
            })?;
        if total_source_bytes > MAX_TOTAL_SOURCE_BYTES {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "total source bytes",
                observed: total_source_bytes,
                limit: MAX_TOTAL_SOURCE_BYTES,
            });
        }
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*description.sha256()),
            description.byte_length(),
        )?;
        inventory_root.push(&leaf)?;
        derivation
            .push_inventory(&ImportInventoryEntry::with_kind(
                entry.kind(),
                entry.path().clone(),
                ImportInventoryState::Present(description),
            ))
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;

        let logical_name = LogicalPageName::parse(entry.logical_name().to_owned())
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        names_sort.push(
            logical_name.key_digest().as_bytes().to_vec(),
            entry.path().as_str().as_bytes().to_vec(),
        )?;

        let leaf_digest = leaf.digest();
        let mut marker_key = leaf_digest.as_bytes().to_vec();
        marker_key.push(0);
        blob_sort.push(marker_key, leaf.encode())?;
        let mut offset = 0_u64;
        for ordinal in 0..entry.chunk_count() {
            let chunk = next_chunk.take().ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed source {} is missing chunk {ordinal}",
                    entry.path()
                ))
            })?;
            if chunk.path() != entry.path() || chunk.ordinal() != ordinal {
                return Err(BootstrapStreamingImportError::InvalidSource(format!(
                    "sealed chunk order differs for {}",
                    entry.path()
                )));
            }
            let byte_length = u32::try_from(chunk.description().byte_length()).map_err(|_| {
                BootstrapStreamingImportError::InvalidSource(
                    "source chunk length cannot be represented".into(),
                )
            })?;
            let descriptor = SourceBlobChunkDescriptorV1::new(
                leaf_digest,
                ordinal,
                entry.chunk_count(),
                offset,
                byte_length,
                SourceBlobChunkDigestV1::from_bytes(*chunk.description().sha256()),
            )?;
            let mut descriptor_key = leaf_digest.as_bytes().to_vec();
            descriptor_key.push(1);
            descriptor_key.extend_from_slice(&ordinal.to_be_bytes());
            blob_sort.push(descriptor_key, descriptor.encode())?;
            offset = offset.checked_add(u64::from(byte_length)).ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource("source chunk offset overflow".into())
            })?;
            next_chunk = chunks.next()?;
        }
        if offset != description.byte_length() {
            return Err(BootstrapStreamingImportError::InvalidSource(format!(
                "sealed chunk lengths do not cover {}",
                entry.path()
            )));
        }
    }
    if observed_sources != source_count || next_chunk.is_some() {
        return Err(BootstrapStreamingImportError::InvalidSource(
            "sealed source entry/chunk counts differ from the capture".into(),
        ));
    }
    let inventory_root = inventory_root.finish();
    let import_id = derivation
        .finish()
        .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
    let blob_receipt = blob_sort.finish(&blob_sorted)?;
    let name_receipt = names_sort.finish(&names_sorted)?;
    record_sort_receipt(instrumentation, blob_receipt);
    record_sort_receipt(instrumentation, name_receipt);
    validate_unique_logical_names(&names_sorted)?;

    let blob_root = build_source_blob_root(&blob_sorted)?;
    let inventory_pages = working.join(BOOTSTRAP_STREAM_INVENTORY_PAGES);
    let blob_pages = working.join(BOOTSTRAP_STREAM_BLOB_PAGES);
    create_private_directory(&inventory_pages)?;
    create_private_directory(&blob_pages)?;
    let inventory_page_count =
        write_source_inventory_pages(capture, inventory_root, &inventory_pages)?;
    let blob_page_count = write_source_blob_pages(&blob_sorted, blob_root, &blob_pages)?;
    instrumentation.source_files = u64::from(source_count);
    instrumentation.source_chunks = capture.source_chunk_count();
    instrumentation.source_bytes = total_source_bytes;
    Ok(BootstrapSourceProtocolPreparation {
        import_id,
        inventory_root,
        inventory_page_count,
        blob_root,
        blob_page_count,
        source_count,
    })
}

fn record_sort_receipt(
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
    receipt: ExternalSortReceipt,
) {
    instrumentation.operation_spool_bytes = instrumentation
        .operation_spool_bytes
        .saturating_add(receipt.bytes);
    instrumentation.external_sort_runs = instrumentation
        .external_sort_runs
        .saturating_add(receipt.runs);
    instrumentation.peak_owned_sort_buffer_bytes = instrumentation
        .peak_owned_sort_buffer_bytes
        .max(receipt.peak_buffer_bytes);
}

fn validate_unique_logical_names(path: &Path) -> Result<(), BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(path)?;
    let mut previous: Option<(Vec<u8>, Vec<u8>)> = None;
    while let Some(record) = reader.next()? {
        if let Some((key, first_path)) = &previous {
            if *key == record.key {
                return Err(BootstrapStreamingImportError::InvalidSource(format!(
                    "captured paths {:?} and {:?} decode to the same logical page name",
                    String::from_utf8_lossy(first_path),
                    String::from_utf8_lossy(&record.value)
                )));
            }
        }
        previous = Some((record.key, record.value));
    }
    Ok(())
}

fn build_source_blob_root(
    sorted: &Path,
) -> Result<SourceBlobChunkRootV1, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(sorted)?;
    let mut builder = SourceBlobChunkRootBuilderV1::new();
    while let Some(record) = reader.next()? {
        match record.key.get(32).copied() {
            Some(0) => builder.begin_source(&SourceLeafV1::decode(&record.value)?)?,
            Some(1) => builder.push(SourceBlobChunkDescriptorV1::decode(&record.value)?)?,
            _ => {
                return Err(BootstrapStreamingImportError::InvalidSource(
                    "invalid source-blob sort record".into(),
                ))
            }
        }
    }
    builder.finish().map_err(Into::into)
}

fn write_source_inventory_pages(
    capture: &BootstrapSourceCapture,
    root: SourceInventoryRootV1,
    directory: &Path,
) -> Result<u32, BootstrapStreamingImportError> {
    let mut builder = SourceInventoryIndexBuilderV1::new(root);
    let mut entries = capture.entries_cursor()?;
    let mut pages = 0_u32;
    while let Some(entry) = entries.next()? {
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*entry.description().sha256()),
            entry.description().byte_length(),
        )?;
        if let Some(page) = builder.push(leaf)? {
            write_exact_new(
                &numbered_path(directory, page.page_ordinal()),
                &page.encode()?,
            )?;
            pages += 1;
        }
    }
    if let Some(page) = builder.finish()? {
        write_exact_new(
            &numbered_path(directory, page.page_ordinal()),
            &page.encode()?,
        )?;
        pages += 1;
    }
    if pages > MAX_SOURCE_INDEX_PAGES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source inventory index pages",
            observed: u64::from(pages),
            limit: u64::from(MAX_SOURCE_INDEX_PAGES),
        });
    }
    Ok(pages)
}

fn write_source_blob_pages(
    sorted: &Path,
    root: SourceBlobChunkRootV1,
    directory: &Path,
) -> Result<u32, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(sorted)?;
    let mut builder = SourceBlobIndexBuilderV1::new(root);
    let mut pages = 0_u32;
    while let Some(record) = reader.next()? {
        if record.key.get(32) != Some(&1) {
            continue;
        }
        if let Some(page) = builder.push(SourceBlobChunkDescriptorV1::decode(&record.value)?)? {
            write_exact_new(
                &numbered_path(directory, page.page_ordinal()),
                &page.encode()?,
            )?;
            pages += 1;
        }
    }
    if let Some(page) = builder.finish()? {
        write_exact_new(
            &numbered_path(directory, page.page_ordinal()),
            &page.encode()?,
        )?;
        pages += 1;
    }
    if pages > MAX_SOURCE_INDEX_PAGES {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "source blob index pages",
            observed: u64::from(pages),
            limit: u64::from(MAX_SOURCE_INDEX_PAGES),
        });
    }
    Ok(pages)
}

#[derive(Clone)]
struct BootstrapOperationRecord {
    operation: SemanticOperation,
    source_leaf: SourceLeafDigestV1,
    source_offset: u64,
    source_length: u64,
    canonical_bytes: Vec<u8>,
}

impl BootstrapOperationRecord {
    fn new(
        operation: SemanticOperation,
        source_leaf: SourceLeafDigestV1,
        source_span: Option<StructuralSpan>,
    ) -> Result<Self, BootstrapStreamingImportError> {
        let canonical_bytes = postcard::to_allocvec(&operation).map_err(|error| {
            BootstrapStreamingImportError::InvalidOperation(format!(
                "cannot encode bootstrap semantic operation: {error}"
            ))
        })?;
        if canonical_bytes.len() > BOOTSTRAP_STREAM_FRAME_BYTES {
            return Err(BootstrapStreamingImportError::SingletonOverLimit(
                "operation record bytes",
            ));
        }
        let (source_offset, source_length) = source_span
            .map(|span| (span.start(), span.end().saturating_sub(span.start())))
            .unwrap_or((0, 0));
        Ok(Self {
            operation,
            source_leaf,
            source_offset,
            source_length,
            canonical_bytes,
        })
    }

    fn encode(&self) -> Result<Vec<u8>, BootstrapStreamingImportError> {
        let operation_length = u32::try_from(self.canonical_bytes.len()).map_err(|_| {
            BootstrapStreamingImportError::InvalidOperation(
                "operation record length cannot be represented".into(),
            )
        })?;
        let mut bytes = Vec::with_capacity(52 + self.canonical_bytes.len());
        bytes.extend_from_slice(self.source_leaf.as_bytes());
        bytes.extend_from_slice(&self.source_offset.to_be_bytes());
        bytes.extend_from_slice(&self.source_length.to_be_bytes());
        bytes.extend_from_slice(&operation_length.to_be_bytes());
        bytes.extend_from_slice(&self.canonical_bytes);
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, BootstrapStreamingImportError> {
        if bytes.len() < 52 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "truncated operation spool record".into(),
            ));
        }
        let source_leaf = SourceLeafDigestV1::from_bytes(
            bytes[..32]
                .try_into()
                .expect("checked operation record digest length"),
        );
        let source_offset = u64::from_be_bytes(
            bytes[32..40]
                .try_into()
                .expect("checked operation record offset length"),
        );
        let source_length = u64::from_be_bytes(
            bytes[40..48]
                .try_into()
                .expect("checked operation record span length"),
        );
        let operation_length = u32::from_be_bytes(
            bytes[48..52]
                .try_into()
                .expect("checked operation record payload length"),
        ) as usize;
        if operation_length != bytes.len() - 52 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "operation spool record length differs".into(),
            ));
        }
        source_offset.checked_add(source_length).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "operation source span overflows".into(),
            )
        })?;
        let canonical_bytes = bytes[52..].to_vec();
        let operation = postcard::from_bytes(&canonical_bytes).map_err(|error| {
            BootstrapStreamingImportError::InvalidOperation(format!(
                "cannot decode bootstrap semantic operation: {error}"
            ))
        })?;
        if postcard::to_allocvec(&operation)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?
            != canonical_bytes
        {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "operation spool record is not canonical".into(),
            ));
        }
        Ok(Self {
            operation,
            source_leaf,
            source_offset,
            source_length,
            canonical_bytes,
        })
    }

    fn operation_leaf(&self) -> Result<OperationLeafV1, BootstrapStreamingImportError> {
        OperationLeafV1::new(
            OperationDigestV1::from_bytes(*ContentDigest::of(&self.canonical_bytes).as_bytes()),
            self.canonical_bytes.len() as u64,
        )
        .map_err(Into::into)
    }

    fn source_span(&self) -> Result<Option<SourceSpanV1>, BootstrapStreamingImportError> {
        if self.source_length == 0 {
            Ok(None)
        } else {
            SourceSpanV1::new(self.source_leaf, self.source_offset, self.source_length)
                .map(Some)
                .map_err(Into::into)
        }
    }
}

struct BootstrapOperationSpool {
    path: PathBuf,
    operation_count: u64,
}

fn spool_bootstrap_operations(
    capture: &BootstrapSourceCapture,
    import_id: ImportId,
    workspace_id: WorkspaceId,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<BootstrapOperationSpool, BootstrapStreamingImportError> {
    let page_path = working.join("phase-page.sorted");
    let block_path = working.join("phase-block.sorted");
    let identity_candidates_path = working.join("identity-candidates.sorted");
    let identity_path = working.join("phase-identity.sorted");
    let preamble_path = working.join("phase-preamble.sorted");
    let mut page_sort = ExternalSort::new(working, "phase-page")?;
    let mut block_sort = ExternalSort::new(working, "phase-block")?;
    let mut identity_candidates = ExternalSort::new(working, "identity-candidates")?;
    let mut preamble_sort = ExternalSort::new(working, "phase-preamble")?;
    let mut source_reader = BootstrapSourceReader::new(capture)?;
    let mut entries = capture.entries_cursor()?;
    let mut operation_count = 0_u64;

    while let Some(entry) = entries.next()? {
        let bytes = source_reader.read_entry(&entry, instrumentation)?;
        let text = std::str::from_utf8(&bytes).map_err(|_| {
            BootstrapStreamingImportError::InvalidSource(format!(
                "captured source {} is not UTF-8",
                entry.path()
            ))
        })?;
        let logical_name = LogicalPageName::parse(entry.logical_name().to_owned())
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        let leaf = SourceLeafV1::new(
            entry.kind(),
            entry.path().clone(),
            SourceContentDigestV1::from_bytes(*entry.description().sha256()),
            entry.description().byte_length(),
        )?;
        let source_leaf = leaf.digest();
        let page_id = import_id.unmatched_page_id(&ImportLocator::page(entry.path().clone()));
        let home_document_id =
            DocumentId::for_unmatched_import_page(workspace_id, entry.path().as_str().as_bytes());
        let full_span = (!bytes.is_empty())
            .then(|| StructuralSpan::new(0, bytes.len() as u64))
            .transpose()
            .map_err(|error| BootstrapStreamingImportError::InvalidSource(error.to_string()))?;
        let page_operation = BootstrapOperationRecord::new(
            SemanticOperation::CreatePage {
                page_id,
                home_document_id,
                name: logical_name,
                path: entry.path().clone(),
                kind: entry.kind(),
            },
            source_leaf,
            full_span,
        )?;
        page_sort.push(
            entry.path().as_str().as_bytes().to_vec(),
            page_operation.encode()?,
        )?;
        operation_count = checked_bootstrap_operation_count(operation_count)?;

        let mut parser_instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(entry.path(), bytes.as_slice(), &mut parser_instrumentation)
            .map_err(|block| BootstrapStreamingImportError::InvalidSource(block.detail))?;
        if tree.nodes.len() as u32 > MAX_PARSED_NODES_PER_SOURCE_FILE {
            return Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "parser nodes per source file",
                observed: tree.nodes.len() as u64,
                limit: u64::from(MAX_PARSED_NODES_PER_SOURCE_FILE),
            });
        }
        instrumentation.parser_nodes = instrumentation
            .parser_nodes
            .checked_add(tree.nodes.len() as u64)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidSource(
                    "parser-node instrumentation overflow".into(),
                )
            })?;
        instrumentation.peak_owned_parser_nodes = instrumentation
            .peak_owned_parser_nodes
            .max(tree.nodes.len() as u64);
        let mut node_ids = Vec::with_capacity(tree.nodes.len());
        for index in 0..tree.nodes.len() {
            let locator = materialize_locator(&tree, index, &mut parser_instrumentation)
                .map_err(|block| BootstrapStreamingImportError::InvalidSource(block.detail))?;
            let block_id =
                import_id.unmatched_block_id(&ImportLocator::block(entry.path().clone(), locator));
            let parent = tree.nodes[index].parent.map(|parent| node_ids[parent]);
            node_ids.push(block_id);
            let span = Some(tree.nodes[index].span);
            let operation = BootstrapOperationRecord::new(
                SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id,
                        home_document_id,
                    },
                    page_id,
                    parent,
                    order: imported_order(tree.nodes[index].sibling_position),
                    content: tree.nodes[index].raw.clone(),
                },
                source_leaf,
                span,
            )?;
            let mut key = (tree.nodes[index].depth as u32).to_be_bytes().to_vec();
            key.extend_from_slice(block_id.as_uuid().as_bytes());
            block_sort.push(key, operation.encode()?)?;
            operation_count = checked_bootstrap_operation_count(operation_count)?;

            if tree.nodes[index].raw_ids.len() == 1 {
                if let Ok(logseq_uuid) = LogseqUuid::parse(tree.nodes[index].raw_ids[0].trim()) {
                    let identity = BootstrapOperationRecord::new(
                        SemanticOperation::MutateBlockLogseqIdentity {
                            block: BlockLocation {
                                block_id,
                                home_document_id,
                            },
                            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
                        },
                        source_leaf,
                        span,
                    )?;
                    let mut value = block_id.as_uuid().as_bytes().to_vec();
                    value.extend_from_slice(&identity.encode()?);
                    identity_candidates.push(logseq_uuid.as_uuid().as_bytes().to_vec(), value)?;
                }
            }
        }
        if tree.preamble.is_some() {
            let preamble = BootstrapOperationRecord::new(
                SemanticOperation::SetPagePreamble {
                    page_id,
                    preamble: tree.preamble,
                },
                source_leaf,
                full_span,
            )?;
            preamble_sort.push(
                entry.path().as_str().as_bytes().to_vec(),
                preamble.encode()?,
            )?;
            operation_count = checked_bootstrap_operation_count(operation_count)?;
        }
        let _ = text;
    }
    source_reader.finish()?;

    for (sort, destination) in [
        (page_sort, &page_path),
        (block_sort, &block_path),
        (identity_candidates, &identity_candidates_path),
        (preamble_sort, &preamble_path),
    ] {
        let receipt = sort.finish(destination)?;
        record_sort_receipt(instrumentation, receipt);
    }
    let identity_count = collapse_unique_identity_candidates(
        &identity_candidates_path,
        &identity_path,
        working,
        instrumentation,
    )?;
    operation_count = operation_count.checked_add(identity_count).ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation("bootstrap operation count overflow".into())
    })?;
    require_bootstrap_operation_limit(operation_count)?;

    let operation_path = working.join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
    let mut output = BufWriter::new(create_new_file(&operation_path)?);
    for phase in [&page_path, &block_path, &identity_path, &preamble_path] {
        let mut input = File::open(phase)?;
        io::copy(&mut input, &mut output)?;
    }
    output.flush()?;
    output.get_ref().sync_all()?;
    sync_parent(&operation_path)?;
    instrumentation.operations = operation_count;
    instrumentation.operation_spool_bytes = instrumentation
        .operation_spool_bytes
        .saturating_add(operation_path.metadata()?.len());
    Ok(BootstrapOperationSpool {
        path: operation_path,
        operation_count,
    })
}

fn collapse_unique_identity_candidates(
    candidates: &Path,
    destination: &Path,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<u64, BootstrapStreamingImportError> {
    let mut reader = SortRecordReader::open(candidates)?;
    let mut output_sort = ExternalSort::new(working, "phase-identity")?;
    let mut pending: Option<SortRecord> = None;
    let mut duplicate = false;
    let mut count = 0_u64;
    while let Some(record) = reader.next()? {
        match &pending {
            Some(previous) if previous.key == record.key => {
                duplicate = true;
            }
            Some(_) => {
                if !duplicate {
                    let unique = pending.take().expect("pending identity exists");
                    if unique.value.len() < 16 {
                        return Err(BootstrapStreamingImportError::InvalidOperation(
                            "truncated identity candidate".into(),
                        ));
                    }
                    output_sort.push(unique.value[..16].to_vec(), unique.value[16..].to_vec())?;
                    count = count.saturating_add(1);
                }
                pending = Some(record);
                duplicate = false;
            }
            None => pending = Some(record),
        }
    }
    if let Some(unique) = pending {
        if !duplicate {
            if unique.value.len() < 16 {
                return Err(BootstrapStreamingImportError::InvalidOperation(
                    "truncated identity candidate".into(),
                ));
            }
            output_sort.push(unique.value[..16].to_vec(), unique.value[16..].to_vec())?;
            count = count.saturating_add(1);
        }
    }
    let receipt = output_sort.finish(destination)?;
    record_sort_receipt(instrumentation, receipt);
    Ok(count)
}

fn checked_bootstrap_operation_count(current: u64) -> Result<u64, BootstrapStreamingImportError> {
    let observed = current.checked_add(1).ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation("bootstrap operation count overflow".into())
    })?;
    require_bootstrap_operation_limit(observed)?;
    Ok(observed)
}

fn require_bootstrap_operation_limit(count: u64) -> Result<(), BootstrapStreamingImportError> {
    let limit = u64::from(MAX_BOOTSTRAP_PARTS) * u64::from(MAX_OPERATIONS_PER_BOOTSTRAP_PART);
    if count > limit {
        Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "bootstrap operations",
            observed: count,
            limit,
        })
    } else {
        Ok(())
    }
}

struct BootstrapOperationSpoolReader {
    records: SortRecordReader,
}

impl BootstrapOperationSpoolReader {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            records: SortRecordReader::open(path)?,
        })
    }

    fn next(&mut self) -> Result<Option<BootstrapOperationRecord>, BootstrapStreamingImportError> {
        self.records
            .next()?
            .map(|record| BootstrapOperationRecord::decode(&record.value))
            .transpose()
    }
}

fn partition_bootstrap_operation_spool(
    operations: &BootstrapOperationSpool,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<u32, BootstrapStreamingImportError> {
    let path = working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL);
    let mut writer = BufWriter::new(create_new_file(&path)?);
    let mut reader = BootstrapOperationSpoolReader::open(&operations.path)?;
    let mut part_operations = 0_u32;
    let mut part_semantic_bytes = 0_u64;
    let mut part_spans = BTreeSet::new();
    let mut part_count = 0_u32;
    let mut observed_operations = 0_u64;
    while let Some(operation) = reader.next()? {
        let semantic_bytes = operation.canonical_bytes.len() as u64;
        let partition_bytes = semantic_bytes
            .checked_add(BOOTSTRAP_STREAM_SEMANTIC_EFFECT_OVERHEAD)
            .ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "semantic-effect partition charge overflow".into(),
                )
            })?;
        if partition_bytes > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART {
            return Err(BootstrapStreamingImportError::SingletonOverLimit(
                "semantic-effect bytes",
            ));
        }
        let source_span = operation.source_span()?;
        let adds_span = source_span.is_some_and(|span| !part_spans.contains(&span));
        let exceeds = part_operations == MAX_OPERATIONS_PER_BOOTSTRAP_PART
            || part_semantic_bytes.saturating_add(partition_bytes)
                > MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART
            || (adds_span && part_spans.len() as u32 == MAX_SOURCE_SPANS_PER_BOOTSTRAP_PART);
        if exceeds {
            if part_operations == 0 {
                return Err(BootstrapStreamingImportError::SingletonOverLimit(
                    "bootstrap part",
                ));
            }
            write_frame(&mut writer, &part_operations.to_be_bytes())?;
            part_count = part_count.checked_add(1).ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "bootstrap part count overflow".into(),
                )
            })?;
            if part_count > MAX_BOOTSTRAP_PARTS {
                return Err(BootstrapStreamingImportError::ResourceLimit {
                    resource: "bootstrap parts",
                    observed: u64::from(part_count),
                    limit: u64::from(MAX_BOOTSTRAP_PARTS),
                });
            }
            instrumentation.source_spans = instrumentation
                .source_spans
                .saturating_add(part_spans.len() as u64);
            part_operations = 0;
            part_semantic_bytes = 0;
            part_spans.clear();
        }
        part_operations += 1;
        part_semantic_bytes += partition_bytes;
        if let Some(span) = source_span {
            part_spans.insert(span);
        }
        observed_operations += 1;
    }
    if part_operations != 0 {
        write_frame(&mut writer, &part_operations.to_be_bytes())?;
        part_count = part_count.checked_add(1).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation("bootstrap part count overflow".into())
        })?;
        instrumentation.source_spans = instrumentation
            .source_spans
            .saturating_add(part_spans.len() as u64);
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    sync_parent(&path)?;
    if observed_operations != operations.operation_count {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "operation spool count differs during partitioning".into(),
        ));
    }
    if part_count > MAX_BOOTSTRAP_PARTS {
        return Err(BootstrapStreamingImportError::ResourceLimit {
            resource: "bootstrap parts",
            observed: u64::from(part_count),
            limit: u64::from(MAX_BOOTSTRAP_PARTS),
        });
    }
    instrumentation.parts = part_count;
    Ok(part_count)
}

struct AuthoredBootstrapParts {
    descriptors: Vec<BootstrapPartDescriptorV1>,
    candidate: Box<DetachedBootstrapCandidate>,
    engine_materials: Vec<DetachedBootstrapAcceptedEngineMaterial>,
    final_frontier: ArchiveLocalFrontierBindingV1,
}

#[allow(clippy::too_many_arguments)]
fn author_bootstrap_parts(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    import_id: ImportId,
    operation_spool: &BootstrapOperationSpool,
    part_count: u32,
    working: &Path,
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<AuthoredBootstrapParts, BootstrapStreamingImportError> {
    let profile_digest = BootstrapPartitionProfileV1::v1().digest();
    let mut preview = boxed_detached_bootstrap_session(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy.clone(),
    )?;
    let mut authoring = boxed_detached_bootstrap_session(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
    )?;
    let author_device_id = DeviceId::for_external_import_author(workspace_id);
    let author_session_id = SessionId::for_external_import_author(workspace_id, import_id);
    let crdt_peer_id = (0..=1024)
        .map(|attempt| CrdtPeerId::external_import_candidate(workspace_id, import_id, attempt))
        .find(|peer| peer.as_u64() != 0)
        .ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "no nonzero deterministic bootstrap CRDT peer".into(),
            )
        })?;

    let parts_directory = working.join(BOOTSTRAP_STREAM_PARTS);
    create_private_directory(&parts_directory)?;
    let mut boundaries = FrameReader::open(
        &working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL),
        std::mem::size_of::<u32>(),
    )?;
    let mut operations = BootstrapOperationSpoolReader::open(&operation_spool.path)?;
    let mut predecessor = None;
    let mut archive_frontier = ArchiveLocalFrontierBindingV1::initial(import_id, profile_digest);
    let mut descriptors = Vec::with_capacity(part_count as usize);
    let mut engine_materials = Vec::with_capacity(part_count as usize);
    let mut authored_operations = 0_u64;

    for ordinal in 0..part_count {
        let boundary = boundaries.next()?.ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "part-boundary spool ended early".into(),
            )
        })?;
        if boundary.len() != 4 {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "invalid part-boundary frame".into(),
            ));
        }
        let operation_count = u32::from_be_bytes(
            boundary
                .try_into()
                .expect("checked part-boundary frame length"),
        );
        let mut records = Vec::with_capacity(operation_count as usize);
        let mut transaction_operations = Vec::with_capacity(operation_count as usize);
        let mut operation_leaves = Vec::with_capacity(operation_count as usize);
        let mut source_spans = BTreeSet::new();
        for _ in 0..operation_count {
            let record = operations.next()?.ok_or_else(|| {
                BootstrapStreamingImportError::InvalidOperation(
                    "operation spool ended before its part boundary".into(),
                )
            })?;
            transaction_operations.push(record.operation.clone());
            operation_leaves.push(record.operation_leaf()?);
            if let Some(span) = record.source_span()? {
                source_spans.insert(span);
            }
            records.push(record);
        }
        authored_operations += u64::from(operation_count);
        let transaction = OperationTransaction::new(transaction_operations)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let source_spans = source_spans.into_iter().collect::<Vec<_>>();
        let source_span_root = SourceSpanRootV1::from_spans(&source_spans)?;
        let operation_root = OperationRootV1::from_operations(&operation_leaves)?;
        let provisional_evidence = BootstrapImportPartEvidenceV1::new(
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            PayloadObjectRootV1::empty(),
            predecessor,
        )?;
        let author = AuthorBatch {
            batch_id: provisional_evidence.batch_id(),
            author_device_id,
            author_session_id,
            crdt_peer_id,
        };
        let preview_part = preview
            .author_part(author, &transaction, provisional_evidence)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let (preview_prepared, preview_engine_material) = preview_part.into_parts();
        drop(preview_engine_material);
        let preview_manifest_bytes = preview_prepared
            .manifest()
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let payload_descriptors = prepared_payload_descriptors(&preview_prepared)?;
        let payload_root = PayloadObjectRootV1::from_objects(&payload_descriptors)?;
        validate_prepared_part_limits(&preview_prepared, operation_count)?;
        drop(preview_prepared);
        let evidence = BootstrapImportPartEvidenceV1::new(
            import_id,
            profile_digest,
            ordinal,
            part_count,
            source_span_root,
            operation_root,
            payload_root,
            predecessor,
        )?;
        if evidence.part_id() != provisional_evidence.part_id()
            || evidence.batch_id() != provisional_evidence.batch_id()
        {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "payload commitment changed bootstrap part identity".into(),
            ));
        }
        let authored = authoring
            .author_part(author, &transaction, evidence)
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let (prepared, engine_material) = authored.into_parts();
        let manifest_bytes = prepared
            .manifest()
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        let actual_payload_descriptors = prepared_payload_descriptors(&prepared)?;
        if manifest_bytes != preview_manifest_bytes
            || actual_payload_descriptors != payload_descriptors
        {
            return Err(BootstrapStreamingImportError::InvalidOperation(
                "exact-evidence authoring differs from its bounded preview".into(),
            ));
        }
        let manifest_digest = ContentDigest::of(&manifest_bytes);
        let manifest_fingerprint =
            BootstrapManifestFingerprintV1::from_bytes(*manifest_digest.as_bytes());
        let manifest_objects = [FullObjectDescriptorV1::manifest_defined(
            *manifest_digest.as_bytes(),
            manifest_bytes.len() as u64,
        )?];
        let descriptor = BootstrapPartDescriptorV1::accepted(
            evidence,
            manifest_fingerprint,
            &payload_descriptors,
            &manifest_objects,
            archive_frontier,
        )?;
        archive_frontier = descriptor.post_frontier();
        write_prepared_bootstrap_part(
            &parts_directory,
            ordinal,
            evidence,
            &source_spans,
            &manifest_bytes,
            prepared.objects(),
            instrumentation,
        )?;
        instrumentation.peak_owned_part_operations = instrumentation
            .peak_owned_part_operations
            .max(u64::from(operation_count));
        descriptors.push(descriptor);
        engine_materials.push(engine_material);
        predecessor = Some(evidence.part_id());
        drop(records);
    }
    if boundaries.next()?.is_some() || operations.next()?.is_some() {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "sealed part boundaries do not consume the exact operation spool".into(),
        ));
    }
    if authored_operations != operation_spool.operation_count {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "authored operation count differs from the sealed spool".into(),
        ));
    }
    let preview_candidate = finish_boxed_detached_bootstrap_session(preview)?;
    let preview_frontier = preview_candidate
        .accepted_frontier_root()
        .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
    drop(preview_candidate);
    let candidate = finish_boxed_detached_bootstrap_session(authoring)?;
    if preview_frontier
        != candidate
            .accepted_frontier_root()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?
    {
        return Err(BootstrapStreamingImportError::InvalidOperation(
            "preview and exact-evidence detached frontiers differ".into(),
        ));
    }
    Ok(AuthoredBootstrapParts {
        descriptors,
        candidate,
        engine_materials,
        final_frontier: archive_frontier,
    })
}

#[inline(never)]
fn boxed_detached_bootstrap_session(
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
) -> Result<Box<DetachedBootstrapAuthoringSession>, BootstrapStreamingImportError> {
    DetachedBootstrapAuthoringSession::new(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
    )
    .map(Box::new)
    .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))
}

#[inline(never)]
fn finish_boxed_detached_bootstrap_session(
    session: Box<DetachedBootstrapAuthoringSession>,
) -> Result<Box<DetachedBootstrapCandidate>, BootstrapStreamingImportError> {
    (*session)
        .finish()
        .map(Box::new)
        .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))
}

fn prepared_payload_descriptors(
    prepared: &super::PreparedBatch,
) -> Result<Vec<PayloadObjectDescriptorV1>, BootstrapStreamingImportError> {
    prepared
        .objects()
        .iter()
        .map(|object| {
            let bytes = object.encode().map_err(|error| {
                BootstrapStreamingImportError::InvalidOperation(error.to_string())
            })?;
            PayloadObjectDescriptorV1::new(ContentDigest::of(&bytes), bytes.len() as u64)
                .map_err(Into::into)
        })
        .collect()
}

fn validate_prepared_part_limits(
    prepared: &super::PreparedBatch,
    operation_count: u32,
) -> Result<(), BootstrapStreamingImportError> {
    let manifest = prepared
        .manifest()
        .encode()
        .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
    if manifest.len() > BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES {
        return if operation_count == 1 {
            Err(BootstrapStreamingImportError::SingletonOverLimit(
                "batch manifest bytes",
            ))
        } else {
            Err(BootstrapStreamingImportError::ResourceLimit {
                resource: "batch manifest bytes",
                observed: manifest.len() as u64,
                limit: BOOTSTRAP_STREAM_MAX_MANIFEST_BYTES as u64,
            })
        };
    }
    let mut total = 0_u64;
    let mut semantic = None;
    for object in prepared.objects() {
        let bytes = object
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        total = total.checked_add(bytes.len() as u64).ok_or_else(|| {
            BootstrapStreamingImportError::InvalidOperation(
                "prepared object byte count overflow".into(),
            )
        })?;
        if object.kind() == ObjectKind::SemanticEffect {
            semantic = Some(object.payload().len() as u64);
        }
    }
    let semantic = semantic.ok_or_else(|| {
        BootstrapStreamingImportError::InvalidOperation(
            "prepared bootstrap part has no semantic effect".into(),
        )
    })?;
    for (resource, observed, limit) in [
        (
            "semantic-effect bytes",
            semantic,
            MAX_SEMANTIC_EFFECT_BYTES_PER_BOOTSTRAP_PART,
        ),
        (
            "payload/object bytes",
            total,
            MAX_BATCH_OBJECT_BYTES_PER_BOOTSTRAP_PART,
        ),
    ] {
        if observed > limit {
            return if operation_count == 1 {
                Err(BootstrapStreamingImportError::SingletonOverLimit(resource))
            } else {
                Err(BootstrapStreamingImportError::ResourceLimit {
                    resource,
                    observed,
                    limit,
                })
            };
        }
    }
    Ok(())
}

fn write_prepared_bootstrap_part(
    parts: &Path,
    ordinal: u32,
    evidence: BootstrapImportPartEvidenceV1,
    source_spans: &[SourceSpanV1],
    manifest_bytes: &[u8],
    objects: &[super::OperationObject],
    instrumentation: &mut BootstrapStreamingImportInstrumentation,
) -> Result<(), BootstrapStreamingImportError> {
    let directory = parts.join(format!("{ordinal:08}"));
    create_private_directory(&directory)?;
    let evidence_bytes = evidence.encode()?;
    let spans = BootstrapPartSpanIndexV1::new(evidence.part_id(), source_spans.to_vec())?;
    let span_bytes = spans.encode()?;
    write_exact_new(
        &directory.join(BOOTSTRAP_STREAM_PART_MANIFEST),
        manifest_bytes,
    )?;
    write_exact_new(
        &directory.join(BOOTSTRAP_STREAM_PART_EVIDENCE),
        &evidence_bytes,
    )?;
    write_exact_new(&directory.join(BOOTSTRAP_STREAM_PART_SPANS), &span_bytes)?;
    let object_path = directory.join(BOOTSTRAP_STREAM_PART_OBJECTS);
    let mut writer = BufWriter::new(create_new_file(&object_path)?);
    let mut object_bytes = 0_u64;
    for object in objects {
        let bytes = object
            .encode()
            .map_err(|error| BootstrapStreamingImportError::InvalidOperation(error.to_string()))?;
        object_bytes = object_bytes.saturating_add(write_frame(&mut writer, &bytes)?);
    }
    writer.flush()?;
    writer.get_ref().sync_all()?;
    sync_parent(&object_path)?;
    let prepared_bytes = manifest_bytes.len() as u64
        + evidence_bytes.len() as u64
        + span_bytes.len() as u64
        + object_bytes;
    instrumentation.prepared_bytes = instrumentation
        .prepared_bytes
        .saturating_add(prepared_bytes);
    instrumentation.peak_owned_part_bytes =
        instrumentation.peak_owned_part_bytes.max(prepared_bytes);
    Ok(())
}

/// Prepare, but do not publish or admit, one complete multipart bootstrap.
///
/// `capture` must be the sealed capture minted by `Graph`; this function
/// consumes it so the final capture-C proof cannot be separated from the
/// preparation it authorizes. `scratch` is caller-owned disposable storage
/// outside the graph. Only the final `sealed.commit` file marks a reusable
/// preparation; all UUID-named work directories are non-authoritative residue.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_inactive_bootstrap_import(
    graph: &Graph,
    capture: BootstrapSourceCapture,
    workspace_id: WorkspaceId,
    lineage_digest: LineageDigest,
    catalog_document_id: DocumentId,
    reference_catalog_policy: ReferenceCatalogPolicyV1,
    scratch: &Path,
) -> Result<InactiveBootstrapPreparedPublication, BootstrapStreamingImportError> {
    prepare_bootstrap_scratch(graph, scratch)?;
    let root = scratch.join(BOOTSTRAP_STREAM_DIRECTORY);
    create_private_directory(&root)?;
    let working = root.join(format!(".building-{}", Uuid::new_v4().simple()));
    create_private_directory(&working)?;
    let artifacts = working.join("artifacts");
    create_private_directory(&artifacts)?;
    let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
    record_capture_instrumentation(&mut instrumentation, capture.instrumentation());

    let source =
        prepare_bootstrap_source_protocol(workspace_id, &capture, &working, &mut instrumentation)?;
    let operations = spool_bootstrap_operations(
        &capture,
        source.import_id,
        workspace_id,
        &working,
        &mut instrumentation,
    )?;
    let part_count =
        partition_bootstrap_operation_spool(&operations, &working, &mut instrumentation)?;
    let graph_resource = graph.canonical_resource_id()?;

    // This is deliberately the final source action. Everything below owns only
    // sealed spools and detached engine/scratch capabilities.
    let final_capture = capture.verify_before_inactive_bootstrap_authoring(graph)?;
    record_capture_instrumentation(&mut instrumentation, &final_capture);
    let authored = author_bootstrap_parts(
        workspace_id,
        lineage_digest,
        catalog_document_id,
        reference_catalog_policy,
        source.import_id,
        &operations,
        part_count,
        &working,
        &mut instrumentation,
    )?;

    let profile_digest = BootstrapPartitionProfileV1::v1().digest();
    let initial_frontier = ArchiveLocalFrontierBindingV1::initial(source.import_id, profile_digest);
    let aggregate = BootstrapAggregateManifestV1::new_for_import(
        workspace_id,
        lineage_digest,
        graph_resource,
        source.import_id,
        source.source_count,
        source.inventory_root,
        source.inventory_page_count,
        source.blob_root,
        source.blob_page_count,
        profile_digest,
        authored.descriptors,
        initial_frontier,
        authored.final_frontier,
    )?;
    let aggregate_bytes = aggregate.encode()?;
    let commit = BootstrapAggregateCommitV1::for_aggregate(&aggregate)?;
    let commit_bytes = commit.encode()?;

    for name in [
        BOOTSTRAP_STREAM_INVENTORY_PAGES,
        BOOTSTRAP_STREAM_BLOB_PAGES,
        BOOTSTRAP_STREAM_PARTS,
    ] {
        fs::rename(working.join(name), artifacts.join(name))?;
    }
    write_exact_new(
        &artifacts.join(BOOTSTRAP_STREAM_AGGREGATE),
        &aggregate_bytes,
    )?;
    write_exact_new(&artifacts.join(BOOTSTRAP_STREAM_COMMIT), &commit_bytes)?;

    let sealed_directory = root.join(hex_bootstrap_digest(commit.publication_id().as_bytes()));
    seal_bootstrap_preparation(&artifacts, &sealed_directory, &commit_bytes)?;
    let sealed_aggregate = read_bounded_file(
        &sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
        super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_MANIFEST_BYTES,
    )?;
    if BootstrapAggregateManifestV1::decode(&sealed_aggregate)? != aggregate {
        return Err(BootstrapStreamingImportError::ConflictingSeal);
    }
    let sealed_commit = read_bounded_file(
        &sealed_directory.join(BOOTSTRAP_STREAM_SEAL),
        super::bootstrap_import::MAX_BOOTSTRAP_AGGREGATE_COMMIT_BYTES,
    )?;
    let decoded_commit = BootstrapAggregateCommitV1::decode(&sealed_commit)?;
    decoded_commit.validate_aggregate(&aggregate)?;
    let _ = fs::remove_dir_all(&working);

    Ok(InactiveBootstrapPreparedPublication {
        source_capture: capture,
        sealed_directory,
        aggregate,
        commit,
        candidate: authored.candidate,
        engine_materials: authored.engine_materials,
        instrumentation,
    })
}

fn record_capture_instrumentation(
    target: &mut BootstrapStreamingImportInstrumentation,
    source: &BootstrapSourceCaptureInstrumentation,
) {
    target.capture_passes = target.capture_passes.saturating_add(source.passes);
    target.external_sort_runs = target.external_sort_runs.saturating_add(source.sort_runs);
    target.source_bytes_read = target
        .source_bytes_read
        .saturating_add(source.physical_bytes);
    target.operation_spool_bytes = target
        .operation_spool_bytes
        .saturating_add(source.spool_bytes);
    target.peak_owned_sort_buffer_bytes = target
        .peak_owned_sort_buffer_bytes
        .max(source.peak_owned_buffer_bytes);
}

fn prepare_bootstrap_scratch(graph: &Graph, scratch: &Path) -> io::Result<()> {
    create_private_directory(scratch)?;
    let scratch = fs::canonicalize(scratch)?;
    let graph_root = fs::canonicalize(&graph.root)?;
    if scratch == graph_root || scratch.starts_with(&graph_root) {
        return Err(invalid_bootstrap_data(
            "inactive bootstrap scratch must be outside the graph",
        ));
    }
    Ok(())
}

fn seal_bootstrap_preparation(
    source: &Path,
    destination: &Path,
    commit_bytes: &[u8],
) -> Result<(), BootstrapStreamingImportError> {
    create_private_directory(destination)?;
    copy_bootstrap_tree_exact(source, destination)?;
    publish_exact_file(&destination.join(BOOTSTRAP_STREAM_SEAL), commit_bytes)?;
    File::open(destination)?.sync_all()?;
    sync_parent(destination)?;
    Ok(())
}

fn copy_bootstrap_tree_exact(
    source: &Path,
    destination: &Path,
) -> Result<(), BootstrapStreamingImportError> {
    // Each exact child has an independent destination name, so filesystem
    // enumeration order cannot affect the sealed tree. Keep only one entry
    // live while copying graph-sized artifact directories.
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "bootstrap artifact tree contains a symlink".into(),
            ));
        }
        if metadata.is_dir() {
            create_private_directory(&destination_path)?;
            copy_bootstrap_tree_exact(&source_path, &destination_path)?;
            File::open(&destination_path)?.sync_all()?;
        } else if metadata.is_file() {
            copy_bootstrap_file_exact(&source_path, &destination_path)?;
        } else {
            return Err(BootstrapStreamingImportError::InvalidSource(
                "bootstrap artifact tree contains a non-file entry".into(),
            ));
        }
    }
    File::open(destination)?.sync_all()?;
    Ok(())
}

fn copy_bootstrap_file_exact(
    source: &Path,
    destination: &Path,
) -> Result<(), BootstrapStreamingImportError> {
    match create_new_file(destination) {
        Ok(mut output) => {
            let mut input = File::open(source)?;
            io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            sync_parent(destination)?;
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if bootstrap_files_equal(source, destination)? {
                Ok(())
            } else {
                Err(BootstrapStreamingImportError::ConflictingSeal)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn bootstrap_files_equal(left: &Path, right: &Path) -> io::Result<bool> {
    let left_metadata = fs::symlink_metadata(left)?;
    let right_metadata = fs::symlink_metadata(right)?;
    if left_metadata.file_type().is_symlink()
        || right_metadata.file_type().is_symlink()
        || !left_metadata.is_file()
        || !right_metadata.is_file()
        || left_metadata.len() != right_metadata.len()
    {
        return Ok(false);
    }
    let mut left = BufReader::with_capacity(64 * 1024, File::open(left)?);
    let mut right = BufReader::with_capacity(64 * 1024, File::open(right)?);
    let mut left_buffer = [0; 64 * 1024];
    let mut right_buffer = [0; 64 * 1024];
    loop {
        let left_read = left.read(&mut left_buffer)?;
        let right_read = right.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

fn hex_bootstrap_digest(digest: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[derive(Clone, Copy)]
struct ImportReplayLimits {
    entries: usize,
    base_bytes: u64,
    rendered_bytes: u64,
}

const IMPORT_REPLAY_LIMITS: ImportReplayLimits = ImportReplayLimits {
    entries: MAX_IMPORT_REPLAY_ENTRIES,
    base_bytes: MAX_IMPORT_REPLAY_BYTES,
    rendered_bytes: MAX_IMPORT_RENDERED_TARGET_BYTES,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactBytes {
    bytes: Vec<u8>,
    description: BlobDescription,
}

impl ExactBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        let description = BlobDescription::of(&bytes);
        Self { bytes, description }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn description(&self) -> BlobDescription {
        self.description
    }

    fn from_description(bytes: Vec<u8>, description: BlobDescription) -> Self {
        Self { bytes, description }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RawObservation {
    Present(ExactBytes),
    Absent,
}

impl RawObservation {
    pub fn present(bytes: Vec<u8>) -> Self {
        Self::Present(ExactBytes::new(bytes))
    }

    pub const fn description(&self) -> Option<BlobDescription> {
        match self {
            Self::Present(bytes) => Some(bytes.description()),
            Self::Absent => None,
        }
    }
}

/// Exact graph observations keyed by exact, case-preserved managed paths.
///
/// Construction rejects duplicate requested paths instead of silently
/// overwriting one BTreeMap value with another.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RawInventory {
    entries: BTreeMap<ManagedPath, RawObservation>,
}

impl RawInventory {
    pub fn from_entries(
        entries: impl IntoIterator<Item = (ManagedPath, RawObservation)>,
    ) -> Result<Self, InventoryError> {
        let mut inventory = BTreeMap::new();
        let mut path_bytes = 0_u64;
        for (path, observation) in entries {
            if inventory.len() == MAX_IMPORT_FILES {
                return Err(InventoryError::ResourceBudgetExceeded {
                    resource: "managed file count",
                    observed: inventory.len().saturating_add(1) as u64,
                    limit: MAX_IMPORT_FILES as u64,
                });
            }
            path_bytes = charge_budget(
                "aggregate managed path bytes",
                path_bytes,
                path.as_str().len() as u64,
                MAX_IMPORT_PATH_BYTES,
            )?;
            if inventory.insert(path.clone(), observation).is_some() {
                return Err(InventoryError::DuplicateRequestedPath(
                    path.as_str().to_owned(),
                ));
            }
        }
        require_portable_unique(inventory.keys())?;
        Ok(Self { entries: inventory })
    }

    pub fn entries(&self) -> &BTreeMap<ManagedPath, RawObservation> {
        &self.entries
    }

    pub fn present(&self, path: &str) -> Option<&ExactBytes> {
        self.entries
            .iter()
            .find(|(candidate, _)| candidate.as_str() == path)
            .and_then(|(_, observation)| match observation {
                RawObservation::Present(bytes) => Some(bytes),
                RawObservation::Absent => None,
            })
    }

    fn derivation_entries(
        &self,
        path_identities: &BTreeMap<ManagedPath, ImportedPathIdentity>,
    ) -> Result<Vec<ImportInventoryEntry>, ImportExecutionError> {
        self.entries
            .iter()
            .map(|(path, observation)| {
                let identity =
                    path_identities
                        .get(path)
                        .ok_or(ImportExecutionError::IncompletePlan(
                            "sealed inventory path has no Graph-decoded managed kind",
                        ))?;
                Ok(ImportInventoryEntry::with_kind(
                    identity.kind,
                    path.clone(),
                    match observation {
                        RawObservation::Present(bytes) => {
                            ImportInventoryState::Present(bytes.description())
                        }
                        RawObservation::Absent => ImportInventoryState::Absent,
                    },
                ))
            })
            .collect()
    }
}

#[derive(Debug)]
pub enum InventoryError {
    UnsafePath(String),
    DuplicateRequestedPath(String),
    PortablePathCollision {
        first: String,
        second: String,
    },
    ResourceBudgetExceeded {
        resource: &'static str,
        observed: u64,
        limit: u64,
    },
    UnsupportedManagedLayout {
        pages_directory: String,
        journals_directory: String,
    },
    UnsafeEntry {
        path: Option<String>,
        message: String,
    },
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(path) => write!(f, "unsafe managed path: {path:?}"),
            Self::DuplicateRequestedPath(path) => {
                write!(f, "managed path was requested more than once: {path}")
            }
            Self::PortablePathCollision { first, second } => write!(
                f,
                "managed paths share one portable key: {first} and {second}"
            ),
            Self::ResourceBudgetExceeded {
                resource,
                observed,
                limit,
            } => write!(
                f,
                "{resource} budget exceeded: observed {observed}, limit {limit}"
            ),
            Self::UnsupportedManagedLayout {
                pages_directory,
                journals_directory,
            } => write!(
                f,
                "unsupported managed layout: pages={pages_directory:?}, journals={journals_directory:?}"
            ),
            Self::UnsafeEntry { path, message } => match path {
                Some(path) => write!(f, "unsafe managed input {path}: {message}"),
                None => write!(f, "unsafe managed input: {message}"),
            },
        }
    }
}

impl std::error::Error for InventoryError {}

fn require_portable_unique<'a>(
    paths: impl IntoIterator<Item = &'a ManagedPath>,
) -> Result<(), InventoryError> {
    let mut portable = BTreeMap::new();
    for path in paths {
        if let Some(first) = portable.insert(path.portable_key(), path.as_str().to_owned()) {
            return Err(InventoryError::PortablePathCollision {
                first,
                second: path.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn charge_budget(
    resource: &'static str,
    current: u64,
    amount: u64,
    limit: u64,
) -> Result<u64, InventoryError> {
    let observed = current.checked_add(amount).unwrap_or(u64::MAX);
    if observed > limit {
        Err(InventoryError::ResourceBudgetExceeded {
            resource,
            observed,
            limit,
        })
    } else {
        Ok(observed)
    }
}

fn reserve_base_replay(
    instrumentation: &mut ImportInstrumentation,
    declared_base_bytes: u64,
    limits: ImportReplayLimits,
    path: &ManagedPath,
) -> Result<(), ImportBlock> {
    if instrumentation.base_replay_entries == limits.entries {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "base replay entry budget exceeded: limit {}",
                limits.entries
            ),
        ));
    }
    let replay_bytes = instrumentation
        .base_replay_bytes
        .checked_add(declared_base_bytes)
        .unwrap_or(u64::MAX);
    if replay_bytes > limits.base_bytes {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "base replay byte budget exceeded: observed {replay_bytes}, limit {}",
                limits.base_bytes
            ),
        ));
    }
    instrumentation.base_replay_entries = instrumentation.base_replay_entries.saturating_add(1);
    instrumentation.base_replay_bytes = replay_bytes;
    Ok(())
}

fn retain_rendered_target(
    instrumentation: &mut ImportInstrumentation,
    bytes: u64,
    limits: ImportReplayLimits,
    path: &ManagedPath,
) -> Result<(), ImportBlock> {
    let rendered_bytes = instrumentation
        .rendered_target_bytes
        .checked_add(bytes)
        .unwrap_or(u64::MAX);
    if rendered_bytes > limits.rendered_bytes {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "rendered target byte budget exceeded: observed {rendered_bytes}, limit {}",
                limits.rendered_bytes
            ),
        ));
    }
    instrumentation.rendered_target_bytes = rendered_bytes;
    Ok(())
}

/// Read only the explicitly named affected paths. No directory enumeration is
/// performed, including when a requested path is absent.
pub fn inventory_affected(
    graph: &Graph,
    requested_paths: &[&str],
) -> Result<RawInventory, InventoryError> {
    if requested_paths.len() > MAX_IMPORT_FILES {
        return Err(InventoryError::ResourceBudgetExceeded {
            resource: "requested managed path count",
            observed: requested_paths.len() as u64,
            limit: MAX_IMPORT_FILES as u64,
        });
    }
    let mut entries = Vec::with_capacity(requested_paths.len());
    let mut seen = BTreeSet::new();
    let mut portable = BTreeMap::new();
    let mut raw_bytes = 0_u64;
    for requested in requested_paths {
        let path = ManagedPath::parse((*requested).to_owned())
            .map_err(|_| InventoryError::UnsafePath((*requested).to_owned()))?;
        if !seen.insert(path.clone()) {
            return Err(InventoryError::DuplicateRequestedPath(
                path.as_str().to_owned(),
            ));
        }
        if let Some(first) = portable.insert(path.portable_key(), path.as_str().to_owned()) {
            return Err(InventoryError::PortablePathCollision {
                first,
                second: path.as_str().to_owned(),
            });
        }
        let observation = match graph.read_raw_managed_text(&path).map_err(|error| {
            InventoryError::UnsafeEntry {
                path: Some(path.as_str().to_owned()),
                message: error.to_string(),
            }
        })? {
            Some(observation) => {
                raw_bytes = charge_budget(
                    "aggregate raw bytes",
                    raw_bytes,
                    observation.bytes().len() as u64,
                    MAX_IMPORT_RAW_BYTES,
                )?;
                RawObservation::present(observation.into_bytes())
            }
            None => RawObservation::Absent,
        };
        entries.push((path, observation));
    }
    RawInventory::from_entries(entries)
}

/// The only whole-graph raw inventory entry point. It is intentionally named
/// for initial shadow import so ordinary reconciliation cannot obtain a global
/// scan accidentally.
///
/// This is capture evidence, not semantic publication authority. Shadow import
/// must repeat the same capability-bound inventory/semantic comparison at its
/// later import boundary; a caller may not retain this snapshot and assume the
/// live graph remained unchanged.
pub fn inventory_initial_shadow(graph: &Graph) -> Result<RawInventory, InventoryError> {
    let captured = graph
        .fresh_initial_shadow_raw_managed_text_inventory()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidData
                && error.to_string().contains("bound exceeded")
            {
                InventoryError::ResourceBudgetExceeded {
                    resource: "initial shadow resources",
                    observed: 1,
                    limit: 0,
                }
            } else {
                InventoryError::UnsafeEntry {
                    path: None,
                    message: error.to_string(),
                }
            }
        })?;
    RawInventory::from_entries(
        captured
            .into_iter()
            .map(|(path, bytes)| (path, RawObservation::present(bytes))),
    )
}

/// Sealed import base. Only `capture_import_scope` can mint one after the
/// enrolled receipt store and accepted engine jointly authenticate it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ReceiptBackedPage {
    intent: ProjectionIntent,
    completion: ProjectionCompletion,
    replayed_target: ExactBytes,
    page: super::MaterializedPage,
}

impl ReceiptBackedPage {
    const fn page_id(&self) -> PageId {
        self.intent.page_id()
    }

    fn path(&self) -> &ManagedPath {
        self.intent.path()
    }

    const fn logical_completion_id(&self) -> LogicalCompletionId {
        self.completion.logical_completion_id()
    }

    fn bytes(&self) -> &[u8] {
        self.replayed_target.bytes()
    }

    const fn description(&self) -> BlobDescription {
        self.replayed_target.description()
    }

    fn annotations(&self) -> &[AnnotatedIdentity] {
        self.intent.annotations()
    }

    fn materialized_page(&self) -> &super::MaterializedPage {
        &self.page
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ScopedPathEvidence {
    Existing(ReceiptBackedPage),
    Released(LogicalCompletionId),
    New,
}

/// One complete affected-scope authority snapshot. Its fields and constructor
/// are private, so downstream code cannot omit an existing receipt, mix
/// frontiers, or relabel an engine-owned path as new.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportScopeSnapshot {
    workspace_id: WorkspaceId,
    paths: BTreeMap<ManagedPath, ScopedPathEvidence>,
    path_identities: BTreeMap<ManagedPath, ImportedPathIdentity>,
}

/// Exact logical identity of one requested external path, captured through the
/// Graph's normal managed-entry decoder before the sealed plan is built.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ImportedPathIdentity {
    name: LogicalPageName,
    kind: ManagedTextKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct InventoryPathFingerprint {
    state: ImportInventoryState,
    file_resource_id: Option<ContentDigest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AffectedReceiptEntry {
    completed: ProjectionCompletedReceipt,
    intent: ProjectionIntent,
    completion: ProjectionCompletion,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct AffectedReceiptCatalog {
    by_path: BTreeMap<ManagedPath, Vec<AffectedReceiptEntry>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CatalogAuthority {
    digest: ContentDigest,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ImportBlockReason {
    MissingBase,
    CorruptBase,
    AuthorityUnavailable,
    ConflictingLocalTail,
    StaleScope,
    DuplicateAnchorDependent,
    AmbiguousStructuralMatch,
    AmbiguousDestructiveMatch,
    TwoSidedDivergence,
    UnsafeInput,
    UnsupportedManagedLayout,
    ResourceLimit,
    PortablePathCollision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportBlock {
    pub reason: ImportBlockReason,
    pub paths: Vec<String>,
    pub logical_completion_ids: Vec<LogicalCompletionId>,
    pub observation: Option<(ManagedPath, ImportInventoryState)>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImportInstrumentation {
    pub requested_paths: usize,
    pub inventory_passes: usize,
    pub bytes_read: u64,
    pub bytes_hashed: u64,
    pub peak_owned_raw_bytes: u64,
    pub path_bytes: u64,
    pub catalog_entries: usize,
    pub catalog_bytes_hashed: u64,
    pub base_replay_entries: usize,
    pub base_replay_bytes: u64,
    pub rendered_target_bytes: u64,
    pub catalog_path_inserts: usize,
    pub catalog_path_lookups: usize,
    pub inventory_path_lookups: usize,
    pub parsed_nodes: usize,
    pub max_depth: usize,
    pub locator_components_materialized: usize,
    pub structural_class_nodes: usize,
    pub structural_class_allocations: usize,
    pub structural_key_components: usize,
    pub structural_key_comparisons: usize,
    pub exact_bucket_inserts: usize,
    pub exact_bucket_lookups: usize,
    pub ordered_alignment_visits: usize,
    pub retained_block_matches: usize,
    pub rejected_raw_id_occurrences: usize,
}

impl ImportInstrumentation {
    /// Sum of explicitly recorded byte/component/event counters. This is a
    /// regression signal, not a claim that every platform/library comparison
    /// has one portable unit cost; independent hard ceilings remain authoritative.
    pub fn recorded_work_units(self) -> usize {
        let byte_work = self
            .bytes_read
            .saturating_add(self.bytes_hashed)
            .saturating_add(self.path_bytes)
            .saturating_add(self.catalog_bytes_hashed)
            .saturating_add(self.base_replay_bytes)
            .saturating_add(self.rendered_target_bytes);
        let byte_work = usize::try_from(byte_work).unwrap_or(usize::MAX);
        self.requested_paths
            .saturating_add(byte_work)
            .saturating_add(self.catalog_entries)
            .saturating_add(self.base_replay_entries)
            .saturating_add(self.catalog_path_inserts)
            .saturating_add(self.catalog_path_lookups)
            .saturating_add(self.inventory_path_lookups)
            .saturating_add(self.parsed_nodes)
            .saturating_add(self.locator_components_materialized)
            .saturating_add(self.structural_class_nodes)
            .saturating_add(self.structural_class_allocations)
            .saturating_add(self.structural_key_components)
            .saturating_add(self.structural_key_comparisons)
            .saturating_add(self.exact_bucket_inserts)
            .saturating_add(self.exact_bucket_lookups)
            .saturating_add(self.ordered_alignment_visits)
            .saturating_add(self.retained_block_matches)
            .saturating_add(self.rejected_raw_id_occurrences)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageMatchBasis {
    SamePathCompletion,
    ReceiptBackedExactRename,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageImportMatch {
    path: ManagedPath,
    previous_path: ManagedPath,
    page_id: PageId,
    basis: PageMatchBasis,
}

impl PageImportMatch {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn previous_path(&self) -> &ManagedPath {
        &self.previous_path
    }

    pub const fn page_id(&self) -> PageId {
        self.page_id
    }

    pub const fn basis(&self) -> PageMatchBasis {
        self.basis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockMatchBasis {
    UniqueLogseqUuid,
    ReceiptStructuralExact,
    ReceiptOrderedTreeAlignment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockImportMatch {
    path: ManagedPath,
    locator: StructuralLocator,
    block_id: BlockId,
    basis: BlockMatchBasis,
}

impl BlockImportMatch {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn locator(&self) -> &StructuralLocator {
        &self.locator
    }

    pub const fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub const fn basis(&self) -> BlockMatchBasis {
        self.basis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectedRawIdReason {
    InvalidSyntax,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RejectedRawId {
    path: ManagedPath,
    locator: StructuralLocator,
    raw_value: String,
    reason: RejectedRawIdReason,
}

impl RejectedRawId {
    pub fn path(&self) -> &ManagedPath {
        &self.path
    }

    pub fn locator(&self) -> &StructuralLocator {
        &self.locator
    }

    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    pub const fn reason(&self) -> RejectedRawIdReason {
        self.reason
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportMatches {
    pages: Vec<PageImportMatch>,
    blocks: Vec<BlockImportMatch>,
    rejected_raw_ids: Vec<RejectedRawId>,
}

impl ImportMatches {
    pub fn pages(&self) -> &[PageImportMatch] {
        &self.pages
    }

    pub fn blocks(&self) -> &[BlockImportMatch] {
        &self.blocks
    }

    pub fn rejected_raw_ids(&self) -> &[RejectedRawId] {
        &self.rejected_raw_ids
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportPlanStatus {
    Noop,
    Reconcile,
    Blocked,
}

/// Opaque diagnostic import result.
///
/// This read-only checkpoint deliberately carries no publication witness,
/// mutation capability, or reusable preflight authority. A later checkpoint
/// must recapture its predicates inside a one-shot semantic publisher.
///
/// ```compile_fail
/// use tine_core::oplog::{ImportPlan, ImportPlanStatus};
///
/// fn forge() -> ImportPlan {
///     ImportPlan {
///         status: ImportPlanStatus::Reconcile,
///         import_id: None,
///         inventory: None,
///         matches: None,
///         blocks: Vec::new(),
///         instrumentation: Default::default(),
///     }
/// }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportPlan {
    status: ImportPlanStatus,
    import_id: Option<ImportId>,
    inventory: Option<RawInventory>,
    matches: Option<ImportMatches>,
    scope: Option<ImportScopeSnapshot>,
    execution: Option<ImportExecutionMaterial>,
    blocks: Vec<ImportBlock>,
    instrumentation: ImportInstrumentation,
}

impl ImportPlan {
    pub const fn status(&self) -> ImportPlanStatus {
        self.status
    }

    pub const fn import_id(&self) -> Option<ImportId> {
        self.import_id
    }

    pub fn inventory(&self) -> Option<&RawInventory> {
        self.inventory.as_ref()
    }

    pub fn matches(&self) -> Option<&ImportMatches> {
        self.matches.as_ref()
    }

    pub fn blocks(&self) -> &[ImportBlock] {
        &self.blocks
    }

    pub const fn instrumentation(&self) -> ImportInstrumentation {
        self.instrumentation
    }

    /// Return the sealed, non-authorizing execution material for one accepted
    /// reconciliation. The hot-engine adapter must still recapture all live
    /// predicates before it drafts or publishes a batch.
    #[allow(dead_code)]
    pub(crate) fn execution_material(
        &self,
    ) -> Result<&ImportExecutionMaterial, ImportExecutionError> {
        match self.status {
            ImportPlanStatus::Noop | ImportPlanStatus::Blocked => {
                Err(ImportExecutionError::RefusedStatus(self.status))
            }
            ImportPlanStatus::Reconcile => {
                self.scope
                    .as_ref()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed import scope",
                    ))?;
                self.execution
                    .as_ref()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed execution material",
                    ))
            }
        }
    }

    /// Consume a reconciliable plan at the engine handoff boundary.  This
    /// deliberately moves the observation bytes: the execution adapter must
    /// not clone an already-bounded external observation just to author it.
    #[allow(dead_code)]
    pub(crate) fn into_execution_material(
        mut self,
    ) -> Result<ImportExecutionMaterial, ImportExecutionError> {
        match self.status {
            ImportPlanStatus::Noop | ImportPlanStatus::Blocked => {
                Err(ImportExecutionError::RefusedStatus(self.status))
            }
            ImportPlanStatus::Reconcile => {
                self.scope
                    .take()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed import scope",
                    ))?;
                self.execution
                    .take()
                    .ok_or(ImportExecutionError::IncompletePlan(
                        "reconcile plan has no sealed execution material",
                    ))
            }
        }
    }
}

/// Minimal crate-internal handoff from receipt-backed import planning to the
/// hot-engine draft adapter. It carries no write capability, engine reference,
/// captured projection input, or publish authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportExecutionMaterial {
    import_id: ImportId,
    transaction: OperationTransaction,
    observation: ExternalImportObservationMaterial,
}

// The hot-engine adapter consumes this sealed material for drafting only;
// capability recapture and publication remain separate authority boundaries.
#[allow(dead_code)]
impl ImportExecutionMaterial {
    pub(crate) const fn import_id(&self) -> ImportId {
        self.import_id
    }

    pub(crate) fn batch_id(&self) -> BatchId {
        self.import_id.batch_id()
    }

    pub(crate) const fn origin(&self) -> BatchOrigin {
        BatchOrigin::ExternalReconciliation {
            import_id: self.import_id,
        }
    }

    pub(crate) fn transaction(&self) -> &OperationTransaction {
        &self.transaction
    }

    pub(crate) fn observation(&self) -> &ExternalImportObservationMaterial {
        &self.observation
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ImportId,
        OperationTransaction,
        ExternalImportObservationMaterial,
    ) {
        (self.import_id, self.transaction, self.observation)
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ImportExecutionError {
    RefusedStatus(ImportPlanStatus),
    IncompletePlan(&'static str),
    InvalidMaterial(String),
    OperationLimit,
    Observation(ExternalImportObservationMaterialError),
}

impl fmt::Display for ImportExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RefusedStatus(status) => {
                write!(
                    formatter,
                    "import plan status {status:?} cannot produce execution material"
                )
            }
            Self::IncompletePlan(detail) => formatter.write_str(detail),
            Self::InvalidMaterial(detail) => formatter.write_str(detail),
            Self::OperationLimit => write!(
                formatter,
                "external reconciliation operation count exceeds {MAX_TRANSACTION_OPERATIONS}"
            ),
            Self::Observation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportExecutionError {}

impl From<ExternalImportObservationMaterialError> for ImportExecutionError {
    fn from(error: ExternalImportObservationMaterialError) -> Self {
        Self::Observation(error)
    }
}

pub fn plan_affected_import(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[&str],
) -> ImportPlan {
    let mut instrumentation = ImportInstrumentation {
        requested_paths: requested_paths.len(),
        ..ImportInstrumentation::default()
    };
    let paths = match parse_requested_paths(requested_paths) {
        Ok(paths) => paths,
        Err(error) => return blocked_inventory_error(error, instrumentation),
    };
    instrumentation.path_bytes = paths.iter().map(|path| path.as_str().len() as u64).sum();
    let accepted_frontier = match engine.accepted_frontier_root() {
        Ok(root) => root,
        Err(error) => {
            return blocked_authority_error(
                None,
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    None,
                    error.to_string(),
                ),
                instrumentation,
            );
        }
    };
    let (catalog, catalog_authority) =
        match capture_affected_catalog(receipts, engine, &paths, &mut instrumentation) {
            Ok(snapshot) => snapshot,
            Err(block) => return blocked_authority_error(None, block, instrumentation),
        };
    let (inventory, inventory_fingerprints, first_raw_bytes) =
        match capture_inventory(graph, &paths, true, 0, &mut instrumentation) {
            Ok((Some(inventory), fingerprints, raw_bytes)) => (inventory, fingerprints, raw_bytes),
            Ok((None, _, _)) => unreachable!("retaining capture returns inventory"),
            Err(error) => return blocked_inventory_error(error, instrumentation),
        };
    let scope = match capture_import_scope(
        graph,
        receipts,
        engine,
        &paths,
        catalog,
        &mut instrumentation,
    ) {
        Ok(scope) => scope,
        Err(mut block) => {
            if block.observation.is_none() {
                block.observation = block
                    .paths
                    .first()
                    .and_then(|path| inventory_observation(&inventory, path));
            }
            return blocked_authority_error(Some(inventory), block, instrumentation);
        }
    };
    snapshot_revalidation_hook();
    let (_, second_fingerprints, _) =
        match capture_inventory(graph, &paths, false, first_raw_bytes, &mut instrumentation) {
            Ok(capture) => capture,
            Err(error) => {
                return blocked_authority_error(
                    Some(inventory),
                    authority_block(ImportBlockReason::StaleScope, None, error.to_string()),
                    instrumentation,
                );
            }
        };
    let (_, post_catalog_authority) =
        match capture_affected_catalog(receipts, engine, &paths, &mut instrumentation) {
            Ok(snapshot) => snapshot,
            Err(mut block) => {
                block.reason = ImportBlockReason::StaleScope;
                return blocked_authority_error(Some(inventory), block, instrumentation);
            }
        };
    let post_frontier = match post_snapshot_frontier(engine) {
        Ok(root) => root,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                authority_block(ImportBlockReason::StaleScope, None, error.to_string()),
                instrumentation,
            );
        }
    };
    if inventory_fingerprints != second_fingerprints
        || catalog_authority != post_catalog_authority
        || accepted_frontier != post_frontier
    {
        return blocked_authority_error(
            Some(inventory),
            authority_block(
                ImportBlockReason::StaleScope,
                None,
                "inventory, exact affected receipt authority, or accepted frontier changed between snapshot passes",
            ),
            instrumentation,
        );
    };
    // Equal bounded collections detect stale diagnostic input only under the
    // explicit quiescent-writer boundary. They are neither a portable atomic
    // filesystem snapshot nor authority for later publication.
    plan_import(inventory, scope, engine, instrumentation)
}

#[cfg(test)]
fn snapshot_revalidation_hook() {
    SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(not(test))]
fn snapshot_revalidation_hook() {}

#[cfg(test)]
fn post_snapshot_frontier(
    engine: &ShardedHotEngine,
) -> Result<AcceptedFrontierRoot, super::EngineError> {
    POST_FRONTIER_OVERRIDE
        .with(|root| root.borrow_mut().take())
        .map_or_else(|| engine.accepted_frontier_root(), Ok)
}

#[cfg(not(test))]
fn post_snapshot_frontier(
    engine: &ShardedHotEngine,
) -> Result<AcceptedFrontierRoot, super::EngineError> {
    engine.accepted_frontier_root()
}

fn parse_requested_paths(requested_paths: &[&str]) -> Result<Vec<ManagedPath>, InventoryError> {
    if requested_paths.len() > MAX_IMPORT_FILES {
        return Err(InventoryError::ResourceBudgetExceeded {
            resource: "requested managed path count",
            observed: requested_paths.len() as u64,
            limit: MAX_IMPORT_FILES as u64,
        });
    }
    let mut paths = Vec::with_capacity(requested_paths.len());
    let mut exact = BTreeSet::new();
    let mut path_bytes = 0_u64;
    for requested in requested_paths {
        path_bytes = charge_budget(
            "aggregate requested path bytes",
            path_bytes,
            requested.len() as u64,
            MAX_IMPORT_PATH_BYTES,
        )?;
        let path = ManagedPath::parse((*requested).to_owned())
            .map_err(|_| InventoryError::UnsafePath((*requested).to_owned()))?;
        if !exact.insert(path.clone()) {
            return Err(InventoryError::DuplicateRequestedPath(
                path.as_str().to_owned(),
            ));
        }
        paths.push(path);
    }
    require_portable_unique(&paths)?;
    paths.sort_unstable();
    Ok(paths)
}

fn capture_inventory(
    graph: &Graph,
    paths: &[ManagedPath],
    retain: bool,
    retained_raw_bytes: u64,
    instrumentation: &mut ImportInstrumentation,
) -> Result<
    (
        Option<RawInventory>,
        BTreeMap<ManagedPath, InventoryPathFingerprint>,
        u64,
    ),
    InventoryError,
> {
    instrumentation.inventory_passes = instrumentation.inventory_passes.saturating_add(1);
    let mut entries = retain.then(|| Vec::with_capacity(paths.len()));
    let mut fingerprints = BTreeMap::new();
    let mut raw_bytes = 0_u64;
    for path in paths {
        let observation =
            graph
                .read_raw_managed_text(path)
                .map_err(|error| InventoryError::UnsafeEntry {
                    path: Some(path.as_str().to_owned()),
                    message: error.to_string(),
                })?;
        let (raw, fingerprint) = match observation {
            Some(observation) => {
                let description = observation.description();
                raw_bytes = charge_budget(
                    "aggregate raw bytes",
                    raw_bytes,
                    observation.bytes().len() as u64,
                    MAX_IMPORT_RAW_BYTES,
                )?;
                instrumentation.bytes_read = instrumentation
                    .bytes_read
                    .saturating_add(observation.physical_bytes_read());
                instrumentation.bytes_hashed = instrumentation
                    .bytes_hashed
                    .saturating_add(observation.physical_bytes_read());
                instrumentation.peak_owned_raw_bytes = instrumentation.peak_owned_raw_bytes.max(
                    retained_raw_bytes.saturating_add(observation.peak_capture_buffer_bytes()),
                );
                let fingerprint = InventoryPathFingerprint {
                    state: ImportInventoryState::Present(description),
                    file_resource_id: Some(observation.file_resource_id()),
                };
                let (bytes, description) = observation.into_parts();
                let raw = RawObservation::Present(ExactBytes::from_description(bytes, description));
                (raw, fingerprint)
            }
            None => (
                RawObservation::Absent,
                InventoryPathFingerprint {
                    state: ImportInventoryState::Absent,
                    file_resource_id: None,
                },
            ),
        };
        if let Some(entries) = &mut entries {
            entries.push((path.clone(), raw));
            instrumentation.peak_owned_raw_bytes =
                instrumentation.peak_owned_raw_bytes.max(raw_bytes);
        }
        fingerprints.insert(path.clone(), fingerprint);
    }
    let inventory = entries.map(RawInventory::from_entries).transpose()?;
    Ok((inventory, fingerprints, raw_bytes))
}

/// Capture only the durable receipts reachable from the enrolled completed-work
/// mapping for the requested exact paths.  The work index's authenticated
/// Patricia root supplies the path-to-intent ids; the receipt store then makes
/// the matching immutable point reads.  No receipt directory is enumerated.
fn capture_affected_catalog(
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[ManagedPath],
    instrumentation: &mut ImportInstrumentation,
) -> Result<(AffectedReceiptCatalog, CatalogAuthority), ImportBlock> {
    let (_, work_index) = engine.enrolled_projection_runtime().map_err(|error| {
        authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            error.to_string(),
        )
    })?;
    if work_index.receipt_store_id() != receipts.store_id() {
        return Err(authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "enrolled work index is not bound to the receipt store",
        ));
    }

    let mut catalog = AffectedReceiptCatalog::default();
    let mut captured_entries = 0usize;
    let mut hasher = Sha256::new();
    hasher.update(b"tine/import-affected-receipt-snapshot/v2\0");
    hasher.update(receipts.store_id().as_bytes());
    for path in requested_paths {
        hasher.update((path.as_str().len() as u64).to_be_bytes());
        hasher.update(path.as_str().as_bytes());
        let completed = work_index
            .completed_receipts_for_path(path)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    format!("authenticated completed-work path lookup failed: {error}"),
                )
            })?;
        if completed.len() > MAX_IMPORT_CATALOG_ENTRIES.saturating_sub(captured_entries) {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!(
                    "affected completed-work entry budget exceeded: observed {} after {}, limit {}",
                    completed.len(),
                    captured_entries,
                    MAX_IMPORT_CATALOG_ENTRIES
                ),
            ));
        }
        let mut entries = Vec::with_capacity(completed.len());
        for completed in completed {
            let (intent, completion) =
                receipts
                    .load_completed_receipt(&completed)
                    .map_err(|error| {
                        let reason =
                            if matches!(error, ProjectionStoreError::MissingPriorCompletion) {
                                ImportBlockReason::MissingBase
                            } else {
                                ImportBlockReason::CorruptBase
                            };
                        authority_block(
                            reason,
                            Some(path),
                            format!("durable exact receipt lookup is invalid: {error}"),
                        )
                    })?;
            let intent_bytes = intent.encode().map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
            let completion_bytes = completion.encode().map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
            instrumentation.catalog_entries = instrumentation.catalog_entries.saturating_add(1);
            instrumentation.catalog_bytes_hashed = instrumentation
                .catalog_bytes_hashed
                .saturating_add(intent_bytes.len() as u64)
                .saturating_add(completion_bytes.len() as u64);
            hasher.update(completed.intent_id().as_bytes());
            hasher.update((intent_bytes.len() as u64).to_be_bytes());
            hasher.update(intent_bytes);
            hasher.update((completion_bytes.len() as u64).to_be_bytes());
            hasher.update(completion_bytes);
            entries.push(AffectedReceiptEntry {
                completed,
                intent,
                completion,
            });
            captured_entries = captured_entries.saturating_add(1);
        }
        catalog.by_path.insert(path.clone(), entries);
    }
    Ok((
        catalog,
        CatalogAuthority {
            digest: ContentDigest::from_bytes(hasher.finalize().into()),
        },
    ))
}

fn capture_import_scope(
    graph: &Graph,
    receipts: &ProjectionReceiptStore,
    engine: &ShardedHotEngine,
    requested_paths: &[ManagedPath],
    catalog: AffectedReceiptCatalog,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ImportScopeSnapshot, ImportBlock> {
    let endpoint = engine.projection_endpoint_binding().ok_or_else(|| {
        authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "import authority requires an enrolled projection endpoint",
        )
    })?;
    if engine.workspace_id() != receipts.workspace_id()
        || receipts.endpoint_binding() != Some(endpoint)
        || engine.projection_receipt_store_id() != Some(receipts.store_id())
        || graph.canonical_resource_id().map_err(|error| {
            authority_block(
                ImportBlockReason::AuthorityUnavailable,
                None,
                error.to_string(),
            )
        })? != endpoint.graph_resource_id
    {
        return Err(authority_block(
            ImportBlockReason::AuthorityUnavailable,
            None,
            "graph, engine, receipt workspace, or endpoint binding differs",
        ));
    }

    let mut paths = BTreeMap::new();
    let mut path_identities = BTreeMap::new();
    for path in requested_paths {
        let entry = graph
            .managed_entry_for_managed_path(path)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::UnsafeInput,
                    Some(path),
                    format!("managed path cannot be decoded with Graph loading semantics: {error}"),
                )
            })?;
        let name = LogicalPageName::parse(entry.name).map_err(|error| {
            authority_block(
                ImportBlockReason::UnsafeInput,
                Some(path),
                format!("managed path has an invalid logical page name: {error}"),
            )
        })?;
        let kind = match entry.kind {
            PageKind::Page => ManagedTextKind::Page,
            PageKind::Journal => ManagedTextKind::Journal,
        };
        path_identities.insert(path.clone(), ImportedPathIdentity { name, kind });
        instrumentation.catalog_path_lookups =
            instrumentation.catalog_path_lookups.saturating_add(1);
        let catalog_entries = catalog.by_path.get(path).map(Vec::as_slice).unwrap_or(&[]);
        let current_owner = engine.current_page_at_path(path).map_err(|error| {
            authority_block(
                ImportBlockReason::AuthorityUnavailable,
                Some(path),
                error.to_string(),
            )
        })?;
        let page_id = match current_owner {
            CurrentPageAtPath::ExactOwner(occupied) => occupied.page_id(),
            CurrentPageAtPath::Released(release) => {
                let (_, work_index) = engine.enrolled_projection_runtime().map_err(|error| {
                    authority_block(
                        ImportBlockReason::AuthorityUnavailable,
                        Some(path),
                        error.to_string(),
                    )
                })?;
                let work = engine
                    .authorize_projected_release(&work_index, &release)
                    .map_err(|error| {
                        authority_block(
                            ImportBlockReason::ConflictingLocalTail,
                            Some(path),
                            format!("released path lacks completed durable work: {error}"),
                        )
                    })?;
                let mut completion_id = None;
                for entry in catalog_entries {
                    if entry.intent.workspace_id() != engine.workspace_id()
                        || entry.intent.page_id() != release.prior_page_id()
                        || entry.intent.path() != path
                        || entry.intent.frontier() != work.post_frontier()
                        || entry.intent.target() != BlobDescription::of(&[])
                        || entry.completed.page_id() != work.page_id()
                        || entry.completed.frontier() != work.post_frontier()
                        || entry.completed.target() != super::ProjectionWorkTarget::Absent
                    {
                        continue;
                    }
                    let logical = entry.completion.logical_completion_id();
                    if completion_id.replace(logical).is_some() {
                        return Err(authority_block(
                            ImportBlockReason::CorruptBase,
                            Some(path),
                            "multiple completed receipts claim one authenticated path release",
                        ));
                    }
                }
                let completion_id = completion_id.ok_or_else(|| {
                    authority_block(
                        ImportBlockReason::ConflictingLocalTail,
                        Some(path),
                        "authenticated path release has no exact completed receipt",
                    )
                })?;
                paths.insert(path.clone(), ScopedPathEvidence::Released(completion_id));
                continue;
            }
            CurrentPageAtPath::Unowned => {
                if !catalog_entries.is_empty() {
                    return Err(authority_block(
                        ImportBlockReason::ConflictingLocalTail,
                        Some(path),
                        "receipt-backed path is no longer owned at the accepted engine frontier",
                    ));
                }
                paths.insert(path.clone(), ScopedPathEvidence::New);
                continue;
            }
            CurrentPageAtPath::PortableCollision(occupied) => {
                return Err(authority_block(
                    ImportBlockReason::PortablePathCollision,
                    Some(path),
                    format!(
                        "requested path collides with engine-owned {} for page {}",
                        occupied.exact_path(),
                        occupied.page_id()
                    ),
                ));
            }
            CurrentPageAtPath::ReleasedPortableCollision(release) => {
                return Err(authority_block(
                    ImportBlockReason::PortablePathCollision,
                    Some(path),
                    format!(
                        "requested path collides with authenticated released spelling {} for page {}",
                        release.prior_exact_path(),
                        release.prior_page_id()
                    ),
                ));
            }
        };

        let current = engine
            .authorize_projection_write(page_id)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(path),
                    format!("accepted Ready projection authority is unavailable: {error}"),
                )
            })?;
        if current.state().page.path != *path {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(path),
                "portable-path ownership and materialized page path disagree",
            ));
        }

        let mut exact = None;
        let mut replay_cache =
            BTreeMap::<Option<BlobDescription>, (ProjectionIntent, Vec<u8>)>::new();
        for entry in catalog_entries {
            if entry.intent.workspace_id() != engine.workspace_id()
                || entry.intent.page_id() != page_id
                || entry.intent.frontier() != &current.state().frontier
                || entry.intent.claim_evidence() != current.state().claim_evidence
            {
                continue;
            }
            let base_key = match entry.intent.precondition() {
                super::ProjectionPrecondition::Absent => None,
                super::ProjectionPrecondition::Base(description) => Some(*description),
            };
            if let std::collections::btree_map::Entry::Vacant(slot) = replay_cache.entry(base_key) {
                let declared_base_bytes = base_key.map_or(0, BlobDescription::byte_length);
                reserve_base_replay(
                    instrumentation,
                    declared_base_bytes,
                    IMPORT_REPLAY_LIMITS,
                    path,
                )?;
                let base = receipts.load_base(&entry.intent).map_err(|error| {
                    authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        format!("canonical base evidence is unavailable: {error}"),
                    )
                })?;
                let replay = plan_projection(
                    engine.workspace_id(),
                    current.state(),
                    base.as_ref().map(super::BaseBlob::bytes),
                )
                .map_err(|error| {
                    authority_block(
                        ImportBlockReason::AuthorityUnavailable,
                        Some(path),
                        format!("accepted state cannot be replayed canonically: {error}"),
                    )
                })?;
                retain_rendered_target(
                    instrumentation,
                    replay.target().len() as u64,
                    IMPORT_REPLAY_LIMITS,
                    path,
                )?;
                slot.insert(replay.into_intent_and_target());
            }
            let (replayed_intent, _) = &replay_cache[&base_key];
            if replayed_intent == &entry.intent {
                if exact.is_some() {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "multiple durable receipt rows claim one current accepted path/frontier",
                    ));
                }
                exact = Some((entry, base_key));
            }
        }
        let Some((entry, base_key)) = exact else {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(path),
                "no durable completion/base exactly matches the current accepted affected frontier",
            ));
        };
        let replayed_target = replay_cache
            .remove(&base_key)
            .expect("exact replay key remains cached")
            .1;
        let completion = entry.completion.clone();
        completion
            .validate_against(&entry.intent)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::CorruptBase,
                    Some(path),
                    error.to_string(),
                )
            })?;
        paths.insert(
            path.clone(),
            ScopedPathEvidence::Existing(ReceiptBackedPage {
                intent: entry.intent.clone(),
                completion,
                replayed_target: ExactBytes::from_description(
                    replayed_target,
                    entry.intent.target(),
                ),
                page: current.state().page.clone(),
            }),
        );
    }

    Ok(ImportScopeSnapshot {
        workspace_id: engine.workspace_id(),
        paths,
        path_identities,
    })
}

fn authority_block(
    reason: ImportBlockReason,
    path: Option<&ManagedPath>,
    detail: impl Into<String>,
) -> ImportBlock {
    ImportBlock {
        reason,
        paths: path
            .into_iter()
            .map(|path| path.as_str().to_owned())
            .collect(),
        logical_completion_ids: Vec::new(),
        observation: None,
        detail: detail.into(),
    }
}

fn plan_import(
    inventory: RawInventory,
    scope: ImportScopeSnapshot,
    engine: &ShardedHotEngine,
    mut instrumentation: ImportInstrumentation,
) -> ImportPlan {
    if scope.paths.len() != inventory.entries().len()
        || scope.path_identities.len() != inventory.entries().len()
        || scope
            .paths
            .keys()
            .zip(inventory.entries().keys())
            .any(|(left, right)| left != right)
        || scope
            .path_identities
            .keys()
            .zip(inventory.entries().keys())
            .any(|(left, right)| left != right)
    {
        return blocked_authority_error(
            Some(inventory),
            authority_block(
                ImportBlockReason::StaleScope,
                None,
                "sealed scope and exact inventory path sets differ",
            ),
            instrumentation,
        );
    }
    let completed = scope
        .paths
        .values()
        .filter_map(|evidence| match evidence {
            ScopedPathEvidence::Existing(page) => Some(page),
            ScopedPathEvidence::Released(_) | ScopedPathEvidence::New => None,
        })
        .collect::<Vec<_>>();

    let conflict_copy = inventory
        .entries()
        .keys()
        .find(|path| path_is_sync_conflict(Path::new(path.as_str())))
        .cloned();
    if let Some(path) = conflict_copy {
        return blocked_authority_error(
            Some(inventory),
            ImportBlock {
                reason: ImportBlockReason::UnsafeInput,
                paths: vec![path.as_str().to_owned()],
                logical_completion_ids: Vec::new(),
                observation: None,
                detail: "provider conflict copies are diagnostic inputs and cannot authorize import identity or deletion".into(),
            },
            instrumentation,
        );
    }

    let invalid_inventory = inventory.entries().iter().find_map(|(path, observation)| {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        matches!(observation, RawObservation::Present(bytes) if std::str::from_utf8(bytes.bytes()).is_err())
            .then(|| path.clone())
    });
    if let Some(path) = invalid_inventory {
        let block = ImportBlock {
            reason: ImportBlockReason::UnsafeInput,
            paths: vec![path.as_str().to_owned()],
            logical_completion_ids: Vec::new(),
            observation: inventory_observation(&inventory, path.as_str()),
            detail: "raw bytes were retained, but semantic import requires valid UTF-8".into(),
        };
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }
    if let Some(page) = completed
        .iter()
        .find(|page| std::str::from_utf8(page.bytes()).is_err())
    {
        let block = receipt_block(
            ImportBlockReason::CorruptBase,
            page.path(),
            Some(page.logical_completion_id()),
            &inventory,
            "receipt-backed replay target is not UTF-8",
        );
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

    let page_matches = match match_pages(&inventory, &completed, &mut instrumentation) {
        Ok(matches) => matches,
        Err(block) => return blocked_authority_error(Some(inventory), block, instrumentation),
    };
    let mut matches = ImportMatches {
        pages: page_matches,
        ..ImportMatches::default()
    };
    if let Err(block) = match_blocks(&inventory, &completed, &mut matches, &mut instrumentation) {
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

    let mut completion_ids = completed
        .iter()
        .map(|page| page.logical_completion_id())
        .collect::<Vec<_>>();
    completion_ids.extend(scope.paths.values().filter_map(|evidence| match evidence {
        ScopedPathEvidence::Released(completion_id) => Some(*completion_id),
        ScopedPathEvidence::Existing(_) | ScopedPathEvidence::New => None,
    }));
    completion_ids.sort_unstable();
    completion_ids.dedup();
    let derivation_entries = match inventory.derivation_entries(&scope.path_identities) {
        Ok(entries) => entries,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                ImportBlock {
                    reason: ImportBlockReason::StaleScope,
                    paths: Vec::new(),
                    logical_completion_ids: completion_ids,
                    observation: None,
                    detail: error.to_string(),
                },
                instrumentation,
            );
        }
    };
    let import_id = match ImportId::derive(
        scope.workspace_id,
        &completion_ids,
        &derivation_entries,
        DIFF_SCHEMA_VERSION,
    ) {
        Ok(import_id) => import_id,
        Err(error) => {
            return blocked_authority_error(
                Some(inventory),
                ImportBlock {
                    reason: ImportBlockReason::CorruptBase,
                    paths: Vec::new(),
                    logical_completion_ids: completion_ids,
                    observation: None,
                    detail: error.to_string(),
                },
                instrumentation,
            );
        }
    };

    let page_transition =
        match build_desired_page_transition(&inventory, &matches, &scope, import_id) {
            Ok(transition) => transition,
            Err(block) => {
                return blocked_authority_error(Some(inventory), block, instrumentation);
            }
        };
    if let Err(block) = preflight_desired_page_names(&inventory, &page_transition, engine) {
        return blocked_authority_error(Some(inventory), block, instrumentation);
    }

    let completed_paths = completed
        .iter()
        .map(|page| page.path().clone())
        .collect::<BTreeSet<_>>();
    let changed = completed.iter().any(|page| {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        !matches!(
            inventory.entries().get(page.path()),
            Some(RawObservation::Present(bytes)) if bytes.description() == page.description()
        )
    }) || inventory.entries().iter().any(|(path, observation)| {
        matches!(observation, RawObservation::Present(_)) && !completed_paths.contains(path)
    });
    let execution = if changed {
        match build_execution_material(
            import_id,
            &inventory,
            &matches,
            &scope,
            &page_transition,
            &mut instrumentation,
        ) {
            Ok(execution) => Some(execution),
            Err(error) => {
                return blocked_authority_error(
                    Some(inventory),
                    ImportBlock {
                        reason: if matches!(&error, ImportExecutionError::OperationLimit) {
                            ImportBlockReason::ResourceLimit
                        } else {
                            ImportBlockReason::UnsafeInput
                        },
                        paths: Vec::new(),
                        logical_completion_ids: completion_ids,
                        observation: None,
                        detail: format!(
                            "sealed external reconciliation cannot produce canonical execution material: {error}"
                        ),
                    },
                    instrumentation,
                );
            }
        }
    } else {
        None
    };
    ImportPlan {
        status: if changed {
            ImportPlanStatus::Reconcile
        } else {
            ImportPlanStatus::Noop
        },
        import_id: Some(import_id),
        inventory: Some(inventory),
        matches: Some(matches),
        scope: changed.then_some(scope),
        execution,
        blocks: Vec::new(),
        instrumentation,
    }
}

/// Refuse a transaction before authoring when two affected files would acquire
/// one logical page name, or when an affected destination name is already
/// owned by another authenticated page.  Paths are deliberately not used as a
/// name namespace: duplicate basenames at different paths remain visible
/// ambiguity instead of a silently successful reconciliation.
fn preflight_desired_page_names(
    inventory: &RawInventory,
    transition: &DesiredPageTransition,
    engine: &ShardedHotEngine,
) -> Result<(), ImportBlock> {
    let mut desired = BTreeMap::new();
    for (path, page) in &transition.pages {
        if let Some((prior_path, prior_page_id, prior_name)) = desired.insert(
            page.name.key_digest(),
            (path.clone(), page.page_id, page.name.clone()),
        ) {
            if prior_page_id != page.page_id {
                return Err(ImportBlock {
                    reason: ImportBlockReason::ConflictingLocalTail,
                    paths: vec![prior_path.as_str().to_owned(), path.as_str().to_owned()],
                    logical_completion_ids: Vec::new(),
                    observation: inventory_observation(inventory, path.as_str()),
                    detail: format!(
                        "affected paths decode to the same logical page name: {} and {}",
                        prior_name.as_str(),
                        page.name.as_str()
                    ),
                });
            }
        }
    }
    for (_, (path, page_id, name)) in desired {
        let owner = engine
            .current_page_for_logical_name(&name)
            .map_err(|error| {
                authority_block(
                    ImportBlockReason::AuthorityUnavailable,
                    Some(&path),
                    format!("authenticated logical page-name lookup failed: {error}"),
                )
            })?;
        if owner.is_some_and(|owner| {
            owner != page_id && !transition.released_name_owners.contains(&owner)
        }) {
            return Err(authority_block(
                ImportBlockReason::ConflictingLocalTail,
                Some(&path),
                format!(
                    "decoded destination logical page name {} is already owned by page {}",
                    name.as_str(),
                    owner.expect("checked above")
                ),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct CurrentImportBlock {
    page_id: PageId,
    block: super::MaterializedBlock,
}

#[derive(Clone, Debug)]
struct DesiredImportPage {
    page_id: PageId,
    home_document_id: DocumentId,
    name: LogicalPageName,
    path: ManagedPath,
    kind: ManagedTextKind,
    existing: bool,
}

#[derive(Clone, Debug)]
struct DesiredPageTransition {
    pages: BTreeMap<ManagedPath, DesiredImportPage>,
    /// Current affected owners whose present logical name is absent from the
    /// final ownership set for that same page identity. The page-name index
    /// validates a transaction's final catalog atomically, so chains and cycles
    /// may consume these released names without exposing an intermediate state.
    released_name_owners: BTreeSet<PageId>,
}

fn build_desired_page_transition(
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
    import_id: ImportId,
) -> Result<DesiredPageTransition, ImportBlock> {
    let mut page_matches = BTreeMap::<ManagedPath, &PageImportMatch>::new();
    for page_match in matches.pages() {
        if page_matches
            .insert(page_match.path().clone(), page_match)
            .is_some()
        {
            return Err(authority_block(
                ImportBlockReason::CorruptBase,
                Some(page_match.path()),
                "sealed import matches contain duplicate external page paths",
            ));
        }
    }

    let mut pages = BTreeMap::new();
    let mut desired_paths_by_page = BTreeMap::new();
    for (path, observation) in inventory.entries() {
        if !matches!(observation, RawObservation::Present(_)) {
            continue;
        }
        let path_identity = scope.path_identities.get(path).ok_or_else(|| {
            authority_block(
                ImportBlockReason::StaleScope,
                Some(path),
                "present inventory path has no Graph-decoded logical identity",
            )
        })?;
        let desired = match page_matches.get(path) {
            Some(page_match) => {
                let Some(ScopedPathEvidence::Existing(existing)) =
                    scope.paths.get(page_match.previous_path())
                else {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "matched external page has no sealed receipt-backed predecessor",
                    ));
                };
                let current = existing.materialized_page();
                if current.page_id != page_match.page_id() {
                    return Err(authority_block(
                        ImportBlockReason::CorruptBase,
                        Some(path),
                        "matched external page identity differs from its sealed predecessor",
                    ));
                }
                DesiredImportPage {
                    page_id: current.page_id,
                    home_document_id: current.home_document_id,
                    name: path_identity.name.clone(),
                    path: path.clone(),
                    kind: path_identity.kind,
                    existing: true,
                }
            }
            None => DesiredImportPage {
                page_id: import_id.unmatched_page_id(&ImportLocator::page(path.clone())),
                home_document_id: DocumentId::for_unmatched_import_page(
                    scope.workspace_id,
                    path.as_str().as_bytes(),
                ),
                name: path_identity.name.clone(),
                path: path.clone(),
                kind: path_identity.kind,
                existing: false,
            },
        };
        if let Some(prior_path) = desired_paths_by_page.insert(desired.page_id, path.clone()) {
            return Err(ImportBlock {
                reason: ImportBlockReason::ConflictingLocalTail,
                paths: vec![prior_path.as_str().to_owned(), path.as_str().to_owned()],
                logical_completion_ids: Vec::new(),
                observation: inventory_observation(inventory, path.as_str()),
                detail: "one affected page identity would survive at more than one path".into(),
            });
        }
        pages.insert(path.clone(), desired);
    }

    let final_names_by_page = pages
        .values()
        .map(|page| (page.page_id, page.name.key_digest()))
        .collect::<BTreeMap<_, _>>();
    let released_name_owners = scope
        .paths
        .values()
        .filter_map(|evidence| {
            let ScopedPathEvidence::Existing(existing) = evidence else {
                return None;
            };
            let current = existing.materialized_page();
            (final_names_by_page.get(&current.page_id).copied() != Some(current.name.key_digest()))
                .then_some(current.page_id)
        })
        .collect();
    Ok(DesiredPageTransition {
        pages,
        released_name_owners,
    })
}

#[derive(Clone, Debug)]
struct DesiredImportBlock {
    block_id: BlockId,
    page_id: PageId,
    home_document_id: DocumentId,
    parent: Option<BlockId>,
    order: String,
    content: String,
    logseq_uuid: Option<LogseqUuid>,
    existing: bool,
}

fn push_operation(
    operations: &mut Vec<SemanticOperation>,
    operation: SemanticOperation,
) -> Result<(), ImportExecutionError> {
    if operations.len() == MAX_TRANSACTION_OPERATIONS {
        return Err(ImportExecutionError::OperationLimit);
    }
    operations.push(operation);
    Ok(())
}

fn build_execution_material(
    import_id: ImportId,
    inventory: &RawInventory,
    matches: &ImportMatches,
    scope: &ImportScopeSnapshot,
    page_transition: &DesiredPageTransition,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ImportExecutionMaterial, ImportExecutionError> {
    let mut current_pages = BTreeMap::<PageId, &ReceiptBackedPage>::new();
    let mut current_blocks = BTreeMap::<BlockId, CurrentImportBlock>::new();
    for evidence in scope.paths.values() {
        let ScopedPathEvidence::Existing(page) = evidence else {
            continue;
        };
        let materialized = page.materialized_page();
        if materialized.page_id != page.page_id() || materialized.path != *page.path() {
            return Err(ImportExecutionError::InvalidMaterial(
                "receipt-backed page identity does not match its materialized accepted state"
                    .into(),
            ));
        }
        if current_pages.insert(materialized.page_id, page).is_some() {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed import scope contains one page more than once".into(),
            ));
        }
        for block in &materialized.blocks {
            if current_blocks
                .insert(
                    block.block_id,
                    CurrentImportBlock {
                        page_id: materialized.page_id,
                        block: block.clone(),
                    },
                )
                .is_some()
            {
                return Err(ImportExecutionError::InvalidMaterial(
                    "sealed import scope contains one visible block more than once".into(),
                ));
            }
        }
    }

    let desired_pages = &page_transition.pages;

    let mut trees = BTreeMap::<ManagedPath, ParsedTree>::new();
    for (path, observation) in inventory.entries() {
        let RawObservation::Present(bytes) = observation else {
            continue;
        };
        if std::str::from_utf8(bytes.bytes()).is_err() {
            return Err(ImportExecutionError::InvalidMaterial(format!(
                "sealed inventory path {path} is not valid UTF-8"
            )));
        }
        trees.insert(
            path.clone(),
            parse_nodes(path, bytes.bytes(), instrumentation)
                .map_err(|block| ImportExecutionError::InvalidMaterial(block.detail))?,
        );
    }

    let mut block_matches = BTreeMap::<(ManagedPath, StructuralLocator), BlockId>::new();
    for block_match in matches.blocks() {
        if !trees.contains_key(block_match.path()) {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed block match refers to an absent external path".into(),
            ));
        }
        if block_matches
            .insert(
                (block_match.path().clone(), block_match.locator().clone()),
                block_match.block_id(),
            )
            .is_some()
        {
            return Err(ImportExecutionError::InvalidMaterial(
                "sealed import matches contain duplicate external block locators".into(),
            ));
        }
    }
    let rejected_raw_ids = matches
        .rejected_raw_ids()
        .iter()
        .map(|rejected| (rejected.path().clone(), rejected.locator().clone()))
        .collect::<BTreeSet<_>>();

    let mut desired_blocks = BTreeMap::<BlockId, DesiredImportBlock>::new();
    let mut desired_node_ids = BTreeMap::<(ManagedPath, usize), BlockId>::new();
    let mut observation_entries = Vec::with_capacity(inventory.entries().len());
    for (path, observation) in inventory.entries() {
        let kind = scope
            .path_identities
            .get(path)
            .ok_or(ImportExecutionError::IncompletePlan(
                "sealed inventory path has no Graph-decoded managed kind",
            ))?
            .kind;
        let state = match observation {
            RawObservation::Absent => ExternalImportObservationState::Absent,
            RawObservation::Present(bytes) => {
                let tree = trees.get(path).ok_or_else(|| {
                    ImportExecutionError::InvalidMaterial(
                        "sealed present inventory path has no parsed tree".into(),
                    )
                })?;
                let page = desired_pages.get(path).ok_or_else(|| {
                    ImportExecutionError::InvalidMaterial(
                        "sealed present inventory path has no desired page".into(),
                    )
                })?;
                let mut annotations = Vec::with_capacity(tree.nodes.len());
                for index in 0..tree.nodes.len() {
                    let locator = materialize_locator(tree, index, instrumentation)
                        .map_err(|block| ImportExecutionError::InvalidMaterial(block.detail))?;
                    let matched = block_matches.get(&(path.clone(), locator.clone())).copied();
                    let (block_id, existing, home_document_id) = match matched {
                        Some(block_id) => {
                            let current = current_blocks.get(&block_id).ok_or_else(|| {
                                ImportExecutionError::InvalidMaterial(
                                    "sealed block match has no accepted current block".into(),
                                )
                            })?;
                            (block_id, true, current.block.home_document_id)
                        }
                        None => (
                            import_id.unmatched_block_id(&ImportLocator::block(
                                path.clone(),
                                locator.clone(),
                            )),
                            false,
                            page.home_document_id,
                        ),
                    };
                    if desired_node_ids
                        .insert((path.clone(), index), block_id)
                        .is_some()
                    {
                        return Err(ImportExecutionError::InvalidMaterial(
                            "sealed parsed tree contains a duplicate block node".into(),
                        ));
                    }
                    let logseq_uuid =
                        external_logseq_uuid(path, &locator, &tree.nodes[index], &rejected_raw_ids);
                    annotations.push(AnnotatedIdentity::new(
                        locator.clone(),
                        tree.nodes[index].span,
                        block_id,
                        logseq_uuid,
                    ));
                    let parent = tree.nodes[index].parent.map(|parent| {
                        desired_node_ids
                            .get(&(path.clone(), parent))
                            .expect("parsed tree parents precede their children")
                            .to_owned()
                    });
                    let desired = DesiredImportBlock {
                        block_id,
                        page_id: page.page_id,
                        home_document_id,
                        parent,
                        order: imported_order(tree.nodes[index].sibling_position),
                        content: tree.nodes[index].raw.clone(),
                        logseq_uuid,
                        existing,
                    };
                    if desired_blocks.insert(block_id, desired).is_some() {
                        return Err(ImportExecutionError::InvalidMaterial(
                            "sealed matches assign one block identity more than once".into(),
                        ));
                    }
                }
                ExternalImportObservationState::present(bytes.bytes().to_vec(), annotations)
                    .map_err(|error| {
                        ImportExecutionError::Observation(
                            ExternalImportObservationMaterialError::Observation(error),
                        )
                    })?
            }
        };
        observation_entries.push(
            ExternalImportObservationEntry::new(path.clone(), kind, state).map_err(|error| {
                ImportExecutionError::Observation(
                    ExternalImportObservationMaterialError::Observation(error),
                )
            })?,
        );
    }
    let observation =
        ExternalImportObservationMaterial::new(scope.workspace_id, import_id, observation_entries)
            .map_err(|error| {
                ImportExecutionError::Observation(
                    ExternalImportObservationMaterialError::Observation(error),
                )
            })?;

    let mut operations = Vec::new();
    for page in desired_pages.values().filter(|page| !page.existing) {
        push_operation(
            &mut operations,
            SemanticOperation::CreatePage {
                page_id: page.page_id,
                home_document_id: page.home_document_id,
                name: page.name.clone(),
                path: page.path.clone(),
                kind: page.kind,
            },
        )?;
    }
    for page in desired_pages.values().filter(|page| page.existing) {
        let current = current_pages.get(&page.page_id).ok_or_else(|| {
            ImportExecutionError::InvalidMaterial(
                "desired existing page is absent from sealed accepted state".into(),
            )
        })?;
        let current = current.materialized_page();
        let current_kind = scope
            .path_identities
            .get(&current.path)
            .ok_or(ImportExecutionError::IncompletePlan(
                "sealed existing page has no Graph-decoded current managed kind",
            ))?
            .kind;
        if current.name != page.name || current.path != page.path || current_kind != page.kind {
            push_operation(
                &mut operations,
                SemanticOperation::ReconcileExternalPageState {
                    page_id: page.page_id,
                    name: page.name.clone(),
                    path: page.path.clone(),
                    kind: page.kind,
                },
            )?;
        }
    }
    let mut new_blocks = desired_blocks
        .values()
        .filter(|block| !block.existing)
        .collect::<Vec<_>>();
    new_blocks.sort_unstable_by(|left, right| {
        desired_block_depth(left.block_id, &desired_blocks)
            .cmp(&desired_block_depth(right.block_id, &desired_blocks))
            .then_with(|| left.block_id.cmp(&right.block_id))
    });
    for desired in new_blocks {
        push_operation(
            &mut operations,
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: desired.block_id,
                    home_document_id: desired.home_document_id,
                },
                page_id: desired.page_id,
                parent: desired.parent,
                order: desired.order.clone(),
                content: desired.content.clone(),
            },
        )?;
    }

    let mut moves = desired_blocks
        .iter()
        .filter_map(|(block_id, desired)| {
            let current = current_blocks.get(block_id)?;
            (current.page_id != desired.page_id).then_some((*block_id, desired, current))
        })
        .collect::<Vec<_>>();
    moves.sort_unstable_by(|(left_id, _, left), (right_id, _, right)| {
        current_block_depth(*left_id, &current_blocks)
            .cmp(&current_block_depth(*right_id, &current_blocks))
            .reverse()
            .then_with(|| left_id.cmp(right_id))
            .then_with(|| left.page_id.cmp(&right.page_id))
    });
    for (block_id, desired, current) in &moves {
        push_operation(
            &mut operations,
            SemanticOperation::MoveSubtree {
                root: BlockLocation {
                    block_id: *block_id,
                    home_document_id: current.block.home_document_id,
                },
                from_page_id: current.page_id,
                to_page_id: desired.page_id,
                parent: desired.parent,
                order: desired.order.clone(),
            },
        )?;
    }

    let moved_blocks = moves
        .iter()
        .map(|(block_id, _, _)| *block_id)
        .collect::<BTreeSet<_>>();
    for (block_id, desired) in &desired_blocks {
        let Some(current) = current_blocks.get(block_id) else {
            continue;
        };
        if !moved_blocks.contains(block_id)
            && (current.block.parent != desired.parent || current.block.order != desired.order)
        {
            push_operation(
                &mut operations,
                SemanticOperation::ReorderBlock {
                    block_id: *block_id,
                    page_id: desired.page_id,
                    parent: desired.parent,
                    order: desired.order.clone(),
                },
            )?;
        }
    }

    let mut deletions = current_blocks
        .iter()
        .filter_map(|(block_id, current)| {
            (!desired_blocks.contains_key(block_id)
                && current
                    .block
                    .parent
                    .is_none_or(|parent| desired_blocks.contains_key(&parent)))
            .then_some((*block_id, current.page_id))
        })
        .collect::<Vec<_>>();
    deletions.sort_unstable();
    for (block_id, page_id) in deletions {
        push_operation(
            &mut operations,
            SemanticOperation::DeleteSubtree {
                root_block_id: block_id,
                page_id,
            },
        )?;
    }

    for (block_id, desired) in &desired_blocks {
        let Some(current) = current_blocks.get(block_id) else {
            continue;
        };
        if current.block.content != desired.content {
            push_operation(
                &mut operations,
                SemanticOperation::EditBlockContent {
                    block: BlockLocation {
                        block_id: *block_id,
                        home_document_id: current.block.home_document_id,
                    },
                    content: desired.content.clone(),
                },
            )?;
        }
    }
    for (block_id, desired) in &desired_blocks {
        let current_uuid = current_blocks
            .get(block_id)
            .map(|current| current.block.logseq_uuid);
        let mutation = match (current_uuid.flatten(), desired.logseq_uuid) {
            (None, Some(logseq_uuid)) => {
                Some(LogseqIdentityMutation::AssignExternal { logseq_uuid })
            }
            (Some(current), Some(logseq_uuid)) if current != logseq_uuid => {
                Some(LogseqIdentityMutation::ReplaceExternal { logseq_uuid })
            }
            (Some(_), None) => Some(LogseqIdentityMutation::RemoveExternal),
            (None, None) | (Some(_), Some(_)) => None,
        };
        if let Some(mutation) = mutation {
            push_operation(
                &mut operations,
                SemanticOperation::MutateBlockLogseqIdentity {
                    block: BlockLocation {
                        block_id: *block_id,
                        home_document_id: desired.home_document_id,
                    },
                    mutation,
                },
            )?;
        }
    }

    for (path, page) in desired_pages {
        let Some(tree) = trees.get(path) else {
            continue;
        };
        let current_preamble = current_pages
            .get(&page.page_id)
            .map(|current| current.materialized_page().preamble.clone());
        if current_preamble != Some(tree.preamble.clone())
            && (page.existing || tree.preamble.is_some())
        {
            push_operation(
                &mut operations,
                SemanticOperation::SetPagePreamble {
                    page_id: page.page_id,
                    preamble: tree.preamble.clone(),
                },
            )?;
        }
    }

    let desired_page_ids = desired_pages
        .values()
        .map(|page| page.page_id)
        .collect::<BTreeSet<_>>();
    for page_id in current_pages.keys() {
        if !desired_page_ids.contains(page_id) {
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage { page_id: *page_id },
            )?;
        }
    }

    let transaction = OperationTransaction::new(operations)
        .map_err(|error| ImportExecutionError::InvalidMaterial(error.to_string()))?;
    Ok(ImportExecutionMaterial {
        import_id,
        transaction,
        observation,
    })
}

fn imported_order(sibling_position: u32) -> String {
    format!("{sibling_position:010}")
}

fn external_logseq_uuid(
    path: &ManagedPath,
    locator: &StructuralLocator,
    node: &ParsedNode,
    rejected_raw_ids: &BTreeSet<(ManagedPath, StructuralLocator)>,
) -> Option<LogseqUuid> {
    if rejected_raw_ids.contains(&(path.clone(), locator.clone())) || node.raw_ids.len() != 1 {
        return None;
    }
    LogseqUuid::parse(node.raw_ids[0].trim()).ok()
}

fn current_block_depth(
    block_id: BlockId,
    current_blocks: &BTreeMap<BlockId, CurrentImportBlock>,
) -> usize {
    let mut depth = 0_usize;
    let mut cursor = Some(block_id);
    let mut visited = BTreeSet::new();
    while let Some(block_id) = cursor {
        if !visited.insert(block_id) {
            return usize::MAX;
        }
        let Some(block) = current_blocks.get(&block_id) else {
            return usize::MAX;
        };
        depth = depth.saturating_add(1);
        cursor = block.block.parent;
    }
    depth
}

fn desired_block_depth(
    block_id: BlockId,
    desired_blocks: &BTreeMap<BlockId, DesiredImportBlock>,
) -> usize {
    let mut depth = 0_usize;
    let mut cursor = Some(block_id);
    let mut visited = BTreeSet::new();
    while let Some(block_id) = cursor {
        if !visited.insert(block_id) {
            return usize::MAX;
        }
        let Some(block) = desired_blocks.get(&block_id) else {
            return usize::MAX;
        };
        depth = depth.saturating_add(1);
        cursor = block.parent;
    }
    depth
}

fn blocked_inventory_error(
    error: InventoryError,
    instrumentation: ImportInstrumentation,
) -> ImportPlan {
    let (reason, paths) = match &error {
        InventoryError::UnsupportedManagedLayout { .. } => {
            (ImportBlockReason::UnsupportedManagedLayout, Vec::new())
        }
        InventoryError::UnsafePath(path) | InventoryError::DuplicateRequestedPath(path) => {
            (ImportBlockReason::UnsafeInput, vec![path.clone()])
        }
        InventoryError::PortablePathCollision { first, second } => (
            ImportBlockReason::PortablePathCollision,
            vec![first.clone(), second.clone()],
        ),
        InventoryError::ResourceBudgetExceeded { .. } => {
            (ImportBlockReason::ResourceLimit, Vec::new())
        }
        InventoryError::UnsafeEntry { path, .. } => (
            ImportBlockReason::UnsafeInput,
            path.iter().cloned().collect(),
        ),
    };
    ImportPlan {
        status: ImportPlanStatus::Blocked,
        import_id: None,
        inventory: None,
        matches: None,
        scope: None,
        execution: None,
        blocks: vec![ImportBlock {
            reason,
            paths,
            logical_completion_ids: Vec::new(),
            observation: None,
            detail: error.to_string(),
        }],
        instrumentation,
    }
}

fn blocked_authority_error(
    inventory: Option<RawInventory>,
    block: ImportBlock,
    instrumentation: ImportInstrumentation,
) -> ImportPlan {
    ImportPlan {
        status: ImportPlanStatus::Blocked,
        import_id: None,
        inventory,
        matches: None,
        scope: None,
        execution: None,
        blocks: vec![block],
        instrumentation,
    }
}

fn receipt_block(
    reason: ImportBlockReason,
    path: &ManagedPath,
    completion_id: Option<LogicalCompletionId>,
    inventory: &RawInventory,
    detail: impl Into<String>,
) -> ImportBlock {
    ImportBlock {
        reason,
        paths: vec![path.as_str().to_owned()],
        logical_completion_ids: completion_id.into_iter().collect(),
        observation: inventory_observation(inventory, path.as_str()),
        detail: detail.into(),
    }
}

fn inventory_observation(
    inventory: &RawInventory,
    path: &str,
) -> Option<(ManagedPath, ImportInventoryState)> {
    inventory
        .entries()
        .iter()
        .find(|(candidate, _)| candidate.as_str() == path)
        .map(|(path, observation)| {
            let state = match observation {
                RawObservation::Present(bytes) => {
                    ImportInventoryState::Present(bytes.description())
                }
                RawObservation::Absent => ImportInventoryState::Absent,
            };
            (path.clone(), state)
        })
}

fn match_pages(
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    instrumentation: &mut ImportInstrumentation,
) -> Result<Vec<PageImportMatch>, ImportBlock> {
    let completed_paths = completed
        .iter()
        .map(|page| page.path().clone())
        .collect::<BTreeSet<_>>();
    let mut new_by_description = BTreeMap::<BlobDescription, Vec<&ManagedPath>>::new();
    for (path, observation) in inventory.entries() {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        if completed_paths.contains(path) {
            continue;
        }
        if let RawObservation::Present(bytes) = observation {
            new_by_description
                .entry(bytes.description())
                .or_default()
                .push(path);
        }
    }

    let mut source_to_candidate = BTreeMap::<ManagedPath, ManagedPath>::new();
    let mut candidate_to_sources = BTreeMap::<ManagedPath, Vec<&ReceiptBackedPage>>::new();
    for page in completed {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        if !matches!(
            inventory.entries().get(page.path()),
            Some(RawObservation::Absent)
        ) {
            continue;
        }
        let candidates = new_by_description
            .get(&page.description())
            .into_iter()
            .flatten()
            .filter(|path| {
                instrumentation.inventory_path_lookups =
                    instrumentation.inventory_path_lookups.saturating_add(1);
                inventory.entries().get(*path).is_some_and(|observation| {
                    matches!(observation, RawObservation::Present(bytes) if bytes.bytes() == page.bytes())
                })
            })
            .copied()
            .collect::<Vec<_>>();
        if candidates.len() > 1 {
            return Err(ImportBlock {
                reason: ImportBlockReason::AmbiguousDestructiveMatch,
                paths: std::iter::once(page.path().as_str().to_owned())
                    .chain(
                        candidates
                            .iter()
                            .map(|candidate| candidate.as_str().to_owned()),
                    )
                    .collect(),
                logical_completion_ids: vec![page.logical_completion_id()],
                observation: inventory_observation(inventory, page.path().as_str()),
                detail: "one absent receipt path has multiple exact new-path candidates".into(),
            });
        }
        if let Some(candidate) = candidates.first() {
            source_to_candidate.insert(page.path().clone(), (*candidate).clone());
            candidate_to_sources
                .entry((*candidate).clone())
                .or_default()
                .push(page);
        }
    }
    if let Some((candidate, sources)) = candidate_to_sources
        .iter()
        .find(|(_, sources)| sources.len() > 1)
    {
        return Err(ImportBlock {
            reason: ImportBlockReason::AmbiguousDestructiveMatch,
            paths: sources
                .iter()
                .map(|page| page.path().as_str().to_owned())
                .chain(std::iter::once(candidate.as_str().to_owned()))
                .collect(),
            logical_completion_ids: sources
                .iter()
                .map(|page| page.logical_completion_id())
                .collect(),
            observation: inventory_observation(inventory, candidate.as_str()),
            detail: "multiple absent receipt paths claim one exact new path".into(),
        });
    }

    let mut matches = Vec::new();
    for page in completed {
        instrumentation.inventory_path_lookups =
            instrumentation.inventory_path_lookups.saturating_add(1);
        match inventory.entries().get(page.path()) {
            Some(RawObservation::Present(_)) => matches.push(PageImportMatch {
                path: page.path().clone(),
                previous_path: page.path().clone(),
                page_id: page.page_id(),
                basis: PageMatchBasis::SamePathCompletion,
            }),
            Some(RawObservation::Absent) => {
                if let Some(path) = source_to_candidate.get(page.path()) {
                    matches.push(PageImportMatch {
                        path: path.clone(),
                        previous_path: page.path().clone(),
                        page_id: page.page_id(),
                        basis: PageMatchBasis::ReceiptBackedExactRename,
                    });
                }
            }
            None => unreachable!("receipt paths are required in the affected inventory"),
        }
    }
    matches.sort_unstable_by(|left, right| left.path.cmp(&right.path));
    Ok(matches)
}

struct ParsedNode {
    parent: Option<usize>,
    sibling_position: u32,
    depth: usize,
    children: Vec<usize>,
    span: StructuralSpan,
    raw: String,
    raw_ids: Vec<String>,
}

struct ParsedTree {
    path: ManagedPath,
    preamble: Option<String>,
    roots: Vec<usize>,
    nodes: Vec<ParsedNode>,
}

fn parse_nodes(
    path: &ManagedPath,
    bytes: &[u8],
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
    let text = std::str::from_utf8(bytes).expect("UTF-8 checked before semantic parsing");
    preflight_depth(path, text, instrumentation.parsed_nodes)?;
    let parsed = if path.as_str().ends_with(".org") {
        crate::org::parse_org_with_source_spans(text)
    } else {
        crate::doc::parse_with_source_spans(text)
    };
    flatten_document(path, parsed, instrumentation)
}

fn preflight_depth(path: &ManagedPath, text: &str, parsed_nodes: usize) -> Result<(), ImportBlock> {
    let mut candidate_nodes = 0_usize;
    for line in text.lines() {
        let is_org = path.as_str().ends_with(".org");
        let depth = if is_org {
            let stars = line
                .as_bytes()
                .iter()
                .take_while(|byte| **byte == b'*')
                .count();
            if stars > 0 && line.as_bytes().get(stars) == Some(&b' ') {
                candidate_nodes = candidate_nodes.saturating_add(1);
            }
            stars
        } else {
            let tabs = line
                .as_bytes()
                .iter()
                .take_while(|byte| **byte == b'\t')
                .count();
            let spaces = line
                .as_bytes()
                .iter()
                .skip(tabs)
                .take_while(|byte| **byte == b' ')
                .count();
            let content = &line[tabs + spaces..];
            if content == "-" || content.starts_with("- ") {
                candidate_nodes = candidate_nodes.saturating_add(1);
            }
            tabs.saturating_add(spaces / 2).saturating_add(1)
        };
        if depth > MAX_IMPORT_DEPTH {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!(
                    "document nesting depth exceeds import limit {MAX_IMPORT_DEPTH} before parsing"
                ),
            ));
        }
    }
    let observed = parsed_nodes.saturating_add(candidate_nodes);
    if observed > MAX_IMPORT_PARSED_NODES {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(path),
            format!(
                "parsed-node budget would be exceeded before parsing: observed {observed}, limit {MAX_IMPORT_PARSED_NODES}"
            ),
        ));
    }
    Ok(())
}

fn flatten_document(
    path: &ManagedPath,
    parsed: crate::doc::ParsedDocument,
    instrumentation: &mut ImportInstrumentation,
) -> Result<ParsedTree, ImportBlock> {
    let spans = parsed
        .block_spans
        .into_iter()
        .map(|span| {
            StructuralSpan::new(span.start as u64, span.end as u64).map_err(|error| {
                authority_block(
                    ImportBlockReason::UnsafeInput,
                    Some(path),
                    error.to_string(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let document = parsed.document;
    let mut nodes = Vec::<ParsedNode>::new();
    let mut roots = Vec::new();
    let mut pending = document
        .roots
        .iter()
        .enumerate()
        .rev()
        .map(|(position, block)| (block, None, position as u32, 1_usize))
        .collect::<Vec<_>>();
    while let Some((block, parent, sibling_position, depth)) = pending.pop() {
        if depth > MAX_IMPORT_DEPTH {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!("parsed document depth exceeds import limit {MAX_IMPORT_DEPTH}"),
            ));
        }
        if instrumentation.parsed_nodes == MAX_IMPORT_PARSED_NODES {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(path),
                format!("parsed-node budget exceeded: limit {MAX_IMPORT_PARSED_NODES}"),
            ));
        }
        let raw_ids = block
            .properties()
            .into_iter()
            .filter_map(|(key, value)| {
                (crate::doc::property_key_norm(&key) == "id").then_some(value)
            })
            .collect();
        let index = nodes.len();
        let span = spans.get(index).copied().ok_or_else(|| {
            authority_block(
                ImportBlockReason::UnsafeInput,
                Some(path),
                "parser tree has more blocks than exact source-span capture",
            )
        })?;
        nodes.push(ParsedNode {
            parent,
            sibling_position,
            depth,
            children: Vec::with_capacity(block.children.len()),
            span,
            raw: block.raw.clone(),
            raw_ids,
        });
        instrumentation.parsed_nodes = instrumentation.parsed_nodes.saturating_add(1);
        instrumentation.max_depth = instrumentation.max_depth.max(depth);
        if let Some(parent) = parent {
            nodes[parent].children.push(index);
        } else {
            roots.push(index);
        }
        for (position, child) in block.children.iter().enumerate().rev() {
            pending.push((child, Some(index), position as u32, depth.saturating_add(1)));
        }
    }
    if spans.len() != nodes.len() {
        return Err(authority_block(
            ImportBlockReason::UnsafeInput,
            Some(path),
            "exact source-span capture disagrees with the parsed block tree",
        ));
    }
    Ok(ParsedTree {
        path: path.clone(),
        preamble: document.pre_block.clone(),
        roots,
        nodes,
    })
}

fn materialize_locator(
    tree: &ParsedTree,
    index: usize,
    instrumentation: &mut ImportInstrumentation,
) -> Result<StructuralLocator, ImportBlock> {
    let depth = tree.nodes[index].depth;
    let next = instrumentation
        .locator_components_materialized
        .saturating_add(depth);
    if next > MAX_IMPORT_LOCATOR_COMPONENTS {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(&tree.path),
            format!(
                "structural-locator component budget exceeded: observed {next}, limit {MAX_IMPORT_LOCATOR_COMPONENTS}"
            ),
        ));
    }
    instrumentation.locator_components_materialized = next;
    let mut components = Vec::with_capacity(depth);
    let mut cursor = Some(index);
    while let Some(node) = cursor {
        components.push(tree.nodes[node].sibling_position);
        cursor = tree.nodes[node].parent;
    }
    components.reverse();
    StructuralLocator::new(components).map_err(|error| {
        authority_block(
            ImportBlockReason::CorruptBase,
            Some(&tree.path),
            error.to_string(),
        )
    })
}

fn resolve_locator(
    tree: &ParsedTree,
    locator: &StructuralLocator,
    instrumentation: &mut ImportInstrumentation,
) -> Result<Option<usize>, ImportBlock> {
    let next = instrumentation
        .locator_components_materialized
        .saturating_add(locator.components().len());
    if next > MAX_IMPORT_LOCATOR_COMPONENTS {
        return Err(authority_block(
            ImportBlockReason::ResourceLimit,
            Some(&tree.path),
            format!(
                "structural-locator component budget exceeded: observed {next}, limit {MAX_IMPORT_LOCATOR_COMPONENTS}"
            ),
        ));
    }
    instrumentation.locator_components_materialized = next;
    let mut components = locator.components().iter().copied();
    let Some(root) = components.next() else {
        return Ok(None);
    };
    let Some(mut current) = tree.roots.get(root as usize).copied() else {
        return Ok(None);
    };
    for component in components {
        let Some(child) = tree.nodes[current]
            .children
            .get(component as usize)
            .copied()
        else {
            return Ok(None);
        };
        current = child;
    }
    Ok(Some(current))
}

struct StructuralClassEntry {
    raw: String,
    child_classes: Vec<usize>,
    class: usize,
}

#[derive(Default)]
struct StructuralInterner {
    buckets: HashMap<ContentDigest, Vec<StructuralClassEntry>>,
    next_class: usize,
}

impl StructuralInterner {
    fn new() -> Self {
        Self::default()
    }
}

/// Assign exact structural classes through a digest index whose candidates are
/// always collision-checked against raw bytes and child classes. Hash-table
/// lookup avoids ordered vector-key comparisons with adversarial common
/// prefixes, and every candidate comparison is charged.
fn structural_classes(
    tree: &ParsedTree,
    interner: &mut StructuralInterner,
    instrumentation: &mut ImportInstrumentation,
) -> Result<Vec<usize>, ImportBlock> {
    let mut classes = vec![0; tree.nodes.len()];
    for index in (0..tree.nodes.len()).rev() {
        let child_classes = tree.nodes[index]
            .children
            .iter()
            .map(|child| classes[*child])
            .collect::<Vec<_>>();
        instrumentation.structural_key_components = instrumentation
            .structural_key_components
            .saturating_add(1)
            .saturating_add(child_classes.len());
        if instrumentation.structural_key_components > MAX_IMPORT_STRUCTURAL_KEY_WORK {
            return Err(authority_block(
                ImportBlockReason::ResourceLimit,
                Some(&tree.path),
                format!(
                    "structural key component budget exceeded: limit {MAX_IMPORT_STRUCTURAL_KEY_WORK}"
                ),
            ));
        }
        let node = &tree.nodes[index];
        let mut hasher = Sha256::new();
        hasher.update(b"tine/import-structural-class/v1\0");
        hasher.update((node.raw.len() as u64).to_be_bytes());
        hasher.update(node.raw.as_bytes());
        hasher.update((child_classes.len() as u64).to_be_bytes());
        for class in &child_classes {
            hasher.update((*class as u64).to_be_bytes());
        }
        instrumentation.bytes_hashed = instrumentation
            .bytes_hashed
            .saturating_add(node.raw.len() as u64)
            .saturating_add((child_classes.len() as u64).saturating_mul(8));
        let digest = ContentDigest::from_bytes(hasher.finalize().into());
        let bucket = interner.buckets.entry(digest).or_default();
        let mut class = None;
        for candidate in bucket.iter() {
            instrumentation.structural_key_comparisons = instrumentation
                .structural_key_comparisons
                .saturating_add(node.raw.len())
                .saturating_add(child_classes.len());
            if instrumentation.structural_key_comparisons > MAX_IMPORT_STRUCTURAL_KEY_WORK {
                return Err(authority_block(
                    ImportBlockReason::ResourceLimit,
                    Some(&tree.path),
                    format!(
                        "structural key comparison budget exceeded: limit {MAX_IMPORT_STRUCTURAL_KEY_WORK}"
                    ),
                ));
            }
            if candidate.raw == node.raw && candidate.child_classes == child_classes {
                class = Some(candidate.class);
                break;
            }
        }
        let class = match class {
            Some(class) => class,
            None => {
                let class = interner.next_class;
                interner.next_class = interner.next_class.saturating_add(1);
                instrumentation.structural_class_allocations = instrumentation
                    .structural_class_allocations
                    .saturating_add(1);
                bucket.push(StructuralClassEntry {
                    raw: node.raw.clone(),
                    child_classes,
                    class,
                });
                class
            }
        };
        classes[index] = class;
    }
    Ok(classes)
}

fn match_blocks(
    inventory: &RawInventory,
    completed: &[&ReceiptBackedPage],
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    let mut external_by_path = BTreeMap::<ManagedPath, ParsedTree>::new();
    for (path, observation) in inventory.entries() {
        if let RawObservation::Present(bytes) = observation {
            external_by_path.insert(
                path.clone(),
                parse_nodes(path, bytes.bytes(), instrumentation)?,
            );
        }
    }
    let mut base_by_path = BTreeMap::<ManagedPath, ParsedTree>::new();
    for page in completed {
        base_by_path.insert(
            page.path().clone(),
            parse_nodes(page.path(), page.bytes(), instrumentation)?,
        );
    }

    let mut external_anchors = BTreeMap::<LogseqUuid, Vec<(ManagedPath, usize, String)>>::new();
    let mut rejected = BTreeSet::<(ManagedPath, usize)>::new();
    for tree in external_by_path.values() {
        for (index, node) in tree.nodes.iter().enumerate() {
            if node.raw_ids.is_empty() {
                continue;
            }
            if node.raw_ids.len() != 1 {
                rejected.insert((tree.path.clone(), index));
                for raw_id in &node.raw_ids {
                    let reason = if LogseqUuid::parse(raw_id.trim()).is_ok() {
                        RejectedRawIdReason::Duplicate
                    } else {
                        RejectedRawIdReason::InvalidSyntax
                    };
                    matches.rejected_raw_ids.push(RejectedRawId {
                        path: tree.path.clone(),
                        locator: materialize_locator(tree, index, instrumentation)?,
                        raw_value: raw_id.clone(),
                        reason,
                    });
                }
                continue;
            }
            let raw_id = &node.raw_ids[0];
            match LogseqUuid::parse(raw_id.trim()) {
                Ok(uuid) => external_anchors.entry(uuid).or_default().push((
                    tree.path.clone(),
                    index,
                    raw_id.clone(),
                )),
                Err(_) => {
                    rejected.insert((tree.path.clone(), index));
                    matches.rejected_raw_ids.push(RejectedRawId {
                        path: tree.path.clone(),
                        locator: materialize_locator(tree, index, instrumentation)?,
                        raw_value: raw_id.clone(),
                        reason: RejectedRawIdReason::InvalidSyntax,
                    });
                }
            }
        }
    }
    for owners in external_anchors.values().filter(|owners| owners.len() > 1) {
        for (path, index, raw_value) in owners {
            rejected.insert((path.clone(), *index));
            let tree = &external_by_path[path];
            matches.rejected_raw_ids.push(RejectedRawId {
                path: path.clone(),
                locator: materialize_locator(tree, *index, instrumentation)?,
                raw_value: raw_value.clone(),
                reason: RejectedRawIdReason::Duplicate,
            });
        }
    }
    instrumentation.rejected_raw_id_occurrences = matches.rejected_raw_ids.len();
    matches.rejected_raw_ids.sort_unstable_by(|left, right| {
        (&left.path, &left.locator, &left.raw_value).cmp(&(
            &right.path,
            &right.locator,
            &right.raw_value,
        ))
    });

    let mut receipt_anchors =
        BTreeMap::<LogseqUuid, Vec<(BlockId, LogicalCompletionId, ManagedPath, usize)>>::new();
    let mut annotations_by_path = BTreeMap::<ManagedPath, BTreeMap<usize, BlockId>>::new();
    for page in completed {
        let tree = &base_by_path[page.path()];
        let mut annotations = BTreeMap::new();
        for annotation in page.annotations() {
            let Some(index) = resolve_locator(tree, annotation.locator(), instrumentation)? else {
                continue;
            };
            annotations.insert(index, annotation.block_id());
            if let Some(uuid) = annotation.logseq_uuid() {
                receipt_anchors.entry(uuid).or_default().push((
                    annotation.block_id(),
                    page.logical_completion_id(),
                    page.path().clone(),
                    index,
                ));
            }
        }
        annotations_by_path.insert(page.path().clone(), annotations);
    }
    let mut matched_external = BTreeSet::<(ManagedPath, usize)>::new();
    let mut matched_base = BTreeMap::<(ManagedPath, usize), (ManagedPath, usize)>::new();
    let mut used_blocks = BTreeSet::<BlockId>::new();
    for (uuid, owners) in external_anchors
        .iter()
        .filter(|(_, owners)| owners.len() == 1)
    {
        let Some(receipt_owners) = receipt_anchors.get(uuid) else {
            continue;
        };
        if receipt_owners.len() != 1 {
            let (path, _, _) = &owners[0];
            return Err(ImportBlock {
                reason: ImportBlockReason::DuplicateAnchorDependent,
                paths: vec![path.as_str().to_owned()],
                logical_completion_ids: receipt_owners
                    .iter()
                    .map(|(_, completion, _, _)| *completion)
                    .collect(),
                observation: inventory_observation(inventory, path.as_str()),
                detail: format!("UUID {uuid} has multiple receipt-backed owners"),
            });
        }
        let (path, external_index, _) = &owners[0];
        let (block_id, _, base_path, base_index) = &receipt_owners[0];
        let external_tree = &external_by_path[path];
        matches.blocks.push(BlockImportMatch {
            path: path.clone(),
            locator: materialize_locator(external_tree, *external_index, instrumentation)?,
            block_id: *block_id,
            basis: BlockMatchBasis::UniqueLogseqUuid,
        });
        used_blocks.insert(*block_id);
        matched_external.insert((path.clone(), *external_index));
        matched_base.insert(
            (base_path.clone(), *base_index),
            (path.clone(), *external_index),
        );
    }

    let mut structural_interner = StructuralInterner::new();
    let mut base_classes_by_path = BTreeMap::new();
    for (path, tree) in &base_by_path {
        let classes = structural_classes(tree, &mut structural_interner, instrumentation)?;
        instrumentation.structural_class_nodes = instrumentation
            .structural_class_nodes
            .saturating_add(tree.nodes.len());
        base_classes_by_path.insert(path.clone(), classes);
    }
    let mut external_classes_by_path = BTreeMap::new();
    for (path, tree) in &external_by_path {
        let classes = structural_classes(tree, &mut structural_interner, instrumentation)?;
        instrumentation.structural_class_nodes = instrumentation
            .structural_class_nodes
            .saturating_add(tree.nodes.len());
        external_classes_by_path.insert(path.clone(), classes);
    }

    let mut base_exact = BTreeMap::<usize, Vec<(ManagedPath, usize, BlockId)>>::new();
    for (path, tree) in &base_by_path {
        let annotations = &annotations_by_path[path];
        let classes = &base_classes_by_path[path];
        for index in 0..tree.nodes.len() {
            if annotations.contains_key(&index)
                && !matched_base.contains_key(&(path.clone(), index))
            {
                instrumentation.exact_bucket_inserts =
                    instrumentation.exact_bucket_inserts.saturating_add(1);
                base_exact.entry(classes[index]).or_default().push((
                    path.clone(),
                    index,
                    annotations[&index],
                ));
            }
        }
    }
    let mut external_exact = BTreeMap::<usize, Vec<(ManagedPath, usize)>>::new();
    for (path, tree) in &external_by_path {
        let classes = &external_classes_by_path[path];
        for index in 0..tree.nodes.len() {
            let key = (path.clone(), index);
            if !rejected.contains(&key) && !matched_external.contains(&key) {
                instrumentation.exact_bucket_inserts =
                    instrumentation.exact_bucket_inserts.saturating_add(1);
                external_exact
                    .entry(classes[index])
                    .or_default()
                    .push((path.clone(), index));
            }
        }
    }
    let base_class_counts = base_exact
        .iter()
        .map(|(class, candidates)| (*class, candidates.len()))
        .collect::<BTreeMap<_, _>>();
    let external_class_counts = external_exact
        .iter()
        .map(|(class, candidates)| (*class, candidates.len()))
        .collect::<BTreeMap<_, _>>();
    for (class, base_candidates) in &base_exact {
        instrumentation.exact_bucket_lookups =
            instrumentation.exact_bucket_lookups.saturating_add(1);
        let Some(external_candidates) = external_exact.get(class) else {
            continue;
        };
        if base_candidates.len() != 1 || external_candidates.len() != 1 {
            continue;
        }
        let (base_path, base_index, block_id) = &base_candidates[0];
        let (external_path, external_index) = &external_candidates[0];
        if used_blocks.insert(*block_id) {
            record_block_match(
                matches,
                &mut matched_external,
                &mut matched_base,
                base_path,
                *base_index,
                &external_by_path[external_path],
                *external_index,
                *block_id,
                BlockMatchBasis::ReceiptStructuralExact,
                instrumentation,
            )?;
        }
    }

    let page_matches = matches.pages.clone();
    for page_match in &page_matches {
        let base_tree = &base_by_path[&page_match.previous_path];
        let external_tree = &external_by_path[&page_match.path];
        let annotations = &annotations_by_path[&page_match.previous_path];
        align_ordered_tree(
            &page_match.previous_path,
            base_tree,
            external_tree,
            &base_classes_by_path[&page_match.previous_path],
            &external_classes_by_path[&page_match.path],
            &base_class_counts,
            &external_class_counts,
            annotations,
            &rejected,
            &mut used_blocks,
            &mut matched_external,
            &mut matched_base,
            matches,
            instrumentation,
        )?;
    }
    matches.blocks.sort_unstable_by(|left, right| {
        (&left.path, &left.locator).cmp(&(&right.path, &right.locator))
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_block_match(
    matches: &mut ImportMatches,
    matched_external: &mut BTreeSet<(ManagedPath, usize)>,
    matched_base: &mut BTreeMap<(ManagedPath, usize), (ManagedPath, usize)>,
    base_path: &ManagedPath,
    base_index: usize,
    external_tree: &ParsedTree,
    external_index: usize,
    block_id: BlockId,
    basis: BlockMatchBasis,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    matches.blocks.push(BlockImportMatch {
        path: external_tree.path.clone(),
        locator: materialize_locator(external_tree, external_index, instrumentation)?,
        block_id,
        basis,
    });
    matched_external.insert((external_tree.path.clone(), external_index));
    matched_base.insert(
        (base_path.clone(), base_index),
        (external_tree.path.clone(), external_index),
    );
    instrumentation.retained_block_matches =
        instrumentation.retained_block_matches.saturating_add(1);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn align_ordered_tree(
    base_path: &ManagedPath,
    base_tree: &ParsedTree,
    external_tree: &ParsedTree,
    base_classes: &[usize],
    external_classes: &[usize],
    base_class_counts: &BTreeMap<usize, usize>,
    external_class_counts: &BTreeMap<usize, usize>,
    annotations: &BTreeMap<usize, BlockId>,
    rejected: &BTreeSet<(ManagedPath, usize)>,
    used_blocks: &mut BTreeSet<BlockId>,
    matched_external: &mut BTreeSet<(ManagedPath, usize)>,
    matched_base: &mut BTreeMap<(ManagedPath, usize), (ManagedPath, usize)>,
    matches: &mut ImportMatches,
    instrumentation: &mut ImportInstrumentation,
) -> Result<(), ImportBlock> {
    let mut pending = vec![(None, None)];
    pending.extend(
        matched_base
            .iter()
            .filter(|((path, _), (external_path, _))| {
                path == base_path && external_path == &external_tree.path
            })
            .map(|((_, base), (_, external))| (Some(*base), Some(*external))),
    );
    let mut visited = BTreeSet::new();
    while let Some((base_parent, external_parent)) = pending.pop() {
        if !visited.insert((base_parent, external_parent)) {
            continue;
        }
        let base_sequence = base_parent
            .map(|parent| base_tree.nodes[parent].children.as_slice())
            .unwrap_or(&base_tree.roots);
        let external_sequence = external_parent
            .map(|parent| external_tree.nodes[parent].children.as_slice())
            .unwrap_or(&external_tree.roots);
        instrumentation.ordered_alignment_visits = instrumentation
            .ordered_alignment_visits
            .saturating_add(base_sequence.len())
            .saturating_add(external_sequence.len());

        let external_positions = external_sequence
            .iter()
            .enumerate()
            .map(|(position, index)| (*index, position))
            .collect::<BTreeMap<_, _>>();
        let mut boundaries = Vec::new();
        let mut last_external = None;
        for (base_position, base_index) in base_sequence.iter().enumerate() {
            let Some((external_path, external_index)) =
                matched_base.get(&(base_path.clone(), *base_index))
            else {
                continue;
            };
            if external_path != &external_tree.path {
                continue;
            }
            let Some(external_position) = external_positions.get(external_index).copied() else {
                continue;
            };
            if last_external.is_some_and(|last| external_position <= last) {
                boundaries.clear();
                break;
            }
            boundaries.push((base_position, external_position));
            last_external = Some(external_position);
        }
        let trusted_anchor_count = base_sequence
            .iter()
            .filter(|base_index| matched_base.contains_key(&(base_path.clone(), **base_index)))
            .count();
        if trusted_anchor_count > 0 && boundaries.len() != trusted_anchor_count {
            continue;
        }

        let mut previous_base = 0;
        let mut previous_external = 0;
        for (next_base, next_external) in boundaries.into_iter().chain(std::iter::once((
            base_sequence.len(),
            external_sequence.len(),
        ))) {
            let base_gap = base_sequence[previous_base..next_base]
                .iter()
                .copied()
                .filter(|index| {
                    annotations.get(index).is_some_and(|block_id| {
                        !used_blocks.contains(block_id)
                            && !matched_base.contains_key(&(base_path.clone(), *index))
                    })
                })
                .collect::<Vec<_>>();
            let external_gap = external_sequence[previous_external..next_external]
                .iter()
                .copied()
                .filter(|index| {
                    let key = (external_tree.path.clone(), *index);
                    !rejected.contains(&key) && !matched_external.contains(&key)
                })
                .collect::<Vec<_>>();
            if let ([base_index], [external_index]) = (base_gap.as_slice(), external_gap.as_slice())
            {
                if base_class_counts.get(&base_classes[*base_index]) != Some(&1)
                    || external_class_counts.get(&external_classes[*external_index]) != Some(&1)
                {
                    if next_base < base_sequence.len() && next_external < external_sequence.len() {
                        previous_base = next_base.saturating_add(1);
                        previous_external = next_external.saturating_add(1);
                    }
                    continue;
                }
                let block_id = annotations[base_index];
                if used_blocks.insert(block_id) {
                    record_block_match(
                        matches,
                        matched_external,
                        matched_base,
                        base_path,
                        *base_index,
                        external_tree,
                        *external_index,
                        block_id,
                        BlockMatchBasis::ReceiptOrderedTreeAlignment,
                        instrumentation,
                    )?;
                    pending.push((Some(*base_index), Some(*external_index)));
                }
            }
            if next_base < base_sequence.len() && next_external < external_sequence.len() {
                previous_base = next_base.saturating_add(1);
                previous_external = next_external.saturating_add(1);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictCopyClass {
    GeneratedExact,
    External,
    MixedUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictClassificationError {
    path: ManagedPath,
}

impl fmt::Display for ConflictClassificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not a recognized sync conflict copy", self.path)
    }
}

impl std::error::Error for ConflictClassificationError {}

/// Diagnostic classification from caller-supplied exact hashes.
///
/// The function is read-only and never removes the inventory entry or file.
/// Its result is not sealed generated-output evidence and must never authorize
/// deletion; a later deletion path must obtain its own authoritative proof.
pub fn classify_conflict_copy(
    path: ManagedPath,
    observed: &ExactBytes,
    generated_target: BlobDescription,
    exact_external: Option<BlobDescription>,
) -> Result<ConflictCopyClass, ConflictClassificationError> {
    if !path_is_sync_conflict(Path::new(path.as_str())) {
        return Err(ConflictClassificationError { path });
    }
    Ok(if observed.description() == generated_target {
        ConflictCopyClass::GeneratedExact
    } else if exact_external.is_some_and(|external| external == observed.description()) {
        ConflictCopyClass::External
    } else {
        ConflictCopyClass::MixedUnknown
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::*;
    use crate::oplog::{
        write_projection_exact, AuthorBatch, BatchId, BlockLocation, CrdtPeerId, DeviceId,
        DocumentId, LineageDigest, ManagedTextKind, ObjectStore, OperationTransaction,
        PortablePathIndexRoot, ProjectionEndpointBinding, ProjectionEndpointId, SemanticOperation,
        SessionId,
    };

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("tine-import-snapshot-{label}-{}", Uuid::new_v4()));
            fs::create_dir_all(path.join("graph/pages")).unwrap();
            fs::create_dir_all(path.join("graph/journals")).unwrap();
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

    struct SnapshotFixture {
        _root: TestRoot,
        graph_root: PathBuf,
        graph: Graph,
        receipts: ProjectionReceiptStore,
        engine: ShardedHotEngine,
        intents: Vec<ProjectionIntent>,
        empty_history_head: Vec<u8>,
    }

    impl SnapshotFixture {
        fn new(label: &str, paths: &[&str]) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, None, None)
        }

        fn new_with_initial_uuid(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
        ) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, initial_uuid, None)
        }

        fn new_with_graph_config(label: &str, paths: &[&str], config: &str) -> Self {
            Self::new_with_initial_uuid_and_config(label, paths, None, Some(config))
        }

        fn new_with_initial_uuid_and_config(
            label: &str,
            paths: &[&str],
            initial_uuid: Option<LogseqUuid>,
            config: Option<&str>,
        ) -> Self {
            let root = TestRoot::new(label);
            let graph_root = root.path().join("graph");
            if let Some(config) = config {
                fs::create_dir_all(graph_root.join("logseq")).unwrap();
                fs::write(graph_root.join("logseq/config.edn"), config).unwrap();
            }
            let graph = Graph::open(&graph_root);
            for path in paths {
                let parent = graph_root.join(path).parent().unwrap().to_path_buf();
                fs::create_dir_all(parent).unwrap();
            }
            let workspace = WorkspaceId::from_uuid(Uuid::from_u128(1));
            let endpoint = ProjectionEndpointBinding::enroll_graph(
                &graph,
                ProjectionEndpointId::from_uuid(Uuid::from_u128(2)),
                DeviceId::from_uuid(Uuid::from_u128(3)),
            )
            .unwrap();
            let receipts = ProjectionReceiptStore::open_for_endpoint(
                &root.path().join("receipts"),
                workspace,
                endpoint,
            )
            .unwrap();
            let lineage = LineageDigest::of(b"snapshot-test");
            let catalog = DocumentId::from_uuid(Uuid::from_u128(4));
            let author = ShardedHotEngine::new(workspace, lineage, catalog);
            let mut operations = Vec::new();
            let mut page_ids = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                let seed = 100 + index as u128 * 10;
                let page_id = PageId::from_uuid(Uuid::from_u128(seed));
                let home = DocumentId::from_uuid(Uuid::from_u128(seed + 1));
                let managed_path = ManagedPath::parse((*path).to_owned()).unwrap();
                let kind = graph.classify_managed_text_path(&managed_path).unwrap();
                page_ids.push(page_id);
                operations.push(SemanticOperation::CreatePage {
                    page_id,
                    home_document_id: home,
                    name: crate::oplog::LogicalPageName::parse(format!("Snapshot Page {index}"))
                        .unwrap(),
                    path: managed_path,
                    kind,
                });
                operations.push(SemanticOperation::CreateBlock {
                    block: BlockLocation {
                        block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                        home_document_id: home,
                    },
                    page_id,
                    parent: None,
                    order: "a".into(),
                    content: match (index, initial_uuid) {
                        (0, Some(logseq_uuid)) => format!("page {index}\nid:: {logseq_uuid}"),
                        _ => format!("page {index}"),
                    },
                });
                if index == 0 {
                    if let Some(logseq_uuid) = initial_uuid {
                        operations.push(SemanticOperation::MutateBlockLogseqIdentity {
                            block: BlockLocation {
                                block_id: BlockId::from_uuid(Uuid::from_u128(seed + 2)),
                                home_document_id: home,
                            },
                            mutation: LogseqIdentityMutation::AssignExternal { logseq_uuid },
                        });
                    }
                }
            }
            let transaction = OperationTransaction::new(operations).unwrap();
            let batch_id = BatchId::from_uuid(Uuid::from_u128(5));
            let prepared = author
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id,
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(6)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(7)),
                        crdt_peer_id: CrdtPeerId::from_u64(8),
                    },
                    &transaction,
                )
                .unwrap();
            let archive = root.path().join("archive");
            ObjectStore::open(&archive, workspace)
                .unwrap()
                .publish_prepared(&prepared)
                .unwrap();
            let mut engine = ShardedHotEngine::with_enrolled_projection(
                ObjectStore::open(&archive, workspace).unwrap(),
                lineage,
                catalog,
                &graph,
                &receipts,
            );
            let empty_history_head = fs::read(
                archive
                    .join("engine-history")
                    .join(endpoint.endpoint_id.to_string())
                    .join("engine-history.head"),
            )
            .unwrap();
            engine.stage_archive_batch(batch_id).unwrap();
            let intents = page_ids
                .into_iter()
                .map(|page_id| {
                    write_projection_exact(&graph, &receipts, &engine, page_id, None)
                        .unwrap()
                        .plan
                        .intent()
                        .clone()
                })
                .collect();
            Self {
                _root: root,
                graph_root,
                graph,
                receipts,
                engine,
                intents,
                empty_history_head,
            }
        }

        fn plan(&self, paths: &[&str]) -> ImportPlan {
            plan_affected_import(&self.graph, &self.receipts, &self.engine, paths)
        }
    }

    fn completion_name(intent: &ProjectionIntent) -> String {
        let mut value = String::new();
        for byte in intent.id().unwrap().as_bytes() {
            use std::fmt::Write as _;
            write!(&mut value, "{byte:02x}").unwrap();
        }
        format!("{value}.completion")
    }

    #[test]
    fn snapshot_revalidation_rejects_content_replacement_between_passes() {
        let fixture = SnapshotFixture::new("content", &["pages/a.md"]);
        let target = fixture.graph_root.join("pages/a.md");
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::write(target, b"- replaced\n").unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn snapshot_revalidation_rejects_two_path_rename_between_passes() {
        let fixture = SnapshotFixture::new("rename", &["pages/a.md", "pages/b.md"]);
        let a = fixture.graph_root.join("pages/a.md");
        let b = fixture.graph_root.join("pages/b.md");
        let temporary = fixture.graph_root.join("pages/swap.tmp");
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::rename(&a, &temporary).unwrap();
                fs::rename(&b, &a).unwrap();
                fs::rename(&temporary, &b).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md", "pages/b.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn snapshot_revalidation_rejects_catalog_change_between_passes() {
        let fixture = SnapshotFixture::new("catalog", &["pages/a.md"]);
        let completion = fixture
            .receipts
            .root_path()
            .join("completions")
            .join(completion_name(&fixture.intents[0]));
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(completion).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn snapshot_revalidation_rejects_accepted_frontier_change_between_passes() {
        let fixture = SnapshotFixture::new("frontier", &["pages/a.md"]);
        let other = SnapshotFixture::new("frontier-other", &["pages/a.md", "pages/b.md"]);
        POST_FRONTIER_OVERRIDE.with(|root| {
            *root.borrow_mut() = Some(other.engine.accepted_frontier_root().unwrap());
        });
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(plan.blocks()[0].reason, ImportBlockReason::StaleScope);
    }

    #[test]
    fn execution_material_refuses_noop_and_blocked_plans() {
        let fixture = SnapshotFixture::new("execution-refusal", &["pages/a.md"]);
        let noop = fixture.plan(&["pages/a.md"]);
        assert_eq!(noop.status(), ImportPlanStatus::Noop);
        assert_eq!(
            noop.execution_material().unwrap_err(),
            ImportExecutionError::RefusedStatus(ImportPlanStatus::Noop)
        );

        fs::write(fixture.graph_root.join("pages/a.md"), [0xff]).unwrap();
        let blocked = fixture.plan(&["pages/a.md"]);
        assert_eq!(blocked.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            blocked.execution_material().unwrap_err(),
            ImportExecutionError::RefusedStatus(ImportPlanStatus::Blocked)
        );

        fs::write(
            fixture
                .graph_root
                .join("pages/a.sync-conflict-20260725-120000-AAAAAAA.md"),
            b"- diagnostic only\n",
        )
        .unwrap();
        let conflict = fixture.plan(&["pages/a.sync-conflict-20260725-120000-AAAAAAA.md"]);
        assert_eq!(conflict.status(), ImportPlanStatus::Blocked);
        assert!(conflict
            .blocks()
            .iter()
            .any(|block| block.detail.contains("diagnostic inputs")));
    }

    #[test]
    fn identical_sealed_reconciliations_produce_identical_execution_and_observation_bytes() {
        let left = SnapshotFixture::new("execution-identical-left", &["pages/a.md"]);
        let right = SnapshotFixture::new("execution-identical-right", &["pages/a.md"]);
        fs::write(left.graph_root.join("pages/a.md"), b"- changed\n").unwrap();
        fs::write(right.graph_root.join("pages/a.md"), b"- changed\n").unwrap();

        let left_plan = left.plan(&["pages/a.md"]);
        let right_plan = right.plan(&["pages/a.md"]);
        assert_eq!(left_plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(right_plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(left_plan.import_id(), right_plan.import_id());
        let left_material = left_plan.execution_material().unwrap();
        let right_material = right_plan.execution_material().unwrap();
        assert_eq!(left_material, right_material);
        assert_eq!(
            left_material.batch_id(),
            left_material.import_id().batch_id()
        );
        assert_eq!(
            left_material.origin(),
            BatchOrigin::ExternalReconciliation {
                import_id: left_material.import_id()
            }
        );

        let left_object = left_material
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let right_object = right_material
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        assert_eq!(left_object, right_object);
        assert_eq!(
            left_object.descriptor().unwrap(),
            right_object.descriptor().unwrap()
        );
        let mut malformed = left_object.payload().to_vec();
        malformed[0] ^= 0xff;
        assert!(
            super::super::external_import::ExternalImportObservation::decode(&malformed).is_err()
        );
    }

    #[test]
    fn execution_material_preserves_explicit_external_id_change_and_removal() {
        let old = LogseqUuid::from_uuid(Uuid::from_u128(910));
        let changed = LogseqUuid::from_uuid(Uuid::from_u128(911));
        let replacement = SnapshotFixture::new_with_initial_uuid(
            "execution-id-replacement",
            &["pages/a.md"],
            Some(old),
        );
        fs::write(
            replacement.graph_root.join("pages/a.md"),
            format!("- page 0\n  id:: {changed}\n"),
        )
        .unwrap();
        let replacement_plan = replacement.plan(&["pages/a.md"]);
        let replacement_transaction = &replacement_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(replacement_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::ReplaceExternal { logseq_uuid },
                    ..
                } if *logseq_uuid == changed
            )
        }));

        let removal = SnapshotFixture::new_with_initial_uuid(
            "execution-id-removal",
            &["pages/a.md"],
            Some(old),
        );
        fs::write(removal.graph_root.join("pages/a.md"), b"- page 0\n").unwrap();
        let removal_plan = removal.plan(&["pages/a.md"]);
        let removal_transaction = &removal_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(removal_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::RemoveExternal,
                    ..
                }
            )
        }));
    }

    #[test]
    fn execution_material_retains_invalid_and_duplicate_raw_ids_without_identity_authority() {
        let duplicate = SnapshotFixture::new("execution-duplicate-id", &["pages/a.md"]);
        let duplicate_bytes = format!(
            "- page 0\n  id:: {}\n  id:: {}\n",
            LogseqUuid::from_uuid(Uuid::from_u128(920)),
            LogseqUuid::from_uuid(Uuid::from_u128(920)),
        )
        .into_bytes();
        fs::write(duplicate.graph_root.join("pages/a.md"), &duplicate_bytes).unwrap();
        let duplicate_plan = duplicate.plan(&["pages/a.md"]);
        let duplicate_object = duplicate_plan
            .execution_material()
            .unwrap()
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let duplicate_observation =
            super::super::external_import::ExternalImportObservation::decode(
                duplicate_object.payload(),
            )
            .unwrap();
        let duplicate_entry = &duplicate_observation.entries()[0];
        assert_eq!(
            duplicate_entry.state().bytes(),
            Some(duplicate_bytes.as_slice())
        );
        assert!(duplicate_entry
            .state()
            .annotations()
            .iter()
            .all(|annotation| annotation.logseq_uuid().is_none()));

        let invalid = SnapshotFixture::new("execution-invalid-id", &["pages/a.md"]);
        let invalid_bytes = b"- page 0\n  id:: definitely-not-a-uuid\n";
        fs::write(invalid.graph_root.join("pages/a.md"), invalid_bytes).unwrap();
        let invalid_plan = invalid.plan(&["pages/a.md"]);
        let invalid_object = invalid_plan
            .execution_material()
            .unwrap()
            .observation()
            .clone()
            .into_operation_object(PortablePathIndexRoot::empty())
            .unwrap();
        let invalid_observation = super::super::external_import::ExternalImportObservation::decode(
            invalid_object.payload(),
        )
        .unwrap();
        let invalid_entry = &invalid_observation.entries()[0];
        assert_eq!(
            invalid_entry.state().bytes(),
            Some(invalid_bytes.as_slice())
        );
        assert!(invalid_entry
            .state()
            .annotations()
            .iter()
            .all(|annotation| annotation.logseq_uuid().is_none()));
    }

    #[test]
    fn execution_material_retains_nested_rename_and_delete_semantics() {
        let renamed = SnapshotFixture::new("execution-nested-rename", &["pages/topic/old-name.md"]);
        fs::create_dir_all(renamed.graph_root.join("pages/topic/next")).unwrap();
        fs::rename(
            renamed.graph_root.join("pages/topic/old-name.md"),
            renamed.graph_root.join("pages/topic/next/new-name.md"),
        )
        .unwrap();
        let rename_plan =
            renamed.plan(&["pages/topic/old-name.md", "pages/topic/next/new-name.md"]);
        let rename_transaction = &rename_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(rename_transaction.iter().any(|operation| {
            matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { path, .. }
                    if path.as_str() == "pages/topic/next/new-name.md"
            )
        }));

        let deleted =
            SnapshotFixture::new("execution-nested-delete", &["pages/topic/delete-me.md"]);
        fs::remove_file(deleted.graph_root.join("pages/topic/delete-me.md")).unwrap();
        let delete_plan = deleted.plan(&["pages/topic/delete-me.md"]);
        let delete_transaction = &delete_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations;
        assert!(delete_transaction
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeleteSubtree { .. })));
        assert!(delete_transaction
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeletePage { .. })));
    }

    #[test]
    fn execution_material_uses_graph_filename_decoding_for_affected_new_paths() {
        let legacy = SnapshotFixture::new_with_graph_config(
            "execution-path-names-legacy",
            &["pages/seed.md"],
            "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"yyyy-MM-dd\"}\n",
        );
        for path in [
            "pages/first/second/Project%2FPlan.md",
            "pages/left/shared.md",
            "pages/right/shared.md",
            "journals/archive/deep/25-07-2026.md",
        ] {
            let target = legacy.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let legacy_plan = legacy.plan(&[
            "pages/first/second/Project%2FPlan.md",
            "journals/archive/deep/25-07-2026.md",
        ]);
        assert_eq!(legacy_plan.status(), ImportPlanStatus::Reconcile);
        let legacy_creates = legacy_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    name, path, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            legacy_creates["pages/first/second/Project%2FPlan.md"],
            ("Project/Plan", ManagedTextKind::Page),
            "nested directories select the managed root but never become a page namespace"
        );
        assert_eq!(
            legacy_creates["journals/archive/deep/25-07-2026.md"],
            ("2026-07-25", ManagedTextKind::Journal),
            "nested journals use the configured JournalFormat title"
        );

        let duplicate_names = legacy.plan(&["pages/left/shared.md", "pages/right/shared.md"]);
        assert_eq!(duplicate_names.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            duplicate_names.blocks()[0].reason,
            ImportBlockReason::ConflictingLocalTail,
            "same basenames in distinct paths are a visible ambiguity, never a successful import"
        );

        let triple_lowbar = SnapshotFixture::new_with_graph_config(
            "execution-path-names-triple-lowbar",
            &["pages/seed.md"],
            "{:file/name-format :triple-lowbar}\n",
        );
        for path in [
            "pages/deep/Team___Planning.md",
            "pages/deep/literal%5F%5F%5Fmarker.md",
        ] {
            let target = triple_lowbar.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let triple_plan = triple_lowbar.plan(&[
            "pages/deep/Team___Planning.md",
            "pages/deep/literal%5F%5F%5Fmarker.md",
        ]);
        assert_eq!(triple_plan.status(), ImportPlanStatus::Reconcile);
        let triple_creates = triple_plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    name, path, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            triple_creates["pages/deep/Team___Planning.md"],
            ("Team/Planning", ManagedTextKind::Page)
        );
        assert_eq!(
            triple_creates["pages/deep/literal%5F%5F%5Fmarker.md"],
            ("literal___marker", ManagedTextKind::Page),
            "TripleLowbar decodes separators before percent escapes, preserving encoded literals"
        );
    }

    #[test]
    fn configured_nested_managed_roots_use_graph_kind_and_filename_decoding() {
        let fixture = SnapshotFixture::new_with_graph_config(
            "configured-nested-roots",
            &["content/pages/seed.md"],
            "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"\n\
              :journal/file-name-format \"dd-MM-yyyy\"\n\
              :journal/page-title-format \"yyyy-MM-dd\"}\n",
        );
        for path in [
            "content/pages/deep/Project%2FPlan.md",
            "content/journals/archive/deep/25-07-2026.md",
        ] {
            let target = fixture.graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, b"- external\n").unwrap();
        }
        let plan = fixture.plan(&[
            "content/pages/deep/Project%2FPlan.md",
            "content/journals/archive/deep/25-07-2026.md",
        ]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let created = plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::CreatePage {
                    path, name, kind, ..
                } => Some((path.as_str(), (name.as_str(), *kind))),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            created["content/pages/deep/Project%2FPlan.md"],
            ("Project/Plan", ManagedTextKind::Page)
        );
        assert_eq!(
            created["content/journals/archive/deep/25-07-2026.md"],
            ("2026-07-25", ManagedTextKind::Journal)
        );
    }

    #[test]
    fn initial_shadow_inventory_enrolls_configured_nested_roots() {
        let root = TestRoot::new("initial-configured-roots");
        let graph_root = root.path().join("graph");
        fs::create_dir_all(graph_root.join("logseq")).unwrap();
        fs::write(
            graph_root.join("logseq/config.edn"),
            "{:pages-directory \"content/pages\"\n\
              :journals-directory \"content/journals\"}\n",
        )
        .unwrap();
        for (path, bytes) in [
            ("content/pages/deep/page.md", b"- page\n".as_slice()),
            (
                "content/journals/archive/journal.org",
                b"* journal\n".as_slice(),
            ),
        ] {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, bytes).unwrap();
        }
        let inventory = inventory_initial_shadow(&Graph::open(&graph_root)).unwrap();
        assert_eq!(
            inventory
                .entries()
                .keys()
                .map(ManagedPath::as_str)
                .collect::<Vec<_>>(),
            vec![
                "content/journals/archive/journal.org",
                "content/pages/deep/page.md",
            ]
        );
        assert!(inventory.entries().values().all(
            |observation| matches!(observation, RawObservation::Present(bytes) if !bytes.bytes().is_empty())
        ));
    }

    #[test]
    fn exact_rename_adopts_graph_decoded_destination_name_before_authoring() {
        let fixture = SnapshotFixture::new("rename-destination-name", &["pages/old.md"]);
        let destination = fixture.graph_root.join("pages/Project%2FPlan.md");
        fs::rename(fixture.graph_root.join("pages/old.md"), &destination).unwrap();
        let plan = fixture.plan(&["pages/old.md", "pages/Project%2FPlan.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        assert!(plan
            .execution_material()
            .unwrap()
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::ReconcileExternalPageState { name, path, .. }
                    if name.as_str() == "Project/Plan" && path.as_str() == "pages/Project%2FPlan.md"
            )));
    }

    #[test]
    fn sealed_external_execution_drafts_through_the_engine_in_parent_before_child_order() {
        let fixture = SnapshotFixture::new("external-engine-draft", &["pages/old.md"]);
        let destination = fixture.graph_root.join("pages/Project%2FPlan.md");
        fs::rename(fixture.graph_root.join("pages/old.md"), &destination).unwrap();
        fs::write(
            fixture.graph_root.join("pages/new.md"),
            b"- parent\n\t- child\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/old.md", "pages/Project%2FPlan.md", "pages/new.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let material = plan.into_execution_material().unwrap();
        let operations = &material.transaction().operations;
        assert!(operations.iter().any(|operation| matches!(
            operation,
            SemanticOperation::ReconcileExternalPageState { name, path, .. }
                if name.as_str() == "Project/Plan" && path.as_str() == "pages/Project%2FPlan.md"
        )));

        let created = operations
            .iter()
            .enumerate()
            .filter_map(|(index, operation)| match operation {
                SemanticOperation::CreateBlock { block, parent, .. } => {
                    Some((index, block.block_id, *parent))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let (child_index, _, parent) = created
            .iter()
            .copied()
            .find(|(_, _, parent)| parent.is_some())
            .expect("new nested child must be created");
        let parent = parent.expect("selected child has a parent");
        let parent_index = created
            .iter()
            .find_map(|(index, block_id, _)| (*block_id == parent).then_some(*index))
            .expect("new child parent must be created in this transaction");
        assert!(parent_index < child_index);

        // The external-import draft adapter applies the sealed operations to the
        // engine's prospective documents, so this catches ordering and
        // semantic preflight regressions that an operation-list inspection
        // would miss. Final external publication deliberately remains behind
        // the capability recapture boundary.
        let draft = fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_412)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_413),
                },
                material,
            )
            .unwrap();
        assert!(!draft.requirements().is_empty());

        // Identity replacement and removal travel through the same engine
        // draft path; these are semantic mutations, not observation-only
        // annotations.
        let prior = LogseqUuid::from_uuid(Uuid::from_u128(9_410));
        let replacement = LogseqUuid::from_uuid(Uuid::from_u128(9_411));
        let replacement_fixture = SnapshotFixture::new_with_initial_uuid(
            "external-engine-id-replacement",
            &["pages/a.md"],
            Some(prior),
        );
        fs::write(
            replacement_fixture.graph_root.join("pages/a.md"),
            format!("- page 0\n  id:: {replacement}\n"),
        )
        .unwrap();
        let replacement_plan = replacement_fixture.plan(&["pages/a.md"]);
        let replacement_material = replacement_plan.into_execution_material().unwrap();
        assert!(replacement_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::ReplaceExternal { logseq_uuid },
                    ..
                } if *logseq_uuid == replacement
            )));
        replacement_fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: replacement_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_414)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_415),
                },
                replacement_material,
            )
            .unwrap();

        let removal_fixture = SnapshotFixture::new_with_initial_uuid(
            "external-engine-id-removal",
            &["pages/a.md"],
            Some(prior),
        );
        fs::write(removal_fixture.graph_root.join("pages/a.md"), b"- page 0\n").unwrap();
        let removal_plan = removal_fixture.plan(&["pages/a.md"]);
        let removal_material = removal_plan.into_execution_material().unwrap();
        assert!(removal_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(
                operation,
                SemanticOperation::MutateBlockLogseqIdentity {
                    mutation: LogseqIdentityMutation::RemoveExternal,
                    ..
                }
            )));
        removal_fixture
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: removal_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_416)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_417),
                },
                removal_material,
            )
            .unwrap();
    }

    #[test]
    fn existing_authenticated_logical_name_collision_blocks_before_execution_material() {
        let fixture = SnapshotFixture::new("existing-name-collision", &["pages/seed.md"]);
        let target = fixture.graph_root.join("pages/Snapshot%20Page%200.md");
        fs::write(&target, b"- external\n").unwrap();
        let plan = fixture.plan(&["pages/Snapshot%20Page%200.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Blocked);
        assert_eq!(
            plan.blocks()[0].reason,
            ImportBlockReason::ConflictingLocalTail
        );
        assert!(plan.blocks()[0].detail.contains("already owned"));
    }

    #[test]
    fn atomic_page_name_transition_allows_chains_deletion_reuse_and_cycles() {
        let chain = SnapshotFixture::new("name-chain", &["pages/a.md", "pages/b.md"]);
        fs::rename(
            chain.graph_root.join("pages/a.md"),
            chain.graph_root.join("pages/Snapshot%20Page%201.md"),
        )
        .unwrap();
        fs::rename(
            chain.graph_root.join("pages/b.md"),
            chain.graph_root.join("pages/final.md"),
        )
        .unwrap();
        let chain_plan = chain.plan(&[
            "pages/a.md",
            "pages/b.md",
            "pages/Snapshot%20Page%201.md",
            "pages/final.md",
        ]);
        assert_eq!(chain_plan.status(), ImportPlanStatus::Reconcile);
        let chain_material = chain_plan.into_execution_material().unwrap();
        let chain_renames = chain_material
            .transaction()
            .operations
            .iter()
            .filter_map(|operation| match operation {
                SemanticOperation::ReconcileExternalPageState {
                    page_id,
                    name,
                    path,
                    ..
                } => Some((*page_id, name.as_str(), path.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(chain_renames.len(), 2);
        assert!(chain_renames
            .iter()
            .any(|(_, name, _)| *name == "Snapshot Page 1"));
        chain
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: chain_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_420)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_421),
                },
                chain_material,
            )
            .unwrap();

        let reuse = SnapshotFixture::new("delete-name-reuse", &["pages/a.md", "pages/b.md"]);
        fs::remove_file(reuse.graph_root.join("pages/b.md")).unwrap();
        fs::write(
            reuse.graph_root.join("pages/Snapshot%20Page%201.md"),
            b"- replacement identity\n",
        )
        .unwrap();
        let reuse_plan = reuse.plan(&["pages/b.md", "pages/Snapshot%20Page%201.md"]);
        assert_eq!(reuse_plan.status(), ImportPlanStatus::Reconcile);
        let reuse_material = reuse_plan.into_execution_material().unwrap();
        assert!(reuse_material.transaction().operations.iter().any(
            |operation| matches!(operation, SemanticOperation::CreatePage { name, .. }
                if name.as_str() == "Snapshot Page 1")
        ));
        assert!(reuse_material
            .transaction()
            .operations
            .iter()
            .any(|operation| matches!(operation, SemanticOperation::DeletePage { .. })));
        reuse
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: reuse_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_422)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_423),
                },
                reuse_material,
            )
            .unwrap();

        let cycle = SnapshotFixture::new("name-cycle", &["pages/a.md", "pages/b.md"]);
        let temporary = cycle.graph_root.join("pages/cycle.tmp");
        fs::rename(cycle.graph_root.join("pages/a.md"), &temporary).unwrap();
        fs::rename(
            cycle.graph_root.join("pages/b.md"),
            cycle.graph_root.join("pages/Snapshot%20Page%200.md"),
        )
        .unwrap();
        fs::rename(
            temporary,
            cycle.graph_root.join("pages/Snapshot%20Page%201.md"),
        )
        .unwrap();
        let cycle_plan = cycle.plan(&[
            "pages/a.md",
            "pages/b.md",
            "pages/Snapshot%20Page%200.md",
            "pages/Snapshot%20Page%201.md",
        ]);
        assert_eq!(cycle_plan.status(), ImportPlanStatus::Reconcile);
        let cycle_material = cycle_plan.into_execution_material().unwrap();
        cycle
            .engine
            .draft_external_import_transaction(
                AuthorBatch {
                    batch_id: cycle_material.batch_id(),
                    author_device_id: DeviceId::from_uuid(Uuid::from_u128(3)),
                    author_session_id: SessionId::from_uuid(Uuid::from_u128(9_424)),
                    crdt_peer_id: CrdtPeerId::from_u64(9_425),
                },
                cycle_material,
            )
            .unwrap();
    }

    #[test]
    fn external_observation_annotations_use_each_nested_block_exact_byte_span() {
        let fixture = SnapshotFixture::new("nested-exact-spans", &["pages/a.md"]);
        let bytes = b"- parent\n\t- child\n";
        fs::write(fixture.graph_root.join("pages/a.md"), bytes).unwrap();
        let plan = fixture.plan(&["pages/a.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let entry = &plan.execution_material().unwrap().observation().entries()[0];
        let spans = entry
            .state()
            .annotations()
            .iter()
            .map(|annotation| (annotation.span().start(), annotation.span().end()))
            .collect::<Vec<_>>();
        assert_eq!(spans, vec![(0, 9), (9, bytes.len() as u64)]);
    }

    #[test]
    fn parser_owned_spans_cover_literal_regions_crlf_preambles_and_multiple_roots() {
        fn offsets(text: &[u8], needles: &[&[u8]]) -> Vec<u64> {
            needles
                .iter()
                .map(|needle| {
                    text.windows(needle.len())
                        .position(|window| window == *needle)
                        .unwrap() as u64
                })
                .collect()
        }

        let markdown = b"title:: Page\r\n\r\n- parent\r\n  continuation\r\n  ```\r\n  - literal\r\n  ```\r\n  - child\r\n- root two\r\n";
        let markdown_path = ManagedPath::parse("pages/literal.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&markdown_path, markdown, &mut instrumentation).unwrap();
        let starts = offsets(markdown, &[b"- parent", b"  - child", b"- root two"]);
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], markdown.len() as u64),
            ]
        );
        assert!(tree.nodes[0].raw.contains("- literal"));
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.roots, vec![0, 2]);

        let org = b"#+TITLE: Page\r\n* parent\r\n#+BEGIN_SRC\r\n* literal\r\n#+END_SRC\r\n** child\r\n* root two\r\n";
        let org_path = ManagedPath::parse("pages/literal.org").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&org_path, org, &mut instrumentation).unwrap();
        let starts = offsets(org, &[b"* parent", b"** child", b"* root two"]);
        assert_eq!(tree.nodes.len(), 3);
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (starts[0], starts[1]),
                (starts[1], starts[2]),
                (starts[2], org.len() as u64),
            ]
        );
        assert!(tree.nodes[0].raw.contains("* literal"));
        assert_eq!(tree.nodes[0].children, vec![1]);
        assert_eq!(tree.roots, vec![0, 2]);
    }

    #[test]
    fn promoted_collapsed_preamble_heading_has_an_exact_parser_owned_span() {
        let bytes =
            b"page:: property\r\n\r\n# Collapsed\r\ncollapsed:: true\r\n- child\r\n- sibling\r\n";
        let path = ManagedPath::parse("pages/promoted.md").unwrap();
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&path, bytes, &mut instrumentation).unwrap();
        let heading = bytes
            .windows(b"# Collapsed".len())
            .position(|window| window == b"# Collapsed")
            .unwrap() as u64;
        let child = bytes
            .windows(b"- child".len())
            .position(|window| window == b"- child")
            .unwrap() as u64;
        let sibling = bytes
            .windows(b"- sibling".len())
            .position(|window| window == b"- sibling")
            .unwrap() as u64;
        assert_eq!(
            tree.nodes
                .iter()
                .map(|node| (node.span.start(), node.span.end()))
                .collect::<Vec<_>>(),
            vec![
                (heading, child),
                (child, sibling),
                (sibling, bytes.len() as u64),
            ]
        );
        assert_eq!(tree.roots, vec![0]);
        assert_eq!(tree.nodes[0].children, vec![1, 2]);
    }

    #[test]
    fn completed_direct_receipt_cannot_reach_catalog_capture_after_empty_rollback() {
        let fixture = SnapshotFixture::new("direct-receipt-empty-rollback", &["pages/a.md"]);
        let SnapshotFixture {
            _root,
            graph,
            receipts,
            engine,
            empty_history_head,
            ..
        } = fixture;
        let workspace = engine.workspace_id();
        let endpoint = engine.projection_endpoint_binding().unwrap();
        let archive = _root.path().join("archive");
        let history_head = archive
            .join("engine-history")
            .join(endpoint.endpoint_id.to_string())
            .join("engine-history.head");
        drop(engine);
        fs::write(history_head, empty_history_head).unwrap();

        let reopened = ShardedHotEngine::with_enrolled_projection(
            ObjectStore::open(&archive, workspace).unwrap(),
            LineageDigest::of(b"snapshot-test"),
            DocumentId::from_uuid(Uuid::from_u128(4)),
            &graph,
            &receipts,
        );
        let mut instrumentation = ImportInstrumentation::default();
        let requested = vec![ManagedPath::parse("pages/a.md").unwrap()];
        let block =
            capture_affected_catalog(&receipts, &reopened, &requested, &mut instrumentation)
                .unwrap_err();
        assert_eq!(block.reason, ImportBlockReason::AuthorityUnavailable);
        assert_eq!(instrumentation.catalog_entries, 0);
        assert!(block.detail.contains("history") || block.detail.contains("projection"));
    }

    #[test]
    fn affected_receipt_capture_ignores_unrelated_completed_receipts() {
        use crate::oplog::projection_store::{
            projection_store_test_counters, reset_projection_store_test_counters,
        };

        let paths = (0..33)
            .map(|index| format!("pages/unrelated/{index:02}.md"))
            .collect::<Vec<_>>();
        let path_refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = SnapshotFixture::new("bounded-receipt-capture", &path_refs);
        fs::write(
            fixture.graph_root.join("pages/unrelated/00.md"),
            b"- changed\n",
        )
        .unwrap();
        let unrelated_completion = fixture
            .receipts
            .root_path()
            .join("completions")
            .join(completion_name(&fixture.intents[32]));
        reset_projection_store_test_counters();
        SNAPSHOT_REVALIDATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(unrelated_completion).unwrap();
            }));
        });
        let plan = fixture.plan(&["pages/unrelated/00.md"]);
        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        let counters = projection_store_test_counters();
        assert_eq!(counters.catalog_directory_entries, 0);
        assert_eq!(
            counters.completion_lookups, 2,
            "only the requested receipt is point-loaded in each snapshot pass"
        );
        assert_eq!(plan.instrumentation().catalog_entries, 2);
    }

    #[test]
    fn affected_import_never_scans_unrelated_pages_for_home_documents() {
        let paths = (0..32)
            .map(|index| format!("pages/unrelated/{index:02}.md"))
            .collect::<Vec<_>>();
        let path_refs = paths.iter().map(String::as_str).collect::<Vec<_>>();
        let fixture = SnapshotFixture::new("affected-home-scope", &path_refs);
        fs::write(
            fixture.graph_root.join("pages/unrelated/00.md"),
            b"- externally changed\n",
        )
        .unwrap();

        let plan = fixture.plan(&["pages/unrelated/00.md"]);

        assert_eq!(plan.status(), ImportPlanStatus::Reconcile);
        assert_eq!(plan.instrumentation().catalog_path_lookups, 1);
        assert_eq!(
            fixture.engine.canonical_snapshot_calls_for_test(),
            0,
            "page-home capture must stay at the point-scoped accepted materialization"
        );
    }

    #[test]
    fn aggregate_budget_refuses_before_overflow_or_allocation() {
        assert_eq!(
            charge_budget(
                "aggregate raw bytes",
                MAX_IMPORT_RAW_BYTES - 1,
                1,
                MAX_IMPORT_RAW_BYTES
            )
            .unwrap(),
            MAX_IMPORT_RAW_BYTES
        );
        assert!(matches!(
            charge_budget(
                "aggregate raw bytes",
                MAX_IMPORT_RAW_BYTES,
                1,
                MAX_IMPORT_RAW_BYTES
            ),
            Err(InventoryError::ResourceBudgetExceeded {
                resource: "aggregate raw bytes",
                ..
            })
        ));
        assert!(charge_budget("aggregate raw bytes", u64::MAX, 1, MAX_IMPORT_RAW_BYTES).is_err());

        let path = ManagedPath::parse("pages/a.md").unwrap();
        assert_eq!(
            preflight_depth(&path, "- one more\n", MAX_IMPORT_PARSED_NODES)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
        let tree = ParsedTree {
            path,
            preamble: None,
            roots: vec![0],
            nodes: vec![ParsedNode {
                parent: None,
                sibling_position: 0,
                depth: 1,
                children: Vec::new(),
                span: StructuralSpan::new(0, 0).unwrap(),
                raw: "node".into(),
                raw_ids: Vec::new(),
            }],
        };
        let mut instrumentation = ImportInstrumentation {
            locator_components_materialized: MAX_IMPORT_LOCATOR_COMPONENTS,
            ..ImportInstrumentation::default()
        };
        assert_eq!(
            materialize_locator(&tree, 0, &mut instrumentation)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );

        let mut replay = ImportInstrumentation::default();
        let replay_limits = ImportReplayLimits {
            entries: 2,
            base_bytes: 8,
            rendered_bytes: 8,
        };
        let path = ManagedPath::parse("pages/replay.md").unwrap();
        reserve_base_replay(&mut replay, 4, replay_limits, &path).unwrap();
        reserve_base_replay(&mut replay, 4, replay_limits, &path).unwrap();
        assert_eq!(
            reserve_base_replay(&mut replay, 0, replay_limits, &path)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
        retain_rendered_target(&mut replay, 8, replay_limits, &path).unwrap();
        assert_eq!(
            retain_rendered_target(&mut replay, 1, replay_limits, &path)
                .unwrap_err()
                .reason,
            ImportBlockReason::ResourceLimit
        );
    }

    #[test]
    fn operation_count_bound_refuses_the_100001st_operation_before_publication() {
        let mut operations = Vec::with_capacity(MAX_TRANSACTION_OPERATIONS);
        for index in 0..MAX_TRANSACTION_OPERATIONS {
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
            )
            .unwrap();
        }
        assert_eq!(operations.len(), MAX_TRANSACTION_OPERATIONS);
        assert!(matches!(
            push_operation(
                &mut operations,
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(100_001)),
                },
            ),
            Err(ImportExecutionError::OperationLimit)
        ));
    }

    #[test]
    fn structural_common_prefix_work_and_repeated_deep_locators_are_charged() {
        let path = ManagedPath::parse("pages/structural.md").unwrap();
        let mut text = String::new();
        for _ in 0..64 {
            text.push_str("- parent\n");
            for _ in 0..16 {
                text.push_str("\t- same child\n");
            }
        }
        let mut instrumentation = ImportInstrumentation::default();
        let tree = parse_nodes(&path, text.as_bytes(), &mut instrumentation).unwrap();
        let mut interner = StructuralInterner::new();
        structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
        structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
        assert!(instrumentation.structural_key_components >= tree.nodes.len() * 2);
        assert!(instrumentation.structural_key_comparisons > tree.nodes.len());

        let mut nodes = Vec::new();
        for depth in 1..=MAX_IMPORT_DEPTH {
            nodes.push(ParsedNode {
                parent: if depth > 1 { Some(depth - 2) } else { None },
                sibling_position: 0,
                depth,
                children: (depth < MAX_IMPORT_DEPTH)
                    .then_some(depth)
                    .into_iter()
                    .collect(),
                span: StructuralSpan::new(0, 0).unwrap(),
                raw: "node".into(),
                raw_ids: vec!["duplicate".into(), "duplicate".into()],
            });
        }
        let deep = ParsedTree {
            path,
            preamble: None,
            roots: vec![0],
            nodes,
        };
        let before = instrumentation.locator_components_materialized;
        materialize_locator(&deep, MAX_IMPORT_DEPTH - 1, &mut instrumentation).unwrap();
        materialize_locator(&deep, MAX_IMPORT_DEPTH - 1, &mut instrumentation).unwrap();
        assert_eq!(
            instrumentation.locator_components_materialized - before,
            MAX_IMPORT_DEPTH * 2
        );
    }

    #[test]
    fn structural_class_allocation_work_is_linear_across_many_pages() {
        fn measured(page_count: usize) -> ImportInstrumentation {
            let mut interner = StructuralInterner::new();
            let mut instrumentation = ImportInstrumentation::default();
            for index in 0..page_count {
                let tree = ParsedTree {
                    path: ManagedPath::parse(&format!("pages/p{index:08}.md")).unwrap(),
                    preamble: None,
                    roots: vec![0],
                    nodes: vec![ParsedNode {
                        parent: None,
                        sibling_position: 0,
                        depth: 1,
                        children: Vec::new(),
                        span: StructuralSpan::new(0, 0).unwrap(),
                        raw: format!("unique-{index:08}"),
                        raw_ids: Vec::new(),
                    }],
                };
                structural_classes(&tree, &mut interner, &mut instrumentation).unwrap();
                instrumentation.structural_class_nodes = instrumentation
                    .structural_class_nodes
                    .saturating_add(tree.nodes.len());
            }
            instrumentation
        }

        let small = measured(1_024);
        let large = measured(8_192);
        assert_eq!(small.structural_class_allocations, 1_024);
        assert_eq!(large.structural_class_allocations, 8_192);
        assert_eq!(small.structural_key_comparisons, 0);
        assert_eq!(large.structural_key_comparisons, 0);
        assert!(
            large.recorded_work_units() <= small.recorded_work_units().saturating_mul(8),
            "structural work did not scale linearly: small={small:?}, large={large:?}"
        );
    }

    fn prepare_streaming_bootstrap(
        label: &str,
        files: &[(&str, &str)],
    ) -> (TestRoot, InactiveBootstrapPreparedPublication, WorkspaceId) {
        let root = TestRoot::new(label);
        let graph_root = root.path().join("graph");
        for (path, contents) in files {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, contents).unwrap();
        }
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture-scratch");
        let preparation_scratch = root.path().join("preparation-scratch");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5a01));
        let prepared = prepare_inactive_bootstrap_import(
            &graph,
            capture,
            workspace,
            LineageDigest::of(b"inactive-streaming-bootstrap-test"),
            DocumentId::from_uuid(Uuid::from_u128(0x5a02)),
            ReferenceCatalogPolicyV1::default(),
            &preparation_scratch,
        )
        .unwrap();
        (root, prepared, workspace)
    }

    #[test]
    fn inactive_streaming_bootstrap_zero_source_is_canonical_and_sealed() {
        let (_root, prepared, _) = prepare_streaming_bootstrap("streaming-zero", &[]);
        assert!(prepared.aggregate().parts().is_empty());
        assert_eq!(prepared.commit().part_count(), 0);
        assert_eq!(prepared.candidate().part_count(), 0);
        assert_eq!(prepared.instrumentation().operations, 0);
        assert_eq!(prepared.instrumentation().parts, 0);
        prepared
            .commit()
            .validate_aggregate(prepared.aggregate())
            .unwrap();
        assert_eq!(
            BootstrapAggregateCommitV1::decode(&prepared.commit_bytes().unwrap()).unwrap(),
            prepared.commit()
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_import_id_and_prepared_bytes_are_canonical() {
        let (_root, prepared, workspace) = prepare_streaming_bootstrap(
            "streaming-one",
            &[(
                "pages/Unicode α.md",
                "title:: Streaming title\n- parent\n  - child",
            )],
        );
        let mut inventory = Vec::new();
        let mut cursor = prepared.source_capture().entries_cursor().unwrap();
        while let Some(entry) = cursor.next().unwrap() {
            inventory.push(ImportInventoryEntry::with_kind(
                entry.kind(),
                entry.path().clone(),
                ImportInventoryState::Present(entry.description()),
            ));
        }
        let materialized =
            ImportId::derive(workspace, &[], &inventory, DIFF_SCHEMA_VERSION).unwrap();
        assert_eq!(prepared.aggregate().import_id(), materialized);
        assert_eq!(prepared.aggregate().parts().len(), 1);
        assert_eq!(prepared.instrumentation().operations, 4);

        let mut part = prepared.open_part(0).unwrap();
        let evidence = part.evidence().unwrap();
        assert_eq!(evidence.operation_root().operation_count(), 4);
        let span_index = part.span_index().unwrap();
        span_index.validate_part(evidence).unwrap();
        let manifest = super::super::OperationBatch::decode(part.manifest_bytes()).unwrap();
        let mut objects = Vec::new();
        while let Some(bytes) = part.next_object_bytes().unwrap() {
            objects.push(super::super::OperationObject::decode(&bytes).unwrap());
        }
        super::super::PreparedBatch::new(manifest, objects).unwrap();
    }

    fn synthetic_operation_spool(
        directory: &Path,
        operation_count: u64,
    ) -> BootstrapOperationSpool {
        let path = directory.join("synthetic-operations.sorted");
        let mut writer = BufWriter::new(create_new_file(&path).unwrap());
        for index in 0..operation_count {
            let record = BootstrapOperationRecord::new(
                SemanticOperation::DeletePage {
                    page_id: PageId::from_uuid(Uuid::from_u128(index as u128 + 1)),
                },
                SourceLeafDigestV1::from_bytes([0x61; 32]),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: index.to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        BootstrapOperationSpool {
            path,
            operation_count,
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_partitions_zero_one_4096_and_4097_without_retention() {
        for (count, expected_parts) in [(0, 0), (1, 1), (4096, 1), (4097, 2)] {
            let root = TestRoot::new(&format!("streaming-partition-{count}"));
            let working = root.path().join("partition");
            fs::create_dir(&working).unwrap();
            let spool = synthetic_operation_spool(&working, count);
            let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
            assert_eq!(
                partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation)
                    .unwrap(),
                expected_parts
            );
            assert_eq!(instrumentation.parts, expected_parts);
            assert!(instrumentation.peak_owned_part_operations == 0);
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_partitions_100001_synthetic_operations_boundedly() {
        let root = TestRoot::new("streaming-partition-100001");
        let working = root.path().join("partition");
        fs::create_dir(&working).unwrap();
        let spool = synthetic_operation_spool(&working, 100_001);
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        assert_eq!(
            partition_bootstrap_operation_spool(&spool, &working, &mut instrumentation).unwrap(),
            25
        );
        assert_eq!(instrumentation.parts, 25);
        assert_eq!(instrumentation.peak_owned_part_operations, 0);
        assert!(instrumentation.source_spans <= 100_001);
    }

    #[test]
    fn inactive_streaming_bootstrap_capture_c_mutation_leaves_no_seal() {
        let root = TestRoot::new("streaming-capture-c-mutation");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/mutable.md"), "- before").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        fs::write(graph_root.join("pages/mutable.md"), "- after").unwrap();
        assert!(prepare_inactive_bootstrap_import(
            &graph,
            capture,
            WorkspaceId::from_uuid(Uuid::from_u128(0x5b01)),
            LineageDigest::of(b"capture-c-mutation"),
            DocumentId::from_uuid(Uuid::from_u128(0x5b02)),
            ReferenceCatalogPolicyV1::default(),
            &preparation_scratch,
        )
        .is_err());
        let publication_root = preparation_scratch.join(BOOTSTRAP_STREAM_DIRECTORY);
        assert!(
            !publication_root.exists()
                || fs::read_dir(publication_root).unwrap().all(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".building-"))
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_repeated_run_reuses_exact_seal() {
        let root = TestRoot::new("streaming-repeat");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/repeat.md"), "- repeat").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5c01));
        let prepare_once = || {
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"streaming-repeat"),
                DocumentId::from_uuid(Uuid::from_u128(0x5c02)),
                ReferenceCatalogPolicyV1::default(),
                &preparation_scratch,
            )
            .unwrap()
        };
        let first = prepare_once();
        let first_aggregate = first.aggregate_bytes().unwrap();
        let first_commit = first.commit_bytes().unwrap();
        drop(first);
        let second = prepare_once();
        assert_eq!(second.aggregate_bytes().unwrap(), first_aggregate);
        assert_eq!(second.commit_bytes().unwrap(), first_commit);
    }

    #[test]
    fn inactive_streaming_bootstrap_seals_many_artifact_entries_streamingly() {
        let root = TestRoot::new("streaming-seal-many-entries");
        let artifacts = root.path().join("artifacts");
        let sealed = root.path().join("sealed");
        let nested = artifacts.join("nested");
        fs::create_dir(&artifacts).unwrap();
        fs::create_dir(&nested).unwrap();

        for index in 0_u32..128 {
            let directory = if index % 2 == 0 { &artifacts } else { &nested };
            fs::write(
                directory.join(format!("artifact-{index:03}.bin")),
                index.to_be_bytes(),
            )
            .unwrap();
        }

        for _ in 0..2 {
            seal_bootstrap_preparation(&artifacts, &sealed, b"commit").unwrap();
        }

        assert_eq!(
            fs::read(sealed.join(BOOTSTRAP_STREAM_SEAL)).unwrap(),
            b"commit"
        );
        let sealed_nested = sealed.join("nested");
        for index in 0_u32..128 {
            let directory = if index % 2 == 0 {
                &sealed
            } else {
                &sealed_nested
            };
            assert_eq!(
                fs::read(directory.join(format!("artifact-{index:03}.bin"))).unwrap(),
                index.to_be_bytes()
            );
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_conflicting_sealed_artifact_fails_closed() {
        let root = TestRoot::new("streaming-conflicting-seal");
        let graph_root = root.path().join("graph");
        fs::write(graph_root.join("pages/conflict.md"), "- conflict").unwrap();
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let preparation_scratch = root.path().join("preparation");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&preparation_scratch).unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5c11));
        let prepare_once = || {
            let capture = graph
                .capture_inactive_bootstrap_sources(&capture_scratch)
                .unwrap();
            prepare_inactive_bootstrap_import(
                &graph,
                capture,
                workspace,
                LineageDigest::of(b"streaming-conflicting-seal"),
                DocumentId::from_uuid(Uuid::from_u128(0x5c12)),
                ReferenceCatalogPolicyV1::default(),
                &preparation_scratch,
            )
        };
        let first = prepare_once().unwrap();
        fs::write(
            first.sealed_directory.join(BOOTSTRAP_STREAM_AGGREGATE),
            b"accidental corruption",
        )
        .unwrap();
        drop(first);
        assert!(matches!(
            prepare_once(),
            Err(BootstrapStreamingImportError::ConflictingSeal)
        ));
    }

    #[test]
    fn inactive_streaming_bootstrap_authors_4096_and_4097_operation_boundaries() {
        for (block_count, expected_parts) in [(4095, 1), (4096, 2)] {
            let mut source = String::new();
            for index in 0..block_count {
                source.push_str(&format!("- block {index:04}\n"));
            }
            let label = format!("streaming-author-boundary-{block_count}");
            let (_root, prepared, _) =
                prepare_streaming_bootstrap(&label, &[("pages/boundary.md", &source)]);
            assert_eq!(prepared.aggregate().parts().len(), expected_parts);
            assert_eq!(
                prepared.aggregate().parts()[0]
                    .evidence()
                    .operation_root()
                    .operation_count(),
                4096
            );
            if expected_parts == 2 {
                assert_eq!(
                    prepared.aggregate().parts()[1]
                        .evidence()
                        .operation_root()
                        .operation_count(),
                    1
                );
            }
        }
    }

    #[test]
    fn inactive_streaming_bootstrap_cross_part_child_and_later_content_depend_on_candidate() {
        let root = TestRoot::new("streaming-cross-part");
        let working = root.path().join("author");
        fs::create_dir(&working).unwrap();
        let page = PageId::from_uuid(Uuid::from_u128(0x5d01));
        let home = DocumentId::from_uuid(Uuid::from_u128(0x5d02));
        let parent = BlockId::from_uuid(Uuid::from_u128(0x5d03));
        let child = BlockId::from_uuid(Uuid::from_u128(0x5d04));
        let operations = [
            SemanticOperation::CreatePage {
                page_id: page,
                home_document_id: home,
                name: LogicalPageName::parse("Cross part").unwrap(),
                path: ManagedPath::parse("pages/cross-part.md").unwrap(),
                kind: ManagedTextKind::Page,
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: parent,
                    home_document_id: home,
                },
                page_id: page,
                parent: None,
                order: "0000000000".into(),
                content: "parent".into(),
            },
            SemanticOperation::CreateBlock {
                block: BlockLocation {
                    block_id: child,
                    home_document_id: home,
                },
                page_id: page,
                parent: Some(parent),
                order: "0000000000".into(),
                content: "child".into(),
            },
            SemanticOperation::EditBlockContent {
                block: BlockLocation {
                    block_id: parent,
                    home_document_id: home,
                },
                content: "parent updated later".into(),
            },
        ];
        let operation_path = working.join(BOOTSTRAP_STREAM_OPERATION_SPOOL);
        let mut writer = BufWriter::new(create_new_file(&operation_path).unwrap());
        for (index, operation) in operations.into_iter().enumerate() {
            let record = BootstrapOperationRecord::new(
                operation,
                SourceLeafDigestV1::from_bytes([0x62; 32]),
                None,
            )
            .unwrap();
            write_sort_record(
                &mut writer,
                &SortRecord {
                    key: (index as u64).to_be_bytes().to_vec(),
                    value: record.encode().unwrap(),
                },
            )
            .unwrap();
        }
        writer.flush().unwrap();
        writer.get_ref().sync_all().unwrap();
        let boundary_path = working.join(BOOTSTRAP_STREAM_BOUNDARY_SPOOL);
        let mut boundaries = BufWriter::new(create_new_file(&boundary_path).unwrap());
        write_frame(&mut boundaries, &2_u32.to_be_bytes()).unwrap();
        write_frame(&mut boundaries, &2_u32.to_be_bytes()).unwrap();
        boundaries.flush().unwrap();
        boundaries.get_ref().sync_all().unwrap();
        let mut instrumentation = BootstrapStreamingImportInstrumentation::default();
        let authored = author_bootstrap_parts(
            WorkspaceId::from_uuid(Uuid::from_u128(0x5d05)),
            LineageDigest::of(b"cross-part-author"),
            DocumentId::from_uuid(Uuid::from_u128(0x5d06)),
            ReferenceCatalogPolicyV1::default(),
            ImportId::from_digest([0x63; 32]),
            &BootstrapOperationSpool {
                path: operation_path,
                operation_count: 4,
            },
            2,
            &working,
            &mut instrumentation,
        )
        .unwrap();
        assert_eq!(authored.descriptors.len(), 2);
        assert_eq!(authored.candidate.part_count(), 2);
        assert_eq!(
            authored.descriptors[1].evidence().predecessor(),
            Some(authored.descriptors[0].part_id())
        );
    }

    #[test]
    fn inactive_streaming_bootstrap_matches_materialized_initial_semantic_order_and_layout() {
        let root = TestRoot::new("streaming-differential");
        let graph_root = root.path().join("graph");
        fs::create_dir_all(graph_root.join("logseq")).unwrap();
        fs::write(
            graph_root.join("logseq/config.edn"),
            r#"{:pages-directory "notes"
                :journals-directory "diary"
                :file/name-format :triple-lowbar
                :journal/file-name-format "dd-MM-yyyy"
                :journal/page-title-format "yyyy-MM-dd"}"#,
        )
        .unwrap();
        for (path, contents) in [
            ("Root.md", "title:: Root logical name\n\n- root\n"),
            (
                "notes/arbitrary/nested/Markdown.md",
                "title:: Markdown title ☕\n\n- parent\n  - child\n",
            ),
            ("notes/深い/Déjà___計画.markdown", "- filename page\n"),
            (
                "diary/nested/25-07-2026.org",
                "#+TITLE: Org title ��\n\n* org page\n",
            ),
            ("diary/nested/26-07-2026.md", "- journal\n"),
        ] {
            let target = graph_root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, contents).unwrap();
        }
        let graph = Graph::open(&graph_root);
        let capture_scratch = root.path().join("capture");
        let working = root.path().join("working");
        fs::create_dir(&capture_scratch).unwrap();
        fs::create_dir(&working).unwrap();
        let capture = graph
            .capture_inactive_bootstrap_sources(&capture_scratch)
            .unwrap();
        let workspace = WorkspaceId::from_uuid(Uuid::from_u128(0x5e01));

        let mut source_instrumentation = BootstrapStreamingImportInstrumentation::default();
        let source = prepare_bootstrap_source_protocol(
            workspace,
            &capture,
            &working,
            &mut source_instrumentation,
        )
        .unwrap();
        let streaming = spool_bootstrap_operations(
            &capture,
            source.import_id,
            workspace,
            &working,
            &mut source_instrumentation,
        )
        .unwrap();
        let mut streaming_reader = BootstrapOperationSpoolReader::open(&streaming.path).unwrap();
        let mut streaming_operations = Vec::new();
        while let Some(operation) = streaming_reader.next().unwrap() {
            streaming_operations.push(operation.operation);
        }

        let mut source_reader = BootstrapSourceReader::new(&capture).unwrap();
        let mut entries = capture.entries_cursor().unwrap();
        let mut raw_entries = Vec::new();
        let mut scope_paths = BTreeMap::new();
        let mut path_identities = BTreeMap::new();
        let mut read_instrumentation = BootstrapStreamingImportInstrumentation::default();
        while let Some(entry) = entries.next().unwrap() {
            let bytes = source_reader
                .read_entry(&entry, &mut read_instrumentation)
                .unwrap();
            raw_entries.push((entry.path().clone(), RawObservation::present(bytes)));
            scope_paths.insert(entry.path().clone(), ScopedPathEvidence::New);
            path_identities.insert(
                entry.path().clone(),
                ImportedPathIdentity {
                    name: LogicalPageName::parse(entry.logical_name()).unwrap(),
                    kind: entry.kind(),
                },
            );
        }
        source_reader.finish().unwrap();
        let inventory = RawInventory::from_entries(raw_entries).unwrap();
        let materialized_id = ImportId::derive(
            workspace,
            &[],
            &inventory.derivation_entries(&path_identities).unwrap(),
            DIFF_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(source.import_id, materialized_id);
        let scope = ImportScopeSnapshot {
            workspace_id: workspace,
            paths: scope_paths,
            path_identities,
        };
        let matches = ImportMatches::default();
        let transition =
            build_desired_page_transition(&inventory, &matches, &scope, materialized_id).unwrap();
        let old = build_execution_material(
            materialized_id,
            &inventory,
            &matches,
            &scope,
            &transition,
            &mut ImportInstrumentation::default(),
        )
        .unwrap();
        assert_eq!(streaming_operations, old.transaction.operations);
        let streaming_transaction =
            OperationTransaction::new(streaming_operations.clone()).unwrap();
        let canonical_snapshot = |transaction: &OperationTransaction| {
            let mut engine = ShardedHotEngine::new(
                workspace,
                LineageDigest::of(b"streaming-differential-snapshot"),
                DocumentId::from_uuid(Uuid::from_u128(0x5e02)),
            );
            engine
                .configure_reference_catalog_policy(ReferenceCatalogPolicyV1::default())
                .unwrap();
            let prepared = engine
                .prepare_bootstrap_transaction(
                    AuthorBatch {
                        batch_id: BatchId::from_uuid(Uuid::from_u128(0x5e03)),
                        author_device_id: DeviceId::from_uuid(Uuid::from_u128(0x5e04)),
                        author_session_id: SessionId::from_uuid(Uuid::from_u128(0x5e05)),
                        crdt_peer_id: CrdtPeerId::from_u64(0x5e06),
                    },
                    transaction,
                )
                .unwrap();
            assert!(matches!(
                engine
                    .stage_ready(super::super::ValidatedBatch::new(prepared))
                    .disposition,
                super::super::BatchDisposition::Accepted { .. }
            ));
            engine.canonical_snapshot().unwrap()
        };
        assert_eq!(
            canonical_snapshot(&streaming_transaction),
            canonical_snapshot(&old.transaction)
        );
        assert_eq!(
            capture
                .entries_cursor()
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
                .as_str(),
            "Root.md"
        );
    }
}
