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

pub(super) fn decode(bytes: &[u8]) -> Result<Vec<Record>, ()> {
    let records: Vec<Record> = open(DRAFT, bytes)?;
    let mut pages = BTreeSet::new();
    if records.is_empty()
        || records
            .iter()
            .any(|r| r.wseq == 0 || !pages.insert(&r.page))
    {
        return Err(());
    }
    Ok(records)
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
}

/// Files are the authority. Equal sequences may only be identical copies.
pub(super) fn scan(files: Vec<(String, Vec<u8>)>) -> Scan {
    let mut result = Scan::default();
    let mut sequences: BTreeMap<u64, (Record, BTreeSet<String>)> = BTreeMap::new();
    for (name, bytes) in files {
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
    result.logical = logical(result.files.values());
    result
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
    let digest = Sha256::digest(page.as_bytes());
    let key: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("p-{key}-{}.draft", random.get(..16).unwrap())
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

/// Only single-page vehicles can retire independently; op files must explode.
pub(super) fn older_vehicles(
    files: &BTreeMap<String, Vec<Record>>,
    page: &str,
    keep: Option<u64>,
) -> Vec<String> {
    let mut files: Vec<_> = files
        .iter()
        .filter(|(name, records)| {
            name.starts_with("p-")
                && records[0].page == page
                && keep.is_none_or(|wseq| records[0].wseq < wseq)
        })
        .map(|(name, records)| (records[0].wseq, name.clone()))
        .collect();
    files.sort();
    files.into_iter().map(|(_, name)| name).collect()
}
