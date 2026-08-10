//! Durable-publication local journal protocol v2.
//!
//! Frames retain the schema-1 codec. A fixed segment header gives each
//! physical generation an identity, while a separately and atomically
//! replaced frontier is the sole authority for the committed byte prefix.

use std::fmt;
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;

use cap_std::fs::{Dir, OpenOptions};
use uuid::Uuid;

use crate::local_journal::{
    encode_frame, lock_exclusive_nonblocking, open_regular_read_write_nofollow,
    require_safe_segment_name, unlock, LocalJournalAppend, LocalJournalError, LocalJournalFrame,
    LocalJournalPayloadKind, LocalJournalRecovery, LocalJournalStats,
    MAX_LOCAL_JOURNAL_FRAME_BYTES, MAX_LOCAL_JOURNAL_SEGMENT_BYTES, MIN_FRAME_BYTES,
};
use crate::{
    publish_immutable_exact, read_required_regular, sync_dir_required, ContentDigest,
    FilesystemError,
};

pub const LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION: u32 = 2;
pub const LOCAL_JOURNAL_SEGMENT_V2_MAGIC: &str = "TINEJNL2";
pub const LOCAL_JOURNAL_FRONTIER_V2_MAGIC: &str = "TINEFRT2";
pub const LOCAL_JOURNAL_SEGMENT_HEADER_BYTES: usize = 136;
pub const LOCAL_JOURNAL_FRONTIER_BYTES: usize = 240;
pub const LOCAL_JOURNAL_FRONTIER_SUFFIX: &str = ".frontier-v2";

const SEGMENT_MAGIC: &[u8; 8] = b"TINEJNL2";
const FRONTIER_MAGIC: &[u8; 8] = b"TINEFRT2";
const CHECKSUM_BYTES: usize = 32;
const HEADER_CHECKSUM_OFFSET: usize = LOCAL_JOURNAL_SEGMENT_HEADER_BYTES - CHECKSUM_BYTES;
const FRONTIER_CHECKSUM_OFFSET: usize = LOCAL_JOURNAL_FRONTIER_BYTES - CHECKSUM_BYTES;
const SCAN_BUFFER_BYTES: usize = 64 * 1024;

/// Complete selector later persisted by Tine's schema-2 authority anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalJournalSegmentV2Selection {
    segment_name: String,
    frontier_name: String,
    segment_id: Uuid,
    segment_name_digest: ContentDigest,
    device_id: Uuid,
    base_sequence: u64,
}

impl LocalJournalSegmentV2Selection {
    pub fn new(
        segment_name: impl Into<String>,
        segment_id: Uuid,
        device_id: Uuid,
        base_sequence: u64,
    ) -> Result<Self, LocalJournalError> {
        let segment_name = segment_name.into();
        require_safe_segment_name(&segment_name)?;
        let frontier_name = format!("{segment_name}{LOCAL_JOURNAL_FRONTIER_SUFFIX}");
        require_safe_segment_name(&frontier_name)?;
        Ok(Self {
            segment_name_digest: ContentDigest::of(segment_name.as_bytes()),
            segment_name,
            frontier_name,
            segment_id,
            device_id,
            base_sequence,
        })
    }

    pub fn random(
        segment_name: impl Into<String>,
        device_id: Uuid,
        base_sequence: u64,
    ) -> Result<Self, LocalJournalError> {
        Self::new(segment_name, Uuid::new_v4(), device_id, base_sequence)
    }

    pub fn segment_name(&self) -> &str {
        &self.segment_name
    }

    pub fn frontier_name(&self) -> &str {
        &self.frontier_name
    }

    pub const fn segment_id(&self) -> Uuid {
        self.segment_id
    }

    pub const fn segment_name_digest(&self) -> ContentDigest {
        self.segment_name_digest
    }

    pub const fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub const fn base_sequence(&self) -> u64 {
        self.base_sequence
    }
}

/// Typed certainty boundary for append failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalJournalAppendError {
    DefinitelyNotAppended(LocalJournalError),
    AppendOutcomeUnknown(LocalJournalError),
}

impl LocalJournalAppendError {
    pub const fn outcome_is_unknown(&self) -> bool {
        matches!(self, Self::AppendOutcomeUnknown(_))
    }

    pub const fn cause(&self) -> &LocalJournalError {
        match self {
            Self::DefinitelyNotAppended(error) | Self::AppendOutcomeUnknown(error) => error,
        }
    }
}

impl fmt::Display for LocalJournalAppendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefinitelyNotAppended(error) => {
                write!(formatter, "append definitely did not start: {error}")
            }
            Self::AppendOutcomeUnknown(error) => {
                write!(formatter, "append outcome is unknown until reopen: {error}")
            }
        }
    }
}

impl std::error::Error for LocalJournalAppendError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SegmentHeaderV2 {
    segment_id: Uuid,
    segment_name_digest: ContentDigest,
    device_id: Uuid,
    base_sequence: u64,
}

impl SegmentHeaderV2 {
    fn for_selection(selection: &LocalJournalSegmentV2Selection) -> Self {
        Self {
            segment_id: selection.segment_id,
            segment_name_digest: selection.segment_name_digest,
            device_id: selection.device_id,
            base_sequence: selection.base_sequence,
        }
    }

    fn encode(&self) -> [u8; LOCAL_JOURNAL_SEGMENT_HEADER_BYTES] {
        let mut bytes = [0_u8; LOCAL_JOURNAL_SEGMENT_HEADER_BYTES];
        bytes[..8].copy_from_slice(SEGMENT_MAGIC);
        bytes[8..12].copy_from_slice(&LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION.to_be_bytes());
        bytes[16..32].copy_from_slice(self.segment_id.as_bytes());
        bytes[32..64].copy_from_slice(self.segment_name_digest.as_bytes());
        bytes[64..80].copy_from_slice(self.device_id.as_bytes());
        bytes[80..88].copy_from_slice(&self.base_sequence.to_be_bytes());
        let digest = ContentDigest::of(&bytes[..HEADER_CHECKSUM_OFFSET]);
        bytes[HEADER_CHECKSUM_OFFSET..].copy_from_slice(digest.as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, LocalJournalError> {
        if bytes.len() != LOCAL_JOURNAL_SEGMENT_HEADER_BYTES {
            return corrupt(0, "segment header has a noncanonical length");
        }
        if &bytes[..8] != SEGMENT_MAGIC {
            return corrupt(0, "segment header magic is invalid");
        }
        let version = u32::from_be_bytes(bytes[8..12].try_into().expect("fixed version"));
        if version != LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION {
            return corrupt(0, "segment protocol version is unsupported");
        }
        if bytes[12..16].iter().any(|byte| *byte != 0)
            || bytes[88..HEADER_CHECKSUM_OFFSET]
                .iter()
                .any(|byte| *byte != 0)
        {
            return corrupt(0, "segment header reserved bytes are nonzero");
        }
        if bytes[HEADER_CHECKSUM_OFFSET..]
            != ContentDigest::of(&bytes[..HEADER_CHECKSUM_OFFSET]).as_bytes()[..]
        {
            return corrupt(0, "segment header checksum mismatch");
        }
        Ok(Self {
            segment_id: Uuid::from_bytes(bytes[16..32].try_into().expect("fixed UUID")),
            segment_name_digest: ContentDigest::from_bytes(
                bytes[32..64].try_into().expect("fixed digest"),
            ),
            device_id: Uuid::from_bytes(bytes[64..80].try_into().expect("fixed UUID")),
            base_sequence: u64::from_be_bytes(bytes[80..88].try_into().expect("fixed base")),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FrontierV2 {
    header: SegmentHeaderV2,
    next_sequence: u64,
    committed_extent: u64,
    terminal: Option<TerminalFrame>,
    predecessor_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalFrame {
    sequence: u64,
    encoded_len: u64,
    frame_digest: ContentDigest,
}

impl FrontierV2 {
    fn initial(header: SegmentHeaderV2) -> Self {
        let base = header.base_sequence;
        Self {
            header,
            next_sequence: base,
            committed_extent: LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64,
            terminal: None,
            predecessor_digest: ContentDigest::from_bytes([0; 32]),
        }
    }

    fn successor(&self, frame: &[u8]) -> Result<Self, LocalJournalError> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(LocalJournalError::SequenceExhausted)?;
        let encoded_len = frame.len() as u64;
        let committed_extent = self
            .committed_extent
            .checked_add(encoded_len)
            .filter(|extent| *extent <= MAX_LOCAL_JOURNAL_SEGMENT_BYTES)
            .ok_or(LocalJournalError::SegmentTooLarge(
                self.committed_extent.saturating_add(encoded_len),
            ))?;
        Ok(Self {
            header: self.header.clone(),
            next_sequence,
            committed_extent,
            terminal: Some(TerminalFrame {
                sequence,
                encoded_len,
                frame_digest: ContentDigest::of(frame),
            }),
            predecessor_digest: ContentDigest::of(&self.encode()),
        })
    }

    fn encode(&self) -> [u8; LOCAL_JOURNAL_FRONTIER_BYTES] {
        let mut bytes = [0_u8; LOCAL_JOURNAL_FRONTIER_BYTES];
        bytes[..8].copy_from_slice(FRONTIER_MAGIC);
        bytes[8..12].copy_from_slice(&LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION.to_be_bytes());
        bytes[16..32].copy_from_slice(self.header.segment_id.as_bytes());
        bytes[32..64].copy_from_slice(self.header.segment_name_digest.as_bytes());
        bytes[64..80].copy_from_slice(self.header.device_id.as_bytes());
        bytes[80..88].copy_from_slice(&self.header.base_sequence.to_be_bytes());
        bytes[88..96].copy_from_slice(&self.next_sequence.to_be_bytes());
        bytes[96..104].copy_from_slice(&self.committed_extent.to_be_bytes());
        if let Some(terminal) = &self.terminal {
            bytes[104] = 1;
            bytes[112..120].copy_from_slice(&terminal.sequence.to_be_bytes());
            bytes[120..128].copy_from_slice(&terminal.encoded_len.to_be_bytes());
            bytes[128..160].copy_from_slice(terminal.frame_digest.as_bytes());
        }
        bytes[160..192].copy_from_slice(self.predecessor_digest.as_bytes());
        let digest = ContentDigest::of(&bytes[..FRONTIER_CHECKSUM_OFFSET]);
        bytes[FRONTIER_CHECKSUM_OFFSET..].copy_from_slice(digest.as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, LocalJournalError> {
        if bytes.len() != LOCAL_JOURNAL_FRONTIER_BYTES {
            return corrupt(0, "frontier has a noncanonical length");
        }
        if &bytes[..8] != FRONTIER_MAGIC {
            return corrupt(0, "frontier magic is invalid");
        }
        let version = u32::from_be_bytes(bytes[8..12].try_into().expect("fixed version"));
        if version != LOCAL_JOURNAL_SEGMENT_PROTOCOL_VERSION {
            return corrupt(0, "frontier protocol version is unsupported");
        }
        if bytes[12..16].iter().any(|byte| *byte != 0)
            || bytes[105..112].iter().any(|byte| *byte != 0)
            || bytes[192..FRONTIER_CHECKSUM_OFFSET]
                .iter()
                .any(|byte| *byte != 0)
        {
            return corrupt(0, "frontier reserved bytes are nonzero");
        }
        if bytes[FRONTIER_CHECKSUM_OFFSET..]
            != ContentDigest::of(&bytes[..FRONTIER_CHECKSUM_OFFSET]).as_bytes()[..]
        {
            return corrupt(0, "frontier checksum mismatch");
        }
        let header = SegmentHeaderV2 {
            segment_id: Uuid::from_bytes(bytes[16..32].try_into().expect("fixed UUID")),
            segment_name_digest: ContentDigest::from_bytes(
                bytes[32..64].try_into().expect("fixed digest"),
            ),
            device_id: Uuid::from_bytes(bytes[64..80].try_into().expect("fixed UUID")),
            base_sequence: u64::from_be_bytes(bytes[80..88].try_into().expect("fixed base")),
        };
        let next_sequence = u64::from_be_bytes(bytes[88..96].try_into().expect("fixed next"));
        let committed_extent = u64::from_be_bytes(bytes[96..104].try_into().expect("fixed extent"));
        let terminal = match bytes[104] {
            0 => {
                if bytes[112..160].iter().any(|byte| *byte != 0) {
                    return corrupt(0, "empty frontier has terminal frame fields");
                }
                None
            }
            1 => Some(TerminalFrame {
                sequence: u64::from_be_bytes(bytes[112..120].try_into().expect("fixed sequence")),
                encoded_len: u64::from_be_bytes(bytes[120..128].try_into().expect("fixed length")),
                frame_digest: ContentDigest::from_bytes(
                    bytes[128..160].try_into().expect("fixed digest"),
                ),
            }),
            _ => return corrupt(0, "frontier terminal tag is invalid"),
        };
        Ok(Self {
            header,
            next_sequence,
            committed_extent,
            terminal,
            predecessor_digest: ContentDigest::from_bytes(
                bytes[160..192].try_into().expect("fixed digest"),
            ),
        })
    }
}

/// A locked v2 segment whose append cursor is selected solely by its frontier.
pub struct LocalJournalSegmentV2<K> {
    dir: Dir,
    file: fs::File,
    selection: LocalJournalSegmentV2Selection,
    frontier: FrontierV2,
    poisoned: bool,
    stats: LocalJournalStats,
    payload_kind: PhantomData<fn() -> K>,
}

impl<K: LocalJournalPayloadKind> LocalJournalSegmentV2<K> {
    /// Prepare a non-authoritative v2 segment/header pair. Tine may publish its
    /// schema-2 anchor only after this succeeds.
    pub fn prepare(
        dir: &Dir,
        selection: &LocalJournalSegmentV2Selection,
    ) -> Result<(), LocalJournalError> {
        #[cfg(windows)]
        return Err(LocalJournalError::UnsupportedDurableReplacement);

        #[cfg(not(windows))]
        {
            let header = SegmentHeaderV2::for_selection(selection);
            create_exact_durable(dir, selection.segment_name(), &header.encode())?;
            if let Err(error) = create_exact_durable(
                dir,
                selection.frontier_name(),
                &FrontierV2::initial(header).encode(),
            ) {
                return Err(error);
            }
            Ok(())
        }
    }

    pub fn open_selected(
        dir: &Dir,
        selection: &LocalJournalSegmentV2Selection,
    ) -> Result<(Self, LocalJournalRecovery<K>), LocalJournalError> {
        require_safe_segment_name(selection.segment_name())?;
        let mut file = open_regular_read_write_nofollow(dir, selection.segment_name())?;
        if !lock_exclusive_nonblocking(&file)? {
            return Err(LocalJournalError::SegmentAlreadyOpen(
                selection.segment_name().to_owned(),
            ));
        }
        if !file.metadata()?.is_file() {
            return Err(LocalJournalError::UnsafeSegmentName(
                selection.segment_name().to_owned(),
            ));
        }
        let header = read_header(&file)?;
        let expected_header = SegmentHeaderV2::for_selection(selection);
        if header != expected_header {
            return corrupt(0, "segment header does not match the selected generation");
        }
        let frontier_bytes = read_frontier_exact(dir, selection.frontier_name())?;
        let frontier = FrontierV2::decode(&frontier_bytes)?;
        if frontier.header != header {
            return corrupt(0, "frontier does not bind the selected segment header");
        }
        let scan = scan_v2_prefix::<K>(&file, &frontier)?;
        let file_len = file.metadata()?.len();
        let mut stats = LocalJournalStats::default();
        if file_len > frontier.committed_extent {
            file.set_len(frontier.committed_extent)?;
            file.sync_data()?;
            stats.data_durability_syncs = 1;
            stats.recovery_truncations = 1;
        }
        file.seek(SeekFrom::Start(frontier.committed_extent))?;
        Ok((
            Self {
                dir: dir.try_clone()?,
                file,
                selection: selection.clone(),
                frontier,
                poisoned: false,
                stats,
                payload_kind: PhantomData,
            },
            LocalJournalRecovery {
                frames_recovered: scan.frames_recovered,
                discarded_tail_bytes: file_len - scan.committed_extent,
                last_frame: scan.last_frame,
            },
        ))
    }

    pub const fn selection(&self) -> &LocalJournalSegmentV2Selection {
        &self.selection
    }

    pub const fn next_sequence(&self) -> u64 {
        self.frontier.next_sequence
    }

    pub const fn committed_bytes(&self) -> u64 {
        self.frontier.committed_extent
    }

    pub const fn stats(&self) -> LocalJournalStats {
        self.stats
    }

    pub fn append(
        &mut self,
        payload_kind: K,
        payload: &[u8],
    ) -> Result<LocalJournalAppend, LocalJournalAppendError> {
        if self.poisoned {
            return Err(LocalJournalAppendError::DefinitelyNotAppended(
                LocalJournalError::SegmentPoisoned,
            ));
        }
        #[cfg(windows)]
        return Err(LocalJournalAppendError::DefinitelyNotAppended(
            LocalJournalError::UnsupportedDurableReplacement,
        ));

        let sequence = self.frontier.next_sequence;
        let bytes = encode_frame(self.selection.device_id, sequence, payload_kind, payload)
            .map_err(LocalJournalAppendError::DefinitelyNotAppended)?;
        let successor = self
            .frontier
            .successor(&bytes)
            .map_err(LocalJournalAppendError::DefinitelyNotAppended)?;
        fail_before_write_for_test().map_err(LocalJournalAppendError::DefinitelyNotAppended)?;

        self.poisoned = true;
        if let Err(error) = write_segment_for_append(&mut self.file, &bytes) {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error));
        }
        if let Err(error) = self.file.sync_data() {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error.into()));
        }
        crash_after_segment_sync_for_test();
        if let Err(error) = fail_after_segment_sync_for_test() {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error));
        }
        let successor_bytes = successor.encode();
        if let Err(error) =
            publish_frontier_durable(&self.dir, self.selection.frontier_name(), &successor_bytes)
        {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error));
        }
        if let Err(error) =
            verify_regular_exact(&self.dir, self.selection.frontier_name(), &successor_bytes)
        {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error));
        }
        if let Err(error) = fail_after_frontier_verify_for_test() {
            return Err(LocalJournalAppendError::AppendOutcomeUnknown(error));
        }

        self.frontier = successor;
        self.poisoned = false;
        self.stats.frames_appended += 1;
        self.stats.bytes_appended += bytes.len() as u64;
        self.stats.data_durability_syncs += 2;
        self.stats.directory_durability_syncs += 1;
        Ok(LocalJournalAppend {
            device_id: self.selection.device_id,
            sequence,
            frame_bytes: bytes.len() as u64,
            payload_digest: ContentDigest::of(payload),
            data_durability_syncs: 2,
        })
    }

    pub fn replay(
        &self,
        mut visit: impl FnMut(LocalJournalFrame<K>),
    ) -> Result<u64, LocalJournalError> {
        let mut reader = BufReader::with_capacity(SCAN_BUFFER_BYTES, self.file.try_clone()?);
        let mut offset = LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64;
        let mut visited = 0;
        while offset < self.frontier.committed_extent {
            let (frame, bytes) =
                read_complete_frame::<K>(&mut reader, offset, self.frontier.committed_extent)?;
            offset += bytes.len() as u64;
            visited += 1;
            visit(frame);
        }
        Ok(visited)
    }
}

impl<K> Drop for LocalJournalSegmentV2<K> {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

/// Locked, non-mutating inspection of a legacy v1 segment.
pub struct LockedLocalJournalV1Segment<K> {
    file: fs::File,
    name: String,
    device_id: Uuid,
    base_sequence: u64,
    next_sequence: u64,
    committed_bytes: u64,
    frames: u64,
    last_frame: Option<LocalJournalFrame<K>>,
}

impl<K: LocalJournalPayloadKind> LockedLocalJournalV1Segment<K> {
    pub fn inspect(
        dir: &Dir,
        name: &str,
        device_id: Uuid,
        base_sequence: u64,
    ) -> Result<Self, LocalJournalError> {
        require_safe_segment_name(name)?;
        let file = open_regular_read_write_nofollow(dir, name)?;
        if !lock_exclusive_nonblocking(&file)? {
            return Err(LocalJournalError::SegmentAlreadyOpen(name.to_owned()));
        }
        if !file.metadata()?.is_file() {
            return Err(LocalJournalError::UnsafeSegmentName(name.to_owned()));
        }
        let scan = match inspect_v1::<K>(&file, device_id, base_sequence) {
            Ok(scan) => scan,
            Err(error) => {
                // Make lock release explicit on an ineligible/corrupt legacy
                // segment. Callers may inspect multiple immutable candidates
                // in one process and must not observe a transient self-lock.
                unlock(&file);
                return Err(error);
            }
        };
        Ok(Self {
            file,
            name: name.to_owned(),
            device_id,
            base_sequence,
            next_sequence: scan.next_sequence,
            committed_bytes: scan.committed_extent,
            frames: scan.frames_recovered,
            last_frame: scan.last_frame,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn device_id(&self) -> Uuid {
        self.device_id
    }

    pub const fn base_sequence(&self) -> u64 {
        self.base_sequence
    }

    pub const fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    pub const fn committed_bytes(&self) -> u64 {
        self.committed_bytes
    }

    pub const fn frames(&self) -> u64 {
        self.frames
    }

    pub const fn last_frame(&self) -> Option<&LocalJournalFrame<K>> {
        self.last_frame.as_ref()
    }

    pub fn replay(
        &self,
        mut visit: impl FnMut(LocalJournalFrame<K>),
    ) -> Result<u64, LocalJournalError> {
        let mut reader = BufReader::with_capacity(SCAN_BUFFER_BYTES, self.file.try_clone()?);
        let mut offset = 0;
        let mut visited = 0;
        while offset < self.committed_bytes {
            let (frame, bytes) =
                read_complete_frame::<K>(&mut reader, offset, self.committed_bytes)?;
            offset += bytes.len() as u64;
            visited += 1;
            visit(frame);
        }
        Ok(visited)
    }
}

impl<K> Drop for LockedLocalJournalV1Segment<K> {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

struct PrefixScan<K> {
    committed_extent: u64,
    frames_recovered: u64,
    next_sequence: u64,
    last_frame: Option<LocalJournalFrame<K>>,
}

fn scan_v2_prefix<K: LocalJournalPayloadKind>(
    file: &fs::File,
    frontier: &FrontierV2,
) -> Result<PrefixScan<K>, LocalJournalError> {
    let file_len = file.metadata()?.len();
    if file_len > MAX_LOCAL_JOURNAL_SEGMENT_BYTES {
        return Err(LocalJournalError::SegmentTooLarge(file_len));
    }
    if frontier.committed_extent < LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64
        || frontier.committed_extent > MAX_LOCAL_JOURNAL_SEGMENT_BYTES
        || file_len < frontier.committed_extent
    {
        return corrupt(0, "frontier committed extent is outside the segment");
    }
    let mut reader = BufReader::with_capacity(SCAN_BUFFER_BYTES, file.try_clone()?);
    let mut offset = LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64;
    let mut next = frontier.header.base_sequence;
    let mut frames = 0;
    let mut last = None;
    let mut terminal_bytes = None;
    while offset < frontier.committed_extent {
        let frame_offset = offset;
        let (frame, bytes) =
            read_complete_frame::<K>(&mut reader, offset, frontier.committed_extent)?;
        if frame.device_id() != frontier.header.device_id {
            return Err(LocalJournalError::SegmentDeviceMismatch {
                offset,
                expected: frontier.header.device_id,
                found: frame.device_id(),
            });
        }
        if frame.sequence() != next {
            return Err(LocalJournalError::SegmentSequenceGap {
                offset,
                expected: next,
                found: frame.sequence(),
            });
        }
        offset += bytes.len() as u64;
        next = next
            .checked_add(1)
            .ok_or(LocalJournalError::SequenceExhausted)?;
        frames += 1;
        terminal_bytes = Some(bytes);
        last = Some(frame);
        if offset > frontier.committed_extent {
            return corrupt(frame_offset, "frame crosses the committed frontier");
        }
    }
    if offset != frontier.committed_extent || next != frontier.next_sequence {
        return corrupt(
            offset,
            "frontier extent or next sequence does not match its frames",
        );
    }
    match (&frontier.terminal, &terminal_bytes, &last) {
        (None, None, None)
            if frontier.committed_extent == LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64
                && frontier.next_sequence == frontier.header.base_sequence
                && frontier.predecessor_digest == ContentDigest::from_bytes([0; 32]) => {}
        (Some(expected), Some(bytes), Some(frame))
            if expected.sequence == frame.sequence()
                && expected.encoded_len == bytes.len() as u64
                && expected.frame_digest == ContentDigest::of(bytes)
                && frontier.predecessor_digest != ContentDigest::from_bytes([0; 32]) => {}
        _ => {
            return corrupt(
                offset,
                "frontier terminal metadata does not match the segment",
            )
        }
    }
    Ok(PrefixScan {
        committed_extent: offset,
        frames_recovered: frames,
        next_sequence: next,
        last_frame: last,
    })
}

fn inspect_v1<K: LocalJournalPayloadKind>(
    file: &fs::File,
    device_id: Uuid,
    base_sequence: u64,
) -> Result<PrefixScan<K>, LocalJournalError> {
    let file_len = file.metadata()?.len();
    if file_len > MAX_LOCAL_JOURNAL_SEGMENT_BYTES {
        return Err(LocalJournalError::SegmentTooLarge(file_len));
    }
    let mut reader = BufReader::with_capacity(SCAN_BUFFER_BYTES, file.try_clone()?);
    let mut offset = 0;
    let mut next = base_sequence;
    let mut frames = 0;
    let mut last = None;
    while offset < file_len {
        let remaining = file_len - offset;
        if remaining < MIN_FRAME_BYTES as u64 {
            return Err(LocalJournalError::AmbiguousLegacySuffix {
                offset,
                length: remaining,
            });
        }
        let (frame, bytes) = match read_complete_frame::<K>(&mut reader, offset, file_len) {
            Ok(frame) => frame,
            Err(LocalJournalError::CorruptSegment { cause, .. })
                if cause.contains("declared frame length") =>
            {
                return Err(LocalJournalError::AmbiguousLegacySuffix {
                    offset,
                    length: remaining,
                });
            }
            Err(error) => return Err(error),
        };
        if frame.device_id() != device_id {
            return Err(LocalJournalError::SegmentDeviceMismatch {
                offset,
                expected: device_id,
                found: frame.device_id(),
            });
        }
        if frame.sequence() != next {
            return Err(LocalJournalError::SegmentSequenceGap {
                offset,
                expected: next,
                found: frame.sequence(),
            });
        }
        offset += bytes.len() as u64;
        next = next
            .checked_add(1)
            .ok_or(LocalJournalError::SequenceExhausted)?;
        frames += 1;
        last = Some(frame);
    }
    Ok(PrefixScan {
        committed_extent: offset,
        frames_recovered: frames,
        next_sequence: next,
        last_frame: last,
    })
}

fn read_header(file: &fs::File) -> Result<SegmentHeaderV2, LocalJournalError> {
    if file.metadata()?.len() < LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64 {
        return corrupt(0, "segment header is truncated");
    }
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = [0; LOCAL_JOURNAL_SEGMENT_HEADER_BYTES];
    reader.read_exact(&mut bytes)?;
    SegmentHeaderV2::decode(&bytes)
}

fn read_complete_frame<K: LocalJournalPayloadKind>(
    reader: &mut BufReader<fs::File>,
    offset: u64,
    limit: u64,
) -> Result<(LocalJournalFrame<K>, Vec<u8>), LocalJournalError> {
    const PREFIX: usize = 20;
    let remaining = limit
        .checked_sub(offset)
        .ok_or_else(|| LocalJournalError::CorruptSegment {
            offset,
            cause: "frame offset exceeds committed extent".to_owned(),
        })?;
    if remaining < MIN_FRAME_BYTES as u64 {
        return corrupt(offset, "committed frame is truncated");
    }
    reader.seek(SeekFrom::Start(offset))?;
    let mut prefix = [0_u8; PREFIX];
    reader.read_exact(&mut prefix)?;
    if &prefix[..8] != b"TINEJRN1" {
        return corrupt(offset, "frame magic is invalid");
    }
    let header_len = u32::from_be_bytes(prefix[8..12].try_into().expect("fixed header")) as u64;
    let payload_len = u64::from_be_bytes(prefix[12..20].try_into().expect("fixed payload"));
    let total = (PREFIX as u64)
        .checked_add(header_len)
        .and_then(|value| value.checked_add(payload_len))
        .and_then(|value| value.checked_add(CHECKSUM_BYTES as u64))
        .ok_or(LocalJournalError::LengthOverflow)?;
    if total > MAX_LOCAL_JOURNAL_FRAME_BYTES as u64 {
        return Err(LocalJournalError::FrameTooLarge(total as usize));
    }
    if total > remaining {
        return corrupt(
            offset,
            format!("declared frame length {total} exceeds the {remaining} remaining bytes"),
        );
    }
    let total_usize = usize::try_from(total).map_err(|_| LocalJournalError::LengthOverflow)?;
    let mut bytes = vec![0; total_usize];
    bytes[..PREFIX].copy_from_slice(&prefix);
    reader.read_exact(&mut bytes[PREFIX..])?;
    let frame =
        LocalJournalFrame::decode(&bytes).map_err(|error| LocalJournalError::CorruptSegment {
            offset,
            cause: error.to_string(),
        })?;
    Ok((frame, bytes))
}

fn create_exact_durable(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), LocalJournalError> {
    publish_immutable_exact(dir, name, bytes).map_err(|error| match error {
        FilesystemError::ByteCollision | FilesystemError::StoredLengthMismatch { .. } => {
            LocalJournalError::PreparedArtifactExists(name.to_owned())
        }
        error => LocalJournalError::Io(format!("durable preparation of {name} failed: {error}")),
    })?;
    verify_regular_exact(dir, name, bytes)
}

fn read_frontier_exact(dir: &Dir, name: &str) -> Result<Vec<u8>, LocalJournalError> {
    read_regular_exact_length(dir, name, LOCAL_JOURNAL_FRONTIER_BYTES)
}

fn read_regular_exact_length(
    dir: &Dir,
    name: &str,
    expected_length: usize,
) -> Result<Vec<u8>, LocalJournalError> {
    read_required_regular(
        dir,
        name,
        expected_length.saturating_add(1) as u64,
        Some(expected_length as u64),
    )
    .map_err(|error| match error {
        FilesystemError::StoredLengthMismatch { .. }
        | FilesystemError::StoredFileTooLarge { .. }
        | FilesystemError::ByteCollision => LocalJournalError::CorruptSegment {
            offset: 0,
            cause: format!("frontier is not canonical: {error}"),
        },
        error => LocalJournalError::Io(error.to_string()),
    })
}

fn verify_regular_exact(dir: &Dir, name: &str, expected: &[u8]) -> Result<(), LocalJournalError> {
    let actual = read_regular_exact_length(dir, name, expected.len())?;
    if actual != expected {
        return corrupt(
            0,
            "reopened frontier bytes differ from the published successor",
        );
    }
    Ok(())
}

fn write_segment_for_append(file: &mut fs::File, bytes: &[u8]) -> Result<(), LocalJournalError> {
    #[cfg(test)]
    if take_fault(FaultPoint::SegmentPartialWrite) {
        file.write_all(&bytes[..bytes.len() / 2])?;
        return Err(injected_fault("after partial segment write"));
    }
    file.write_all(bytes)?;
    #[cfg(test)]
    if take_fault(FaultPoint::AfterSegmentWrite) {
        return Err(injected_fault("after full segment write"));
    }
    Ok(())
}

#[cfg(unix)]
fn publish_frontier_durable(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), LocalJournalError> {
    #[cfg(test)]
    if take_fault(FaultPoint::BeforeFrontierTemp) {
        return Err(injected_fault("before frontier temp creation"));
    }
    let temp = format!(".{name}.{}.tmp", Uuid::new_v4().simple());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = dir.open_with(&temp, &options)?.into_std();
    file.write_all(bytes)?;
    #[cfg(test)]
    if take_fault(FaultPoint::AfterFrontierTempWrite) {
        return Err(injected_fault("after frontier temp write"));
    }
    file.sync_data()?;
    #[cfg(test)]
    if take_fault(FaultPoint::AfterFrontierTempSync) {
        return Err(injected_fault("after frontier temp sync"));
    }
    #[cfg(test)]
    if take_fault(FaultPoint::BeforeFrontierReplace) {
        return Err(injected_fault("before frontier replacement"));
    }
    dir.rename(&temp, dir, name)?;
    sync_dir_required(dir)?;
    crash_after_frontier_replace_for_test();
    #[cfg(test)]
    if take_fault(FaultPoint::AfterFrontierReplace) {
        return Err(injected_fault("after frontier replacement"));
    }
    Ok(())
}

#[cfg(windows)]
fn publish_frontier_durable(
    _dir: &Dir,
    _name: &str,
    _bytes: &[u8],
) -> Result<(), LocalJournalError> {
    // TODO(storage-journal-v2): implement MoveFileExW with
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH, then reopen and
    // byte-verify. Never substitute std::fs::rename here.
    Err(LocalJournalError::UnsupportedDurableReplacement)
}

#[cfg(not(any(unix, windows)))]
fn publish_frontier_durable(
    _dir: &Dir,
    _name: &str,
    _bytes: &[u8],
) -> Result<(), LocalJournalError> {
    Err(LocalJournalError::UnsupportedDurableReplacement)
}

fn corrupt<T>(offset: u64, cause: impl Into<String>) -> Result<T, LocalJournalError> {
    Err(LocalJournalError::CorruptSegment {
        offset,
        cause: cause.into(),
    })
}

#[cfg(not(test))]
fn fail_before_write_for_test() -> Result<(), LocalJournalError> {
    Ok(())
}

#[cfg(not(test))]
fn fail_after_segment_sync_for_test() -> Result<(), LocalJournalError> {
    Ok(())
}

#[cfg(not(test))]
fn crash_after_frontier_replace_for_test() {}

#[cfg(not(test))]
fn crash_after_segment_sync_for_test() {}

#[cfg(not(test))]
fn fail_after_frontier_verify_for_test() -> Result<(), LocalJournalError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};

    #[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
    enum TestKind {
        Effect,
        Update,
    }

    struct Fixture {
        root: PathBuf,
        dir: Dir,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "tine-local-journal-v2-{label}-{}",
                Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&root).unwrap();
            let dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
            Self { root, dir }
        }

        fn selection(&self) -> LocalJournalSegmentV2Selection {
            LocalJournalSegmentV2Selection::new(
                "device.journal-v2",
                Uuid::from_u128(0x5151),
                Uuid::from_u128(0xdede),
                41,
            )
            .unwrap()
        }

        fn prepare(&self) -> LocalJournalSegmentV2Selection {
            let selection = self.selection();
            LocalJournalSegmentV2::<TestKind>::prepare(&self.dir, &selection).unwrap();
            selection
        }

        fn bytes(&self, name: &str) -> Vec<u8> {
            fs::read(self.root.join(name)).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn open(
        fixture: &Fixture,
        selection: &LocalJournalSegmentV2Selection,
    ) -> (
        LocalJournalSegmentV2<TestKind>,
        LocalJournalRecovery<TestKind>,
    ) {
        LocalJournalSegmentV2::open_selected(&fixture.dir, selection).unwrap()
    }

    fn arm(point: FaultPoint) {
        FAULT.with(|fault| {
            assert_eq!(fault.replace(Some(point)), None);
        });
    }

    fn assert_unknown_and_poisoned(
        segment: &mut LocalJournalSegmentV2<TestKind>,
        point: FaultPoint,
    ) {
        arm(point);
        assert!(matches!(
            segment.append(TestKind::Effect, b"candidate"),
            Err(LocalJournalAppendError::AppendOutcomeUnknown(_))
        ));
        assert!(matches!(
            segment.append(TestKind::Effect, b"retry"),
            Err(LocalJournalAppendError::DefinitelyNotAppended(
                LocalJournalError::SegmentPoisoned
            ))
        ));
    }

    #[test]
    fn fixed_header_and_frontier_are_canonical_and_selection_bound() {
        let fixture = Fixture::new("canonical");
        let selection = fixture.prepare();
        let header = fixture.bytes(selection.segment_name());
        let frontier = fixture.bytes(selection.frontier_name());
        assert_eq!(header.len(), LOCAL_JOURNAL_SEGMENT_HEADER_BYTES);
        assert_eq!(frontier.len(), LOCAL_JOURNAL_FRONTIER_BYTES);
        assert_eq!(
            SegmentHeaderV2::decode(&header).unwrap(),
            SegmentHeaderV2::for_selection(&selection)
        );
        assert_eq!(
            FrontierV2::decode(&frontier).unwrap(),
            FrontierV2::initial(SegmentHeaderV2::for_selection(&selection))
        );
        assert_eq!(
            selection.segment_name_digest(),
            ContentDigest::of(selection.segment_name().as_bytes())
        );

        let mut noncanonical = header;
        noncanonical[12] = 1;
        let checksum = ContentDigest::of(&noncanonical[..HEADER_CHECKSUM_OFFSET]);
        noncanonical[HEADER_CHECKSUM_OFFSET..].copy_from_slice(checksum.as_bytes());
        assert!(SegmentHeaderV2::decode(&noncanonical).is_err());

        LocalJournalSegmentV2::<TestKind>::prepare(&fixture.dir, &selection).unwrap();
        let conflicting = LocalJournalSegmentV2Selection::new(
            selection.segment_name(),
            Uuid::from_u128(0x9999),
            selection.device_id(),
            selection.base_sequence(),
        )
        .unwrap();
        assert!(matches!(
            LocalJournalSegmentV2::<TestKind>::prepare(&fixture.dir, &conflicting),
            Err(LocalJournalError::PreparedArtifactExists(_))
        ));
    }

    #[test]
    fn append_publishes_successor_before_advancing_memory_and_pays_two_data_syncs() {
        let fixture = Fixture::new("append");
        let selection = fixture.prepare();
        let (mut segment, recovery) = open(&fixture, &selection);
        assert_eq!(recovery.frames_recovered, 0);
        let append = segment.append(TestKind::Effect, b"durable").unwrap();
        assert_eq!(append.sequence, 41);
        assert_eq!(append.data_durability_syncs, 2);
        assert_eq!(segment.next_sequence(), 42);
        assert_eq!(segment.stats().data_durability_syncs, 2);
        drop(segment);

        let (segment, recovery) = open(&fixture, &selection);
        assert_eq!(recovery.frames_recovered, 1);
        assert_eq!(recovery.discarded_tail_bytes, 0);
        assert_eq!(recovery.last_frame.as_ref().unwrap().payload(), b"durable");
        let mut frames = Vec::new();
        assert_eq!(segment.replay(|frame| frames.push(frame)).unwrap(), 1);
        assert_eq!(frames[0].sequence(), 41);
    }

    #[test]
    fn prewrite_failure_is_definite_and_leaves_writer_usable() {
        let fixture = Fixture::new("prewrite");
        let selection = fixture.prepare();
        let (mut segment, _) = open(&fixture, &selection);
        arm(FaultPoint::BeforeSegmentWrite);
        assert!(matches!(
            segment.append(TestKind::Effect, b"not-written"),
            Err(LocalJournalAppendError::DefinitelyNotAppended(_))
        ));
        assert_eq!(
            segment
                .append(TestKind::Effect, b"written")
                .unwrap()
                .sequence,
            41
        );
    }

    #[test]
    fn every_postwrite_failure_is_unknown_and_reopen_uses_only_old_or_successor_frontier() {
        let cases = [
            (FaultPoint::SegmentPartialWrite, 0),
            (FaultPoint::AfterSegmentWrite, 0),
            (FaultPoint::AfterSegmentSync, 0),
            (FaultPoint::BeforeFrontierTemp, 0),
            (FaultPoint::AfterFrontierTempWrite, 0),
            (FaultPoint::AfterFrontierTempSync, 0),
            (FaultPoint::BeforeFrontierReplace, 0),
            (FaultPoint::AfterFrontierReplace, 1),
            (FaultPoint::AfterFrontierVerify, 1),
        ];
        for (point, expected_frames) in cases {
            let fixture = Fixture::new(&format!("fault-{point:?}"));
            let selection = fixture.prepare();
            let (mut segment, _) = open(&fixture, &selection);
            assert_unknown_and_poisoned(&mut segment, point);
            drop(segment);
            let (segment, recovery) = open(&fixture, &selection);
            assert_eq!(recovery.frames_recovered, expected_frames, "{point:?}");
            assert_eq!(segment.next_sequence(), 41 + expected_frames, "{point:?}");
            if expected_frames == 0 {
                assert_eq!(
                    segment.committed_bytes(),
                    LOCAL_JOURNAL_SEGMENT_HEADER_BYTES as u64
                );
                assert_eq!(segment.stats().recovery_truncations, 1);
            }
        }
    }

    #[test]
    fn valid_old_frontier_discards_even_a_complete_physical_suffix_and_ignores_temps() {
        let fixture = Fixture::new("old-frontier");
        let selection = fixture.prepare();
        let initial = fixture.bytes(selection.frontier_name());
        let (mut segment, _) = open(&fixture, &selection);
        segment.append(TestKind::Effect, b"suffix").unwrap();
        drop(segment);
        fs::write(fixture.root.join(selection.frontier_name()), initial).unwrap();
        fs::write(
            fixture
                .root
                .join(format!(".{}.orphan.tmp", selection.frontier_name())),
            b"never authority",
        )
        .unwrap();

        let (segment, recovery) = open(&fixture, &selection);
        assert_eq!(recovery.frames_recovered, 0);
        assert!(recovery.discarded_tail_bytes > 0);
        assert_eq!(segment.stats().recovery_truncations, 1);
        assert_eq!(
            fixture.bytes(selection.segment_name()).len(),
            LOCAL_JOURNAL_SEGMENT_HEADER_BYTES
        );
    }

    #[test]
    fn old_frontier_discards_physical_suffixes_at_required_boundary_lengths() {
        let candidate = encode_frame(
            Uuid::from_u128(0xdede),
            41,
            TestKind::Effect,
            b"boundary frame",
        )
        .unwrap();
        for length in [0, 1, 20, 51, 52, candidate.len() - 1, candidate.len()] {
            let fixture = Fixture::new(&format!("suffix-boundary-{length}"));
            let selection = fixture.prepare();
            let mut segment = fixture.bytes(selection.segment_name());
            segment.extend_from_slice(&candidate[..length]);
            fs::write(fixture.root.join(selection.segment_name()), segment).unwrap();

            let (opened, recovery) = open(&fixture, &selection);
            assert_eq!(recovery.frames_recovered, 0, "suffix length {length}");
            assert_eq!(recovery.discarded_tail_bytes, length as u64);
            assert_eq!(opened.stats().recovery_truncations, u64::from(length > 0));
            assert_eq!(
                fixture.bytes(selection.segment_name()).len(),
                LOCAL_JOURNAL_SEGMENT_HEADER_BYTES
            );
        }
    }

    #[test]
    fn missing_short_corrupt_foreign_or_internally_inconsistent_frontiers_fail_without_mutation() {
        enum Damage {
            Missing,
            Short,
            Checksum,
            Foreign,
            Ahead,
            Terminal,
        }
        for damage in [
            Damage::Missing,
            Damage::Short,
            Damage::Checksum,
            Damage::Foreign,
            Damage::Ahead,
            Damage::Terminal,
        ] {
            let fixture = Fixture::new("bad-frontier");
            let selection = fixture.prepare();
            let segment_before = fixture.bytes(selection.segment_name());
            let frontier_path = fixture.root.join(selection.frontier_name());
            match damage {
                Damage::Missing => fs::remove_file(&frontier_path).unwrap(),
                Damage::Short => fs::write(&frontier_path, b"short").unwrap(),
                Damage::Checksum => {
                    let mut bytes = fs::read(&frontier_path).unwrap();
                    bytes[30] ^= 1;
                    fs::write(&frontier_path, bytes).unwrap();
                }
                Damage::Foreign => {
                    let foreign = LocalJournalSegmentV2Selection::new(
                        selection.segment_name(),
                        Uuid::from_u128(0xffff),
                        selection.device_id(),
                        selection.base_sequence(),
                    )
                    .unwrap();
                    let bytes =
                        FrontierV2::initial(SegmentHeaderV2::for_selection(&foreign)).encode();
                    fs::write(&frontier_path, bytes).unwrap();
                }
                Damage::Ahead => {
                    let mut frontier =
                        FrontierV2::decode(&fs::read(&frontier_path).unwrap()).unwrap();
                    frontier.committed_extent += 1;
                    fs::write(&frontier_path, frontier.encode()).unwrap();
                }
                Damage::Terminal => {
                    let mut frontier =
                        FrontierV2::decode(&fs::read(&frontier_path).unwrap()).unwrap();
                    frontier.next_sequence += 1;
                    fs::write(&frontier_path, frontier.encode()).unwrap();
                }
            }
            assert!(
                LocalJournalSegmentV2::<TestKind>::open_selected(&fixture.dir, &selection).is_err()
            );
            assert_eq!(fixture.bytes(selection.segment_name()), segment_before);
        }
    }

    #[test]
    fn committed_frame_or_terminal_metadata_corruption_fails_closed() {
        for terminal_only in [false, true] {
            let fixture = Fixture::new("committed-corruption");
            let selection = fixture.prepare();
            let (mut segment, _) = open(&fixture, &selection);
            segment.append(TestKind::Effect, b"committed").unwrap();
            drop(segment);
            if terminal_only {
                let path = fixture.root.join(selection.frontier_name());
                let mut frontier = FrontierV2::decode(&fs::read(&path).unwrap()).unwrap();
                frontier.terminal.as_mut().unwrap().frame_digest = ContentDigest::of(b"wrong");
                fs::write(path, frontier.encode()).unwrap();
            } else {
                let path = fixture.root.join(selection.segment_name());
                let mut bytes = fs::read(&path).unwrap();
                bytes[LOCAL_JOURNAL_SEGMENT_HEADER_BYTES + MIN_FRAME_BYTES] ^= 1;
                fs::write(path, bytes).unwrap();
            }
            let before = fixture.bytes(selection.segment_name());
            assert!(
                LocalJournalSegmentV2::<TestKind>::open_selected(&fixture.dir, &selection).is_err()
            );
            assert_eq!(fixture.bytes(selection.segment_name()), before);
        }
    }

    #[test]
    fn a_returned_frame_later_shorter_than_its_frontier_fails_without_truncation() {
        let fixture = Fixture::new("returned-short");
        let selection = fixture.prepare();
        let (mut segment, _) = open(&fixture, &selection);
        segment.append(TestKind::Effect, b"returned").unwrap();
        drop(segment);
        let path = fixture.root.join(selection.segment_name());
        let mut shortened = fs::read(&path).unwrap();
        shortened.pop();
        fs::write(&path, &shortened).unwrap();

        assert!(
            LocalJournalSegmentV2::<TestKind>::open_selected(&fixture.dir, &selection).is_err()
        );
        assert_eq!(fixture.bytes(selection.segment_name()), shortened);
    }

    #[test]
    fn v1_inspection_accepts_exact_eof_and_refuses_every_nonempty_ambiguous_suffix_without_mutation(
    ) {
        let fixture = Fixture::new("v1-inspect");
        let device = Uuid::from_u128(0xaaaa);
        {
            let (mut segment, _) = crate::LocalJournalSegment::<TestKind>::open_from_sequence(
                &fixture.dir,
                "legacy.journal",
                device,
                7,
            )
            .unwrap();
            segment.append(TestKind::Effect, b"legacy").unwrap();
        }
        let exact = fixture.bytes("legacy.journal");
        {
            let inspection = LockedLocalJournalV1Segment::<TestKind>::inspect(
                &fixture.dir,
                "legacy.journal",
                device,
                7,
            )
            .unwrap();
            assert_eq!(inspection.frames(), 1);
            assert_eq!(inspection.next_sequence(), 8);
            assert_eq!(inspection.last_frame().unwrap().payload(), b"legacy");
        }

        let mut suffixes = (1..MIN_FRAME_BYTES)
            .map(|length| vec![0x42; length])
            .collect::<Vec<_>>();
        let full = encode_frame(device, 8, TestKind::Update, b"partial").unwrap();
        suffixes.push(full[..MIN_FRAME_BYTES].to_vec());
        for suffix in suffixes {
            let mut ambiguous = exact.clone();
            ambiguous.extend_from_slice(&suffix);
            fs::write(fixture.root.join("legacy.journal"), &ambiguous).unwrap();
            let result = LockedLocalJournalV1Segment::<TestKind>::inspect(
                &fixture.dir,
                "legacy.journal",
                device,
                7,
            );
            assert!(
                matches!(result, Err(LocalJournalError::AmbiguousLegacySuffix { .. })),
                "{}-byte suffix produced {:?}",
                suffix.len(),
                result.err()
            );
            assert_eq!(fixture.bytes("legacy.journal"), ambiguous);
        }
    }

    #[test]
    fn v1_inspection_locks_and_refuses_full_invalid_frames_without_mutation() {
        let fixture = Fixture::new("v1-lock-corrupt");
        let device = Uuid::from_u128(0xbbbb);
        let valid = encode_frame(device, 0, TestKind::Effect, b"valid").unwrap();
        fs::write(fixture.root.join("legacy.journal"), &valid).unwrap();
        let held = LockedLocalJournalV1Segment::<TestKind>::inspect(
            &fixture.dir,
            "legacy.journal",
            device,
            0,
        )
        .unwrap();
        assert!(matches!(
            LockedLocalJournalV1Segment::<TestKind>::inspect(
                &fixture.dir,
                "legacy.journal",
                device,
                0,
            ),
            Err(LocalJournalError::SegmentAlreadyOpen(_))
        ));
        drop(held);

        let mut corrupt = valid;
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        fs::write(fixture.root.join("legacy.journal"), &corrupt).unwrap();
        assert!(matches!(
            LockedLocalJournalV1Segment::<TestKind>::inspect(
                &fixture.dir,
                "legacy.journal",
                device,
                0,
            ),
            Err(LocalJournalError::CorruptSegment { .. })
        ));
        assert_eq!(fixture.bytes("legacy.journal"), corrupt);
    }

    #[cfg(unix)]
    #[test]
    fn process_crashes_reopen_to_old_or_successor_frontier() {
        for (stage, expected_frames) in [
            ("after-segment-sync", 0_u64),
            ("after-frontier-replace", 1_u64),
        ] {
            let fixture = Fixture::new(&format!("crash-{stage}"));
            let selection = fixture.prepare();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("local_journal_v2::tests::crash_append_child")
                .arg("--ignored")
                .env("TINE_STORAGE_JOURNAL_CRASH_ROOT", &fixture.root)
                .env("TINE_STORAGE_JOURNAL_CRASH_STAGE", stage)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(73), "child did not crash at {stage}");
            let (segment, recovery) = open(&fixture, &selection);
            assert_eq!(recovery.frames_recovered, expected_frames, "{stage}");
            assert_eq!(segment.next_sequence(), 41 + expected_frames, "{stage}");
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess helper invoked by process_crashes_reopen_to_old_or_successor_frontier"]
    fn crash_append_child() {
        let Some(root) = std::env::var_os("TINE_STORAGE_JOURNAL_CRASH_ROOT") else {
            return;
        };
        let root = Path::new(&root);
        let dir = Dir::open_ambient_dir(root, cap_std::ambient_authority()).unwrap();
        let selection = LocalJournalSegmentV2Selection::new(
            "device.journal-v2",
            Uuid::from_u128(0x5151),
            Uuid::from_u128(0xdede),
            41,
        )
        .unwrap();
        let (mut segment, _) =
            LocalJournalSegmentV2::<TestKind>::open_selected(&dir, &selection).unwrap();
        let _ = segment.append(TestKind::Effect, b"crash candidate");
        panic!("requested crash point was not reached");
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    BeforeSegmentWrite,
    SegmentPartialWrite,
    AfterSegmentWrite,
    AfterSegmentSync,
    BeforeFrontierTemp,
    AfterFrontierTempWrite,
    AfterFrontierTempSync,
    BeforeFrontierReplace,
    AfterFrontierReplace,
    AfterFrontierVerify,
}

#[cfg(test)]
thread_local! {
    static FAULT: std::cell::Cell<Option<FaultPoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn take_fault(point: FaultPoint) -> bool {
    FAULT.with(|fault| {
        if fault.get() == Some(point) {
            fault.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
fn injected_fault(stage: &str) -> LocalJournalError {
    LocalJournalError::Io(format!("injected fault {stage}"))
}

#[cfg(test)]
fn fail_before_write_for_test() -> Result<(), LocalJournalError> {
    if take_fault(FaultPoint::BeforeSegmentWrite) {
        Err(injected_fault("before segment write"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn fail_after_segment_sync_for_test() -> Result<(), LocalJournalError> {
    if take_fault(FaultPoint::AfterSegmentSync) {
        Err(injected_fault("after segment sync"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn crash_for_test(stage: &str) {
    if std::env::var_os("TINE_STORAGE_JOURNAL_CRASH_STAGE").as_deref()
        == Some(std::ffi::OsStr::new(stage))
    {
        #[cfg(unix)]
        // SAFETY: this is an intentionally abrupt crash-test child. `_exit`
        // skips destructors and stdio/allocator cleanup so reopen observes
        // only durability work completed before this boundary.
        unsafe {
            libc::_exit(73);
        }
        #[cfg(not(unix))]
        std::process::exit(73);
    }
}

#[cfg(test)]
fn crash_after_segment_sync_for_test() {
    crash_for_test("after-segment-sync");
}

#[cfg(test)]
fn crash_after_frontier_replace_for_test() {
    crash_for_test("after-frontier-replace");
}

#[cfg(test)]
fn fail_after_frontier_verify_for_test() -> Result<(), LocalJournalError> {
    if take_fault(FaultPoint::AfterFrontierVerify) {
        Err(injected_fault("after frontier verification"))
    } else {
        Ok(())
    }
}
