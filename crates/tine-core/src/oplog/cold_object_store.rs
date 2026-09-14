//! The sealed archive: one immutable pack set, one sorted-table index, one
//! marker — and the cold whole-object tier that lives inside it.
//!
//! # What this is
//!
//! `sealed-v3` is the single private directory under the archive generation
//! that holds every sealed index object Tine keeps: the accepted history
//! (batch, sequence, document-change, covered-object, causal-tip, identity and
//! document-roster domains) and the cold-history locator index (cold-object and
//! cold-manifest domains). Before E1 those lived in two directories —
//! `clean-open-checkpoint-v2/` and `cold-history-v1/` — as persistent
//! path-copied treaps with ONE FILE PER NODE. That shape wrote ≈140 sealed
//! nodes and ≈70 cold files per accepted batch (≈0.9 MB per single-block edit)
//! and reached `EXT4-fs: Directory index full` at 4.75 M entries, after which
//! every later cut failed forever. Both costs are the data structure, not the
//! container (P4c2 §1).
//!
//! # The shape (P4c2 §4)
//!
//! * A **table** (`tine_storage::sealed_tables`) is an immutable, sorted,
//!   fixed-width `(key, value)` array with a fence array and a trailing digest.
//!   A cut appends one level-0 delta table per touched domain; when a level
//!   holds `TierPlan::FANOUT` tables they merge into one at the next level, so
//!   a domain spans `O(log N)` tables and a cut writes `O(T)` index bytes for
//!   the `T` entries it actually changed.
//! * A **pack** is the container: `record* footer footer_len:u64be magic:8`,
//!   where a record is `sha256(payload):32 payload_len:u64be payload`. A table
//!   is one pack record (class `COLD_CLASS_TABLE`); so is the root
//!   (`COLD_CLASS_ROOT`), a cold object, a cold manifest, and every
//!   variable-length sealed record the tables point at (`COLD_CLASS_RECORD`).
//!   Nothing appends to a published pack.
//! * The **root record** is `SealedTableRoot` — per domain, the ordered table
//!   list — plus this module's pack table. The `current` marker names the root
//!   record's locator and is installed LAST (I-2).
//!
//! # Virtual pack addressing — why a locator has no pack name in it
//!
//! Packs are tiered exactly like tables: `FANOUT` packs of one level merge into
//! one pack of the next, which is what keeps the directory at
//! `O(log N + live bytes / pack target)` entries instead of one pack per cut.
//! A merge relocates record BYTES, and a locator that named `(pack uuid,
//! offset)` would then be stale in every table pointing into those packs —
//! turning each pack merge into a whole-index rewrite, which is precisely the
//! `O(N)`-per-cut cost this packet exists to remove.
//!
//! So a record is addressed in ONE monotonically growing **virtual byte space**
//! shared by every pack: `ColdLocatorV1 { offset, length }`, where `offset` is
//! the record's position in that space. Each pack covers one contiguous virtual
//! range, declared in its own footer and listed in the root's pack table. A
//! merge concatenates the bodies of a contiguous run of packs, so every
//! record's virtual offset is UNCHANGED and not one table entry has to move.
//! Resolving a locator is a binary search of the pack table (≤ `FANOUT` × levels
//! entries) plus one ranged read.
//!
//! # Authentication (P4c2 §4.3, D-2/D-3)
//!
//! Every pack record self-verifies against its header digest, and a table IS
//! one pack record, so a table's digest is checked once when a reader loads it
//! and never per lookup. Tables and roots are disposable derived state: a table
//! that fails its digest is REBUILT from the surviving pack footers, never a
//! refusal to open the graph. Only the accepted originals inside the packs are
//! truth; losing those is a separate, named, payload-loss state.
//!
//! # Blank slate (D-1)
//!
//! There is exactly one current representation. There is no reader for the
//! retired `sealed-v2` node encoding, no dual read path and no migration:
//! unrecognized pre-E1 sealed state is preserved as a backup and the store is
//! rebuilt from the untouched Markdown/Org tree.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::object_store::{filesystem_error_without_collision, ObjectStore, StoreError};
use super::{BatchId, ContentDigest, MAX_MANIFEST_BYTES, MAX_OBJECT_BYTES};
use tine_storage::sealed_accepted_index::SealedAcceptedIndexError;
use tine_storage::sealed_tables::{
    compact_tables, merge_tables, SealedTableDomainRoot, SealedTableRoot, TableBuilder, TableBytes,
    TableDomain, TableLocator, TableRef, TableSetReader, TableView, TierPlan,
};

/// The one private sealed namespace, rooted in the retained archive capability.
///
/// It replaces BOTH `clean-open-checkpoint-v2/` and `cold-history-v1/`: one
/// directory, one marker, one root, one pack set (P4c2 §4.4).
pub(crate) const SEALED_DIRECTORY: &str = "sealed-v3";

/// The canonical root marker. Installed last, after every pack it names is
/// already durable.
pub(crate) const SEALED_ROOT_MARKER: &str = "current";
const COLD_PACK_PREFIX: &str = "pack-v1";
const COLD_SCHEMA_VERSION: u32 = 2;

/// `sha256(payload) || payload_len` prefixed to every packed record.
const COLD_RECORD_HEADER_BYTES: usize = 40;
const COLD_PACK_MAGIC: [u8; 8] = *b"TINECLD2";
/// A construction target, not an occupancy cap (D-5): one legal record larger
/// than this is packed alone rather than refused.
pub(crate) const COLD_PACK_TARGET_BYTES: usize = 4 * 1024 * 1024;
/// One record is at most `MAX_OBJECT_BYTES`; a pack holds one oversize record
/// plus its footer, or many target-sized ones.
const MAX_COLD_PACK_BYTES: u64 = 512 * 1024 * 1024;
const MAX_COLD_PACK_FOOTER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COLD_ROOT_BYTES: u64 = 64 * 1024 * 1024;
/// The marker carries the pack table, which is `O(FANOUT x levels)` entries of
/// 41 bytes — never a term that grows with history.
const MAX_SEALED_MARKER_BYTES: u64 = 1024 * 1024;

/// A cold logical object record, keyed by its full SHA-256.
const COLD_CLASS_OBJECT: u8 = 1;
/// A cold batch manifest record, keyed by its `BatchId`.
const COLD_CLASS_MANIFEST: u8 = 2;
// Class 3 was `COLD_CLASS_INNER_ROOT`, the packed descriptor of one 128-bit
// prefix's inner authenticated map. A sorted table over 32-byte keys has no
// prefix-bucket problem, so the two-level composition and its descriptor class
// are gone. The value is not reused.
/// One immutable sorted table, keyed by its table digest.
const COLD_CLASS_TABLE: u8 = 4;
/// The root record: every domain's ordered table list, plus the pack table.
const COLD_CLASS_ROOT: u8 = 5;
/// A variable-length sealed record a table points at by locator — an accepted
/// causal or status record, an identity admission value, a document capsule.
/// Keyed by its content digest.
const COLD_CLASS_RECORD: u8 = 6;

/// The tier fanout, shared by tables and packs.
pub(crate) const SEALED_TIER_FANOUT: usize = TierPlan::FANOUT;

// ---------------------------------------------------------------------------
// Domains (P4c2 §4.1; the reconciled table is in the contract doc)
// ---------------------------------------------------------------------------

/// `batch_id -> causal record locator ‖ status record locator`.
///
/// `tine-storage` owns this one: the SQLite seam reads it directly.
pub(crate) const DOMAIN_BATCH: TableDomain = tine_storage::sealed_tables::SEALED_BATCH_DOMAIN;
/// `acceptance sequence (be) -> batch id`. Also `tine-storage`'s.
pub(crate) const DOMAIN_SEQUENCE: TableDomain = tine_storage::sealed_tables::SEALED_SEQUENCE_DOMAIN;

/// `document uuid ‖ acceptance sequence (be) -> acceptance sequence (be)`.
///
/// The value repeats the key's sequence so a `predecessor` answer carries it
/// without a second read.
pub(crate) const DOMAIN_DOCUMENT_CHANGE: TableDomain = TableDomain {
    id: 3,
    key_len: 24,
    value_len: 8,
    tombstone: false,
};

/// `object content digest -> present`.
///
/// A membership domain. The value is one byte rather than zero because a
/// zero-width value is not a legal table domain; nothing reads it.
pub(crate) const DOMAIN_COVERED_OBJECT: TableDomain = TableDomain {
    id: 4,
    key_len: 32,
    value_len: 1,
    tombstone: false,
};

/// `causal peer id -> causal tip value digest`.
pub(crate) const DOMAIN_CAUSAL_TIP: TableDomain = TableDomain {
    id: 5,
    key_len: 16,
    value_len: 32,
    tombstone: false,
};

/// `document uuid -> document capsule record locator`. Overwritten on every
/// changed document and TOMBSTONED on document deletion.
pub(crate) const DOMAIN_DOCUMENT_ROSTER: TableDomain = TableDomain {
    id: 14,
    // A `DocumentKey` is 17 bytes (entity) or 33 (membership pair); a table
    // key is fixed width, so the domain is the wider of the two and an entity
    // key is zero-padded. The encoding stays injective because the tag byte
    // already separates the two address families.
    key_len: DOCUMENT_ROSTER_KEY_BYTES as u8,
    value_len: 32,
    tombstone: true,
};

/// The document roster's fixed key width: one `DocumentKey`, zero-padded.
pub(crate) const DOCUMENT_ROSTER_KEY_BYTES: usize = 33;

/// Frame one document address into the roster domain's fixed width.
pub(crate) fn framed_document_key(key: &[u8]) -> Result<[u8; DOCUMENT_ROSTER_KEY_BYTES], String> {
    if key.is_empty() || key.len() > DOCUMENT_ROSTER_KEY_BYTES {
        return Err("sealed document key width is outside the current domain".into());
    }
    let mut framed = [0_u8; DOCUMENT_ROSTER_KEY_BYTES];
    framed[..key.len()].copy_from_slice(key);
    Ok(framed)
}

/// Recover the exact document address from its framed form.
///
/// The tag byte decides the length, so the padding is unambiguous.
pub(crate) fn unframed_document_key(framed: &[u8]) -> Result<Vec<u8>, String> {
    if framed.len() != DOCUMENT_ROSTER_KEY_BYTES {
        return Err("framed sealed document key has the wrong width".into());
    }
    let len = match framed[0] {
        1 => 17,
        2 => 33,
        _ => return Err("framed sealed document key has an unknown address tag".into()),
    };
    if framed[len..].iter().any(|byte| *byte != 0) {
        return Err("framed sealed document key has non-zero padding".into());
    }
    Ok(framed[..len].to_vec())
}

/// `object content digest -> cold record locator`.
pub(crate) const DOMAIN_COLD_OBJECT: TableDomain = TableDomain {
    id: 15,
    key_len: 32,
    value_len: 32,
    tombstone: false,
};

/// `batch id -> cold manifest record locator`.
pub(crate) const DOMAIN_COLD_MANIFEST: TableDomain = TableDomain {
    id: 16,
    key_len: 16,
    value_len: 32,
    tombstone: false,
};

/// `capsule blob digest -> capsule record locator`.
///
/// Document images are content-addressed and shared between generations, so
/// the same blob published by an earlier cut is reused rather than re-appended
/// (I-25). They live in packs like every other record: nothing writes a file
/// into the sealed directory except pack publication and the marker swap.
pub(crate) const DOMAIN_CAPSULE_BLOB: TableDomain = TableDomain {
    id: 17,
    key_len: 32,
    value_len: 32,
    tombstone: false,
};

/// Identity admission keys are variable length (`AuthenticatedMapKey`, ≤ 48
/// bytes) and a table key is fixed width, so an identity key is framed as
/// `len:u8 ‖ key ‖ zero padding` — injective, and the identity domains are
/// point-looked-up and fully enumerated, never scanned in key order, so the
/// length-first ordering this induces is not observable.
pub(crate) const IDENTITY_KEY_BYTES: usize = 49;

/// `framed identity key -> admission value record locator`, all four kinds.
pub(crate) const fn identity_complete_domain(kind_index: u8) -> TableDomain {
    TableDomain {
        id: 6 + kind_index,
        key_len: IDENTITY_KEY_BYTES as u8,
        value_len: 32,
        tombstone: false,
    }
}

/// `framed identity key -> admission value record locator`, current only.
/// The one domain family with removals besides the document roster.
pub(crate) const fn identity_current_domain(kind_index: u8) -> TableDomain {
    TableDomain {
        id: 10 + kind_index,
        key_len: IDENTITY_KEY_BYTES as u8,
        value_len: 32,
        tombstone: true,
    }
}

/// Frame a variable-length identity key into this domain's fixed width.
pub(crate) fn framed_identity_key(key: &[u8]) -> Result<[u8; IDENTITY_KEY_BYTES], String> {
    if key.is_empty() || key.len() > IDENTITY_KEY_BYTES - 1 {
        return Err("sealed identity key width is outside the current domain".into());
    }
    let mut framed = [0_u8; IDENTITY_KEY_BYTES];
    framed[0] = key.len() as u8;
    framed[1..=key.len()].copy_from_slice(key);
    Ok(framed)
}

/// Recover the original identity key from its framed form.
pub(crate) fn unframed_identity_key(framed: &[u8]) -> Result<Vec<u8>, String> {
    if framed.len() != IDENTITY_KEY_BYTES {
        return Err("framed sealed identity key has the wrong width".into());
    }
    let len = framed[0] as usize;
    if len == 0 || len > IDENTITY_KEY_BYTES - 1 {
        return Err("framed sealed identity key declares an impossible length".into());
    }
    if framed[1 + len..].iter().any(|byte| *byte != 0) {
        return Err("framed sealed identity key has non-zero padding".into());
    }
    Ok(framed[1..=len].to_vec())
}

/// Every domain this archive may hold, in ascending id order.
///
/// A domain missing from this list is invisible to repair and to the
/// contract-consistency test, so the list is the single enumeration.
pub(crate) fn all_domains() -> Vec<TableDomain> {
    let mut domains = vec![
        DOMAIN_BATCH,
        DOMAIN_SEQUENCE,
        DOMAIN_DOCUMENT_CHANGE,
        DOMAIN_COVERED_OBJECT,
        DOMAIN_CAUSAL_TIP,
    ];
    for kind in 0..4 {
        domains.push(identity_complete_domain(kind));
    }
    for kind in 0..4 {
        domains.push(identity_current_domain(kind));
    }
    domains.push(DOMAIN_DOCUMENT_ROSTER);
    domains.push(DOMAIN_COLD_OBJECT);
    domains.push(DOMAIN_COLD_MANIFEST);
    domains.push(DOMAIN_CAPSULE_BLOB);
    domains.sort_by_key(|domain| domain.id);
    domains
}

fn domain_by_id(id: u8) -> Option<TableDomain> {
    all_domains().into_iter().find(|domain| domain.id == id)
}

fn pack_filename(pack: Uuid) -> String {
    format!("{COLD_PACK_PREFIX}-{pack}")
}

fn parse_pack_filename(name: &str) -> Option<Uuid> {
    let rest = name.strip_prefix(COLD_PACK_PREFIX)?.strip_prefix('-')?;
    let pack = Uuid::parse_str(rest).ok()?;
    (pack.to_string() == rest).then_some(pack)
}

pub(crate) fn cold_index_error(message: impl Into<String>) -> StoreError {
    StoreError::ColdHistoryIndexUnavailable(message.into())
}

fn cold_object_error(digest: ContentDigest, message: impl Into<String>) -> StoreError {
    StoreError::ColdObjectUnavailable {
        digest,
        reason: message.into(),
    }
}

fn cold_manifest_error(batch_id: BatchId, message: impl Into<String>) -> StoreError {
    StoreError::ColdManifestUnavailable {
        batch_id,
        reason: message.into(),
    }
}

fn cold_manifest_conflict(batch_id: BatchId, message: impl Into<String>) -> StoreError {
    StoreError::ColdManifestConflict {
        batch_id,
        reason: message.into(),
    }
}

/// Physical placement of one packed record in the archive's virtual byte space.
///
/// Deliberately exactly 32 bytes so it occupies a table's fixed value slot
/// directly. It carries NO pack name: see the module header — that is what lets
/// packs merge without rewriting one table entry.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ColdLocatorV1 {
    pub(crate) offset: u64,
    pub(crate) length: u64,
}

impl ColdLocatorV1 {
    pub(crate) fn to_bytes(self) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        bytes[..8].copy_from_slice(&self.offset.to_be_bytes());
        bytes[8..16].copy_from_slice(&self.length.to_be_bytes());
        bytes
    }

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Result<Self, String> {
        if bytes[16..].iter().any(|byte| *byte != 0) {
            return Err("cold locator has non-zero reserved bytes".into());
        }
        let mut offset = [0_u8; 8];
        offset.copy_from_slice(&bytes[..8]);
        let mut length = [0_u8; 8];
        length.copy_from_slice(&bytes[8..16]);
        let locator = Self {
            offset: u64::from_be_bytes(offset),
            length: u64::from_be_bytes(length),
        };
        if locator.length < COLD_RECORD_HEADER_BYTES as u64
            || locator.length > MAX_COLD_PACK_BYTES
            || locator.offset.checked_add(locator.length).is_none()
        {
            return Err("cold locator names an impossible pack range".into());
        }
        Ok(locator)
    }

    fn table_locator(self) -> TableLocator {
        TableLocator(self.to_bytes())
    }

    fn from_table_locator(locator: TableLocator) -> Result<Self, String> {
        Self::from_bytes(locator.0)
    }

    pub(crate) fn from_value(value: &[u8]) -> Result<Self, String> {
        let bytes: [u8; 32] = value
            .try_into()
            .map_err(|_| "cold locator value is not 32 bytes".to_owned())?;
        Self::from_bytes(bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColdPackFooterEntryV1 {
    class: u8,
    key: Vec<u8>,
    /// Virtual offset, not a file offset.
    offset: u64,
    length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColdPackFooterV1 {
    schema: u32,
    /// The virtual range this pack's body covers, `[start, start + body_len)`.
    virtual_start: u64,
    /// This pack's tier level. `FANOUT` packs of one level merge into one of
    /// the next.
    level: u8,
    entries: Vec<ColdPackFooterEntryV1>,
}

/// One pack as the root's pack table names it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackRefV1 {
    virtual_start: u64,
    virtual_end: u64,
    pack: [u8; 16],
    level: u8,
}

impl PackRefV1 {
    fn pack_id(&self) -> Uuid {
        Uuid::from_bytes(self.pack)
    }
}

/// The pack table: every live pack, ordered by virtual start, ranges disjoint
/// and contiguous.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackTableV1 {
    packs: Vec<PackRefV1>,
}

impl PackTableV1 {
    fn validate(&self) -> Result<(), String> {
        let mut expected = None;
        for pack in &self.packs {
            if pack.virtual_end <= pack.virtual_start {
                return Err("sealed pack table names an empty pack".into());
            }
            if let Some(expected) = expected {
                if pack.virtual_start != expected {
                    return Err("sealed pack table is not contiguous".into());
                }
            }
            expected = Some(pack.virtual_end);
        }
        Ok(())
    }

    fn next_virtual(&self) -> u64 {
        self.packs.last().map_or(0, |pack| pack.virtual_end)
    }

    /// The pack holding `offset`, by binary search.
    fn locate(&self, offset: u64) -> Option<PackRefV1> {
        let index = self
            .packs
            .partition_point(|pack| pack.virtual_end <= offset)
            .min(self.packs.len().saturating_sub(1));
        let pack = *self.packs.get(index)?;
        (pack.virtual_start <= offset && offset < pack.virtual_end).then_some(pack)
    }

    pub(crate) fn len(&self) -> usize {
        self.packs.len()
    }

    pub(crate) fn levels(&self) -> usize {
        self.packs
            .iter()
            .map(|pack| usize::from(pack.level) + 1)
            .max()
            .unwrap_or(0)
    }

    /// Every pack this table names, in virtual order.
    pub(crate) fn entries(&self) -> &[PackRefV1] {
        &self.packs
    }

    pub(crate) fn live_bytes(&self) -> u64 {
        self.packs
            .iter()
            .map(|pack| pack.virtual_end - pack.virtual_start)
            .sum()
    }
}

/// What one marker resolves to: the index as the archive currently holds it.
///
/// The pack table lives in the MARKER, not in the root record, because the pack
/// table is what resolves a locator — including the root record's own locator.
/// Putting it in the record it is needed to read would force every open to scan
/// every pack footer to bootstrap itself.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SealedRootRecord {
    pub(crate) tables: SealedTableRoot,
    pub(crate) packs: PackTableV1,
}

impl SealedRootRecord {
    /// `sha256` of `tine-storage`'s canonical table root. The marker commits to
    /// this, and an anchored SQLite frontier folds it in.
    pub(crate) fn table_root_digest(&self) -> Result<ContentDigest, String> {
        self.tables.root_digest().map_err(|error| error.to_string())
    }
}

/// The `current` marker: the pack table, the root record's locator and the
/// table-root digest it commits to.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SealedMarkerV1 {
    schema: u32,
    packs: PackTableV1,
    root_locator: [u8; 32],
    table_root_digest: ContentDigest,
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(value).map_err(|error| error.to_string())
}

fn decode_canonical<T: for<'de> Deserialize<'de> + Serialize>(bytes: &[u8]) -> Result<T, String> {
    let (value, trailing): (T, &[u8]) =
        postcard::take_from_bytes(bytes).map_err(|error| error.to_string())?;
    if !trailing.is_empty() || encode_canonical(&value)? != bytes {
        return Err("sealed archive value is noncanonical".into());
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Directory access
// ---------------------------------------------------------------------------

pub(crate) fn sealed_directory(store: &ObjectStore) -> Result<Dir, StoreError> {
    let root = store.private_derived_root_capability()?;
    super::object_store::ensure_directory_nofollow(&root, SEALED_DIRECTORY)?;
    super::object_store::open_dir_nofollow(&root, SEALED_DIRECTORY)
}

pub(crate) fn open_existing_sealed_directory(
    store: &ObjectStore,
) -> Result<Option<Dir>, StoreError> {
    let root = store.private_derived_root_capability()?;
    tine_storage::open_existing_dir_nofollow(&root, SEALED_DIRECTORY)
        .map_err(filesystem_error_without_collision)
}

fn read_root_marker_bytes(directory: &Dir) -> Result<Option<Vec<u8>>, StoreError> {
    super::object_store::read_optional_regular(
        directory,
        SEALED_ROOT_MARKER,
        MAX_SEALED_MARKER_BYTES,
        None,
    )
}

fn contains_any_pack(directory: &Dir) -> Result<bool, StoreError> {
    for entry in directory
        .entries()
        .map_err(|error| cold_index_error(error.to_string()))?
    {
        let entry = entry.map_err(|error| cold_index_error(error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if parse_pack_filename(name).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What the sealed directory's *derived* marker says about this archive.
///
/// Never-initialized absence is not the same state as a lost marker over
/// preserved packs. Conflating them is what lets a fresh empty-based root be
/// published over old history.
enum SealedRootState {
    NeverInitialized,
    Published {
        marker: Vec<u8>,
        root: SealedRootRecord,
    },
    /// Self-describing packs survive but their derived root is gone or damaged.
    /// A named damaged state; `repair_cold_history_root` recovers it.
    RootLostWithPreservedPacks,
}

/// Fire the one-shot interleaving fault between the marker read and the root
/// resolution in [`read_root_state`], if a test armed one.
///
/// It exists because the window it models — marker installed, superseded packs
/// retired a moment later — is sub-millisecond in production and cannot be
/// raced for reliably. In a shipped build this is an empty call: the hook lives
/// in the test module, so nothing test-only crosses into production state.
#[cfg(test)]
fn sealed_torn_root_read_fault_for_test() {
    let hook = tests::TORN_ROOT_READ_HOOK.lock().unwrap().take();
    if let Some(mut hook) = hook {
        hook();
    }
}

#[cfg(not(test))]
fn sealed_torn_root_read_fault_for_test() {}

/// How many times a torn marker read is re-read before the archive is called
/// damaged. Each retry observes a strictly newer marker (marker-last ordering
/// means a marker only moves when a cut committed), so the loop terminates;
/// the bound exists so a pathological writer cannot starve a reader forever.
const SEALED_ROOT_READ_ATTEMPTS: usize = 8;

fn read_root_state(directory: &Dir) -> Result<SealedRootState, StoreError> {
    let mut attempt = 0;
    loop {
        let Some(marker) = read_root_marker_bytes(directory)? else {
            return if contains_any_pack(directory)? {
                Ok(SealedRootState::RootLostWithPreservedPacks)
            } else {
                Ok(SealedRootState::NeverInitialized)
            };
        };
        sealed_torn_root_read_fault_for_test();
        let error = match decode_marker_and_root(directory, &marker) {
            Ok(root) => return Ok(SealedRootState::Published { marker, root }),
            Err(error) => error,
        };
        // Refusal scenario (I-2): a cut installed its marker and retired the
        // packs the marker we just read named, between our two reads. The
        // archive is intact; our snapshot of it is not. Re-read the marker: if
        // it moved, that is exactly what happened and the retry resolves
        // against the newer one. Only a marker that has NOT moved across the
        // failure is evidence of damage.
        attempt += 1;
        if attempt < SEALED_ROOT_READ_ATTEMPTS
            && read_root_marker_bytes(directory)?.as_deref() != Some(marker.as_slice())
        {
            continue;
        }
        // Derived state over surviving packs: damaged, not absent, and never a
        // graph-open refusal -- repair rebuilds every table from pack footers
        // (D-3, P4c2 4.3).
        return if contains_any_pack(directory)? {
            Ok(SealedRootState::RootLostWithPreservedPacks)
        } else {
            Err(error)
        };
    }
}

fn decode_marker_and_root(directory: &Dir, marker: &[u8]) -> Result<SealedRootRecord, StoreError> {
    let marker: SealedMarkerV1 = decode_canonical(marker).map_err(cold_index_error)?;
    if marker.schema != COLD_SCHEMA_VERSION {
        return Err(cold_index_error(
            "sealed root marker is not the current schema",
        ));
    }
    marker.packs.validate().map_err(cold_index_error)?;
    let locator = ColdLocatorV1::from_bytes(marker.root_locator).map_err(cold_index_error)?;
    let bytes = read_pack_range(directory, &marker.packs, locator, MAX_COLD_ROOT_BYTES)
        .map_err(cold_index_error)?;
    let tables =
        SealedTableRoot::decode(&bytes).map_err(|error| cold_index_error(error.to_string()))?;
    let root = SealedRootRecord {
        tables,
        packs: marker.packs,
    };
    if root.table_root_digest().map_err(cold_index_error)? != marker.table_root_digest {
        return Err(cold_index_error(
            "sealed root record does not match the digest its marker commits to",
        ));
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// Ranged pack reads
// ---------------------------------------------------------------------------

/// Read exactly one packed record's payload, proving it against the record
/// header's digest. Knows nothing about what the payload means, so it serves
/// object records, manifest records, tables and roots alike.
fn read_pack_range(
    directory: &Dir,
    packs: &PackTableV1,
    locator: ColdLocatorV1,
    payload_limit: u64,
) -> Result<Vec<u8>, String> {
    if locator.length > payload_limit.saturating_add(COLD_RECORD_HEADER_BYTES as u64) {
        return Err("cold record exceeds its class byte limit".into());
    }
    let pack = packs
        .locate(locator.offset)
        .ok_or("cold locator names no live pack")?;
    let end = locator
        .offset
        .checked_add(locator.length)
        .ok_or("cold locator range overflows")?;
    if end > pack.virtual_end {
        return Err("cold locator range crosses a pack boundary".into());
    }
    let name = pack_filename(pack.pack_id());
    match directory.symlink_metadata(&name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(format!("cold pack {name} is not a regular no-follow file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(format!("cold pack {name} is missing"));
        }
        Err(error) => return Err(error.to_string()),
    }
    let mut file = tine_storage::open_file_nofollow(directory, &name).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            format!("cold pack {name} is missing")
        } else {
            error.to_string()
        }
    })?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err(format!("cold pack {name} is not a regular no-follow file"));
    }
    let file_offset = locator.offset - pack.virtual_start;
    if file_offset + locator.length > metadata.len() {
        return Err(format!(
            "cold pack {name} is shorter than its locator range"
        ));
    }
    file.seek(SeekFrom::Start(file_offset))
        .map_err(|error| error.to_string())?;
    let mut raw = vec![0_u8; locator.length as usize];
    file.read_exact(&mut raw)
        .map_err(|error| error.to_string())?;
    let (digest, payload_start) = parse_record_header(&raw)?;
    let payload = raw[payload_start..].to_vec();
    if ContentDigest::of(&payload) != digest {
        return Err("cold record bytes differ from their record digest".into());
    }
    Ok(payload)
}

/// Overwrite one packed record's payload bytes in place, for damage fixtures.
///
/// There is no per-object file to truncate any more: a record lives inside a
/// pack at a virtual offset. Tearing it means writing garbage over exactly its
/// payload range, which is what a torn write or a partial sync delivery does
/// to a pack. The record header keeps the ORIGINAL payload digest, so every
/// reader of this record fails its digest check.
#[cfg(test)]
pub(crate) fn tear_packed_record_for_test(
    directory: &Dir,
    locator: ColdLocatorV1,
) -> Result<(), String> {
    use std::io::Write;

    // The CURRENT marker, not a caller's snapshot: a later cut may have merged
    // the pack this locator named. Virtual offsets never move, so the current
    // table still resolves it.
    let packs = match read_root_state(directory).map_err(|error| error.to_string())? {
        SealedRootState::Published { root, .. } => root.packs,
        _ => return Err("sealed archive has published no pack table".into()),
    };
    let pack = packs
        .locate(locator.offset)
        .ok_or("cold locator names no live pack")?;
    let name = pack_filename(pack.pack_id());
    let mut file = directory
        .open_with(&name, cap_std::fs::OpenOptions::new().write(true))
        .map_err(|error| error.to_string())?;
    let payload_len = locator.length - COLD_RECORD_HEADER_BYTES as u64;
    let payload_start = locator.offset - pack.virtual_start + COLD_RECORD_HEADER_BYTES as u64;
    file.seek(SeekFrom::Start(payload_start))
        .map_err(|error| error.to_string())?;
    file.write_all(&vec![0xA5_u8; payload_len as usize])
        .map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}

/// Validate a packed record header and return `(payload digest, payload start)`.
fn parse_record_header(raw: &[u8]) -> Result<(ContentDigest, usize), String> {
    if raw.len() < COLD_RECORD_HEADER_BYTES {
        return Err("cold record is shorter than its header".into());
    }
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&raw[..32]);
    let mut length = [0_u8; 8];
    length.copy_from_slice(&raw[32..COLD_RECORD_HEADER_BYTES]);
    let payload_len = u64::from_be_bytes(length);
    if payload_len != (raw.len() - COLD_RECORD_HEADER_BYTES) as u64 {
        return Err("cold record header length differs from its locator range".into());
    }
    Ok((ContentDigest::from_bytes(digest), COLD_RECORD_HEADER_BYTES))
}

// ---------------------------------------------------------------------------
// Pack inventory and the footer-derived pack table
// ---------------------------------------------------------------------------

struct PackInventory {
    pack: Uuid,
    footer: ColdPackFooterV1,
    body_len: u64,
}

fn read_pack_footer(directory: &Dir, pack: Uuid) -> Result<PackInventory, StoreError> {
    let name = pack_filename(pack);
    let mut file = tine_storage::open_file_nofollow(directory, &name)
        .map_err(|error| cold_index_error(format!("cold pack {name}: {error}")))?;
    let length = file
        .metadata()
        .map_err(|error| cold_index_error(error.to_string()))?
        .len();
    let trailer = (COLD_PACK_MAGIC.len() + 8) as u64;
    if length < trailer || length > MAX_COLD_PACK_BYTES {
        return Err(cold_index_error(format!(
            "cold pack {name} is not a current pack file"
        )));
    }
    file.seek(SeekFrom::Start(length - trailer))
        .map_err(|error| cold_index_error(error.to_string()))?;
    let mut tail = [0_u8; 16];
    file.read_exact(&mut tail)
        .map_err(|error| cold_index_error(error.to_string()))?;
    if tail[8..] != COLD_PACK_MAGIC {
        return Err(cold_index_error(format!(
            "cold pack {name} has no current pack trailer"
        )));
    }
    let mut footer_len = [0_u8; 8];
    footer_len.copy_from_slice(&tail[..8]);
    let footer_len = u64::from_be_bytes(footer_len);
    if footer_len > MAX_COLD_PACK_FOOTER_BYTES || footer_len + trailer > length {
        return Err(cold_index_error(format!(
            "cold pack {name} footer length is impossible"
        )));
    }
    file.seek(SeekFrom::Start(length - trailer - footer_len))
        .map_err(|error| cold_index_error(error.to_string()))?;
    let mut footer_bytes = vec![0_u8; footer_len as usize];
    file.read_exact(&mut footer_bytes)
        .map_err(|error| cold_index_error(error.to_string()))?;
    let footer: ColdPackFooterV1 = decode_canonical(&footer_bytes).map_err(cold_index_error)?;
    if footer.schema != COLD_SCHEMA_VERSION {
        return Err(cold_index_error(format!(
            "cold pack {name} footer is not the current schema"
        )));
    }
    Ok(PackInventory {
        pack,
        body_len: length - trailer - footer_len,
        footer,
    })
}

fn cold_pack_names(directory: &Dir) -> Result<BTreeSet<Uuid>, StoreError> {
    let mut packs = BTreeSet::new();
    for entry in directory
        .entries()
        .map_err(|error| cold_index_error(error.to_string()))?
    {
        let entry = entry.map_err(|error| cold_index_error(error.to_string()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(pack) = parse_pack_filename(name) else {
            continue;
        };
        super::object_store::require_regular_entry(
            &entry
                .file_type()
                .map_err(|error| cold_index_error(error.to_string()))?,
            name,
        )?;
        packs.insert(pack);
    }
    Ok(packs)
}

/// Rebuild the pack table from the packs' own footers.
///
/// Every pack declares its virtual range and level, so the table is derived
/// state like every other index object (D-3). Overlapping ranges mean a
/// superseded pack survived a crash between marker installation and retirement:
/// the pack at the HIGHER level wins, because it is the successor the marker
/// already named.
fn pack_table_from_inventories(inventories: &[PackInventory]) -> PackTableV1 {
    let mut refs: Vec<PackRefV1> = inventories
        .iter()
        .map(|inventory| PackRefV1 {
            virtual_start: inventory.footer.virtual_start,
            virtual_end: inventory.footer.virtual_start + inventory.body_len,
            pack: *inventory.pack.as_bytes(),
            level: inventory.footer.level,
        })
        .collect();
    // Highest level first at a given start, so a surviving superseded run loses
    // to the merged successor that covers it.
    refs.sort_by(|left, right| {
        left.virtual_start
            .cmp(&right.virtual_start)
            .then(right.level.cmp(&left.level))
    });
    let mut table = PackTableV1::default();
    let mut covered = 0_u64;
    for candidate in refs {
        if candidate.virtual_start < covered {
            continue;
        }
        covered = candidate.virtual_end;
        table.packs.push(candidate);
    }
    table
}

// ---------------------------------------------------------------------------
// The read handle
// ---------------------------------------------------------------------------

/// Exact physical work one sealed lookup performed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ColdReadWork {
    /// Table records loaded (each costs one file open and one digest check).
    pub(crate) index_nodes: usize,
    pub(crate) pack_reads: usize,
}

/// Point access to the sealed archive. Carries no hot-tier authority: the
/// caller consults this only after the hot original name is absent.
pub(crate) struct SealedArchiveReader {
    directory: Dir,
    root: SealedRootRecord,
    marker: Vec<u8>,
    /// The PHYSICAL placement this reader currently believes, refreshed from
    /// the marker when a named pack has been retired underneath it.
    ///
    /// Separate from `root.packs` on purpose: `root` is the LOGICAL generation
    /// this reader was opened at and never changes, while placement does. A
    /// tier merge concatenates a contiguous virtual run, so a record's virtual
    /// offset never moves; only the file it lives in does.
    packs: RwLock<PackTableV1>,
    pack_reads: AtomicUsize,
    table_reads: AtomicUsize,
}

impl TableBytes for SealedArchiveReader {
    fn read_table<'a>(
        &'a self,
        locator: &TableLocator,
    ) -> Result<Cow<'a, [u8]>, SealedAcceptedIndexError> {
        let locator = ColdLocatorV1::from_table_locator(*locator)
            .map_err(SealedAcceptedIndexError::Corrupt)?;
        self.table_reads.fetch_add(1, Ordering::Relaxed);
        self.record_bytes(locator, MAX_COLD_ROOT_BYTES)
            .map(Cow::Owned)
            .map_err(SealedAcceptedIndexError::Corrupt)
    }
}

impl SealedArchiveReader {
    /// `Ok(None)` when this archive has never published sealed state. That is
    /// ordinary absence, never a refusal.
    pub(crate) fn open(store: &ObjectStore) -> Result<Option<Self>, StoreError> {
        let Some(directory) = open_existing_sealed_directory(store)? else {
            return Ok(None);
        };
        Self::open_directory(&directory)
    }

    pub(crate) fn open_directory(directory: &Dir) -> Result<Option<Self>, StoreError> {
        match read_root_state(directory)? {
            SealedRootState::NeverInitialized => Ok(None),
            SealedRootState::RootLostWithPreservedPacks => Err(StoreError::ColdHistoryRootMissing),
            SealedRootState::Published { marker, root } => Ok(Some(Self {
                directory: directory
                    .try_clone()
                    .map_err(|error| cold_index_error(error.to_string()))?,
                packs: RwLock::new(root.packs.clone()),
                root,
                marker,
                pack_reads: AtomicUsize::new(0),
                table_reads: AtomicUsize::new(0),
            })),
        }
    }

    /// Reopen this archive at an OLDER root record.
    ///
    /// The marker's pack table is the authority for resolving any locator ever
    /// published: a tier merge concatenates a contiguous virtual run, so a
    /// record's virtual offset never moves and an earlier generation's root
    /// record stays resolvable. That is what lets the two-slot checkpoint
    /// protocol keep its rollback slot after the sealed marker has advanced.
    pub(crate) fn open_directory_at(
        directory: &Dir,
        root_locator: ColdLocatorV1,
    ) -> Result<Option<Self>, StoreError> {
        let Some(current) = Self::open_directory(directory)? else {
            return Ok(None);
        };
        let marker: SealedMarkerV1 =
            decode_canonical(current.marker_bytes()).map_err(cold_index_error)?;
        if root_locator
            == ColdLocatorV1::from_bytes(marker.root_locator).map_err(cold_index_error)?
        {
            return Ok(Some(current));
        }
        let bytes = current
            .record_bytes(root_locator, MAX_COLD_ROOT_BYTES)
            .map_err(cold_index_error)?;
        let tables =
            SealedTableRoot::decode(&bytes).map_err(|error| cold_index_error(error.to_string()))?;
        Ok(Some(Self {
            root: SealedRootRecord {
                tables,
                packs: current.root.packs.clone(),
            },
            packs: RwLock::new(current.root.packs.clone()),
            ..current
        }))
    }

    pub(crate) fn root(&self) -> &SealedRootRecord {
        &self.root
    }

    pub(crate) fn marker_bytes(&self) -> &[u8] {
        &self.marker
    }

    pub(crate) fn work(&self) -> ColdReadWork {
        ColdReadWork {
            index_nodes: self.table_reads.load(Ordering::Relaxed),
            pack_reads: self.pack_reads.load(Ordering::Relaxed),
        }
    }

    pub(crate) fn record_bytes(
        &self,
        locator: ColdLocatorV1,
        limit: u64,
    ) -> Result<Vec<u8>, String> {
        self.pack_reads.fetch_add(1, Ordering::Relaxed);
        let believed = self
            .packs
            .read()
            .map_err(|_| "sealed pack table lock is poisoned".to_owned())?
            .clone();
        let first = match read_pack_range(&self.directory, &believed, locator, limit) {
            Ok(bytes) => return Ok(bytes),
            Err(error) => error,
        };
        // Refusal scenario (I-8, I-2): a cut published after this reader opened
        // MERGED the pack this locator named and retired the original, so the
        // file is gone while the record is not. The virtual byte space is
        // stable by construction -- a merge concatenates a contiguous run, so
        // offsets never move -- which is exactly what makes the CURRENT marker's
        // pack table able to resolve any locator ever published. Re-reading it
        // is recovery, not re-authentication (D-3): the record header's digest
        // still proves the bytes. Without this an honest concurrent reader held
        // across a cut turns a live record into "cold pack ... is missing".
        let current = match read_root_state(&self.directory) {
            Ok(SealedRootState::Published { root, .. }) => root.packs,
            _ => return Err(first),
        };
        if current == believed {
            return Err(first);
        }
        let bytes = read_pack_range(&self.directory, &current, locator, limit)?;
        if let Ok(mut packs) = self.packs.write() {
            *packs = current;
        }
        Ok(bytes)
    }

    /// A reader over one domain's table list, newest first.
    pub(crate) fn tables(&self, domain: TableDomain) -> Result<TableSetReader<'_, Self>, String> {
        TableSetReader::new(self, domain, self.root.tables.tables_for(domain))
            .map_err(|error| error.to_string())
    }

    /// Every live entry of one domain, in key order.
    ///
    /// Only enumeration consumers reach this: the identity current-roots
    /// rebuild and cold-manifest inventory. A point read never does.
    pub(crate) fn domain_entries(
        &self,
        domain: TableDomain,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        domain_live_entries(self, domain, &self.root.tables)
    }

    // -- the cold whole-object tier -----------------------------------------

    fn locate_object(&self, digest: ContentDigest) -> Result<Option<ColdLocatorV1>, StoreError> {
        let tables = self
            .tables(DOMAIN_COLD_OBJECT)
            .map_err(|error| cold_object_error(digest, error))?;
        let Some(value) = tables
            .get(digest.as_bytes())
            .map_err(|error| cold_object_error(digest, error.to_string()))?
        else {
            return Ok(None);
        };
        // `read_table` already counts every table record this lookup loaded;
        // adding the reader's digest verifications on top counted each one
        // twice and made `index_nodes` exceed `pack_reads`.
        ColdLocatorV1::from_value(&value)
            .map(Some)
            .map_err(|error| cold_object_error(digest, error))
    }

    /// Resolve one logical object's exact canonical bytes.
    pub(crate) fn object_bytes(
        &self,
        digest: ContentDigest,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(locator) = self.locate_object(digest)? else {
            return Ok(None);
        };
        let payload = self
            .record_bytes(locator, MAX_OBJECT_BYTES as u64)
            .map_err(|error| cold_object_error(digest, error))?;
        if ContentDigest::of(&payload) != digest {
            return Err(cold_object_error(
                digest,
                "cold record resolves to another logical object",
            ));
        }
        Ok(Some(payload))
    }

    fn locate_manifest(&self, batch_id: BatchId) -> Result<Option<ColdLocatorV1>, StoreError> {
        let tables = self
            .tables(DOMAIN_COLD_MANIFEST)
            .map_err(|error| cold_manifest_error(batch_id, error))?;
        let Some(value) = tables
            .get(batch_id.as_uuid().as_bytes())
            .map_err(|error| cold_manifest_error(batch_id, error.to_string()))?
        else {
            return Ok(None);
        };
        ColdLocatorV1::from_value(&value)
            .map(Some)
            .map_err(|error| cold_manifest_error(batch_id, error))
    }

    /// Resolve one batch manifest's exact canonical bytes.
    pub(crate) fn manifest_bytes(&self, batch_id: BatchId) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(locator) = self.locate_manifest(batch_id)? else {
            return Ok(None);
        };
        self.record_bytes(locator, MAX_MANIFEST_BYTES as u64)
            .map(Some)
            .map_err(|error| cold_manifest_error(batch_id, error))
    }

    /// Enumerate the committed cold manifest membership from the sealed index
    /// itself. Full-history reconstruction is the one consumer allowed to pay
    /// this lifetime-sized walk.
    pub(crate) fn manifest_batch_ids(&self) -> Result<BTreeSet<BatchId>, StoreError> {
        let entries = self
            .domain_entries(DOMAIN_COLD_MANIFEST)
            .map_err(cold_index_error)?;
        let mut batches = BTreeSet::new();
        for key in entries.keys() {
            let bytes: [u8; 16] = key
                .as_slice()
                .try_into()
                .map_err(|_| cold_index_error("cold manifest domain contains a non-BatchId key"))?;
            if !batches.insert(BatchId::from_uuid(Uuid::from_bytes(bytes))) {
                return Err(cold_index_error(
                    "cold manifest domain repeats a BatchId identity",
                ));
            }
        }
        Ok(batches)
    }
}

// ---------------------------------------------------------------------------
// Publication: one cut
// ---------------------------------------------------------------------------

/// One in-progress pack. Records are appended in memory up to the construction
/// target and the pack is published as soon as it is sealed, so publication
/// memory is bounded by that target plus one oversize record.
struct ColdPackBuilder {
    pack: Uuid,
    virtual_start: u64,
    level: u8,
    bytes: Vec<u8>,
    entries: Vec<ColdPackFooterEntryV1>,
}

impl ColdPackBuilder {
    fn new(virtual_start: u64, level: u8) -> Self {
        Self {
            pack: Uuid::new_v4(),
            virtual_start,
            level,
            bytes: Vec::new(),
            entries: Vec::new(),
        }
    }

    fn append(&mut self, class: u8, key: Vec<u8>, payload: &[u8]) -> Result<ColdLocatorV1, String> {
        let offset = self.bytes.len() as u64;
        self.bytes
            .extend_from_slice(ContentDigest::of(payload).as_bytes());
        self.bytes
            .extend_from_slice(&(payload.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(payload);
        let length = self.bytes.len() as u64 - offset;
        if self.bytes.len() as u64 > MAX_COLD_PACK_BYTES {
            return Err("cold pack exceeds its physical record limit".into());
        }
        let locator = ColdLocatorV1 {
            offset: self.virtual_start + offset,
            length,
        };
        self.entries.push(ColdPackFooterEntryV1 {
            class,
            key,
            offset: locator.offset,
            length,
        });
        Ok(locator)
    }

    /// Copy one already-encoded record range verbatim. Used only by a pack
    /// merge, which must preserve every virtual offset.
    fn append_verbatim(&mut self, entry: &ColdPackFooterEntryV1, raw: &[u8]) -> Result<(), String> {
        if self.virtual_start + self.bytes.len() as u64 != entry.offset {
            return Err("pack merge would move a record's virtual offset".into());
        }
        if raw.len() as u64 != entry.length {
            return Err("pack merge record length differs from its footer entry".into());
        }
        self.bytes.extend_from_slice(raw);
        self.entries.push(entry.clone());
        Ok(())
    }

    fn body_len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn finish(mut self) -> Result<(PackRefV1, Vec<u8>), String> {
        let body_len = self.body_len();
        let footer = encode_canonical(&ColdPackFooterV1 {
            schema: COLD_SCHEMA_VERSION,
            virtual_start: self.virtual_start,
            level: self.level,
            entries: self.entries,
        })?;
        if footer.len() as u64 > MAX_COLD_PACK_FOOTER_BYTES {
            return Err("cold pack footer exceeds its physical limit".into());
        }
        self.bytes.extend_from_slice(&footer);
        self.bytes
            .extend_from_slice(&(footer.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(&COLD_PACK_MAGIC);
        Ok((
            PackRefV1 {
                virtual_start: self.virtual_start,
                virtual_end: self.virtual_start + body_len,
                pack: *self.pack.as_bytes(),
                level: self.level,
            },
            self.bytes,
        ))
    }
}

/// Every live entry of one domain, in key order, over one root's table list.
///
/// Only enumeration consumers reach this: the identity current-roots rebuild
/// and the cold-manifest inventory. A point read never does.
fn domain_live_entries<Provider: TableBytes>(
    provider: &Provider,
    domain: TableDomain,
    root: &SealedTableRoot,
) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
    let mut live: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut shadowed: BTreeSet<Vec<u8>> = BTreeSet::new();
    // Newest first: the first table that names a key decides it.
    for table in root.tables_for(domain) {
        let bytes = provider
            .read_table(&table.locator)
            .map_err(|error| error.to_string())?;
        let view = TableView::decode(domain, &bytes).map_err(|error| error.to_string())?;
        for (key, value) in view.iter() {
            if shadowed.contains(key) {
                continue;
            }
            shadowed.insert(key.to_vec());
            if !domain.is_tombstone(value) {
                live.insert(key.to_vec(), value.to_vec());
            }
        }
    }
    Ok(live)
}

/// Exactly where a publication may be interrupted.
///
/// One variant per durable boundary in `publish_with_kill`, in protocol order.
/// A crash at any of these must leave the PREDECESSOR openable and a retry
/// able to complete: that is what marker-last publication buys (I-2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SealedCutKill {
    /// Payload packs on disk; no table, no root, no marker.
    AfterPayloadPacks,
    /// Delta/merged tables on disk; no root record, no marker.
    AfterTables,
    /// The root record is on disk; the marker still names the predecessor.
    AfterRootRecord,
    /// The marker names this cut; superseded packs are not yet retired.
    AfterMarkerBeforeRetire,
    /// A tier merge published its merged pack; the pack table that would name
    /// it is not durable, the inputs are not retired, and the marker still
    /// names the predecessor. The merged pack is unreferenced residue.
    MidPackTierMerge,
}

/// What one published cut cost, in the terms I-14 and I-25 are stated in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SealedCutWork {
    /// Where this cut's root record landed. A checkpoint generation records
    /// this so its rollback slot keeps resolving after the marker advances.
    pub(crate) root_locator: ColdLocatorV1,
    pub(crate) root_digest: ContentDigest,
    pub(crate) index_bytes: u64,
    pub(crate) payload_bytes: u64,
    pub(crate) packs_published: usize,
    pub(crate) packs_retired: usize,
    pub(crate) tables_written: usize,
    pub(crate) tables_merged: usize,
    pub(crate) merged_pack_bytes: u64,
    pub(crate) files_written: usize,
}

impl Default for SealedCutWork {
    fn default() -> Self {
        Self {
            root_locator: ColdLocatorV1::default(),
            // A cut that has published nothing has no root record; this is the
            // digest of the empty byte string, never a claimed root.
            root_digest: ContentDigest::of(&[]),
            index_bytes: 0,
            payload_bytes: 0,
            packs_published: 0,
            packs_retired: 0,
            tables_written: 0,
            tables_merged: 0,
            merged_pack_bytes: 0,
            files_written: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ColdPublicationOutcome {
    pub(crate) objects_published: usize,
    pub(crate) manifests_published: usize,
    pub(crate) objects_already_present: usize,
    pub(crate) manifests_already_present: usize,
    pub(crate) packs_published: usize,
}

/// One sealed cut: new payload records, new domain entries, one new pack, one
/// new root record, one marker swap.
///
/// Everything staged is unreferenced residue until the marker installs. A crash
/// before that leaves the predecessor root — or no root at all — exactly as
/// authoritative as it was (I-2, T5).
pub(crate) struct SealedCut {
    directory: Dir,
    base: SealedRootRecord,
    prior_marker: Option<Vec<u8>>,
    builder: ColdPackBuilder,
    published: Vec<PackRefV1>,
    entries: BTreeMap<u8, BTreeMap<Vec<u8>, Vec<u8>>>,
    /// Records staged by THIS cut, so a builder can read its own writes before
    /// the pack is sealed.
    staged_records: BTreeMap<ColdLocatorV1, Vec<u8>>,
    work: SealedCutWork,
    table_reads: AtomicUsize,
}

impl SealedCut {
    /// Table records this cut has read, in the unit I-14 counts: one table
    /// record is one digest-checked read, staged or packed.
    pub(crate) fn index_reads(&self) -> u64 {
        self.table_reads.load(Ordering::Relaxed) as u64
    }
}

impl TableBytes for SealedCut {
    fn read_table<'a>(
        &'a self,
        locator: &TableLocator,
    ) -> Result<Cow<'a, [u8]>, SealedAcceptedIndexError> {
        let locator = ColdLocatorV1::from_table_locator(*locator)
            .map_err(SealedAcceptedIndexError::Corrupt)?;
        self.table_reads.fetch_add(1, Ordering::Relaxed);
        self.read_staged_or_packed(locator)
            .map(Cow::Owned)
            .map_err(|error| SealedAcceptedIndexError::Corrupt(error.to_string()))
    }
}

impl SealedCut {
    pub(crate) fn open(directory: &Dir) -> Result<Self, StoreError> {
        let (prior_marker, base) = match read_root_state(directory)? {
            SealedRootState::Published { marker, root } => (Some(marker), root),
            SealedRootState::NeverInitialized => (None, SealedRootRecord::default()),
            // Publishing a fresh empty-based root would silently orphan old
            // history. Repair is explicit.
            SealedRootState::RootLostWithPreservedPacks => {
                return Err(StoreError::ColdHistoryRootMissing)
            }
        };
        // Retire residue from an interrupted predecessor cut BEFORE allocating
        // this cut's virtual range. A pack the marker does not name is either
        // an unfinished cut's orphan or a merged pack's already-superseded
        // source; leaving an orphan behind would let this cut allocate the same
        // virtual range, and a later footer-only repair could then not tell the
        // two apart.
        if prior_marker.is_some() {
            retire_unreferenced_packs(directory, &base.packs)?;
        }
        Self::over(directory, base, prior_marker)
    }

    /// Start a cut over an explicit base. Repair uses this with an empty base.
    fn over(
        directory: &Dir,
        base: SealedRootRecord,
        prior_marker: Option<Vec<u8>>,
    ) -> Result<Self, StoreError> {
        let next_virtual = base.packs.next_virtual();
        Ok(Self {
            directory: directory
                .try_clone()
                .map_err(|error| cold_index_error(error.to_string()))?,
            base,
            prior_marker,
            builder: ColdPackBuilder::new(next_virtual, 0),
            published: Vec::new(),
            entries: BTreeMap::new(),
            staged_records: BTreeMap::new(),
            work: SealedCutWork::default(),
            table_reads: AtomicUsize::new(0),
        })
    }

    pub(crate) fn base_root(&self) -> &SealedRootRecord {
        &self.base
    }

    /// Append one payload record. Its bytes are durable when the pack that
    /// holds it is sealed, which always happens before the marker installs.
    pub(crate) fn add_record(
        &mut self,
        class: u8,
        key: Vec<u8>,
        payload: &[u8],
    ) -> Result<ColdLocatorV1, StoreError> {
        let locator = self
            .builder
            .append(class, key, payload)
            .map_err(cold_index_error)?;
        self.staged_records.insert(locator, payload.to_vec());
        if class == COLD_CLASS_TABLE || class == COLD_CLASS_ROOT {
            self.work.index_bytes = self.work.index_bytes.saturating_add(locator.length);
        } else {
            self.work.payload_bytes = self.work.payload_bytes.saturating_add(locator.length);
        }
        if self.builder.body_len() as usize >= COLD_PACK_TARGET_BYTES {
            self.seal_current_pack()?;
        }
        Ok(locator)
    }

    /// Append one variable-length sealed record, addressed by its content
    /// digest. The single entry point for causal, status, identity-value and
    /// capsule records.
    pub(crate) fn add_sealed_record(
        &mut self,
        payload: &[u8],
    ) -> Result<ColdLocatorV1, StoreError> {
        let digest = ContentDigest::of(payload);
        self.add_record(COLD_CLASS_RECORD, digest.as_bytes().to_vec(), payload)
    }

    /// Stage one domain entry. A later `put` of the same key in one cut wins.
    pub(crate) fn put(
        &mut self,
        domain: TableDomain,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), String> {
        if key.len() != domain.key_len as usize || value.len() != domain.value_len as usize {
            return Err("sealed cut entry width does not match its domain".into());
        }
        self.entries
            .entry(domain.id)
            .or_default()
            .insert(key.to_vec(), value.to_vec());
        Ok(())
    }

    /// Stage one removal. Only a tombstone domain may express one.
    pub(crate) fn remove(&mut self, domain: TableDomain, key: &[u8]) -> Result<(), String> {
        let tombstone = domain
            .tombstone_value()
            .ok_or("sealed cut removes a key from a domain with no tombstone")?;
        self.put(domain, key, &tombstone)
    }

    /// This cut's own staged value for a key, if it has one.
    pub(crate) fn staged(&self, domain: TableDomain, key: &[u8]) -> Option<&[u8]> {
        self.entries
            .get(&domain.id)
            .and_then(|entries| entries.get(key))
            .map(Vec::as_slice)
            .filter(|value| !domain.is_tombstone(value))
    }

    fn seal_current_pack(&mut self) -> Result<(), StoreError> {
        if self.builder.entries.is_empty() {
            return Ok(());
        }
        let next_virtual = self.builder.virtual_start + self.builder.body_len();
        let builder = std::mem::replace(&mut self.builder, ColdPackBuilder::new(next_virtual, 0));
        let (reference, bytes) = builder.finish().map_err(cold_index_error)?;
        publish_pack(&self.directory, reference, &bytes)?;
        self.published.push(reference);
        self.work.packs_published += 1;
        self.work.files_written += 1;
        Ok(())
    }

    /// Publish this cut: delta and merged tables, the root record, the marker
    /// last, and only then the retirement of superseded packs.
    pub(crate) fn publish(self) -> Result<SealedCutWork, StoreError> {
        self.publish_with_kill(None)
    }

    /// Publish, optionally stopping at one exact point in the protocol.
    ///
    /// The kill points are the crash matrix: every prefix of this sequence
    /// must leave a directory that opens at the PREDECESSOR and that a retry
    /// can complete (I-2). They exist only under `cfg(test)` callers; the
    /// `None` path is the production one and is byte-identical to it.
    pub(crate) fn publish_with_kill(
        mut self,
        kill: Option<SealedCutKill>,
    ) -> Result<SealedCutWork, StoreError> {
        // 1. Payload records durable first. Tables may only name durable bytes.
        self.seal_current_pack()?;
        if kill == Some(SealedCutKill::AfterPayloadPacks) {
            return Ok(self.work);
        }

        // 2. One delta table per touched domain, plus at most one level merge
        //    per domain per cut.
        let mut domain_roots: BTreeMap<u8, Vec<TableRef>> = self
            .base
            .tables
            .domains
            .iter()
            .map(|domain| (domain.domain_id, domain.tables.clone()))
            .collect();
        let touched: Vec<u8> = self.entries.keys().copied().collect();
        for domain_id in touched {
            let domain = domain_by_id(domain_id)
                .ok_or_else(|| cold_index_error("sealed cut names an unknown domain"))?;
            let entries = self.entries.remove(&domain_id).unwrap_or_default();
            if entries.is_empty() {
                continue;
            }
            let mut builder = TableBuilder::new(domain);
            for (key, value) in &entries {
                builder
                    .insert(key, value)
                    .map_err(|error| cold_index_error(error.to_string()))?;
            }
            let count = builder.len() as u64;
            let bytes = builder
                .finish()
                .map_err(|error| cold_index_error(error.to_string()))?;
            let locator = self.add_record(
                COLD_CLASS_TABLE,
                ContentDigest::of(&bytes).as_bytes().to_vec(),
                &bytes,
            )?;
            self.work.tables_written += 1;
            let delta = TableRef {
                locator: locator.table_locator(),
                level: 0,
                count,
            };
            let current = domain_roots.remove(&domain_id).unwrap_or_default();
            let cut = TierPlan::next_cut(&current, delta);
            let mut next = cut.retain;
            if let Some(level) = cut.merged_level {
                // `merge_tables` keeps tombstones; `compact_tables` drops them
                // and is valid ONLY when the inputs are every table of the
                // domain, because an older table may still hold the value a
                // tombstone hides.
                let every_table = next.is_empty();
                let mut raw = Vec::with_capacity(cut.merge.len());
                for table in &cut.merge {
                    let table_locator = ColdLocatorV1::from_table_locator(table.locator)
                        .map_err(cold_index_error)?;
                    raw.push(self.read_staged_or_packed(table_locator)?);
                }
                let views: Vec<TableView<'_>> = raw
                    .iter()
                    .map(|bytes| TableView::decode(domain, bytes))
                    .collect::<Result<_, _>>()
                    .map_err(|error| cold_index_error(error.to_string()))?;
                let merged = if every_table {
                    compact_tables(domain, &views)
                } else {
                    merge_tables(domain, &views)
                }
                .map_err(|error| cold_index_error(error.to_string()))?;
                let merged_count = TableView::decode(domain, &merged)
                    .map_err(|error| cold_index_error(error.to_string()))?
                    .len() as u64;
                let merged_locator = self.add_record(
                    COLD_CLASS_TABLE,
                    ContentDigest::of(&merged).as_bytes().to_vec(),
                    &merged,
                )?;
                self.work.tables_merged += 1;
                self.work.tables_written += 1;
                // The merged table is NEWER than every table already at its
                // level, so it goes at the FRONT before canonical ordering.
                next.insert(
                    0,
                    TableRef {
                        locator: merged_locator.table_locator(),
                        level,
                        count: merged_count,
                    },
                );
            }
            TierPlan::canonical_order(&mut next);
            domain_roots.insert(domain_id, next);
        }

        // 3. Seal the table pack, so the root record names durable table bytes.
        self.seal_current_pack()?;
        if kill == Some(SealedCutKill::AfterTables) {
            return Ok(self.work);
        }

        // 4. The root record: every domain's table list, and nothing else. The
        //    pack table is the marker's, because it is what resolves this very
        //    record's locator.
        let mut domains: Vec<SealedTableDomainRoot> = domain_roots
            .into_iter()
            .filter(|(_, tables)| !tables.is_empty())
            .map(|(domain_id, tables)| SealedTableDomainRoot { domain_id, tables })
            .collect();
        domains.sort_by_key(|domain| domain.domain_id);
        let tables = SealedTableRoot { domains };
        let root_bytes = tables
            .encode()
            .map_err(|error| cold_index_error(error.to_string()))?;
        let table_root_digest = tables
            .root_digest()
            .map_err(|error| cold_index_error(error.to_string()))?;
        let root_locator = self.add_record(COLD_CLASS_ROOT, Vec::new(), &root_bytes)?;
        self.work.root_locator = root_locator;
        self.work.root_digest = table_root_digest;
        self.seal_current_pack()?;

        // 5. The pack table: this cut's new packs, then at most one level
        //    merge. A merge only concatenates contiguous bodies, so the root
        //    locator resolves whether or not its own pack was merged.
        let mut packs = self.base.packs.clone();
        packs.packs.extend(self.published.iter().copied());
        packs.packs.sort_by_key(|pack| pack.virtual_start);
        packs.validate().map_err(cold_index_error)?;
        let Some((packs, retired)) = self.merge_pack_level(packs, kill)? else {
            return Ok(self.work);
        };
        if kill == Some(SealedCutKill::AfterRootRecord) {
            return Ok(self.work);
        }

        // 6. Marker last. Until this line the whole cut is residue.
        let marker = encode_canonical(&SealedMarkerV1 {
            schema: COLD_SCHEMA_VERSION,
            packs,
            root_locator: root_locator.to_bytes(),
            table_root_digest,
        })
        .map_err(cold_index_error)?;
        install_marker(&self.directory, self.prior_marker.as_deref(), &marker)?;
        self.work.files_written += 1;
        if kill == Some(SealedCutKill::AfterMarkerBeforeRetire) {
            return Ok(self.work);
        }

        // 7. Publish-new-before-retire-old: superseded packs are removed only
        //    after the marker names their successor. A crash here leaves an
        //    unreferenced pack, which the next open ignores and the next cut
        //    may retire.
        for pack in &retired {
            retire_pack(&self.directory, pack.pack_id());
        }
        self.work.packs_retired = retired.len();
        Ok(self.work)
    }

    /// This cut's view of one domain entry: what this cut staged, else what
    /// the base root holds. `None` is genuine absence, tombstone included.
    pub(crate) fn get(&self, domain: TableDomain, key: &[u8]) -> Result<Option<Vec<u8>>, String> {
        if let Some(staged) = self.entries.get(&domain.id).and_then(|rows| rows.get(key)) {
            if domain.is_tombstone(staged) {
                return Ok(None);
            }
            return Ok(Some(staged.clone()));
        }
        self.base_tables(domain)?
            .get(key)
            .map_err(|error| error.to_string())
    }

    /// The greatest key at or below `key`, over the same two layers.
    pub(crate) fn predecessor(
        &self,
        domain: TableDomain,
        key: &[u8],
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>, String> {
        let base = self
            .base_tables(domain)?
            .predecessor(key)
            .map_err(|error| error.to_string())?;
        let staged = self
            .entries
            .get(&domain.id)
            .and_then(|rows| {
                rows.range::<[u8], _>((std::ops::Bound::Unbounded, std::ops::Bound::Included(key)))
                    .next_back()
            })
            .filter(|(_, value)| !domain.is_tombstone(value))
            .map(|(key, value)| (key.clone(), value.clone()));
        Ok(match (base, staged) {
            (Some(base), Some(staged)) => Some(if staged.0 >= base.0 { staged } else { base }),
            (Some(base), None) => Some(base),
            (None, staged) => staged,
        })
    }

    /// Every live entry this cut and its base agree on, for one domain.
    pub(crate) fn domain_entries(
        &self,
        domain: TableDomain,
    ) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, String> {
        let mut live = domain_live_entries(self, domain, &self.base.tables)?;
        if let Some(rows) = self.entries.get(&domain.id) {
            for (key, value) in rows {
                if domain.is_tombstone(value) {
                    live.remove(key);
                } else {
                    live.insert(key.clone(), value.clone());
                }
            }
        }
        Ok(live)
    }

    fn base_tables(&self, domain: TableDomain) -> Result<TableSetReader<'_, Self>, String> {
        TableSetReader::new(self, domain, self.base.tables.tables_for(domain))
            .map_err(|error| error.to_string())
    }

    /// Read one record's bytes, whether it is a predecessor's, one this cut
    /// has already sealed, or one still in the open builder.
    pub(crate) fn read_staged_or_packed(
        &self,
        locator: ColdLocatorV1,
    ) -> Result<Vec<u8>, StoreError> {
        if let Some(bytes) = self.staged_records.get(&locator) {
            return Ok(bytes.clone());
        }
        let mut packs = self.base.packs.clone();
        packs.packs.extend(self.published.iter().copied());
        packs.packs.sort_by_key(|pack| pack.virtual_start);
        read_pack_range(&self.directory, &packs, locator, MAX_COLD_ROOT_BYTES)
            .map_err(cold_index_error)
    }

    /// Merge one pack level, if one has reached the fanout.
    ///
    /// Same-level packs are always a contiguous virtual run — new packs are
    /// appended at level 0 and a merge replaces a contiguous run with one pack
    /// of the next level covering the identical range — so the merged pack is a
    /// byte-for-byte concatenation of its inputs' bodies and every record keeps
    /// its virtual offset.
    /// `Ok(None)` means only the `MidPackTierMerge` crash fixture: the merged
    /// pack is durable and nothing names it yet.
    fn merge_pack_level(
        &mut self,
        packs: PackTableV1,
        kill: Option<SealedCutKill>,
    ) -> Result<Option<(PackTableV1, Vec<PackRefV1>)>, StoreError> {
        let mut levels: Vec<u8> = packs.packs.iter().map(|pack| pack.level).collect();
        levels.sort_unstable();
        levels.dedup();
        for level in levels {
            let at_level: Vec<PackRefV1> = packs
                .packs
                .iter()
                .copied()
                .filter(|pack| pack.level == level)
                .collect();
            if at_level.len() < SEALED_TIER_FANOUT {
                continue;
            }
            let run = &at_level[..SEALED_TIER_FANOUT];
            if run
                .windows(2)
                .any(|pair| pair[0].virtual_end != pair[1].virtual_start)
            {
                // Never merge a non-contiguous run: it would move offsets.
                continue;
            }
            let mut builder = ColdPackBuilder::new(run[0].virtual_start, level.saturating_add(1));
            for reference in run {
                let inventory = read_pack_footer(&self.directory, reference.pack_id())?;
                let body =
                    read_pack_body(&self.directory, reference.pack_id(), inventory.body_len)?;
                for entry in &inventory.footer.entries {
                    let start = (entry.offset - reference.virtual_start) as usize;
                    let end = start + entry.length as usize;
                    let raw = body
                        .get(start..end)
                        .ok_or_else(|| cold_index_error("pack footer entry is outside its body"))?;
                    builder
                        .append_verbatim(entry, raw)
                        .map_err(cold_index_error)?;
                }
            }
            let (reference, bytes) = builder.finish().map_err(cold_index_error)?;
            if reference.virtual_end != run[SEALED_TIER_FANOUT - 1].virtual_end {
                return Err(cold_index_error(
                    "merged pack does not cover its inputs' virtual range",
                ));
            }
            publish_pack(&self.directory, reference, &bytes)?;
            self.work.packs_published += 1;
            self.work.files_written += 1;
            self.work.merged_pack_bytes = self
                .work
                .merged_pack_bytes
                .saturating_add(reference.virtual_end - reference.virtual_start);
            if kill == Some(SealedCutKill::MidPackTierMerge) {
                return Ok(None);
            }
            let retired: Vec<PackRefV1> = run.to_vec();
            let mut next = PackTableV1::default();
            for pack in &packs.packs {
                if retired.contains(pack) {
                    if pack.virtual_start == reference.virtual_start {
                        next.packs.push(reference);
                    }
                    continue;
                }
                next.packs.push(*pack);
            }
            next.packs.sort_by_key(|pack| pack.virtual_start);
            next.validate().map_err(cold_index_error)?;
            return Ok(Some((next, retired)));
        }
        Ok(Some((packs, Vec::new())))
    }
}

/// Delete every pack the given table does not name.
fn retire_unreferenced_packs(directory: &Dir, packs: &PackTableV1) -> Result<(), StoreError> {
    let referenced: BTreeSet<Uuid> = packs.packs.iter().map(PackRefV1::pack_id).collect();
    for pack in cold_pack_names(directory)? {
        if !referenced.contains(&pack) {
            retire_pack(directory, pack);
        }
    }
    Ok(())
}

fn publish_pack(directory: &Dir, reference: PackRefV1, bytes: &[u8]) -> Result<(), StoreError> {
    tine_storage::DurableDirectoryPublication::open(directory)
        .map_err(filesystem_error_without_collision)?
        .publish_new_exact_single_writer(&pack_filename(reference.pack_id()), bytes)
        .map_err(filesystem_error_without_collision)
}

fn read_pack_body(directory: &Dir, pack: Uuid, body_len: u64) -> Result<Vec<u8>, StoreError> {
    let name = pack_filename(pack);
    let mut file = tine_storage::open_file_nofollow(directory, &name)
        .map_err(|error| cold_index_error(format!("cold pack {name}: {error}")))?;
    let mut body = vec![0_u8; body_len as usize];
    file.read_exact(&mut body)
        .map_err(|error| cold_index_error(error.to_string()))?;
    Ok(body)
}

/// Best-effort retirement of a pack the marker no longer names.
fn retire_pack(directory: &Dir, pack: Uuid) {
    let _ = directory.remove_file(pack_filename(pack));
}

fn install_marker(directory: &Dir, prior: Option<&[u8]>, bytes: &[u8]) -> Result<(), StoreError> {
    if bytes.len() as u64 > MAX_SEALED_MARKER_BYTES {
        return Err(cold_index_error(
            "sealed root marker exceeds its fixed codec size",
        ));
    }
    let publication = tine_storage::DurableDirectoryPublication::open(directory)
        .map_err(filesystem_error_without_collision)?;
    match prior {
        Some(existing) if existing == bytes => Ok(()),
        Some(existing) => publication
            .replace_exact(SEALED_ROOT_MARKER, existing, bytes)
            .map_err(filesystem_error_without_collision),
        None => publication
            .publish_new_exact_single_writer(SEALED_ROOT_MARKER, bytes)
            .map_err(filesystem_error_without_collision),
    }
}

// ---------------------------------------------------------------------------
// The cold whole-object tier, on top of the cut
// ---------------------------------------------------------------------------

/// Additively publish exact logical objects and batch manifests into the sealed
/// archive.
///
/// Canonical bytes, content digests and `BatchId`s are preserved verbatim: this
/// is a physical relocation below the object model, never a re-encoding.
pub(crate) fn publish_cold_history(
    store: &ObjectStore,
    objects: &BTreeMap<ContentDigest, Vec<u8>>,
    manifests: &BTreeMap<BatchId, Vec<u8>>,
) -> Result<ColdPublicationOutcome, StoreError> {
    let directory = sealed_directory(store)?;
    publish_cold_history_into(&directory, objects, manifests)
}

pub(crate) fn publish_cold_history_into(
    directory: &Dir,
    objects: &BTreeMap<ContentDigest, Vec<u8>>,
    manifests: &BTreeMap<BatchId, Vec<u8>>,
) -> Result<ColdPublicationOutcome, StoreError> {
    let mut outcome = ColdPublicationOutcome::default();
    // A repeated identity is resolved by BYTES, not by name presence. The whole
    // exactness pass runs before a single record is appended, so a conflict
    // leaves no residue at all.
    let mut new_objects: Vec<(ContentDigest, &Vec<u8>)> = Vec::new();
    let mut new_manifests: Vec<(BatchId, &Vec<u8>)> = Vec::new();
    if let Some(reader) = SealedArchiveReader::open_directory(directory)? {
        for (digest, bytes) in objects {
            match reader.object_bytes(*digest)? {
                Some(existing) if existing == *bytes => outcome.objects_already_present += 1,
                Some(_) => {
                    return Err(cold_object_error(
                        *digest,
                        "cold history holds different bytes under this content address",
                    ))
                }
                None => new_objects.push((*digest, bytes)),
            }
        }
        for (batch_id, bytes) in manifests {
            match reader.manifest_bytes(*batch_id)? {
                Some(existing) if existing == *bytes => outcome.manifests_already_present += 1,
                Some(_) => return Err(cold_manifest_conflict(
                    *batch_id,
                    "cold history already holds a different canonical manifest under this batch id",
                )),
                None => new_manifests.push((*batch_id, bytes)),
            }
        }
    } else {
        new_objects = objects
            .iter()
            .map(|(digest, bytes)| (*digest, bytes))
            .collect();
        new_manifests = manifests
            .iter()
            .map(|(batch_id, bytes)| (*batch_id, bytes))
            .collect();
    }
    if new_objects.is_empty() && new_manifests.is_empty() {
        return Ok(outcome);
    }
    let mut cut = SealedCut::open(directory)?;
    for (digest, payload) in new_objects {
        add_cold_object(&mut cut, digest, payload)?;
        outcome.objects_published += 1;
    }
    for (batch_id, payload) in new_manifests {
        add_cold_manifest(&mut cut, batch_id, payload)?;
        outcome.manifests_published += 1;
    }
    let work = cut.publish()?;
    outcome.packs_published = work.packs_published;
    Ok(outcome)
}

fn add_cold_object(
    cut: &mut SealedCut,
    digest: ContentDigest,
    payload: &[u8],
) -> Result<(), StoreError> {
    if payload.len() > MAX_OBJECT_BYTES {
        return Err(cold_object_error(
            digest,
            "logical object exceeds the current object byte limit",
        ));
    }
    if ContentDigest::of(payload) != digest {
        return Err(cold_object_error(
            digest,
            "logical object bytes differ from their content address",
        ));
    }
    let locator = cut.add_record(COLD_CLASS_OBJECT, digest.as_bytes().to_vec(), payload)?;
    cut.put(DOMAIN_COLD_OBJECT, digest.as_bytes(), &locator.to_bytes())
        .map_err(cold_index_error)
}

fn add_cold_manifest(
    cut: &mut SealedCut,
    batch_id: BatchId,
    payload: &[u8],
) -> Result<(), StoreError> {
    if payload.len() > MAX_MANIFEST_BYTES {
        return Err(cold_manifest_error(
            batch_id,
            "logical manifest exceeds the current manifest byte limit",
        ));
    }
    // A `BatchId` is an identity, not a content address, so bind these exact
    // bytes to it before they are packed under that key.
    let manifest = super::OperationBatch::decode(payload)?;
    if manifest.batch_id() != batch_id {
        return Err(cold_manifest_conflict(
            batch_id,
            "these canonical manifest bytes belong to another batch id",
        ));
    }
    let key = batch_id.as_uuid().into_bytes();
    let locator = cut.add_record(COLD_CLASS_MANIFEST, key.to_vec(), payload)?;
    cut.put(DOMAIN_COLD_MANIFEST, &key, &locator.to_bytes())
        .map_err(cold_index_error)
}

/// Copy the exact hot originals named by these batches into cold history.
pub(crate) fn publish_cold_history_for_batches(
    store: &ObjectStore,
    batches: &BTreeSet<BatchId>,
) -> Result<ColdPublicationOutcome, StoreError> {
    let mut objects = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for batch_id in batches {
        // A recovery generation can cover a batch that an earlier committed
        // generation has already retired from hot storage. Relocation is
        // idempotent over the logical archive, not conditional on a duplicate
        // still existing in the hot namespace.
        let manifest_bytes = store.resolve_logical_manifest_bytes(*batch_id)?;
        let manifest = super::OperationBatch::decode(&manifest_bytes)?;
        for descriptor in manifest.required_objects() {
            let digest = descriptor.content_digest();
            if objects.contains_key(&digest) {
                continue;
            }
            objects.insert(digest, store.resolve_logical_object_bytes(digest)?);
        }
        manifests.insert(*batch_id, manifest_bytes);
    }
    publish_cold_history(store, &objects, &manifests)
}

// ---------------------------------------------------------------------------
// Repair
// ---------------------------------------------------------------------------

/// One logical record recovered from a pack footer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColdInventoryKey {
    Object(ContentDigest),
    Manifest(BatchId),
}

/// The result of an explicit sealed-root repair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ColdRepairOutcome {
    /// True only when a damaged state was actually recovered.
    pub(crate) repaired: bool,
    pub(crate) packs_scanned: usize,
    pub(crate) objects_recovered: usize,
    pub(crate) manifests_recovered: usize,
    pub(crate) domains_rebuilt: usize,
}

/// Rebuild EVERY domain's tables from the pack footers alone.
///
/// This is the explicit damaged-state path named by
/// [`StoreError::ColdHistoryRootMissing`], and the only place the sealed
/// archive is enumerated on a recovery. It reuses the surviving records exactly
/// where they already are — no byte is rewritten, relocated or re-encoded — and
/// rebuilds only the derived index (D-3): one sort per domain, one fresh root,
/// one fresh marker. A healthy or never-initialized archive is left untouched.
///
/// Note what it can and cannot recover. Cold objects and manifests are keyed by
/// their footer entries, so they rebuild exactly. The accepted-history domains
/// are recovered from the surviving TABLE records themselves: every table this
/// archive ever sealed is in some pack, so replaying them oldest-first
/// reconstructs each domain's live entry set without consulting the lost root.
pub(crate) fn repair_cold_history_root(
    store: &ObjectStore,
) -> Result<ColdRepairOutcome, StoreError> {
    let Some(directory) = open_existing_sealed_directory(store)? else {
        return Ok(ColdRepairOutcome::default());
    };
    repair_sealed_root(&directory)
}

pub(crate) fn repair_sealed_root(directory: &Dir) -> Result<ColdRepairOutcome, StoreError> {
    let prior_marker = read_root_marker_bytes(directory)?;
    if let Some(bytes) = prior_marker.as_deref() {
        if decode_marker_and_root(directory, bytes).is_ok() {
            return Ok(ColdRepairOutcome::default());
        }
    }
    if !contains_any_pack(directory)? {
        return if prior_marker.is_none() {
            Ok(ColdRepairOutcome::default())
        } else {
            // A damaged marker with no surviving pack is never replaced with a
            // fresh empty history: there is nothing to rebuild it from.
            Err(cold_index_error(
                "damaged sealed root has no preserved packs to rebuild from",
            ))
        };
    }

    let mut inventories = Vec::new();
    for pack in cold_pack_names(directory)? {
        inventories.push(read_pack_footer(directory, pack)?);
    }
    let packs = pack_table_from_inventories(&inventories);
    let packs_scanned = inventories.len();

    // Replay every surviving table oldest-first per domain, so a newer table's
    // entry shadows an older one exactly as a lookup would.
    let mut domain_entries: BTreeMap<u8, BTreeMap<Vec<u8>, Vec<u8>>> = BTreeMap::new();
    let mut tables: Vec<(u64, ColdLocatorV1)> = Vec::new();
    let mut objects: BTreeMap<ContentDigest, ColdLocatorV1> = BTreeMap::new();
    let mut manifests: BTreeMap<BatchId, ColdLocatorV1> = BTreeMap::new();
    for inventory in &inventories {
        let live = packs
            .locate(inventory.footer.virtual_start)
            .is_some_and(|pack| pack.pack_id() == inventory.pack);
        if !live {
            continue;
        }
        for entry in &inventory.footer.entries {
            let locator = ColdLocatorV1 {
                offset: entry.offset,
                length: entry.length,
            };
            match entry.class {
                COLD_CLASS_TABLE => tables.push((entry.offset, locator)),
                COLD_CLASS_OBJECT if entry.key.len() == 32 => {
                    let mut bytes = [0_u8; 32];
                    bytes.copy_from_slice(&entry.key);
                    let digest = ContentDigest::from_bytes(bytes);
                    let payload =
                        read_pack_range(directory, &packs, locator, MAX_OBJECT_BYTES as u64)
                            .map_err(|error| cold_object_error(digest, error))?;
                    if ContentDigest::of(&payload) != digest {
                        return Err(cold_object_error(
                            digest,
                            "a packed record is filed under another object's content address",
                        ));
                    }
                    objects.entry(digest).or_insert(locator);
                }
                COLD_CLASS_MANIFEST if entry.key.len() == 16 => {
                    let mut bytes = [0_u8; 16];
                    bytes.copy_from_slice(&entry.key);
                    let batch_id = BatchId::from_uuid(Uuid::from_bytes(bytes));
                    let payload =
                        read_pack_range(directory, &packs, locator, MAX_MANIFEST_BYTES as u64)
                            .map_err(|error| cold_manifest_error(batch_id, error))?;
                    let manifest = super::OperationBatch::decode(&payload)?;
                    if manifest.batch_id() != batch_id {
                        return Err(cold_manifest_conflict(
                            batch_id,
                            "a packed manifest record is filed under another batch id",
                        ));
                    }
                    if let Some(previous) = manifests.get(&batch_id) {
                        let existing = read_pack_range(
                            directory,
                            &packs,
                            *previous,
                            MAX_MANIFEST_BYTES as u64,
                        )
                        .map_err(|error| cold_manifest_error(batch_id, error))?;
                        if existing != payload {
                            return Err(cold_manifest_conflict(
                                batch_id,
                                "preserved packs hold two different canonical manifests for this batch id",
                            ));
                        }
                        continue;
                    }
                    manifests.insert(batch_id, locator);
                }
                COLD_CLASS_ROOT | COLD_CLASS_RECORD => {}
                _ => {
                    return Err(cold_index_error(
                        "a preserved pack footer names an unknown record class",
                    ))
                }
            }
        }
    }
    tables.sort_unstable();
    for (_, locator) in tables {
        let bytes = read_pack_range(directory, &packs, locator, MAX_COLD_ROOT_BYTES)
            .map_err(cold_index_error)?;
        // A table declares its own domain in its header, so the rebuild does
        // not need the lost root to interpret it.
        // `TINETBL1` (8) ‖ schema:u32be (4) ‖ domain id — byte 12, not byte 9.
        // A table's own header is what lets the rebuild interpret it without
        // the lost root; reading the wrong byte made every repair of a torn
        // marker refuse with "a preserved table record names an unknown
        // domain" (`manager_torn_cold_root_is_rebuildable_from_preserved_packs`).
        let Some(domain_id) = bytes.get(12).copied() else {
            return Err(cold_index_error("a preserved table record is truncated"));
        };
        let Some(domain) = domain_by_id(domain_id) else {
            return Err(cold_index_error(
                "a preserved table record names an unknown domain",
            ));
        };
        // A table that fails its digest is damaged derived state, not a
        // graph-open refusal: skip it and keep rebuilding (D-3, P4c2 §4.3).
        let Ok(view) = TableView::decode(domain, &bytes) else {
            continue;
        };
        let entries = domain_entries.entry(domain_id).or_default();
        for (key, value) in view.iter() {
            entries.insert(key.to_vec(), value.to_vec());
        }
    }
    // Cold object and manifest domains are authoritative from the footers, not
    // from the surviving tables: a footer entry is the record's own claim.
    let cold_objects = domain_entries.entry(DOMAIN_COLD_OBJECT.id).or_default();
    cold_objects.clear();
    for (digest, locator) in &objects {
        cold_objects.insert(digest.as_bytes().to_vec(), locator.to_bytes().to_vec());
    }
    let cold_manifests = domain_entries.entry(DOMAIN_COLD_MANIFEST.id).or_default();
    cold_manifests.clear();
    for (batch_id, locator) in &manifests {
        cold_manifests.insert(
            batch_id.as_uuid().as_bytes().to_vec(),
            locator.to_bytes().to_vec(),
        );
    }
    domain_entries.retain(|_, entries| !entries.is_empty());
    let domains_rebuilt = domain_entries.len();

    // One fresh cut over an empty index base, but the SURVIVING pack table:
    // no record is rewritten and no virtual offset moves.
    let base = SealedRootRecord {
        tables: SealedTableRoot::default(),
        packs,
    };
    let mut cut = SealedCut::over(directory, base, prior_marker)?;
    for (domain_id, entries) in domain_entries {
        let domain = domain_by_id(domain_id)
            .ok_or_else(|| cold_index_error("rebuilt sealed domain is unknown"))?;
        for (key, value) in entries {
            cut.put(domain, &key, &value).map_err(cold_index_error)?;
        }
    }
    cut.publish()?;
    Ok(ColdRepairOutcome {
        repaired: true,
        packs_scanned,
        objects_recovered: objects.len(),
        manifests_recovered: manifests.len(),
        domains_rebuilt,
    })
}

/// Republish every cold logical record into fresh packs and rebuild every root
/// from the pack footers alone.
///
/// Proves that logical identity is independent of physical placement and that
/// the index is genuinely derived, disposable state.
pub(crate) fn repack_cold_history(
    store: &ObjectStore,
) -> Result<ColdPublicationOutcome, StoreError> {
    let Some(directory) = open_existing_sealed_directory(store)? else {
        return Ok(ColdPublicationOutcome::default());
    };
    let Some(reader) = SealedArchiveReader::open_directory(&directory)? else {
        return Ok(ColdPublicationOutcome::default());
    };
    let mut objects = BTreeMap::new();
    for (key, value) in reader
        .domain_entries(DOMAIN_COLD_OBJECT)
        .map_err(cold_index_error)?
    {
        let bytes: [u8; 32] = key
            .as_slice()
            .try_into()
            .map_err(|_| cold_index_error("cold object domain holds a non-digest key"))?;
        let digest = ContentDigest::from_bytes(bytes);
        let locator = ColdLocatorV1::from_value(&value).map_err(cold_index_error)?;
        objects.insert(
            digest,
            reader
                .record_bytes(locator, MAX_OBJECT_BYTES as u64)
                .map_err(|error| cold_object_error(digest, error))?,
        );
    }
    let mut manifests = BTreeMap::new();
    for (key, value) in reader
        .domain_entries(DOMAIN_COLD_MANIFEST)
        .map_err(cold_index_error)?
    {
        let bytes: [u8; 16] = key
            .as_slice()
            .try_into()
            .map_err(|_| cold_index_error("cold manifest domain holds a non-BatchId key"))?;
        let batch_id = BatchId::from_uuid(Uuid::from_bytes(bytes));
        let locator = ColdLocatorV1::from_value(&value).map_err(cold_index_error)?;
        manifests.insert(
            batch_id,
            reader
                .record_bytes(locator, MAX_MANIFEST_BYTES as u64)
                .map_err(|error| cold_manifest_error(batch_id, error))?,
        );
    }
    drop(reader);

    // Republish the exact bytes at fresh virtual offsets, then rebuild the cold
    // domains to name them.
    let mut cut = SealedCut::open(&directory)?;
    let mut outcome = ColdPublicationOutcome::default();
    for (digest, payload) in &objects {
        add_cold_object(&mut cut, *digest, payload)?;
        outcome.objects_published += 1;
    }
    for (batch_id, payload) in &manifests {
        add_cold_manifest(&mut cut, *batch_id, payload)?;
        outcome.manifests_published += 1;
    }
    let work = cut.publish()?;
    outcome.packs_published = work.packs_published;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage a publication's packs and tables durably and stop before the
    /// marker -- the crash window `stage_publication` used to express.
    fn stage_publication_without_marker(
        directory: &Dir,
        objects: &BTreeMap<ContentDigest, Vec<u8>>,
        manifests: &BTreeMap<BatchId, Vec<u8>>,
    ) -> Result<ColdPublicationOutcome, StoreError> {
        let mut outcome = ColdPublicationOutcome::default();
        let mut cut = SealedCut::open(directory)?;
        for (digest, payload) in objects {
            add_cold_object(&mut cut, *digest, payload)?;
            outcome.objects_published += 1;
        }
        for (batch_id, payload) in manifests {
            add_cold_manifest(&mut cut, *batch_id, payload)?;
            outcome.manifests_published += 1;
        }
        let work = cut.publish_with_kill(Some(SealedCutKill::AfterTables))?;
        outcome.packs_published = work.packs_published;
        Ok(outcome)
    }

    /// The pack file a locator lies in, and the offset of its payload inside
    /// that file. A locator names a VIRTUAL offset; only the current marker's
    /// pack table maps it to a file.
    fn pack_file_for(directory: &Dir, locator: ColdLocatorV1) -> (String, usize) {
        let packs = match read_root_state(directory).unwrap() {
            SealedRootState::Published { root, .. } => root.packs,
            _ => panic!("sealed archive has published no pack table"),
        };
        let pack = packs.locate(locator.offset).expect("locator names a pack");
        (
            pack_filename(pack.pack_id()),
            (locator.offset - pack.virtual_start) as usize,
        )
    }

    /// Live rows in one cold domain: the retired root pair's `object_count` /
    /// `manifest_count`, read from the sorted tables that replaced it.
    fn cold_domain_count(store: &ObjectStore, domain: TableDomain) -> u64 {
        let reader = SealedArchiveReader::open(store).unwrap().unwrap();
        reader.domain_entries(domain).unwrap().len() as u64
    }

    /// The sealed table-root digest, in the shape the retired `ColdRootState`
    /// pair had: `Ok(None)` is ordinary absence, `Err` is a named damaged
    /// state, and two equal digests mean the roots did not move.
    fn cold_history_roots(store: &ObjectStore) -> Result<Option<ContentDigest>, StoreError> {
        let Some(reader) = SealedArchiveReader::open(store)? else {
            return Ok(None);
        };
        reader
            .root()
            .table_root_digest()
            .map(Some)
            .map_err(cold_index_error)
    }
    use crate::oplog::{
        BatchCausalDot, BatchInspection, BatchOrigin, CausalPeerId, CrdtPeerCounter, CrdtPeerId,
        DeviceId, DocumentDependencies, DocumentId, FrontierV2, LineageDigest, ObjectKind,
        OperationObject, PreparedBatch, SemanticEffectDigest, SessionId, WorkspaceId,
    };

    /// One-shot interleaving fault armed by
    /// `a_torn_marker_read_across_a_pack_retiring_cut_is_recovered_not_refused`
    /// and fired by `sealed_torn_root_read_fault_for_test`.
    #[allow(clippy::type_complexity)]
    pub(super) static TORN_ROOT_READ_HOOK: std::sync::Mutex<Option<Box<dyn FnMut() + Send>>> =
        std::sync::Mutex::new(None);

    struct TestArchive {
        root: std::path::PathBuf,
        store: ObjectStore,
    }

    impl TestArchive {
        fn open(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!("tine-cold-{label}-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let store =
                ObjectStore::open(&root.join("archive"), WorkspaceId::from_uuid(uuid(1))).unwrap();
            Self { root, store }
        }

        fn cold_directory(&self) -> std::path::PathBuf {
            self.store.root_path().join(SEALED_DIRECTORY)
        }

        /// Remove one batch's hot originals, exactly the way a completed R2
        /// retirement eventually will. Cold history must still reconstruct
        /// every original byte and prove every digest.
        fn remove_hot_originals(&self, batch: &PreparedBatch) {
            let archive = self.store.root_path();
            std::fs::remove_file(
                archive
                    .join("batches")
                    .join(format!("{}.manifest", batch.manifest().batch_id())),
            )
            .unwrap();
            for object in batch.objects() {
                let digest = ContentDigest::of(&object.encode().unwrap());
                std::fs::remove_file(archive.join("objects").join(format!("{digest}.object")))
                    .unwrap();
            }
        }
    }

    impl Drop for TestArchive {
        fn drop(&mut self) {
            crate::test_support::remove_dir_all(std::mem::take(&mut self.root));
        }
    }

    fn uuid(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn sample_batch(workspace_id: WorkspaceId, seed: u128) -> PreparedBatch {
        let semantic_payload = format!("semantic effect payload {seed}").into_bytes();
        let semantic = OperationObject::new(
            workspace_id,
            DocumentId::from_uuid(uuid(0x10_0000 + seed)),
            ObjectKind::SemanticEffect,
            semantic_payload.clone(),
        )
        .unwrap();
        let update = OperationObject::new(
            workspace_id,
            DocumentId::from_uuid(uuid(0x20_0000 + seed)),
            ObjectKind::CrdtUpdate,
            format!("crdt update payload {seed} {}", "x".repeat(64)).into_bytes(),
        )
        .unwrap();
        let objects = vec![semantic, update];
        let descriptors = objects
            .iter()
            .map(|object| object.descriptor().unwrap())
            .collect();
        let device = DeviceId::from_uuid(uuid(30));
        let frontier = FrontierV2::new(vec![DocumentDependencies::new(
            DocumentId::from_uuid(uuid(0x20_0000 + seed)),
            vec![CrdtPeerCounter::new(CrdtPeerId::from_u64(8), 12)],
            Vec::new(),
        )
        .unwrap()])
        .unwrap();
        let manifest = crate::oplog::OperationBatch::new_with_causality(
            workspace_id,
            LineageDigest::of(b"cold-history-lineage"),
            BatchId::from_uuid(uuid(0x1000 + seed)),
            device,
            SessionId::from_uuid(uuid(31)),
            BatchOrigin::LocalMutation,
            BatchCausalDot::new(
                CausalPeerId::from_key(crate::oplog::WriterIncarnationId::fixture_for_device(
                    device,
                )),
                u64::try_from(seed).unwrap() + 1,
            )
            .unwrap(),
            Vec::new(),
            frontier,
            SemanticEffectDigest::of(&semantic_payload),
            descriptors,
        )
        .unwrap();
        PreparedBatch::new(manifest, objects).unwrap()
    }

    fn publish_batches(
        archive: &TestArchive,
        seeds: impl IntoIterator<Item = u128>,
    ) -> Vec<PreparedBatch> {
        let batches: Vec<_> = seeds
            .into_iter()
            .map(|seed| sample_batch(archive.store.workspace_id(), seed))
            .collect();
        for batch in &batches {
            archive.store.publish_prepared(batch).unwrap();
        }
        batches
    }

    fn relocate(archive: &TestArchive, batches: &[PreparedBatch]) -> ColdPublicationOutcome {
        let roster = batches
            .iter()
            .map(|batch| batch.manifest().batch_id())
            .collect();
        archive
            .store
            .publish_cold_history_for_batches(&roster)
            .unwrap()
    }

    #[test]
    fn cold_history_resolves_exact_original_bytes_after_hot_originals_are_removed() {
        let archive = TestArchive::open("reconstruct");
        let batches = publish_batches(&archive, 0..6);
        let outcome = relocate(&archive, &batches);
        assert_eq!(outcome.objects_published, 12);
        assert_eq!(outcome.manifests_published, 6);

        // Qualify the resolver against a fixture where the covered hot
        // originals no longer exist at all.
        for batch in &batches {
            archive.remove_hot_originals(batch);
        }

        for batch in &batches {
            let batch_id = batch.manifest().batch_id();
            let manifest_bytes = archive
                .store
                .resolve_logical_manifest_bytes(batch_id)
                .unwrap();
            assert_eq!(manifest_bytes, batch.manifest().encode().unwrap());
            let resolved = archive
                .store
                .resolve_logical_manifest(batch_id)
                .unwrap()
                .unwrap();
            assert_eq!(resolved.batch_id(), batch_id);

            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                let digest = ContentDigest::of(&bytes);
                assert_eq!(
                    archive.store.resolve_logical_object_bytes(digest).unwrap(),
                    bytes,
                    "cold history must return byte-identical originals"
                );
                let object = archive.store.resolve_logical_object(digest).unwrap();
                assert_eq!(ContentDigest::of(&object.encode().unwrap()), digest);
                assert!(archive.store.contains_logical_object(digest).unwrap());
            }

            // Full replay of the batch: every required object reconstructs and
            // the whole batch validates, with no hot original left on disk.
            match archive
                .store
                .inspect_batch_with_cold_history(batch_id)
                .unwrap()
            {
                BatchInspection::Ready(validated) => {
                    assert_eq!(validated.manifest().batch_id(), batch_id);
                }
                other => panic!("expected a cold-resolved Ready batch, got {other:?}"),
            }
        }

        // The ordinary hot-only reader still reports honest absence and has
        // touched no pack byte.
        assert_eq!(
            archive
                .store
                .inspect_batch(batches[0].manifest().batch_id())
                .unwrap(),
            BatchInspection::Absent
        );
    }

    #[test]
    fn ordinary_hot_reads_never_touch_cold_history() {
        let archive = TestArchive::open("hot-only");
        let batches = publish_batches(&archive, 0..3);
        relocate(&archive, &batches);
        let before = archive.store.instrumentation();

        for batch in &batches {
            assert!(matches!(
                archive
                    .store
                    .inspect_batch(batch.manifest().batch_id())
                    .unwrap(),
                BatchInspection::Ready(_)
            ));
            for object in batch.objects() {
                let digest = ContentDigest::of(&object.encode().unwrap());
                archive.store.read_object(digest).unwrap();
                archive.store.read_object_bytes(digest).unwrap();
            }
            archive
                .store
                .read_manifest(batch.manifest().batch_id())
                .unwrap()
                .unwrap();
        }
        let after = archive.store.instrumentation();
        assert_eq!(after.cold_object_reads, before.cold_object_reads);
        assert_eq!(after.cold_manifest_reads, before.cold_manifest_reads);
        assert_eq!(after.cold_object_reads, 0);
        assert_eq!(after.cold_manifest_reads, 0);

        // With hot originals intact the resolver also stays hot: relocation is
        // additive, so nothing forces a cold read.
        for batch in &batches {
            for object in batch.objects() {
                let digest = ContentDigest::of(&object.encode().unwrap());
                archive.store.resolve_logical_object_bytes(digest).unwrap();
            }
        }
        assert_eq!(archive.store.instrumentation().cold_object_reads, 0);
    }

    #[test]
    fn full_sha256_key_domains_stay_full() {
        let archive = TestArchive::open("full-keys");
        let batches = publish_batches(&archive, 0..2);
        relocate(&archive, &batches);
        let first = batches[0].objects()[0].encode().unwrap();
        let second = batches[1].objects()[0].encode().unwrap();
        let first_digest = ContentDigest::of(&first);
        let second_digest = ContentDigest::of(&second);

        let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
        // Four objects across two batches, four rows: the object domain is ONE
        // sorted table keyed by the whole 256-bit digest. The retired composed
        // map's prefix/inner split is gone with the treap (P4c2 4.1), and with
        // it the bucket-occupancy question it existed to answer; what the test
        // still owns is that no key is truncated.
        assert_eq!(reader.domain_entries(DOMAIN_COLD_OBJECT).unwrap().len(), 4);
        let first_high: [u8; 16] = first_digest.as_bytes()[..16].try_into().unwrap();
        let first_low: [u8; 16] = first_digest.as_bytes()[16..].try_into().unwrap();
        let second_low: [u8; 16] = second_digest.as_bytes()[16..].try_into().unwrap();

        // The low 128 bits are consulted, not discarded: a probe that keeps the
        // high half and changes only the low half is honest absence, never the
        // neighbouring object's bytes.
        let mut probe = [0_u8; 32];
        probe[..16].copy_from_slice(&first_high);
        probe[16..].copy_from_slice(&second_low);
        assert_eq!(
            reader
                .locate_object(ContentDigest::from_bytes(probe))
                .unwrap(),
            None
        );
        probe[16..].copy_from_slice(&first_low);
        probe[31] ^= 0xff;
        assert_eq!(
            reader
                .locate_object(ContentDigest::from_bytes(probe))
                .unwrap(),
            None
        );
        // The high 128 bits are equally load-bearing.
        let mut probe = *first_digest.as_bytes();
        probe[0] ^= 0xff;
        assert_eq!(
            reader
                .locate_object(ContentDigest::from_bytes(probe))
                .unwrap(),
            None
        );
        // Only the exact full 256-bit key resolves the exact original bytes.
        assert_eq!(reader.object_bytes(first_digest).unwrap(), Some(first));
        assert_eq!(reader.object_bytes(second_digest).unwrap(), Some(second));
    }

    /// Index-domain test: many *keys* sharing one 128-bit prefix.
    ///
    /// This asserts nothing about the fixture payloads' hashes -- a SHA-256
    /// prefix collision cannot be manufactured. It exercises the index
    /// boundary directly, by publishing many distinct full keys that share a
    /// high half, which is exactly the shape a serialized-list bucket with a
    /// byte cap could not represent.
    ///
    /// The composed prefix/inner map that once answered this is gone (P4c2
    /// 4.1): the object domain is one sorted table keyed by the whole 256-bit
    /// digest, so a shared prefix is not a structure at all, merely a run of
    /// adjacent keys. The invariant the test owns is unchanged -- every full
    /// key stays independently addressable, a non-member is honest absence,
    /// and lookup cost stays sublinear in how many keys share the prefix.
    #[test]
    fn many_keys_sharing_one_128_bit_prefix_stay_independently_addressable() {
        const SHARED: usize = 2048;
        let archive = TestArchive::open("shared-prefix");
        let directory = sealed_directory(&archive.store).unwrap();
        let high = [0xa5_u8; 16];
        let mut expected: BTreeMap<[u8; 16], Vec<u8>> = BTreeMap::new();
        let mut cut = SealedCut::open(&directory).unwrap();
        for index in 0..SHARED {
            let mut low = [0x11_u8; 16];
            low[..8].copy_from_slice(&(index as u64).to_be_bytes());
            let mut key = [0_u8; 32];
            key[..16].copy_from_slice(&high);
            key[16..].copy_from_slice(&low);
            let payload = format!("shared-prefix payload {index}").into_bytes();
            let locator = cut
                .add_record(COLD_CLASS_OBJECT, key.to_vec(), &payload)
                .unwrap();
            cut.put(DOMAIN_COLD_OBJECT, &key, &locator.to_bytes())
                .unwrap();
            expected.insert(low, payload);
        }
        cut.publish().unwrap();

        let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
        assert_eq!(
            reader.domain_entries(DOMAIN_COLD_OBJECT).unwrap().len(),
            SHARED
        );
        let sample = {
            let mut key = [0_u8; 32];
            key[..16].copy_from_slice(&high);
            key[16..].copy_from_slice(expected.keys().next().unwrap());
            ContentDigest::from_bytes(key)
        };

        // Fail-before, made structural rather than historical: the replaced
        // representation serialized one prefix as a canonical list of
        // `(sha256[16..32], locator)` pairs under a 64 KiB codec ceiling. This
        // fixture's prefix cannot be expressed that way at all, which is the
        // whole point -- a hash prefix has no small fixed number of members.
        let as_serialized_list = encode_canonical(
            &expected
                .keys()
                .map(|low| (*low, [0_u8; 32]))
                .collect::<Vec<([u8; 16], [u8; 32])>>(),
        )
        .unwrap()
        .len();
        assert!(
            as_serialized_list > 64 * 1024,
            "a serialized prefix list of {SHARED} entries is {as_serialized_list} bytes, \
             which must exceed the replaced 64 KiB bucket ceiling for this to be a real fixture"
        );

        for (low, payload) in &expected {
            let mut key = [0_u8; 32];
            key[..16].copy_from_slice(&high);
            key[16..].copy_from_slice(low);
            let locator = reader
                .locate_object(ContentDigest::from_bytes(key))
                .unwrap()
                .expect("every shared-prefix key is addressable by its own low half");
            assert_eq!(
                &reader
                    .record_bytes(locator, MAX_OBJECT_BYTES as u64)
                    .unwrap(),
                payload,
                "a shared-prefix key must resolve to its own record"
            );
        }

        // A non-member low half under the same prefix is honest absence.
        let mut absent = [0_u8; 32];
        absent[..16].copy_from_slice(&high);
        absent[16..].copy_from_slice(&[0xfe_u8; 16]);
        assert_eq!(
            reader
                .locate_object(ContentDigest::from_bytes(absent))
                .unwrap(),
            None
        );

        // Lookup cost under a 2048-member prefix is one payload read, and the
        // index path is sublinear in occupancy.
        let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
        let last = {
            let mut key = [0_u8; 32];
            key[..16].copy_from_slice(&high);
            key[16..].copy_from_slice(expected.keys().next_back().unwrap());
            ContentDigest::from_bytes(key)
        };
        reader.locate_object(last).unwrap().unwrap();
        let work = reader.work();
        assert_eq!(
            work.pack_reads, 1,
            "a point lookup costs exactly one pack read"
        );
        assert!(
            work.index_nodes * 4 < SHARED,
            "index path {} is not sublinear in {SHARED} shared-prefix members",
            work.index_nodes
        );

        // These synthetic index keys are not their payloads' content addresses,
        // so the resolver still refuses to hand the bytes back under them: the
        // index never launders identity.
        assert!(matches!(
            reader.object_bytes(sample).unwrap_err(),
            StoreError::ColdObjectUnavailable { .. }
        ));
    }

    #[test]
    fn lookup_work_stays_bounded_as_unrelated_history_grows() {
        let archive = TestArchive::open("bounded");
        let probe = publish_batches(&archive, 0..1);
        relocate(&archive, &probe);
        let digest = ContentDigest::of(&probe[0].objects()[0].encode().unwrap());
        let batch_id = probe[0].manifest().batch_id();

        let measure = |archive: &TestArchive| {
            let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
            reader.object_bytes(digest).unwrap().unwrap();
            let object = reader.work();
            let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
            reader.manifest_bytes(batch_id).unwrap().unwrap();
            (object, reader.work())
        };

        let (small_object, small_manifest) = measure(&archive);
        for chunk in 0..8u128 {
            let batches = publish_batches(&archive, (chunk * 64 + 1)..(chunk * 64 + 65));
            relocate(&archive, &batches);
        }
        let roots = cold_history_roots(&archive.store).unwrap().unwrap();
        assert_eq!(
            cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT),
            2 * 512 + 2
        );
        assert_eq!(cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST), 513);

        let (large_object, large_manifest) = measure(&archive);
        // T4: a point read costs exactly ONE payload record read, whatever the
        // history size. `pack_reads` counts every ranged read of a pack, and
        // under sealed-v3 a table IS a packed record, so the payload read is
        // `pack_reads - index_nodes`. (The retired composition also cost an
        // inner-root descriptor read; a sorted table answers the key directly.)
        for (label, work) in [
            ("small object", small_object),
            ("large object", large_object),
            ("small manifest", small_manifest),
            ("large manifest", large_manifest),
        ] {
            assert_eq!(
                work.pack_reads - work.index_nodes,
                1,
                "{label} lookup read {} packs over {} table records; exactly one \
                 of them must be the payload",
                work.pack_reads,
                work.index_nodes
            );
        }
        // Index-path reads grow only with map depth, never with history.
        assert!(
            large_object.index_nodes <= small_object.index_nodes + 24,
            "object index path grew from {} to {} across 512 unrelated objects",
            small_object.index_nodes,
            large_object.index_nodes
        );
        assert!(
            large_object.index_nodes * 8
                < usize::try_from(cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT)).unwrap(),
            "index path {} is not sublinear in {} objects",
            large_object.index_nodes,
            cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT)
        );
    }

    #[test]
    fn duplicate_relocation_is_a_no_op_and_preserves_the_predecessor_root() {
        let archive = TestArchive::open("duplicates");
        let batches = publish_batches(&archive, 0..4);
        let first = relocate(&archive, &batches);
        assert_eq!(first.objects_published, 8);
        assert_eq!(first.objects_already_present, 0);
        let roots = cold_history_roots(&archive.store).unwrap().unwrap();

        let second = relocate(&archive, &batches);
        assert_eq!(second.objects_published, 0);
        assert_eq!(second.manifests_published, 0);
        assert_eq!(second.objects_already_present, 8);
        assert_eq!(second.manifests_already_present, 4);
        assert_eq!(second.packs_published, 0);
        assert_eq!(cold_history_roots(&archive.store).unwrap().unwrap(), roots);

        // A later additive publication extends the same roots without
        // disturbing the predecessor's entries.
        let more = publish_batches(&archive, 4..6);
        relocate(&archive, &more);
        let extended = cold_history_roots(&archive.store).unwrap().unwrap();
        assert_eq!(cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT), 12);
        assert_eq!(cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST), 6);
        for batch in &batches {
            archive.remove_hot_originals(batch);
            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                assert_eq!(
                    archive
                        .store
                        .resolve_logical_object_bytes(ContentDigest::of(&bytes))
                        .unwrap(),
                    bytes
                );
            }
        }
    }

    #[test]
    fn an_interrupted_additive_publication_never_becomes_authority() {
        let archive = TestArchive::open("interrupted");
        let installed = publish_batches(&archive, 0..2);
        relocate(&archive, &installed);
        let root_before = cold_history_roots(&archive.store).unwrap().unwrap();

        // Stage a second publication's packs and index nodes durably, then
        // stop before the root marker -- exactly the crash window.
        let pending = publish_batches(&archive, 2..4);
        let directory = sealed_directory(&archive.store).unwrap();
        let mut objects = BTreeMap::new();
        let mut manifests = BTreeMap::new();
        for batch in &pending {
            manifests.insert(
                batch.manifest().batch_id(),
                batch.manifest().encode().unwrap(),
            );
            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                objects.insert(ContentDigest::of(&bytes), bytes);
            }
        }
        let staged = stage_publication_without_marker(&directory, &objects, &manifests).unwrap();
        assert_eq!(staged.objects_published, 4);

        // Residue exists on disk, and confers nothing.
        assert!(cold_pack_names(&directory).unwrap().len() >= 2);
        assert_eq!(
            cold_history_roots(&archive.store).unwrap().unwrap(),
            root_before
        );
        for batch in &pending {
            archive.remove_hot_originals(batch);
            let batch_id = batch.manifest().batch_id();
            assert!(matches!(
                archive.store.resolve_logical_manifest(batch_id).unwrap(),
                None
            ));
            assert!(matches!(
                archive
                    .store
                    .inspect_batch_with_cold_history(batch_id)
                    .unwrap(),
                BatchInspection::Absent
            ));
        }
        // Everything the installed root names is still exactly resolvable.
        for batch in &installed {
            archive.remove_hot_originals(batch);
            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                assert_eq!(
                    archive
                        .store
                        .resolve_logical_object_bytes(ContentDigest::of(&bytes))
                        .unwrap(),
                    bytes
                );
            }
        }
    }

    #[test]
    fn missing_or_corrupt_cold_data_names_its_logical_object_and_spares_healthy_state() {
        let archive = TestArchive::open("damage");
        let batches = publish_batches(&archive, 0..4);
        relocate(&archive, &batches);
        let damaged = &batches[0];
        let healthy = &batches[3];
        for batch in &batches {
            archive.remove_hot_originals(batch);
        }
        let damaged_digest = ContentDigest::of(&damaged.objects()[0].encode().unwrap());
        let healthy_digest = ContentDigest::of(&healthy.objects()[0].encode().unwrap());

        let directory = archive.cold_directory();

        // 1. A corrupt record inside an intact pack refuses, and names the
        //    exact logical object it could not reconstruct. The pack is chosen
        //    by the damaged object's own locator, not by directory order.
        let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
        let locator = reader.locate_object(damaged_digest).unwrap().unwrap();
        drop(reader);
        let (pack_name, record_start) =
            pack_file_for(&sealed_directory(&archive.store).unwrap(), locator);
        let pack_path = directory.join(pack_name);
        let original = std::fs::read(&pack_path).unwrap();
        let mut corrupt = original.clone();
        let payload_start = record_start + COLD_RECORD_HEADER_BYTES;
        corrupt[payload_start] ^= 0xff;
        std::fs::write(&pack_path, &corrupt).unwrap();
        match archive
            .store
            .resolve_logical_object_bytes(damaged_digest)
            .unwrap_err()
        {
            StoreError::ColdObjectUnavailable { digest, .. } => {
                assert_eq!(digest, damaged_digest);
            }
            other => panic!("expected a named cold-object refusal, got {other:?}"),
        }
        // The unrelated object in the same pack is untouched.
        assert!(archive
            .store
            .resolve_logical_object_bytes(healthy_digest)
            .is_ok());
        std::fs::write(&pack_path, &original).unwrap();

        // 2. A missing pack refuses by name and still spares the hot tier.
        std::fs::remove_file(&pack_path).unwrap();
        match archive
            .store
            .resolve_logical_object_bytes(damaged_digest)
            .unwrap_err()
        {
            StoreError::ColdObjectUnavailable { digest, reason } => {
                assert_eq!(digest, damaged_digest);
                assert!(reason.contains("missing"), "reason was {reason}");
            }
            other => panic!("expected a named cold-object refusal, got {other:?}"),
        }
        // A live batch published after the damage still opens and validates:
        // cold damage is history authority, never active-state authority.
        let live = publish_batches(&archive, 100..101);
        assert!(matches!(
            archive
                .store
                .inspect_batch(live[0].manifest().batch_id())
                .unwrap(),
            BatchInspection::Ready(_)
        ));
        std::fs::write(&pack_path, &original).unwrap();

        // 3. Damaging the object domain's packed TABLE record refuses every
        //    object lookup by name and still never invents absence.
        //
        //    Under the retired composition this was two separate cases -- a
        //    missing outer root node file and a missing inner prefix-map node
        //    file -- because every map node was its own directory entry. Tables
        //    are records inside packs now (I-14: nothing writes a file into the
        //    sealed directory but pack publication and the marker swap), so
        //    there is one index-damage case and localisation to one prefix is
        //    no longer a property the container has.
        let table_locator = {
            let sealed = sealed_directory(&archive.store).unwrap();
            let packs = match read_root_state(&sealed).unwrap() {
                SealedRootState::Published { root, .. } => root.packs,
                _ => panic!("sealed archive has published no pack table"),
            };
            let mut found = None;
            for pack in packs.entries() {
                let inventory = read_pack_footer(&sealed, pack.pack_id()).unwrap();
                for entry in &inventory.footer.entries {
                    if entry.class != COLD_CLASS_TABLE {
                        continue;
                    }
                    let locator = ColdLocatorV1 {
                        offset: entry.offset,
                        length: entry.length,
                    };
                    // A table names its own domain in its header
                    // (`TINETBL1` ‖ schema:u32be ‖ domain id).
                    let bytes =
                        read_pack_range(&sealed, &packs, locator, MAX_COLD_ROOT_BYTES).unwrap();
                    if bytes.get(12).copied() == Some(DOMAIN_COLD_OBJECT.id) {
                        found = Some(locator);
                    }
                }
            }
            found.expect("the object domain has a packed table record")
        };
        let (table_pack, table_start) =
            pack_file_for(&sealed_directory(&archive.store).unwrap(), table_locator);
        let table_path = directory.join(table_pack);
        let table_original = std::fs::read(&table_path).unwrap();
        let mut torn = table_original.clone();
        torn[table_start + COLD_RECORD_HEADER_BYTES] ^= 0xff;
        std::fs::write(&table_path, &torn).unwrap();
        for digest in [damaged_digest, healthy_digest] {
            match archive
                .store
                .resolve_logical_object_bytes(digest)
                .unwrap_err()
            {
                StoreError::ColdObjectUnavailable { digest: named, .. } => {
                    assert_eq!(named, digest);
                }
                other => panic!("expected a named cold-object refusal, got {other:?}"),
            }
        }
        // The hot tier is entirely unaffected by a damaged locator index.
        assert!(matches!(
            archive
                .store
                .inspect_batch(live[0].manifest().batch_id())
                .unwrap(),
            BatchInspection::Ready(_)
        ));
        std::fs::write(&table_path, &table_original).unwrap();
        assert!(archive
            .store
            .resolve_logical_object_bytes(damaged_digest)
            .is_ok());

        // 5. A corrupt root marker refuses the index, not the archive.
        let marker = directory.join("current");
        let marker_bytes = std::fs::read(&marker).unwrap();
        std::fs::write(&marker, b"not a canonical cold root").unwrap();
        // A torn marker over surviving packs is the same named damaged class
        // as a missing one -- the contract has always said so; under sealed-v3
        // both reach it through `SealedRootState::RootLostWithPreservedPacks`
        // rather than through a separate decode refusal.
        assert!(matches!(
            cold_history_roots(&archive.store).unwrap_err(),
            StoreError::ColdHistoryIndexUnavailable(_) | StoreError::ColdHistoryRootMissing
        ));
        assert!(matches!(
            archive
                .store
                .inspect_batch(live[0].manifest().batch_id())
                .unwrap(),
            BatchInspection::Ready(_)
        ));
        std::fs::write(&marker, &marker_bytes).unwrap();
    }

    #[test]
    fn a_lost_root_marker_is_a_named_repair_condition_and_repair_restores_exact_history() {
        let archive = TestArchive::open("root-repair");
        let batches = publish_batches(&archive, 0..4);
        relocate(&archive, &batches);
        let healthy = cold_history_roots(&archive.store).unwrap().unwrap();
        let healthy_objects = cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT);
        let healthy_manifests = cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST);
        for batch in &batches {
            archive.remove_hot_originals(batch);
        }
        let probe = ContentDigest::of(&batches[0].objects()[0].encode().unwrap());
        // Retained in memory only because the hot originals are already gone.
        let archived_manifests: BTreeMap<BatchId, Vec<u8>> = batches
            .iter()
            .map(|batch| {
                (
                    batch.manifest().batch_id(),
                    batch.manifest().encode().unwrap(),
                )
            })
            .collect();

        // Repair on a healthy archive is a no-op.
        assert_eq!(
            archive.store.repair_cold_history_root().unwrap(),
            ColdRepairOutcome::default()
        );

        let marker = archive.cold_directory().join(SEALED_ROOT_MARKER);
        std::fs::remove_file(&marker).unwrap();

        // Every path that could have called this "never published" now names
        // the repair condition instead.
        assert!(matches!(
            SealedArchiveReader::open(&archive.store),
            Err(StoreError::ColdHistoryRootMissing)
        ));
        assert!(matches!(
            cold_history_roots(&archive.store),
            Err(StoreError::ColdHistoryRootMissing)
        ));
        assert!(matches!(
            archive.store.resolve_logical_object_bytes(probe),
            Err(StoreError::ColdHistoryRootMissing)
        ));
        // Publication refuses rather than rooting a fresh empty history over
        // the preserved packs.
        assert!(matches!(
            publish_cold_history(&archive.store, &BTreeMap::new(), &archived_manifests),
            Err(StoreError::ColdHistoryRootMissing)
        ));
        assert!(matches!(
            archive.store.repack_cold_history(),
            Err(StoreError::ColdHistoryRootMissing)
        ));
        // Healthy hot and current state stay available throughout.
        let live = publish_batches(&archive, 100..101);
        assert!(matches!(
            archive
                .store
                .inspect_batch(live[0].manifest().batch_id())
                .unwrap(),
            BatchInspection::Ready(_)
        ));

        let outcome = archive.store.repair_cold_history_root().unwrap();
        assert!(outcome.repaired);
        assert_eq!(outcome.objects_recovered, 8);
        assert_eq!(outcome.manifests_recovered, 4);

        // Repair restored the exact old objects and manifests, with no hot
        // original anywhere on disk.
        let _repaired = cold_history_roots(&archive.store).unwrap().unwrap();
        assert_eq!(
            cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT),
            healthy_objects
        );
        assert_eq!(
            cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST),
            healthy_manifests
        );
        for batch in &batches {
            let batch_id = batch.manifest().batch_id();
            assert_eq!(
                archive
                    .store
                    .resolve_logical_manifest_bytes(batch_id)
                    .unwrap(),
                batch.manifest().encode().unwrap()
            );
            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                assert_eq!(
                    archive
                        .store
                        .resolve_logical_object_bytes(ContentDigest::of(&bytes))
                        .unwrap(),
                    bytes
                );
            }
        }
        // Repaired history accepts ordinary additive publication again -- and
        // recognizes the recovered bytes as exactly present -- and a second
        // repair is once more a no-op.
        assert_eq!(
            publish_cold_history(&archive.store, &BTreeMap::new(), &archived_manifests)
                .unwrap()
                .manifests_already_present,
            4
        );
        assert_eq!(
            archive.store.repair_cold_history_root().unwrap(),
            ColdRepairOutcome::default()
        );
    }

    #[test]
    fn a_never_initialized_archive_is_absence_not_a_repair_condition() {
        let archive = TestArchive::open("never-initialized");
        assert!(SealedArchiveReader::open(&archive.store).unwrap().is_none());
        assert!(cold_history_roots(&archive.store).unwrap().is_none());
        assert_eq!(
            archive.store.repair_cold_history_root().unwrap(),
            ColdRepairOutcome::default()
        );
        // A cold directory that exists but holds no pack is still ordinary
        // absence: this is the shape an interrupted first publication leaves.
        let directory = sealed_directory(&archive.store).unwrap();
        assert!(matches!(
            read_root_state(&directory).unwrap(),
            SealedRootState::NeverInitialized
        ));
        assert!(SealedArchiveReader::open(&archive.store).unwrap().is_none());
        assert_eq!(
            archive.store.repair_cold_history_root().unwrap(),
            ColdRepairOutcome::default()
        );
    }

    #[test]
    fn a_conflicting_manifest_never_displaces_archived_bytes_and_leaves_no_residue() {
        let archive = TestArchive::open("manifest-conflict");
        let batches = publish_batches(&archive, 0..2);
        relocate(&archive, &batches);
        let roots_before = cold_history_roots(&archive.store).unwrap().unwrap();
        let directory = sealed_directory(&archive.store).unwrap();
        let packs_before = cold_pack_names(&directory).unwrap();
        let manifest = batches[0].manifest();
        let original = manifest.encode().unwrap();

        // A different SessionId alone yields a valid, distinct canonical
        // manifest under the same BatchId.
        let conflicting = crate::oplog::OperationBatch::new_with_causality(
            manifest.workspace_id(),
            manifest.lineage_digest(),
            manifest.batch_id(),
            manifest.author_device_id(),
            SessionId::new(),
            manifest.origin(),
            manifest.causal_dot(),
            manifest.causal_dependency_heads().to_vec(),
            manifest.dependency_frontier().clone(),
            manifest.semantic_effect_digest(),
            manifest.required_objects().to_vec(),
        )
        .unwrap();
        let conflicting_bytes = conflicting.encode().unwrap();
        assert_ne!(original, conflicting_bytes);

        match publish_cold_history(
            &archive.store,
            &BTreeMap::new(),
            &BTreeMap::from([(manifest.batch_id(), conflicting_bytes)]),
        )
        .unwrap_err()
        {
            StoreError::ColdManifestConflict { batch_id, .. } => {
                assert_eq!(batch_id, manifest.batch_id());
            }
            other => panic!("expected a named cold-manifest conflict, got {other:?}"),
        }

        // Predecessor root, original bytes and pack set are all untouched: the
        // exactness pass runs before a single record is appended.
        assert_eq!(
            cold_history_roots(&archive.store).unwrap().unwrap(),
            roots_before
        );
        assert_eq!(cold_pack_names(&directory).unwrap(), packs_before);
        for batch in &batches {
            archive.remove_hot_originals(batch);
            assert_eq!(
                archive
                    .store
                    .resolve_logical_manifest_bytes(batch.manifest().batch_id())
                    .unwrap(),
                batch.manifest().encode().unwrap()
            );
        }
        // The identical bytes remain an ordinary counted no-op.
        let repeat = publish_cold_history(
            &archive.store,
            &BTreeMap::new(),
            &BTreeMap::from([(manifest.batch_id(), original)]),
        )
        .unwrap();
        assert_eq!(repeat.manifests_already_present, 1);
        assert_eq!(repeat.manifests_published, 0);
        assert_eq!(repeat.packs_published, 0);
        assert_eq!(
            cold_history_roots(&archive.store).unwrap().unwrap(),
            roots_before
        );
    }

    #[test]
    fn repacking_preserves_every_logical_identity_and_byte() {
        let archive = TestArchive::open("repack");
        let batches = publish_batches(&archive, 0..5);
        relocate(&archive, &batches);
        let before_roots = cold_history_roots(&archive.store).unwrap().unwrap();
        let before_objects = cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT);
        let before_manifests = cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST);
        let before_locators: BTreeMap<ContentDigest, ColdLocatorV1> = {
            let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
            batches
                .iter()
                .flat_map(|batch| batch.objects())
                .map(|object| {
                    let digest = ContentDigest::of(&object.encode().unwrap());
                    (digest, reader.locate_object(digest).unwrap().unwrap())
                })
                .collect()
        };
        for batch in &batches {
            archive.remove_hot_originals(batch);
        }

        let outcome = archive.store.repack_cold_history().unwrap();
        assert_eq!(outcome.objects_published, 10);
        assert_eq!(outcome.manifests_published, 5);
        let after_roots = cold_history_roots(&archive.store).unwrap().unwrap();
        let _ = after_roots;
        assert_eq!(
            cold_domain_count(&archive.store, DOMAIN_COLD_OBJECT),
            before_objects
        );
        assert_eq!(
            cold_domain_count(&archive.store, DOMAIN_COLD_MANIFEST),
            before_manifests
        );

        let reader = SealedArchiveReader::open(&archive.store).unwrap().unwrap();
        let mut relocated = 0;
        for batch in &batches {
            let batch_id = batch.manifest().batch_id();
            assert_eq!(
                archive
                    .store
                    .resolve_logical_manifest_bytes(batch_id)
                    .unwrap(),
                batch.manifest().encode().unwrap(),
                "a repack must not change a manifest byte"
            );
            for object in batch.objects() {
                let bytes = object.encode().unwrap();
                let digest = ContentDigest::of(&bytes);
                assert_eq!(
                    archive.store.resolve_logical_object_bytes(digest).unwrap(),
                    bytes,
                    "a repack must not change an object byte"
                );
                let after = reader.locate_object(digest).unwrap().unwrap();
                if after != before_locators[&digest] {
                    relocated += 1;
                }
            }
        }
        assert!(
            relocated > 0,
            "the repack must actually move records, else it proves nothing"
        );
        // Publish-new-before-retire-old: the predecessor packs are still there.
        assert!(
            cold_pack_names(&sealed_directory(&archive.store).unwrap())
                .unwrap()
                .len()
                >= 2
        );
    }

    // -----------------------------------------------------------------------
    // Manager negative controls (evidence/rebaselining-2026-09-07/
    // p2-manager-negative-controls/regression-tests.rs). Both reproduced on the
    // pass-1 source; they are retained verbatim in intent here.
    // -----------------------------------------------------------------------

    #[test]
    fn manager_cold_duplicate_batch_id_requires_exact_manifest_bytes() {
        let archive = TestArchive::open("manager-conflicting-manifest");
        let batches = publish_batches(&archive, 0..1);
        relocate(&archive, &batches);
        let manifest = batches[0].manifest();
        let original = manifest.encode().unwrap();
        let changed = crate::oplog::OperationBatch::new_with_causality(
            manifest.workspace_id(),
            manifest.lineage_digest(),
            manifest.batch_id(),
            manifest.author_device_id(),
            SessionId::new(),
            manifest.origin(),
            manifest.causal_dot(),
            manifest.causal_dependency_heads().to_vec(),
            manifest.dependency_frontier().clone(),
            manifest.semantic_effect_digest(),
            manifest.required_objects().to_vec(),
        )
        .unwrap();
        let changed_bytes = changed.encode().unwrap();
        assert_eq!(manifest.batch_id(), changed.batch_id());
        assert_ne!(original, changed_bytes);
        let result = publish_cold_history(
            &archive.store,
            &BTreeMap::new(),
            &BTreeMap::from([(changed.batch_id(), changed_bytes)]),
        );
        assert!(
            result.is_err(),
            "a conflicting manifest is not an already archived exact copy"
        );
        assert_eq!(
            SealedArchiveReader::open(&archive.store)
                .unwrap()
                .unwrap()
                .manifest_bytes(manifest.batch_id())
                .unwrap()
                .unwrap(),
            original
        );
    }

    /// Refusal scenario (I-2, D-3): a reader that read the marker, then had the
    /// pack that marker named retired underneath it by a later cut.
    ///
    /// Marker-last publication makes this window real: a cut installs its
    /// marker and only THEN retires the packs the previous marker named, so any
    /// reader between "read the marker" and "resolve the root record through
    /// it" can find the file already gone. Misreading that as damage is
    /// expensive — `contains_any_pack` then reports
    /// `RootLostWithPreservedPacks`, which `SealedCut::open` and
    /// `SealedArchiveReader::open_directory` turn into a hard
    /// `ColdHistoryRootMissing` refusal on a perfectly intact archive. The
    /// window is sub-millisecond in production, so the interleaving is injected
    /// rather than raced for: the hook fires exactly once, between the marker
    /// read and the root resolution.
    #[test]
    fn a_torn_marker_read_across_a_pack_retiring_cut_is_recovered_not_refused() {
        let archive = std::sync::Arc::new(TestArchive::open("torn-marker-read"));
        let seeded = publish_batches(&archive, 0..2);
        relocate(&archive, &seeded);
        let target = seeded[0].manifest().batch_id();
        // Only the cold tier can answer once the hot originals are gone.
        archive.remove_hot_originals(&seeded[0]);

        // Arm the interleaving: after the next marker read, advance the archive
        // far enough that a pack tier merge retires the packs that marker named.
        let hook_archive = std::sync::Arc::clone(&archive);
        *TORN_ROOT_READ_HOOK.lock().unwrap() = Some(Box::new(move || {
            for seed in 100..120_u128 {
                let batches = publish_batches(&hook_archive, seed..seed + 1);
                relocate(&hook_archive, &batches);
            }
        }));

        let bytes = archive
            .store
            .resolve_logical_manifest_bytes(target)
            .expect("an intact cold manifest stays readable across a pack-retiring cut");
        assert_eq!(
            super::super::OperationBatch::decode(&bytes)
                .unwrap()
                .batch_id(),
            target,
            "the recovered read must resolve the same logical record"
        );
        assert!(
            TORN_ROOT_READ_HOOK.lock().unwrap().is_none(),
            "the one-shot interleaving hook must have fired"
        );
    }

    #[test]
    fn manager_torn_cold_root_is_rebuildable_from_preserved_packs() {
        let archive = TestArchive::open("manager-torn-root-repair");
        let batches = publish_batches(&archive, 0..1);
        relocate(&archive, &batches);
        let batch_id = batches[0].manifest().batch_id();
        let original = batches[0].manifest().encode().unwrap();
        archive.remove_hot_originals(&batches[0]);
        assert!(!archive.store.repair_cold_history_root().unwrap().repaired);
        let original_object = batches[0].objects()[0].encode().unwrap();
        let object_digest = ContentDigest::of(&original_object);
        // Named in-scope fault: torn derived marker; original pack bytes survive.
        std::fs::write(archive.cold_directory().join(SEALED_ROOT_MARKER), b"torn").unwrap();
        assert!(SealedArchiveReader::open(&archive.store).is_err());
        let repaired = archive.store.repair_cold_history_root();
        assert!(
            repaired.is_ok(),
            "derived marker damage must be repairable from intact original packs: {repaired:?}"
        );
        assert!(repaired.unwrap().repaired);
        assert_eq!(
            SealedArchiveReader::open(&archive.store)
                .unwrap()
                .unwrap()
                .manifest_bytes(batch_id)
                .unwrap()
                .unwrap(),
            original
        );
        assert_eq!(
            archive
                .store
                .resolve_logical_object_bytes(object_digest)
                .unwrap(),
            original_object
        );
        std::fs::remove_file(archive.cold_directory().join(SEALED_ROOT_MARKER)).unwrap();
        assert!(archive.store.repair_cold_history_root().unwrap().repaired);
        assert_eq!(
            SealedArchiveReader::open(&archive.store)
                .unwrap()
                .unwrap()
                .manifest_bytes(batch_id)
                .unwrap()
                .unwrap(),
            original
        );
    }

    #[test]
    fn a_damaged_root_with_no_preserved_packs_is_never_replaced_with_empty_history() {
        let archive = TestArchive::open("torn-root-no-packs");
        let directory = sealed_directory(&archive.store).unwrap();
        assert!(cold_pack_names(&directory).unwrap().is_empty());
        std::fs::write(
            archive.cold_directory().join(SEALED_ROOT_MARKER),
            b"torn with nothing behind it",
        )
        .unwrap();
        assert!(matches!(
            archive.store.repair_cold_history_root(),
            Err(StoreError::ColdHistoryIndexUnavailable(_))
        ));
        assert_eq!(
            std::fs::read(archive.cold_directory().join(SEALED_ROOT_MARKER)).unwrap(),
            b"torn with nothing behind it"
        );
    }

    #[test]
    fn manager_cold_missing_root_must_not_report_never_published() {
        let archive = TestArchive::open("manager-missing-root");
        let batches = publish_batches(&archive, 0..1);
        relocate(&archive, &batches);
        archive.remove_hot_originals(&batches[0]);
        std::fs::remove_file(archive.cold_directory().join(SEALED_ROOT_MARKER)).unwrap();
        let reopened = SealedArchiveReader::open(&archive.store);
        assert!(
            !matches!(reopened, Ok(None)),
            "preserved cold packs with a lost derived marker need repair or a named error, not ordinary absence"
        );
    }

    #[test]
    fn cold_publication_reuses_the_shared_publication_and_index_primitives() {
        let source = include_str!("cold_object_store.rs");
        // Split at the TEST MODULE, not at the first `#[cfg(test)]`: this
        // module now carries a `cfg(test)` production helper above it, and
        // splitting there silently scanned only the first quarter of the file.
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("the module has a production region");
        // Every `#[cfg(test)]` item inside the production region is a named
        // `*_for_test` helper, which is what lets `shipped_source` strip them
        // by item instead of truncating the scan at the first one.
        for (index, chunk) in production.split("#[cfg(test)]").enumerate().skip(1) {
            let item = chunk
                .lines()
                .find(|line| line.contains("fn "))
                .unwrap_or_default();
            assert!(
                item.contains("_for_test"),
                "production `#[cfg(test)]` item {index} is not a named test helper: {item}"
            );
        }
        let shipped = shipped_source(source);
        for forbidden in ["fs::write", "fs::rename", "OpenOptions", "create_new"] {
            assert!(
                !shipped.contains(forbidden),
                "cold publication must not reimplement a durable write primitive: {forbidden}"
            );
        }
        // `SealedAcceptedIndexWriter`/`Reader` (the path-copied treap) are gone
        // with P4c2 4.1; the sorted-table primitives replace them.
        for required in [
            "DurableDirectoryPublication",
            "publish_new_exact_single_writer",
            "tine_storage::sealed_tables",
            "TierPlan",
        ] {
            assert!(
                production.contains(required),
                "missing primitive: {required}"
            );
        }
    }

    /// The module's SHIPPED source: production minus its `#[cfg(test)]` items.
    ///
    /// Splitting at the first `#[cfg(test)]` is the trap: this module keeps a
    /// damage-fixture helper in the production region, so that split silently
    /// scanned a fifth of the file and every guard below passed vacuously.
    /// `production_cfg_test_items_are_named_for_test` keeps the items this
    /// strips identifiable.
    fn shipped_source(source: &str) -> String {
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("the module has a production region");
        let mut shipped = String::with_capacity(production.len());
        let mut rest = production;
        while let Some(at) = rest.find("#[cfg(test)]") {
            shipped.push_str(&rest[..at]);
            let item = &rest[at..];
            let end = item.find("\n}\n").map_or(item.len(), |offset| offset + 3);
            rest = &item[end..];
        }
        shipped.push_str(rest);
        shipped
    }

    /// I-14: exactly ONE production path creates a file in `sealed-v3`.
    ///
    /// Two, historically: the treap wrote one file per authenticated-map node
    /// (`sealed-v2-<kind>-<digest>`), which is what made a cut's directory-entry
    /// count grow with history and ran the flat directory into ext4's htree
    /// limit. Under sorted tables a table is a RECORD inside a pack, so the
    /// only creators are pack publication and the marker swap; that is the
    /// property this guard makes unwritable rather than merely intended.
    #[test]
    fn only_pack_publication_and_the_marker_swap_create_a_sealed_file() {
        let shipped = shipped_source(include_str!("cold_object_store.rs"));
        for creator in [
            "publish_new_exact_single_writer",
            "publish_immutable_exact_single_writer",
            "replace_exact",
            "create_dir",
        ] {
            let sites = shipped.matches(creator).count();
            let allowed = match creator {
                // `publish_pack` -- the exemplar -- and the marker's first
                // publication, which has no predecessor to replace.
                "publish_new_exact_single_writer" => 2,
                // the marker swap.
                "replace_exact" => 1,
                _ => 0,
            };
            assert_eq!(
                sites, allowed,
                "I-14: `{creator}` appears {sites} times in the sealed container's \
                 production region but only {allowed} file-creating site is allowed. \
                 Exactly two production paths may create a file in `{SEALED_DIRECTORY}`: \
                 pack publication (`publish_pack` -- the exemplar to imitate) and the \
                 marker swap. A per-node or per-record file is what made directory \
                 entries grow with history; put the bytes in a pack record instead."
            );
        }
        // The device-wide absence map has exactly one consumer; a second one
        // would be a second producer of the same answer (I-12).
        let oplog = include_str!("mod.rs");
        let _ = oplog;
        let consumers = [
            include_str!("receiver_absence_summary.rs"),
            include_str!("absence_sweep.rs"),
        ]
        .iter()
        .filter(|source| source.contains("receiver_absence_map::"))
        .count();
        assert_eq!(
            consumers, 1,
            "`receiver_absence_map` must have exactly ONE consumer \
             (`receiver_absence_summary`); a second producer of the same \
             absence answer is the I-12 shape this guard exists to stop"
        );
    }

    #[test]
    fn contract_names_the_current_cold_representation_and_read_resolution() {
        let contract = include_str!("../../../../docs/storage-sync-contract.md");
        for required in [
            "sealed-v3",
            "pack-v1-<uuid>",
            "ColdLocatorV1",
            "ColdHistoryRootMissing",
            "repair_cold_history_root",
            "inspect_batch_with_cold_history",
            "resolve_logical_object_bytes",
            "CoveredBatchRedelivery",
            "MidPackTierMerge",
            "deliberately uncached",
            "a_document_scale_domain_is_hashed_once_per_reader_not_once_per_lookup",
            "a_torn_marker_read_across_a_pack_retiring_cut_is_recovered_not_refused",
            "SEALED_ROOT_READ_ATTEMPTS",
        ] {
            assert!(contract.contains(required), "missing contract: {required}");
        }
        // The torn-read retry bound is load-bearing in the contract's prose, so
        // pin the number against the code rather than letting it drift.
        assert!(
            contract.contains(&format!(
                "`SEALED_ROOT_READ_ATTEMPTS` = {SEALED_ROOT_READ_ATTEMPTS}"
            )),
            "the contract must state the current torn-read retry bound"
        );
        // The two pinned directory names are DIFFERENT commit points and the
        // contract must name both (P4c2 4.4).
        assert!(contract.contains(SEALED_DIRECTORY));
        assert!(
            contract.contains("`current` is the sealed"),
            "the contract must say what the `current` marker commits"
        );
        assert!(
            contract.contains("`checkpoint` is the two-slot checkpoint pointer"),
            "the contract must say what the `checkpoint` pointer commits"
        );
        // Load-bearing constants, asserted against the code rather than
        // restated: drift fails here instead of accumulating silently.
        assert!(
            contract.contains(&format!("`R = {SEALED_TIER_FANOUT}`")),
            "the contract must name the current tier fanout {SEALED_TIER_FANOUT}"
        );
        assert!(
            contract.contains(&format!("{} MiB", COLD_PACK_TARGET_BYTES / (1024 * 1024))),
            "the contract must name the current pack construction target"
        );
        assert!(
            contract.contains(&format!("({COLD_RECORD_HEADER_BYTES} header bytes)")),
            "the contract must name the current record header width"
        );
        // Every domain the code defines appears in the contract's domain table
        // with its exact widths.
        for domain in all_domains() {
            let row = format!("| {} |", domain.id);
            let ranged = matches!(domain.id, 6..=13);
            assert!(
                contract.contains(&row) || ranged,
                "the contract's domain table omits domain {}",
                domain.id
            );
        }
        for width in [
            format!("{DOCUMENT_ROSTER_KEY_BYTES} (framed `DocumentKey`)"),
            format!("{IDENTITY_KEY_BYTES} (framed)"),
        ] {
            assert!(
                contract.contains(&width),
                "the contract's domain table does not carry the current width: {width}"
            );
        }
        // The frontier root's tail semantics are contract, and the two values a
        // reader must agree with a writer about are pinned against the code.
        assert!(
            contract.contains(&format!(
                "schema version {}",
                crate::oplog::hot_engine::ACCEPTED_FRONTIER_ROOT_SCHEMA_VERSION
            )),
            "the contract must name the current accepted-frontier root schema version"
        );
        for required in [
            "tine/oplog/accepted-frontier/v9",
            "rebased_on_an_empty_generation_tail",
            "open_generation",
            "is_physical_archive_damage",
            "EngineError::ArchiveDamaged",
        ] {
            assert!(contract.contains(required), "missing contract: {required}");
        }
    }
}
