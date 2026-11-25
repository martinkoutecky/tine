//! Fresh vehicles, explicit durable terminals, and write-sequence selection.
use super::io::{HostIo, Witness};
use super::{Base, Text};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Record {
    pub page: String,
    pub wseq: u64,
    pub version: u64,
    pub base: Base,
    pub bytes: Text,
}

/// A4 trash custody marker: the exact page key and payload basename.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Marker {
    pub page: String,
    pub payload: String,
}

const DRAFT: &[u8; 8] = b"TINEDRF2";
const MARKER: &[u8; 8] = b"TINETCM2";

/// One envelope for every app-data record: magic, version 2, the repository's
/// existing postcard serializer, SHA-256 of everything before it.
fn seal(magic: &[u8; 8], value: &impl Serialize) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.push(2);
    bytes.extend(postcard::to_stdvec(value).expect("app-data serialization"));
    let checksum = Sha256::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    bytes
}

fn open<T: serde::de::DeserializeOwned>(magic: &[u8; 8], bytes: &[u8]) -> Result<T, ()> {
    if bytes.len() < 41 || bytes.get(..8) != Some(magic.as_slice()) || bytes[8] != 2 {
        return Err(());
    }
    let end = bytes.len() - 32;
    if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..] {
        return Err(());
    }
    postcard::from_bytes(&bytes[9..end]).map_err(|_| ())
}

pub(super) fn encode(records: &[Record]) -> Vec<u8> {
    seal(DRAFT, &records)
}

/// Zero-based `(receiver, source)` record indexes of a paired operation.
pub(super) type Pairs = Vec<(u32, u32)>;

/// The escape byte opening every extended draft body. A legacy body opens
/// with its nonempty record vector's length, never zero, so an older reader
/// decodes an extended body as an empty vector and rejects it.
const ESCAPE: u8 = 0;
const TAG_OPERATION: u8 = 0;
const TAG_DECISION: u8 = 1;

/// A durable choice on one paired endpoint, the receiver or the source
/// (the design's tag 1 record).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Decision {
    /// The operation's stable request id: its vehicle's filename UUID.
    pub op: [u8; 16],
    /// The chosen endpoint's (receiver's or source's) zero-based record
    /// index in the operation draft.
    pub member: u32,
    pub wseq: u64,
    pub choice: Choice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// `0`: take disk (Use Disk). On the receiver it cancels the move, on
    /// the source its removal.
    TakeDisk,
    /// `1`: reviewed text; `2`: superseding editor input. Superseding
    /// input on a source is the durable choice that its newer text replaces
    /// the removal (written before that text publishes); reviewed text, or
    /// either on a receiver, is acted on by no code yet: an operation with
    /// one blocks its destructive step.
    Text {
        superseding: bool,
        version: u64,
        base: Base,
        bytes: Text,
    },
}

impl Choice {
    /// `2`: newer editor input published over a source's removal.
    pub fn superseding(&self) -> bool {
        matches!(
            self,
            Choice::Text {
                superseding: true,
                ..
            }
        )
    }
}

/// One extended or legacy draft body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Body {
    /// The legacy body: a nonempty record vector, recovered exactly as before.
    Records(Vec<Record>),
    /// Tag 0: a paired operation draft.
    Operation { records: Vec<Record>, pairs: Pairs },
    /// Tag 1: a decision record.
    Decision(Decision),
}

fn seal_raw(magic: &[u8; 8], body: &[u8]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.push(2);
    bytes.extend_from_slice(body);
    let checksum = Sha256::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    bytes
}

fn open_raw<'a>(magic: &[u8; 8], bytes: &'a [u8]) -> Result<&'a [u8], ()> {
    if bytes.len() < 41 || bytes.get(..8) != Some(magic.as_slice()) || bytes[8] != 2 {
        return Err(());
    }
    let end = bytes.len() - 32;
    if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..] {
        return Err(());
    }
    Ok(&bytes[9..end])
}

fn take<'a, T: serde::de::DeserializeOwned>(rest: &mut &'a [u8]) -> Result<T, ()> {
    let (value, tail) = postcard::take_from_bytes(rest).map_err(|_| ())?;
    *rest = tail;
    Ok(value)
}

fn push(body: &mut Vec<u8>, value: &impl Serialize) {
    body.extend(postcard::to_stdvec(value).expect("app-data serialization"));
}

/// A paired operation draft: escape, tag 0, the ordered records, the pairs.
pub(super) fn encode_operation(records: &[Record], pairs: &[(u32, u32)]) -> Vec<u8> {
    let mut body = vec![ESCAPE, TAG_OPERATION];
    push(&mut body, &records);
    push(&mut body, &pairs);
    seal_raw(DRAFT, &body)
}

/// A decision record: escape, tag 1, 16 raw UUID bytes, member, write
/// sequence, one choice byte and the choice's payload.
pub(super) fn encode_decision(decision: &Decision) -> Vec<u8> {
    let mut body = vec![ESCAPE, TAG_DECISION];
    body.extend_from_slice(&decision.op);
    push(&mut body, &decision.member);
    push(&mut body, &decision.wseq);
    match &decision.choice {
        Choice::TakeDisk => body.push(0),
        Choice::Text {
            superseding,
            version,
            base,
            bytes,
        } => {
            body.push(if *superseding { 2 } else { 1 });
            push(&mut body, version);
            push(&mut body, base);
            push(&mut body, bytes);
        }
    }
    seal_raw(DRAFT, &body)
}

fn valid_records(records: &[Record]) -> bool {
    let mut pages = BTreeSet::new();
    !records.is_empty() && records.iter().all(|r| r.wseq != 0 && pages.insert(&r.page))
}

/// Bounds, disjointness, a known present source base with a deletion
/// effect, and a present receiver proposal.
fn valid_pairs(records: &[Record], pairs: &[(u32, u32)]) -> bool {
    let mut seen = BTreeSet::new();
    !pairs.is_empty()
        && pairs.iter().all(|&(receiver, source)| {
            let (Some(r), Some(s)) = (records.get(receiver as usize), records.get(source as usize))
            else {
                return false;
            };
            receiver != source
                && seen.insert(receiver)
                && seen.insert(source)
                && r.bytes.is_some()
                && s.bytes.is_none()
                && matches!(&s.base, Base::Known(Some(_)))
        })
}

/// The `(receiver, source)` record indexes of `moves`, when they form a
/// valid pairing of `records` (else the operation keeps the legacy body).
pub(super) fn pairing(records: &[Record], moves: &[(String, String)]) -> Option<Pairs> {
    let index = |page: &str| {
        let at = records.iter().position(|r| r.page == page)?;
        u32::try_from(at).ok()
    };
    let pairs = moves
        .iter()
        .map(|(receiver, source)| Some((index(receiver)?, index(source)?)))
        .collect::<Option<Pairs>>()?;
    (valid_records(records) && valid_pairs(records, &pairs)).then_some(pairs)
}

/// The one decoder for every draft body. Extended bodies must be consumed
/// exactly; anything else is unreadable.
pub(super) fn decode_body(bytes: &[u8]) -> Result<Body, ()> {
    let body = open_raw(DRAFT, bytes)?;
    if body.first() != Some(&ESCAPE) {
        let records: Vec<Record> = postcard::from_bytes(body).map_err(|_| ())?;
        return valid_records(&records)
            .then_some(Body::Records(records))
            .ok_or(());
    }
    let mut rest = &body[1..];
    let tag: u8 = *rest.first().ok_or(())?;
    rest = &rest[1..];
    let result = match tag {
        TAG_OPERATION => {
            let records: Vec<Record> = take(&mut rest)?;
            let pairs: Pairs = take(&mut rest)?;
            if !valid_records(&records) || !valid_pairs(&records, &pairs) {
                return Err(());
            }
            Body::Operation { records, pairs }
        }
        TAG_DECISION => {
            let op: [u8; 16] = rest.get(..16).ok_or(())?.try_into().map_err(|_| ())?;
            rest = &rest[16..];
            let member: u32 = take(&mut rest)?;
            let wseq: u64 = take(&mut rest)?;
            let choice = *rest.first().ok_or(())?;
            rest = &rest[1..];
            let choice = match choice {
                0 => Choice::TakeDisk,
                1 | 2 => Choice::Text {
                    superseding: choice == 2,
                    version: take(&mut rest)?,
                    base: take(&mut rest)?,
                    bytes: take(&mut rest)?,
                },
                _ => return Err(()),
            };
            if wseq == 0 {
                return Err(());
            }
            Body::Decision(Decision {
                op,
                member,
                wseq,
                choice,
            })
        }
        _ => return Err(()),
    };
    rest.is_empty().then_some(result).ok_or(())
}

/// A record vehicle's records (legacy or paired operation draft).
pub(super) fn decode(bytes: &[u8]) -> Result<Vec<Record>, ()> {
    match decode_body(bytes)? {
        Body::Records(records) | Body::Operation { records, .. } => Ok(records),
        Body::Decision(_) => Err(()),
    }
}

/// A paired operation draft's pairs, or None for any other body.
pub(super) fn pairs_of(bytes: &[u8]) -> Option<Pairs> {
    match decode_body(bytes).ok()? {
        Body::Operation { pairs, .. } => Some(pairs),
        _ => None,
    }
}

pub(super) fn encode_marker(marker: &Marker) -> Vec<u8> {
    seal(MARKER, marker)
}

/// A marker names one file directly inside the trash directory, or it is
/// malformed and quarantined, never acted on.
pub(super) fn decode_marker(bytes: &[u8]) -> Result<Marker, ()> {
    let marker: Marker = open(MARKER, bytes)?;
    let mut parts = std::path::Path::new(&marker.payload).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) if !marker.page.is_empty() => Ok(marker),
        _ => Err(()),
    }
}

#[derive(Default)]
pub(super) struct Scan {
    pub files: BTreeMap<String, Vec<Record>>,
    pub logical: BTreeMap<String, Record>,
    pub unreadable: BTreeSet<String>,
    pub max_version: u64,
    pub max_wseq: u64,
    /// Paired operation drafts' pairs, by vehicle name (a subset of `files`).
    pub pairs: BTreeMap<String, Pairs>,
    /// Readable decisions matching a readable paired operation, by name.
    pub decisions: BTreeMap<String, Decision>,
    /// Decision files that block their (readable, paired) operation's
    /// destructive step: unreadable, mismatched, or a choice this code does
    /// not act on. Kept in place, never quarantined: moving one away would
    /// let the next launch replay the removal it may cancel.
    pub blocking: BTreeMap<String, String>,
    /// Readable decisions whose operation is durably absent: cleanup only.
    pub stale_decisions: BTreeSet<String>,
    /// Paired operation drafts that copy a strict subset of another paired
    /// draft's records: a settlement rewrite whose original is still on
    /// disk, so it is not committed. Neither recovered nor adopted; cleanup
    /// only (the original keeps every obligation).
    pub rewrites: BTreeSet<String>,
}

/// `p-<page-hash>-<nonce>-<op-uuid>-<member>.draft`: (op uuid, member).
pub(super) fn decision_name(name: &str) -> Option<([u8; 16], u32)> {
    let rest = name.strip_prefix("p-")?.strip_suffix(".draft")?;
    let mut parts = rest.split('-');
    let (hash, nonce, op, member) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_hexdigit());
    if parts.next().is_some() || !hex(hash, 32) || !hex(nonce, 16) || !hex(op, 32) {
        return None;
    }
    if member.is_empty() || !member.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let op = uuid::Uuid::parse_str(op).ok()?.into_bytes();
    Some((op, member.parse().ok()?))
}

/// The operation vehicle name of a request id.
pub(super) fn op_vehicle(op: &[u8; 16]) -> String {
    format!("op-{}.draft", uuid::Uuid::from_bytes(*op).simple())
}

/// The request id an operation vehicle's name carries.
pub(super) fn op_id(name: &str) -> Option<[u8; 16]> {
    let id = name.strip_prefix("op-")?.strip_suffix(".draft")?;
    Some(uuid::Uuid::try_parse(id).ok()?.into_bytes())
}

/// A fresh decision vehicle name for `member` (endpoint `page`, the receiver
/// or the source) of `op`.
pub(super) fn decision_vehicle(page: &str, op: &[u8; 16], member: u32) -> String {
    let page = page_name(page);
    let stem = page.strip_suffix(".draft").unwrap();
    format!(
        "{stem}-{}-{member}.draft",
        uuid::Uuid::from_bytes(*op).simple()
    )
}

/// Files are the authority. Equal sequences may only be identical copies.
pub(super) fn scan(files: Vec<(String, Vec<u8>)>) -> Scan {
    let mut result = Scan::default();
    let mut sequences: BTreeMap<u64, (Record, BTreeSet<String>)> = BTreeMap::new();
    let mut decisions = vec![];
    for (name, bytes) in files {
        if let Some((op, member)) = decision_name(&name) {
            decisions.push((name, op, member, decode_body(&bytes)));
            continue;
        }
        if let Some(pairs) = pairs_of(&bytes).filter(|_| name.starts_with("op-")) {
            result.pairs.insert(name.clone(), pairs);
        }
        let Some(records) = vehicle(&name, &bytes) else {
            result.unreadable.insert(name);
            continue;
        };
        for record in &records {
            result.max_version = result.max_version.max(record.version);
            result.max_wseq = result.max_wseq.max(record.wseq);
            let (previous, owners) = sequences
                .entry(record.wseq)
                .or_insert_with(|| (record.clone(), BTreeSet::new()));
            owners.insert(name.clone());
            if previous != record {
                result.unreadable.extend(owners.iter().cloned());
            }
        }
        result.files.insert(name, records);
    }
    // A third identical copy of either side of a bad sequence is bad too.
    let bad_sequences: BTreeSet<_> = result
        .unreadable
        .iter()
        .filter_map(|name| result.files.get(name))
        .flatten()
        .map(|r| r.wseq)
        .collect();
    for (name, records) in &result.files {
        if records.iter().any(|r| bad_sequences.contains(&r.wseq)) {
            result.unreadable.insert(name.clone());
        }
    }
    result
        .files
        .retain(|name, _| !result.unreadable.contains(name));
    result
        .pairs
        .retain(|name, _| result.files.contains_key(name));
    // A settlement rewrite commits when its original's unlink is durable;
    // until then the original is the operation. Records are equal only as
    // copies (one write sequence each), so only a rewrite is a strict subset.
    let rewrites: Vec<String> = (result.pairs.keys())
        .filter(|name| {
            let records = &result.files[*name];
            result.pairs.keys().any(|other| {
                let original = &result.files[other];
                other != *name
                    && original.len() > records.len()
                    && records.iter().all(|r| original.contains(r))
            })
        })
        .cloned()
        .collect();
    for name in rewrites {
        result.files.remove(&name);
        result.pairs.remove(&name);
        result.rewrites.insert(name);
    }
    // Cross-check each decision with its operation: filename, id, member
    // (a receiver's or a source's index, its page the name's hash) and
    // sequence.
    for (name, op, member, body) in decisions {
        let vehicle = op_vehicle(&op);
        let Some(pairs) = result.pairs.get(&vehicle) else {
            match body {
                // An operation durably absent leaves its decision as cleanup.
                Ok(Body::Decision(_)) if !result.files.contains_key(&vehicle) => {
                    result.stale_decisions.insert(name);
                }
                _ => {
                    result.unreadable.insert(name);
                }
            }
            continue;
        };
        let records = &result.files[&vehicle];
        let endpoint = pairs
            .iter()
            .flat_map(|&(receiver, source)| [receiver, source])
            .find(|&index| index == member)
            .map(|index| &records[index as usize]);
        let matches = |d: &Decision| {
            d.op == op
                && d.member == member
                && endpoint.is_some_and(|r| name.starts_with(&page_prefix(&r.page)))
        };
        match body {
            Ok(Body::Decision(d)) if matches(&d) => {
                result.max_wseq = result.max_wseq.max(d.wseq);
                // Take disk on either endpoint, or superseding input on a
                // source, is acted on; anything else blocks the operation.
                let source = pairs.iter().any(|&(_, source)| source == member);
                if !(d.choice == Choice::TakeDisk || source && d.choice.superseding()) {
                    result.blocking.insert(name.clone(), vehicle);
                }
                result.decisions.insert(name, d);
            }
            _ => {
                result.blocking.insert(name, vehicle);
            }
        }
    }
    result.logical = logical(result.files.values());
    result
}

/// `p-<page-hash>-`: the start of every vehicle name for `page`.
fn page_prefix(page: &str) -> String {
    let digest = Sha256::digest(page.as_bytes());
    let key: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("p-{key}-")
}

/// One vehicle's records, or None when the file is unreadable by itself
/// (format, checksum, or a name that does not match its records).
pub(super) fn vehicle(name: &str, bytes: &[u8]) -> Option<Vec<Record>> {
    let records = decode(bytes).ok()?;
    ((name.starts_with("p-") && records.len() == 1 || name.starts_with("op-"))
        && name.ends_with(".draft"))
    .then_some(records)
}

/// The highest write sequence per page.
pub(super) fn logical<'a>(
    files: impl IntoIterator<Item = &'a Vec<Record>>,
) -> BTreeMap<String, Record> {
    let mut logical = BTreeMap::<String, Record>::new();
    for record in files.into_iter().flatten() {
        super::work(super::Work::DraftRecords, 1);
        let current = logical
            .entry(record.page.clone())
            .or_insert_with(|| record.clone());
        if record.wseq > current.wseq {
            *current = record.clone();
        }
    }
    logical
}

pub(super) fn page_name(page: &str) -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{}{}.draft", page_prefix(page), random.get(..16).unwrap())
}

pub(super) fn op_name() -> String {
    format!("op-{}.draft", uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Temp,
    Rename,
    Sync,
    CleanupUnlink,
    CleanupSync,
    Unlink,
    UnlinkSync,
    Present,
    Absent,
}

#[derive(Clone, Debug)]
pub(super) struct Vehicle {
    pub name: String,
    pub bytes: Option<Vec<u8>>,
    pub stage: Stage,
    pub failures: u32,
    sync_failures: u32,
}

impl Vehicle {
    pub fn write(name: String, records: &[Record]) -> Self {
        Self {
            name,
            bytes: Some(encode(records)),
            stage: Stage::Temp,
            failures: 0,
            sync_failures: 0,
        }
    }

    /// A paired operation draft (immutable once durable).
    pub fn write_operation(name: String, records: &[Record], pairs: &[(u32, u32)]) -> Self {
        Self {
            bytes: Some(encode_operation(records, pairs)),
            ..Self::write(name, &[])
        }
    }

    /// A decision record, through the same guarded vehicle path.
    pub fn write_decision(name: String, decision: &Decision) -> Self {
        Self {
            bytes: Some(encode_decision(decision)),
            ..Self::write(name, &[])
        }
    }

    pub fn remove(name: String) -> Self {
        Self {
            bytes: None,
            stage: Stage::Unlink,
            ..Self::write(name, &[])
        }
    }

    /// A failing unlink that completed is followed only by its missing sync.
    /// Unsupported is never a durability witness for app-data metadata.
    pub fn advance(&mut self, fs: &mut impl HostIo) {
        let result = match self.stage {
            Stage::Temp => fs.draft_temp(&self.name, self.bytes.as_ref().unwrap()),
            Stage::Rename => fs.draft_rename(&self.name),
            Stage::CleanupUnlink | Stage::Unlink => fs.draft_unlink(&self.name),
            Stage::Sync | Stage::CleanupSync | Stage::UnlinkSync => match fs.draft_sync() {
                Ok(Witness::Durable) => Ok(()),
                Ok(Witness::Unsupported) => Err(super::io::IoFailure {
                    kind: super::io::ErrorKind::Io,
                    completed: false,
                    operation: None,
                    os_error: None,
                }),
                Err(error) => Err(error),
            },
            Stage::Present | Stage::Absent => return,
        };
        let completed = result.is_ok() || result.as_ref().is_err_and(|e| e.completed);
        if result.is_err() {
            self.failures += 1;
        }
        self.stage = match self.stage {
            Stage::Temp if result.is_ok() => Stage::Rename,
            Stage::Temp => Stage::Absent,
            Stage::Rename if result.is_ok() => Stage::Sync,
            Stage::Rename if completed => Stage::CleanupUnlink,
            Stage::Rename => Stage::Absent,
            Stage::Sync if result.is_ok() => Stage::Present,
            Stage::Sync => {
                self.sync_failures += 1;
                if self.sync_failures >= 3 {
                    Stage::CleanupUnlink
                } else {
                    Stage::Sync
                }
            }
            Stage::CleanupUnlink if completed => Stage::CleanupSync,
            Stage::CleanupSync if result.is_ok() => Stage::Absent,
            Stage::Unlink if completed => Stage::UnlinkSync,
            Stage::UnlinkSync if result.is_ok() => Stage::Absent,
            other => other,
        };
    }
}

/// Only single-page vehicles can retire independently; an operation vehicle
/// retires whole or its records move to sibling vehicles (RENAME-COST L1/L2).
pub(super) fn older_vehicles(
    files: &BTreeMap<String, Vec<Record>>,
    page: &str,
    keep: Option<u64>,
) -> Vec<String> {
    let mut files: Vec<_> = files
        .iter()
        .filter(|(name, records)| {
            super::work(super::Work::DraftRecords, 1);
            name.starts_with("p-")
                && records[0].page == page
                && keep.is_none_or(|wseq| records[0].wseq < wseq)
        })
        .map(|(name, records)| (records[0].wseq, name.clone()))
        .collect();
    files.sort();
    files.into_iter().map(|(_, name)| name).collect()
}
