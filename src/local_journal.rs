//! Versioned, canonical, length-prefixed, checksummed local journal frames and
//! the append-only per-device segment that stores them.
//!
//! A trusted-local commit needs exactly one durable record before it may
//! publish its own result. This module owns that record's physical form and
//! nothing about its meaning: the payload is opaque bytes and the payload kind
//! is a domain-supplied type parameter, so `tine-core` keeps its existing
//! semantic-effect and CRDT-update encodings and this crate gains no domain
//! knowledge.
//!
//! Durability contract. One [`LocalJournalSegment::append`] performs exactly one
//! write and exactly one data-durability barrier. The segment's directory entry
//! is made durable once, when the segment file is created, so a steady-state
//! commit never pays a directory synchronization.
//!
//! Recovery contract. Appends are ordered and each is made durable before the
//! caller proceeds, so an interrupted process can only ever have torn the final
//! frame. [`LocalJournalSegment::open`] scans forward, adopts the longest prefix
//! of complete canonical frames, and truncates a torn tail. A decode failure
//! whose declared extent lies wholly inside the file is *not* treated as a torn
//! tail — the bytes that follow prove the region was completely written once —
//! so it is refused as corruption instead of being silently discarded.

use std::fmt;
use std::fs;
use std::io::{self, BufReader, ErrorKind, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
#[cfg(unix)]
use std::os::fd::AsRawFd as _;
#[cfg(unix)]
use std::os::fd::FromRawFd as _;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle as _;

use cap_std::fs::{Dir, OpenOptions};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{sync_dir_required, ContentDigest};

/// Persistent frame envelope version. Any change to the frame layout, the
/// header fields, or their encoding requires a new value.
pub const LOCAL_JOURNAL_FRAME_SCHEMA_VERSION: u32 = 1;

/// Largest complete encoded frame the codec will produce or accept.
pub const MAX_LOCAL_JOURNAL_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Largest encoded frame header. The header is fixed-shape typed metadata, so
/// this bound exists to keep a corrupt length field from provoking a large
/// allocation, not to constrain callers.
pub const MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES: usize = 4 * 1024;

/// Largest segment file this module will scan or extend.
pub const MAX_LOCAL_JOURNAL_SEGMENT_BYTES: u64 = 4 * 1024 * 1024 * 1024;

const FRAME_MAGIC: &[u8; 8] = b"TINEJRN1";
const FRAME_CHECKSUM_BYTES: usize = 32;
/// magic + big-endian `u32` header length + big-endian `u64` payload length.
const FRAME_PREFIX_BYTES: usize = FRAME_MAGIC.len() + 4 + 8;
const MIN_FRAME_BYTES: usize = FRAME_PREFIX_BYTES + FRAME_CHECKSUM_BYTES;
const SEGMENT_SCAN_BUFFER_BYTES: usize = 64 * 1024;
/// Bound on the create/open race between two processes reaching a segment name
/// that does not exist yet. Each iteration makes progress or observes a
/// different filesystem state, so a small constant suffices.
const SEGMENT_OPEN_ATTEMPTS: usize = 8;

/// A domain-supplied discriminant naming the encoding of a frame's payload.
///
/// The blanket implementation means a domain crate only has to derive the usual
/// traits on its own enum; this crate never inspects the value beyond encoding,
/// decoding, and comparing it.
pub trait LocalJournalPayloadKind:
    Copy + fmt::Debug + Eq + Serialize + DeserializeOwned + 'static
{
}

impl<K> LocalJournalPayloadKind for K where
    K: Copy + fmt::Debug + Eq + Serialize + DeserializeOwned + 'static
{
}

/// A failure at the local journal boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalJournalError {
    Io(String),
    Encode(String),
    Decode(String),
    InvalidFrameMagic,
    LengthOverflow,
    FrameTooLarge(usize),
    FrameHeaderTooLarge(usize),
    TruncatedFrame,
    FrameLengthMismatch {
        expected: usize,
        actual: usize,
    },
    ChecksumMismatch,
    NonCanonicalFrameHeader,
    PayloadDigestMismatch,
    UnknownFrameSchemaVersion {
        expected: u32,
        found: u32,
    },
    UnsafeSegmentName(String),
    SegmentAlreadyOpen(String),
    SegmentTooLarge(u64),
    /// A complete, fully written region failed validation. Prior frames are
    /// intact but this segment is not safe to extend without operator action.
    CorruptSegment {
        offset: u64,
        cause: String,
    },
    SegmentDeviceMismatch {
        offset: u64,
        expected: Uuid,
        found: Uuid,
    },
    SegmentSequenceGap {
        offset: u64,
        expected: u64,
        found: u64,
    },
    SequenceExhausted,
    /// An append failed after starting to write. The in-memory append cursor no
    /// longer provably matches the file, so the segment refuses further appends
    /// until it is reopened and rescanned.
    SegmentPoisoned,
}

impl fmt::Display for LocalJournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "local journal I/O failure: {message}"),
            Self::Encode(message) => write!(formatter, "local journal encode failure: {message}"),
            Self::Decode(message) => write!(formatter, "local journal decode failure: {message}"),
            Self::InvalidFrameMagic => formatter.write_str("local journal frame magic is invalid"),
            Self::LengthOverflow => formatter.write_str("local journal frame length overflows"),
            Self::FrameTooLarge(length) => {
                write!(
                    formatter,
                    "local journal frame is too large: {length} bytes"
                )
            }
            Self::FrameHeaderTooLarge(length) => write!(
                formatter,
                "local journal frame header is too large: {length} bytes"
            ),
            Self::TruncatedFrame => formatter.write_str("local journal frame is truncated"),
            Self::FrameLengthMismatch { expected, actual } => write!(
                formatter,
                "local journal frame length mismatch: expected {expected}, got {actual}"
            ),
            Self::ChecksumMismatch => formatter.write_str("local journal frame checksum mismatch"),
            Self::NonCanonicalFrameHeader => {
                formatter.write_str("local journal frame header is not canonical")
            }
            Self::PayloadDigestMismatch => {
                formatter.write_str("local journal frame payload digest mismatch")
            }
            Self::UnknownFrameSchemaVersion { expected, found } => write!(
                formatter,
                "unknown local journal frame schema {found}; expected {expected}"
            ),
            Self::UnsafeSegmentName(name) => {
                write!(formatter, "unsafe local journal segment name: {name}")
            }
            Self::SegmentAlreadyOpen(name) => write!(
                formatter,
                "local journal segment {name} is already open elsewhere"
            ),
            Self::SegmentTooLarge(length) => write!(
                formatter,
                "local journal segment is too large: {length} bytes"
            ),
            Self::CorruptSegment { offset, cause } => write!(
                formatter,
                "local journal segment is corrupt at offset {offset}: {cause}"
            ),
            Self::SegmentDeviceMismatch {
                offset,
                expected,
                found,
            } => write!(
                formatter,
                "local journal frame at offset {offset} belongs to device {found}, not {expected}"
            ),
            Self::SegmentSequenceGap {
                offset,
                expected,
                found,
            } => write!(
                formatter,
                "local journal sequence gap at offset {offset}: expected {expected}, found {found}"
            ),
            Self::SequenceExhausted => {
                formatter.write_str("local journal device sequence is exhausted")
            }
            Self::SegmentPoisoned => {
                formatter.write_str("local journal segment must be reopened after a failed append")
            }
        }
    }
}

impl std::error::Error for LocalJournalError {}

impl From<io::Error> for LocalJournalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// The persistent typed header of one frame.
///
/// Field order and representations are persistent. `payload_digest` binds the
/// payload bytes independently of the whole-frame checksum, so a decoded frame
/// proves its payload both physically (checksum) and by content identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalJournalFrameHeader<K> {
    frame_schema_version: u32,
    device_id: Uuid,
    sequence: u64,
    payload_kind: K,
    payload_digest: ContentDigest,
}

/// One decoded journal frame and its typed payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalJournalFrame<K> {
    device_id: Uuid,
    sequence: u64,
    payload_kind: K,
    payload: Vec<u8>,
}

impl<K: LocalJournalPayloadKind> LocalJournalFrame<K> {
    pub const fn new(device_id: Uuid, sequence: u64, payload_kind: K, payload: Vec<u8>) -> Self {
        Self {
            device_id,
            sequence,
            payload_kind,
            payload,
        }
    }

    pub const fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn payload_kind(&self) -> K {
        self.payload_kind
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }

    pub fn payload_digest(&self) -> ContentDigest {
        ContentDigest::of(&self.payload)
    }

    pub fn encode(&self) -> Result<Vec<u8>, LocalJournalError> {
        encode_frame(
            self.device_id,
            self.sequence,
            self.payload_kind,
            &self.payload,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, LocalJournalError> {
        if bytes.len() > MAX_LOCAL_JOURNAL_FRAME_BYTES {
            return Err(LocalJournalError::FrameTooLarge(bytes.len()));
        }
        if bytes.len() < MIN_FRAME_BYTES {
            return Err(LocalJournalError::TruncatedFrame);
        }
        let prefix: [u8; FRAME_PREFIX_BYTES] = bytes[..FRAME_PREFIX_BYTES]
            .try_into()
            .expect("a checked prefix slice");
        let extent = FrameExtent::parse(&prefix)?;
        if extent.total != bytes.len() {
            return Err(LocalJournalError::FrameLengthMismatch {
                expected: extent.total,
                actual: bytes.len(),
            });
        }
        let body = extent.total - FRAME_CHECKSUM_BYTES;
        if bytes[body..] != ContentDigest::of(&bytes[..body]).as_bytes()[..] {
            return Err(LocalJournalError::ChecksumMismatch);
        }
        let header_bytes = &bytes[FRAME_PREFIX_BYTES..FRAME_PREFIX_BYTES + extent.header_len];
        let header: LocalJournalFrameHeader<K> = postcard::from_bytes(header_bytes)
            .map_err(|error| LocalJournalError::Decode(error.to_string()))?;
        if header.frame_schema_version != LOCAL_JOURNAL_FRAME_SCHEMA_VERSION {
            return Err(LocalJournalError::UnknownFrameSchemaVersion {
                expected: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION,
                found: header.frame_schema_version,
            });
        }
        let canonical_header = postcard::to_allocvec(&header)
            .map_err(|error| LocalJournalError::Encode(error.to_string()))?;
        if canonical_header.as_slice() != header_bytes {
            return Err(LocalJournalError::NonCanonicalFrameHeader);
        }
        let payload = &bytes[FRAME_PREFIX_BYTES + extent.header_len..body];
        if ContentDigest::of(payload) != header.payload_digest {
            return Err(LocalJournalError::PayloadDigestMismatch);
        }
        Ok(Self {
            device_id: header.device_id,
            sequence: header.sequence,
            payload_kind: header.payload_kind,
            payload: payload.to_vec(),
        })
    }
}

/// The declared byte extent of a frame, read from its fixed-size prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameExtent {
    header_len: usize,
    total: usize,
}

impl FrameExtent {
    fn parse(prefix: &[u8; FRAME_PREFIX_BYTES]) -> Result<Self, LocalJournalError> {
        if &prefix[..FRAME_MAGIC.len()] != FRAME_MAGIC {
            return Err(LocalJournalError::InvalidFrameMagic);
        }
        let header_len = u32::from_be_bytes(
            prefix[FRAME_MAGIC.len()..FRAME_MAGIC.len() + 4]
                .try_into()
                .expect("fixed header length field"),
        ) as usize;
        if header_len > MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES {
            return Err(LocalJournalError::FrameHeaderTooLarge(header_len));
        }
        let payload_len = u64::from_be_bytes(
            prefix[FRAME_MAGIC.len() + 4..FRAME_PREFIX_BYTES]
                .try_into()
                .expect("fixed payload length field"),
        );
        let payload_len =
            usize::try_from(payload_len).map_err(|_| LocalJournalError::LengthOverflow)?;
        let total = FRAME_PREFIX_BYTES
            .checked_add(header_len)
            .and_then(|length| length.checked_add(payload_len))
            .and_then(|length| length.checked_add(FRAME_CHECKSUM_BYTES))
            .ok_or(LocalJournalError::LengthOverflow)?;
        if total > MAX_LOCAL_JOURNAL_FRAME_BYTES {
            return Err(LocalJournalError::FrameTooLarge(total));
        }
        Ok(Self { header_len, total })
    }
}

/// Encode one frame without owning its payload, so the append path copies the
/// payload exactly once (into the frame buffer it writes).
fn encode_frame<K: LocalJournalPayloadKind>(
    device_id: Uuid,
    sequence: u64,
    payload_kind: K,
    payload: &[u8],
) -> Result<Vec<u8>, LocalJournalError> {
    let header = LocalJournalFrameHeader {
        frame_schema_version: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION,
        device_id,
        sequence,
        payload_kind,
        payload_digest: ContentDigest::of(payload),
    };
    let header_bytes = postcard::to_allocvec(&header)
        .map_err(|error| LocalJournalError::Encode(error.to_string()))?;
    if header_bytes.len() > MAX_LOCAL_JOURNAL_FRAME_HEADER_BYTES {
        return Err(LocalJournalError::FrameHeaderTooLarge(header_bytes.len()));
    }
    let header_len = u32::try_from(header_bytes.len())
        .map_err(|_| LocalJournalError::FrameHeaderTooLarge(header_bytes.len()))?;
    let payload_len =
        u64::try_from(payload.len()).map_err(|_| LocalJournalError::LengthOverflow)?;
    let total = FRAME_PREFIX_BYTES
        .checked_add(header_bytes.len())
        .and_then(|length| length.checked_add(payload.len()))
        .and_then(|length| length.checked_add(FRAME_CHECKSUM_BYTES))
        .ok_or(LocalJournalError::LengthOverflow)?;
    if total > MAX_LOCAL_JOURNAL_FRAME_BYTES {
        return Err(LocalJournalError::FrameTooLarge(total));
    }
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(FRAME_MAGIC);
    bytes.extend_from_slice(&header_len.to_be_bytes());
    bytes.extend_from_slice(&payload_len.to_be_bytes());
    bytes.extend_from_slice(&header_bytes);
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(ContentDigest::of(&bytes).as_bytes());
    Ok(bytes)
}

/// Exact physical work a segment has performed since it was opened.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocalJournalStats {
    pub frames_appended: u64,
    pub bytes_appended: u64,
    /// `fdatasync`-class barriers. Steady-state commits pay exactly one each.
    pub data_durability_syncs: u64,
    /// Directory-entry barriers. Paid once, when the segment file is created.
    pub directory_durability_syncs: u64,
    /// Torn-tail truncations performed while opening.
    pub recovery_truncations: u64,
}

/// What one open found in an existing segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalJournalRecovery<K> {
    /// Complete canonical frames adopted from the segment.
    pub frames_recovered: u64,
    /// Bytes of a torn final append that were detected, ignored, and truncated.
    pub discarded_tail_bytes: u64,
    /// The last complete frame, retained so a caller can settle its own state
    /// without a second pass.
    pub last_frame: Option<LocalJournalFrame<K>>,
}

/// An append-only journal segment owned by exactly one device.
///
/// The segment holds an exclusive advisory lock on its file for its whole life,
/// so a second open of the same segment — in this process or another — is
/// refused instead of interleaving two append cursors.
pub struct LocalJournalSegment<K> {
    file: fs::File,
    name: String,
    device_id: Uuid,
    next_sequence: u64,
    committed_bytes: u64,
    poisoned: bool,
    stats: LocalJournalStats,
    payload_kind: PhantomData<fn() -> K>,
}

/// The durable outcome of one append.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalJournalAppend {
    pub sequence: u64,
    pub frame_bytes: u64,
    pub payload_digest: ContentDigest,
    /// Durability barriers this append performed. Always exactly one.
    pub data_durability_syncs: u64,
}

impl<K: LocalJournalPayloadKind> LocalJournalSegment<K> {
    /// Open (creating if absent) the segment named `name` under `dir` for
    /// `device_id`, adopting its complete frames and truncating a torn tail.
    pub fn open(
        dir: &Dir,
        name: &str,
        device_id: Uuid,
    ) -> Result<(Self, LocalJournalRecovery<K>), LocalJournalError> {
        require_safe_segment_name(name)?;
        let (file, created) = open_or_create_segment_file(dir, name)?;
        if !lock_exclusive_nonblocking(&file)? {
            return Err(LocalJournalError::SegmentAlreadyOpen(name.to_owned()));
        }
        if !file.metadata()?.is_file() {
            return Err(LocalJournalError::UnsafeSegmentName(name.to_owned()));
        }
        let mut stats = LocalJournalStats::default();
        if created {
            sync_dir_required(dir)?;
            stats.directory_durability_syncs += 1;
        }
        let scan = scan_segment::<K>(&file, device_id)?;
        if scan.discarded_tail_bytes > 0 {
            file.set_len(scan.committed_bytes)?;
            file.sync_data()?;
            stats.data_durability_syncs += 1;
            stats.recovery_truncations += 1;
        }
        let mut file = file;
        file.seek(SeekFrom::Start(scan.committed_bytes))?;
        let segment = Self {
            file,
            name: name.to_owned(),
            device_id,
            next_sequence: scan.next_sequence,
            committed_bytes: scan.committed_bytes,
            poisoned: false,
            stats,
            payload_kind: PhantomData,
        };
        let recovery = LocalJournalRecovery {
            frames_recovered: scan.frames_recovered,
            discarded_tail_bytes: scan.discarded_tail_bytes,
            last_frame: scan.last_frame,
        };
        Ok((segment, recovery))
    }

    pub const fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub const fn committed_bytes(&self) -> u64 {
        self.committed_bytes
    }

    pub const fn stats(&self) -> LocalJournalStats {
        self.stats
    }

    /// Append one frame and make it durable.
    ///
    /// Exactly one write and one data-durability barrier. On return the frame is
    /// guaranteed to survive a restart; a caller may publish its own result.
    pub fn append(
        &mut self,
        payload_kind: K,
        payload: &[u8],
    ) -> Result<LocalJournalAppend, LocalJournalError> {
        if self.poisoned {
            return Err(LocalJournalError::SegmentPoisoned);
        }
        let sequence = self.next_sequence;
        let bytes = encode_frame(self.device_id, sequence, payload_kind, payload)?;
        let frame_bytes = bytes.len() as u64;
        let grown = self
            .committed_bytes
            .checked_add(frame_bytes)
            .filter(|length| *length <= MAX_LOCAL_JOURNAL_SEGMENT_BYTES)
            .ok_or(LocalJournalError::SegmentTooLarge(
                self.committed_bytes.saturating_add(frame_bytes),
            ))?;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(LocalJournalError::SequenceExhausted)?;
        // Any failure from here leaves the file length unproved, so the segment
        // refuses further appends rather than guessing its own cursor.
        if let Err(error) = self.file.write_all(&bytes) {
            self.poisoned = true;
            return Err(error.into());
        }
        if let Err(error) = self.file.sync_data() {
            self.poisoned = true;
            return Err(error.into());
        }
        self.next_sequence = next_sequence;
        self.committed_bytes = grown;
        self.stats.frames_appended += 1;
        self.stats.bytes_appended += frame_bytes;
        self.stats.data_durability_syncs += 1;
        Ok(LocalJournalAppend {
            sequence,
            frame_bytes,
            payload_digest: ContentDigest::of(payload),
            data_durability_syncs: 1,
        })
    }

    /// Stream every committed frame in append order.
    pub fn replay(
        &self,
        mut visit: impl FnMut(LocalJournalFrame<K>),
    ) -> Result<u64, LocalJournalError> {
        let mut reader =
            BufReader::with_capacity(SEGMENT_SCAN_BUFFER_BYTES, self.file.try_clone()?);
        reader.seek(SeekFrom::Start(0))?;
        let mut offset = 0_u64;
        let mut visited = 0_u64;
        let mut buffer = Vec::new();
        while offset < self.committed_bytes {
            let frame = read_frame_at(&mut reader, offset, self.committed_bytes, &mut buffer)?
                .into_complete(offset)?;
            offset += frame.encoded_len as u64;
            visited += 1;
            visit(frame.frame);
        }
        Ok(visited)
    }
}

impl<K> Drop for LocalJournalSegment<K> {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

struct SegmentScan<K> {
    committed_bytes: u64,
    frames_recovered: u64,
    next_sequence: u64,
    discarded_tail_bytes: u64,
    last_frame: Option<LocalJournalFrame<K>>,
}

/// One frame read attempt: either a complete frame or the torn tail boundary.
enum FrameRead<K> {
    Complete {
        frame: LocalJournalFrame<K>,
        encoded_len: usize,
    },
    TornTail,
}

struct CompleteFrame<K> {
    frame: LocalJournalFrame<K>,
    encoded_len: usize,
}

impl<K> FrameRead<K> {
    fn into_complete(self, offset: u64) -> Result<CompleteFrame<K>, LocalJournalError> {
        match self {
            Self::Complete { frame, encoded_len } => Ok(CompleteFrame { frame, encoded_len }),
            Self::TornTail => Err(LocalJournalError::CorruptSegment {
                offset,
                cause: "a committed frame is incomplete".to_owned(),
            }),
        }
    }
}

/// Read the frame that starts at `offset`, classifying failure as a torn tail
/// only when the failure cannot be explained by anything except an interrupted
/// final append.
fn read_frame_at<K: LocalJournalPayloadKind>(
    reader: &mut BufReader<fs::File>,
    offset: u64,
    file_len: u64,
    buffer: &mut Vec<u8>,
) -> Result<FrameRead<K>, LocalJournalError> {
    let remaining = file_len - offset;
    if remaining < MIN_FRAME_BYTES as u64 {
        // Fewer bytes than the smallest possible frame: the append that wrote
        // them never completed.
        return Ok(FrameRead::TornTail);
    }
    let mut prefix = [0_u8; FRAME_PREFIX_BYTES];
    reader.read_exact(&mut prefix)?;
    // The magic and both length fields are the first bytes of the append
    // buffer. An append that reached this far wrote them from that buffer, so a
    // malformed prefix is damage to bytes that were once complete.
    let extent =
        FrameExtent::parse(&prefix).map_err(|cause| LocalJournalError::CorruptSegment {
            offset,
            cause: cause.to_string(),
        })?;
    if (extent.total as u64) > remaining {
        return Ok(FrameRead::TornTail);
    }
    buffer.clear();
    buffer.extend_from_slice(&prefix);
    buffer.resize(extent.total, 0);
    reader.read_exact(&mut buffer[FRAME_PREFIX_BYTES..])?;
    match LocalJournalFrame::<K>::decode(buffer) {
        Ok(frame) => Ok(FrameRead::Complete {
            frame,
            encoded_len: extent.total,
        }),
        // The declared extent ends exactly at end of file: an interrupted append
        // can leave a fully sized but partly written final frame. Anything with
        // committed bytes after it was complete once, so it is corruption.
        Err(_) if offset + extent.total as u64 == file_len => Ok(FrameRead::TornTail),
        Err(cause) => Err(LocalJournalError::CorruptSegment {
            offset,
            cause: cause.to_string(),
        }),
    }
}

fn scan_segment<K: LocalJournalPayloadKind>(
    file: &fs::File,
    device_id: Uuid,
) -> Result<SegmentScan<K>, LocalJournalError> {
    let file_len = file.metadata()?.len();
    if file_len > MAX_LOCAL_JOURNAL_SEGMENT_BYTES {
        return Err(LocalJournalError::SegmentTooLarge(file_len));
    }
    let mut reader = BufReader::with_capacity(SEGMENT_SCAN_BUFFER_BYTES, file.try_clone()?);
    reader.seek(SeekFrom::Start(0))?;
    let mut offset = 0_u64;
    let mut frames_recovered = 0_u64;
    let mut next_sequence = 0_u64;
    let mut last_frame = None;
    let mut buffer = Vec::new();
    while offset < file_len {
        let frame = match read_frame_at::<K>(&mut reader, offset, file_len, &mut buffer)? {
            FrameRead::Complete { frame, encoded_len } => {
                if frame.device_id() != device_id {
                    return Err(LocalJournalError::SegmentDeviceMismatch {
                        offset,
                        expected: device_id,
                        found: frame.device_id(),
                    });
                }
                if frame.sequence() != next_sequence {
                    return Err(LocalJournalError::SegmentSequenceGap {
                        offset,
                        expected: next_sequence,
                        found: frame.sequence(),
                    });
                }
                offset += encoded_len as u64;
                frames_recovered += 1;
                next_sequence = frame
                    .sequence()
                    .checked_add(1)
                    .ok_or(LocalJournalError::SequenceExhausted)?;
                frame
            }
            FrameRead::TornTail => break,
        };
        last_frame = Some(frame);
    }
    Ok(SegmentScan {
        committed_bytes: offset,
        frames_recovered,
        next_sequence,
        discarded_tail_bytes: file_len - offset,
        last_frame,
    })
}

fn require_safe_segment_name(name: &str) -> Result<(), LocalJournalError> {
    let unsafe_name = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0');
    if unsafe_name {
        return Err(LocalJournalError::UnsafeSegmentName(name.to_owned()));
    }
    Ok(())
}

/// Open the segment file, creating it if absent. Returns whether this call
/// created it, so only a creating open pays a directory-entry barrier.
fn open_or_create_segment_file(
    dir: &Dir,
    name: &str,
) -> Result<(fs::File, bool), LocalJournalError> {
    for _ in 0..SEGMENT_OPEN_ATTEMPTS {
        match open_regular_read_write_nofollow(dir, name) {
            Ok(file) => return Ok((file, false)),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        match dir.open_with(name, &options) {
            Ok(file) => return Ok((file.into_std(), true)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(LocalJournalError::Io(format!(
        "local journal segment {name} could not be opened or created"
    )))
}

#[cfg(unix)]
fn open_regular_read_write_nofollow(dir: &Dir, name: &str) -> io::Result<fs::File> {
    use std::ffi::CString;
    use std::os::fd::AsFd as _;

    let path = CString::new(name)
        .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid segment name"))?;
    // SAFETY: `path` is a live NUL-terminated string and `dir` is an opened
    // directory capability. O_NOFOLLOW binds the open to a real entry.
    let descriptor = unsafe {
        libc::openat(
            dir.as_fd().as_raw_fd(),
            path.as_ptr(),
            libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `openat` returned one newly owned descriptor.
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

#[cfg(windows)]
fn open_regular_read_write_nofollow(dir: &Dir, name: &str) -> io::Result<fs::File> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};

    let mut options = OpenOptions::new();
    options.read(true).write(true);
    options.follow(FollowSymlinks::No);
    Ok(dir.open_with(name, &options)?.into_std())
}

#[cfg(not(any(unix, windows)))]
fn open_regular_read_write_nofollow(_dir: &Dir, _name: &str) -> io::Result<fs::File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "no-follow journal opens are unsupported on this target",
    ))
}

#[cfg(unix)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, LocalJournalError> {
    // SAFETY: `file` owns a live descriptor for the duration of the call.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(error.into())
}

#[cfg(unix)]
fn unlock(file: &fs::File) {
    // SAFETY: `file` owns a live descriptor for the duration of the call.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(windows)]
fn lock_exclusive_nonblocking(file: &fs::File) -> Result<bool, LocalJournalError> {
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
    let error = io::Error::last_os_error();
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

#[cfg(not(any(unix, windows)))]
fn lock_exclusive_nonblocking(_file: &fs::File) -> Result<bool, LocalJournalError> {
    Err(LocalJournalError::Io(
        "exclusive journal segment locking is unsupported on this target".to_owned(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock(_file: &fs::File) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    enum TestKind {
        Effect,
        Update,
    }

    struct Fixture {
        root: std::path::PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tine-local-journal-{label}-{}",
                Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&root).unwrap();
            let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            Self { root, dir }
        }

        fn segment_bytes(&self, name: &str) -> Vec<u8> {
            fs::read(self.root.join(name)).unwrap()
        }

        fn write_segment_bytes(&self, name: &str, bytes: &[u8]) {
            fs::write(self.root.join(name), bytes).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn open(
        fixture: &Fixture,
        device: Uuid,
    ) -> (
        LocalJournalSegment<TestKind>,
        LocalJournalRecovery<TestKind>,
    ) {
        LocalJournalSegment::open(&fixture.dir, "device.journal", device).unwrap()
    }

    fn replayed(segment: &LocalJournalSegment<TestKind>) -> Vec<LocalJournalFrame<TestKind>> {
        let mut frames = Vec::new();
        segment.replay(|frame| frames.push(frame)).unwrap();
        frames
    }

    #[test]
    fn a_decoded_frame_reproduces_the_encoded_typed_payload() {
        let device = Uuid::from_u128(0x9e11);
        for (kind, payload) in [
            (TestKind::Effect, Vec::new()),
            (TestKind::Effect, b"semantic-effect-bytes".to_vec()),
            (TestKind::Update, (0..=255_u8).cycle().take(9_973).collect()),
        ] {
            let frame = LocalJournalFrame::new(device, 7, kind, payload.clone());
            let encoded = frame.encode().unwrap();
            let decoded = LocalJournalFrame::<TestKind>::decode(&encoded).unwrap();
            assert_eq!(decoded, frame);
            assert_eq!(decoded.payload(), payload.as_slice());
            assert_eq!(decoded.payload_kind(), kind);
            assert_eq!(decoded.sequence(), 7);
            assert_eq!(decoded.device_id(), device);
            assert_eq!(decoded.payload_digest(), ContentDigest::of(&payload));
            // Re-encoding a decoded frame is byte-identical: the frame is canonical.
            assert_eq!(decoded.encode().unwrap(), encoded);
        }
    }

    #[test]
    fn every_single_byte_corruption_of_a_frame_is_refused() {
        let frame = LocalJournalFrame::new(
            Uuid::from_u128(0x1),
            0,
            TestKind::Effect,
            b"payload".to_vec(),
        );
        let encoded = frame.encode().unwrap();
        for index in 0..encoded.len() {
            let mut corrupt = encoded.clone();
            corrupt[index] ^= 0x01;
            assert!(
                LocalJournalFrame::<TestKind>::decode(&corrupt).is_err(),
                "flipping byte {index} must be refused"
            );
        }
    }

    #[test]
    fn every_truncation_of_a_frame_is_refused() {
        let frame = LocalJournalFrame::new(
            Uuid::from_u128(0x2),
            0,
            TestKind::Update,
            b"payload-bytes".to_vec(),
        );
        let encoded = frame.encode().unwrap();
        for length in 0..encoded.len() {
            assert!(
                LocalJournalFrame::<TestKind>::decode(&encoded[..length]).is_err(),
                "truncating to {length} bytes must be refused"
            );
        }
    }

    #[test]
    fn a_frame_from_an_unknown_schema_version_is_refused() {
        #[derive(Serialize)]
        struct FutureHeader {
            frame_schema_version: u32,
            device_id: Uuid,
            sequence: u64,
            payload_kind: TestKind,
            payload_digest: ContentDigest,
        }

        let payload = b"future".to_vec();
        let header = FutureHeader {
            frame_schema_version: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION + 1,
            device_id: Uuid::from_u128(0x3),
            sequence: 0,
            payload_kind: TestKind::Effect,
            payload_digest: ContentDigest::of(&payload),
        };
        let header_bytes = postcard::to_allocvec(&header).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(FRAME_MAGIC);
        bytes.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&header_bytes);
        bytes.extend_from_slice(&payload);
        bytes.extend_from_slice(ContentDigest::of(&bytes).as_bytes());

        assert_eq!(
            LocalJournalFrame::<TestKind>::decode(&bytes),
            Err(LocalJournalError::UnknownFrameSchemaVersion {
                expected: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION,
                found: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION + 1,
            })
        );
    }

    #[test]
    fn a_frame_whose_payload_digest_disagrees_with_its_payload_is_refused() {
        let device = Uuid::from_u128(0x4);
        let header = LocalJournalFrameHeader {
            frame_schema_version: LOCAL_JOURNAL_FRAME_SCHEMA_VERSION,
            device_id: device,
            sequence: 0,
            payload_kind: TestKind::Effect,
            payload_digest: ContentDigest::of(b"a different payload"),
        };
        let header_bytes = postcard::to_allocvec(&header).unwrap();
        let payload = b"the actual payload";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(FRAME_MAGIC);
        bytes.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&header_bytes);
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(ContentDigest::of(&bytes).as_bytes());

        assert_eq!(
            LocalJournalFrame::<TestKind>::decode(&bytes),
            Err(LocalJournalError::PayloadDigestMismatch)
        );
    }

    #[test]
    fn one_append_performs_exactly_one_durability_barrier() {
        let fixture = Fixture::new("durability");
        let device = Uuid::from_u128(0x5);
        let (mut segment, recovery) = open(&fixture, device);
        assert_eq!(recovery.frames_recovered, 0);
        assert_eq!(recovery.discarded_tail_bytes, 0);
        assert_eq!(recovery.last_frame, None);
        // Creating the segment pays one directory barrier, and only one.
        assert_eq!(segment.stats().directory_durability_syncs, 1);
        assert_eq!(segment.stats().data_durability_syncs, 0);

        for index in 0..5_u64 {
            let appended = segment
                .append(TestKind::Effect, format!("effect-{index}").as_bytes())
                .unwrap();
            assert_eq!(appended.sequence, index);
            assert_eq!(appended.data_durability_syncs, 1);
        }
        let stats = segment.stats();
        assert_eq!(stats.frames_appended, 5);
        assert_eq!(stats.data_durability_syncs, 5);
        assert_eq!(stats.directory_durability_syncs, 1);
        assert_eq!(stats.recovery_truncations, 0);
        assert_eq!(stats.bytes_appended, segment.committed_bytes());
    }

    #[test]
    fn a_completed_append_survives_a_restart() {
        let fixture = Fixture::new("restart");
        let device = Uuid::from_u128(0x6);
        let payloads: Vec<Vec<u8>> = (0..4).map(|index| vec![index as u8; 32 + index]).collect();
        {
            let (mut segment, _) = open(&fixture, device);
            for payload in &payloads {
                segment.append(TestKind::Update, payload).unwrap();
            }
        }
        let (segment, recovery) = open(&fixture, device);
        assert_eq!(recovery.frames_recovered, 4);
        assert_eq!(recovery.discarded_tail_bytes, 0);
        assert_eq!(recovery.last_frame.as_ref().unwrap().sequence(), 3);
        assert_eq!(recovery.last_frame.unwrap().payload(), payloads[3]);
        assert_eq!(segment.next_sequence(), 4);
        assert_eq!(segment.stats().recovery_truncations, 0);
        let frames = replayed(&segment);
        assert_eq!(frames.len(), 4);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame.sequence(), index as u64);
            assert_eq!(frame.payload(), payloads[index]);
            assert_eq!(frame.payload_kind(), TestKind::Update);
        }
    }

    #[test]
    fn a_partial_tail_at_every_byte_boundary_keeps_every_prior_complete_frame() {
        let fixture = Fixture::new("torn-tail");
        let device = Uuid::from_u128(0x7);
        let (complete, prefix_len, final_len) = {
            let (mut segment, _) = open(&fixture, device);
            for index in 0..3_u64 {
                segment
                    .append(TestKind::Effect, format!("kept-{index}").as_bytes())
                    .unwrap();
            }
            let prefix_len = segment.committed_bytes() as usize;
            let appended = segment.append(TestKind::Update, b"torn").unwrap();
            (
                fixture.segment_bytes("device.journal"),
                prefix_len,
                appended.frame_bytes as usize,
            )
        };
        assert_eq!(complete.len(), prefix_len + final_len);

        // Every truncation that cuts into the final append: the three complete
        // frames survive and the torn bytes are discarded.
        for torn in 0..final_len {
            fixture.write_segment_bytes("device.journal", &complete[..prefix_len + torn]);
            let (mut segment, recovery) = open(&fixture, device);
            assert_eq!(
                recovery.frames_recovered, 3,
                "truncating the final frame to {torn} bytes must keep three frames"
            );
            assert_eq!(recovery.discarded_tail_bytes, torn as u64);
            assert_eq!(segment.next_sequence(), 3);
            assert_eq!(segment.committed_bytes(), prefix_len as u64);
            assert_eq!(
                segment.stats().recovery_truncations,
                u64::from(torn > 0),
                "an intact segment must not be truncated"
            );
            let frames = replayed(&segment);
            assert_eq!(frames.len(), 3);
            assert_eq!(frames[2].payload(), b"kept-2");
            // The recovered segment is immediately appendable at the right
            // sequence, and the new frame survives its own reopen.
            let appended = segment.append(TestKind::Effect, b"after-recovery").unwrap();
            assert_eq!(appended.sequence, 3);
            drop(segment);
            let (segment, recovery) = open(&fixture, device);
            assert_eq!(recovery.frames_recovered, 4);
            assert_eq!(recovery.last_frame.unwrap().payload(), b"after-recovery");
            assert_eq!(replayed(&segment).len(), 4);
        }
    }

    #[test]
    fn a_fully_sized_but_damaged_final_frame_is_treated_as_a_torn_tail() {
        let fixture = Fixture::new("damaged-tail");
        let device = Uuid::from_u128(0x8);
        let (complete, prefix_len) = {
            let (mut segment, _) = open(&fixture, device);
            segment.append(TestKind::Effect, b"kept").unwrap();
            let prefix_len = segment.committed_bytes() as usize;
            segment.append(TestKind::Update, b"damaged").unwrap();
            (fixture.segment_bytes("device.journal"), prefix_len)
        };
        // Damage a payload byte of the final frame without changing its length.
        let mut damaged = complete.clone();
        let last = damaged.len() - FRAME_CHECKSUM_BYTES - 1;
        damaged[last] ^= 0xff;
        fixture.write_segment_bytes("device.journal", &damaged);

        let (segment, recovery) = open(&fixture, device);
        assert_eq!(recovery.frames_recovered, 1);
        assert_eq!(
            recovery.discarded_tail_bytes,
            (complete.len() - prefix_len) as u64
        );
        assert_eq!(recovery.last_frame.unwrap().payload(), b"kept");
        assert_eq!(segment.committed_bytes(), prefix_len as u64);
        assert_eq!(segment.stats().recovery_truncations, 1);
    }

    #[test]
    fn damage_to_a_frame_that_committed_bytes_follow_is_refused_as_corruption() {
        let fixture = Fixture::new("corruption");
        let device = Uuid::from_u128(0x9);
        let (complete, first_len) = {
            let (mut segment, _) = open(&fixture, device);
            let first = segment.append(TestKind::Effect, b"first").unwrap();
            segment.append(TestKind::Effect, b"second").unwrap();
            segment.append(TestKind::Effect, b"third").unwrap();
            (
                fixture.segment_bytes("device.journal"),
                first.frame_bytes as usize,
            )
        };

        // A non-final frame that later bytes prove was written completely is
        // corruption, not a torn append: refuse instead of discarding silently.
        for index in [
            FRAME_MAGIC.len() - 1,
            FRAME_PREFIX_BYTES + 2,
            first_len - FRAME_CHECKSUM_BYTES - 1,
            first_len - 1,
        ] {
            let mut corrupt = complete.clone();
            corrupt[index] ^= 0xff;
            fixture.write_segment_bytes("device.journal", &corrupt);
            let opened =
                LocalJournalSegment::<TestKind>::open(&fixture.dir, "device.journal", device);
            assert!(
                matches!(
                    opened,
                    Err(LocalJournalError::CorruptSegment { offset: 0, .. })
                ),
                "damage at byte {index} must be refused as corruption, got {:?}",
                opened.err()
            );
        }
    }

    #[test]
    fn a_frame_from_another_device_or_out_of_sequence_is_refused() {
        let fixture = Fixture::new("identity");
        let device = Uuid::from_u128(0xa);
        let other = Uuid::from_u128(0xb);

        let foreign = LocalJournalFrame::new(other, 0, TestKind::Effect, b"foreign".to_vec());
        fixture.write_segment_bytes("device.journal", &foreign.encode().unwrap());
        assert!(matches!(
            LocalJournalSegment::<TestKind>::open(&fixture.dir, "device.journal", device),
            Err(LocalJournalError::SegmentDeviceMismatch { .. })
        ));

        let skipped = LocalJournalFrame::new(device, 4, TestKind::Effect, b"skipped".to_vec());
        fixture.write_segment_bytes("device.journal", &skipped.encode().unwrap());
        assert!(matches!(
            LocalJournalSegment::<TestKind>::open(&fixture.dir, "device.journal", device),
            Err(LocalJournalError::SegmentSequenceGap {
                offset: 0,
                expected: 0,
                found: 4,
            })
        ));
    }

    #[test]
    fn a_duplicate_open_is_refused_while_the_first_is_live() {
        let fixture = Fixture::new("duplicate-open");
        let device = Uuid::from_u128(0xc);
        let (mut segment, _) = open(&fixture, device);
        segment.append(TestKind::Effect, b"held").unwrap();

        assert!(matches!(
            LocalJournalSegment::<TestKind>::open(&fixture.dir, "device.journal", device),
            Err(LocalJournalError::SegmentAlreadyOpen(_))
        ));

        drop(segment);
        let (segment, recovery) = open(&fixture, device);
        assert_eq!(recovery.frames_recovered, 1);
        assert_eq!(replayed(&segment).len(), 1);
    }

    #[test]
    fn an_unsafe_segment_name_is_refused_before_any_filesystem_work() {
        let fixture = Fixture::new("unsafe-name");
        for name in ["", ".", "..", "nested/name", "back\\slash"] {
            assert!(matches!(
                LocalJournalSegment::<TestKind>::open(&fixture.dir, name, Uuid::from_u128(0xd)),
                Err(LocalJournalError::UnsafeSegmentName(_))
            ));
        }
        assert!(fs::read_dir(&fixture.root).unwrap().next().is_none());
    }
}
