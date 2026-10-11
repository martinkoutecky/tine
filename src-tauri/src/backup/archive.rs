//! Tine's zip backup format (schema 5; docs/storage-contract.md, Graph
//! backups): an in-memory writer and a narrow validator for exactly what the
//! writer produces (REVIEW-backup-zip R6, R7). No disk I/O here and no
//! generic extraction: callers hand in bytes or a bounded reader, and get
//! back verified bytes.
//!
//! The format: payload entries sorted by path, then `tine-backup.json` last;
//! each local header immediately followed by its data; the central directory
//! immediately after the last entry; an end record with no comment. Stored or
//! raw Deflate only; general-purpose flags only bit 11 (UTF-8 name, set
//! exactly when the name is not ASCII); no extra fields, data descriptors,
//! encryption, comments or symlinks; Unix regular-file attributes. ZIP64
//! appears only as the end-record pair when the entry count needs it; the
//! limits keep every size and offset below 4 GiB.

use sha2::{Digest, Sha256};
use std::io::{self, Read, Seek, SeekFrom};

pub(super) const MANIFEST_NAME: &str = "tine-backup.json";
pub(super) const ZIP_SCHEMA: u32 = 5;

// Limits (R7). The writer refuses anything the reader would refuse, so a
// backup this build wrote is one this build can restore.
pub(super) const MAX_ENTRY_BYTES: u64 = 128 << 20;
pub(super) const MAX_TOTAL_BYTES: u64 = 512 << 20;
pub(super) const MAX_MANIFEST_BYTES: u64 = 64 << 20;
pub(super) const MAX_ENTRIES: u64 = 200_000;
pub(super) const MAX_NAME_BYTES: usize = 4096;
pub(super) const MAX_ARCHIVE_BYTES: u64 = 600 << 20;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const LOCAL_LEN: usize = 30;
const CENTRAL_LEN: usize = 46;
const EOCD_LEN: usize = 22;
const ZIP64_EOCD_LEN: usize = 56;
const ZIP64_LOCATOR_LEN: usize = 20;
const UTF8_FLAG: u16 = 1 << 11;
const STORED: u16 = 0;
const DEFLATE: u16 = 8;
/// Version made by: Unix (3), spec 2.0.
const MADE_BY: u16 = (3 << 8) | 20;
const REGULAR_FILE: u32 = 0o100_644 << 16;

/// Why an archive cannot be used. `Damaged` and `TooLarge` are refusals of
/// this build's own format; `Unsupported` is a well-formed file this build
/// does not understand (a checksum-valid manifest of another schema, or a
/// zip feature Tine never writes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    Damaged(&'static str),
    Unsupported(&'static str),
    TooLarge(&'static str),
    Io(io::ErrorKind),
}

// ---------------------------------------------------------------- manifest

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ZipFile {
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) len: u64,
}

/// The schema-5 manifest. Equality (checksum aside) is "the same captured
/// state" for the unchanged-launch skip.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ZipManifest {
    pub(super) schema: u32,
    pub(super) writer: String,
    pub(super) root: String,
    pub(super) journals_dir: String,
    pub(super) pages_dir: String,
    pub(super) assets_dir: String,
    pub(super) graph_text_policy: super::SnapshotGraphTextPolicy,
    pub(super) files: Vec<ZipFile>,
    pub(super) complete: bool,
}

const CHECKSUM: &str = "checksum";

/// The SHA-256 of a manifest's JSON without its checksum field, over its
/// canonical form: compact, object keys sorted at every level, strings
/// unescaped beyond JSON's minimum (Python's `json.dumps(v, sort_keys=True,
/// separators=(",", ":"), ensure_ascii=False)`). Ported from og-backup-cas.
pub(super) fn manifest_checksum(manifest: &serde_json::Value) -> String {
    let mut bytes = Vec::new();
    canonical_json(manifest, &mut bytes);
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_json(value: &serde_json::Value, out: &mut Vec<u8>) {
    fn leaf(value: &impl serde::Serialize, out: &mut Vec<u8>) {
        serde_json::to_writer(out, value).expect("a JSON value serializes");
    }
    match value {
        serde_json::Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                leaf(key, out);
                out.push(b':');
                canonical_json(&fields[key], out);
            }
            out.push(b'}');
        }
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                canonical_json(item, out);
            }
            out.push(b']');
        }
        other => leaf(other, out),
    }
}

pub(super) fn encode_manifest(manifest: &ZipManifest) -> Vec<u8> {
    let mut value = serde_json::to_value(manifest).expect("a manifest serializes");
    let checksum = manifest_checksum(&value);
    value
        .as_object_mut()
        .expect("a manifest is an object")
        .insert(CHECKSUM.into(), checksum.into());
    serde_json::to_vec_pretty(&value).expect("a manifest serializes")
}

/// Parse and check a manifest: a checksum that fails is damage; a valid
/// checksum over a schema, field set or scope version this build does not
/// know is unsupported.
pub(super) fn parse_manifest(bytes: &[u8]) -> Result<ZipManifest, Problem> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Problem::Damaged("manifest is not JSON"))?;
    let checksum = value
        .as_object_mut()
        // `remove_entry`, not `remove`: host_slot_guard_tests resolves callees by name.
        .and_then(|fields| fields.remove_entry(CHECKSUM))
        .map(|(_, checksum)| checksum)
        .ok_or(Problem::Damaged("manifest has no checksum"))?;
    if checksum.as_str() != Some(manifest_checksum(&value).as_str()) {
        return Err(Problem::Damaged("manifest checksum"));
    }
    if value.get("schema").and_then(serde_json::Value::as_u64) != Some(u64::from(ZIP_SCHEMA)) {
        return Err(Problem::Unsupported("manifest schema"));
    }
    let manifest: ZipManifest =
        serde_json::from_value(value).map_err(|_| Problem::Unsupported("manifest fields"))?;
    if !manifest.complete || manifest.graph_text_policy.version != super::GRAPH_TEXT_SCOPE_VERSION {
        return Err(Problem::Unsupported("manifest scope"));
    }
    Ok(manifest)
}

// ---------------------------------------------------------------- paths

/// The entry-path policy, applied by the writer and the reader alike:
/// relative `/`-separated components, none empty, `.` or `..`; no NUL or
/// backslash; at most [`MAX_NAME_BYTES`]. With `windows`, also no drive,
/// stream or reserved-device syntax and no trailing dot or space.
pub(super) fn path_is_safe(path: &str, windows: bool) -> bool {
    if path.is_empty()
        || path.len() > MAX_NAME_BYTES
        || path.contains('\0')
        || path.contains('\\')
        || path.starts_with('/')
    {
        return false;
    }
    path.split('/').all(|part| {
        !part.is_empty() && part != "." && part != ".." && (!windows || windows_part_is_safe(part))
    })
}

fn windows_part_is_safe(part: &str) -> bool {
    if part.ends_with('.') || part.ends_with(' ') {
        return false;
    }
    if part
        .chars()
        .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return false;
    }
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    !reserved
}

/// Whether the platform's graph volumes usually fold case: two paths equal
/// after folding would be one file there. Only restore applies it (a
/// case-sensitive volume on these platforms can hold both names, and its
/// backup must not fail); Store's no-replace publication still refuses a
/// residual collision the folding does not model.
pub(super) const FOLDS_CASE: bool = cfg!(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "ios",
    target_os = "android"
));

/// Every path safe and unique (after case folding where `fold`).
pub(super) fn paths_are_safe<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    windows: bool,
    fold: bool,
) -> bool {
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if !path_is_safe(path, windows) {
            return false;
        }
        let key = if fold {
            path.to_lowercase()
        } else {
            path.to_owned()
        };
        if !seen.insert(key) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------- writer

/// MS-DOS time and date for a `YYYY-MM-DD_HH-MM-SS` stamp (UTC); 1980-01-01
/// for anything earlier or unparsable.
pub(super) fn dos_time(stamp: &str) -> (u16, u16) {
    let field = |range: std::ops::Range<usize>| -> u16 {
        stamp
            .get(range)
            .and_then(|part| part.parse().ok())
            .unwrap_or(0)
    };
    let (year, month, day) = (field(0..4), field(5..7), field(8..10));
    if year < 1980 || year > 2107 || month == 0 || day == 0 {
        return (0, (1 << 5) | 1);
    }
    let time = (field(11..13) << 11) | (field(14..16) << 5) | (field(17..19) / 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    (time, date)
}

/// One entry ready to place: its name, method, CRC and sizes, and the bytes
/// as stored.
pub(super) struct Encoded<'a> {
    pub(super) name: &'a str,
    pub(super) method: u16,
    pub(super) crc: u32,
    pub(super) size: u64,
    pub(super) stored: std::borrow::Cow<'a, [u8]>,
}

impl<'a> Encoded<'a> {
    /// Deflate when it shrinks the entry, else store it.
    fn new(name: &'a str, data: &'a [u8], deflater: &mut flate2::Compress) -> Self {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        deflater.reset();
        let mut deflated = Vec::with_capacity(data.len() / 2 + 64);
        loop {
            if deflated.len() == deflated.capacity() {
                deflated.reserve(deflated.capacity().max(64));
            }
            let consumed = deflater.total_in() as usize;
            let status = deflater
                .compress_vec(
                    &data[consumed..],
                    &mut deflated,
                    flate2::FlushCompress::Finish,
                )
                .expect("deflate into memory");
            if status == flate2::Status::StreamEnd {
                break;
            }
        }
        let (method, stored) = if deflated.len() < data.len() {
            (DEFLATE, deflated.into())
        } else {
            (STORED, data.into())
        };
        Self {
            name,
            method,
            crc: crc.sum(),
            size: data.len() as u64,
            stored,
        }
    }
}

/// Build an archive of `files` (sorted, unique, safe paths) plus the
/// manifest as the last entry. Deflate per entry, Stored when deflate does
/// not shrink it. `Err` names the limit or path rule the content breaks.
pub(super) fn write_archive(
    files: &[(String, Vec<u8>)],
    manifest: &[u8],
    stamp: &str,
) -> Result<Vec<u8>, Problem> {
    if files.len() as u64 + 1 > MAX_ENTRIES {
        return Err(Problem::TooLarge("entry count"));
    }
    if manifest.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(Problem::TooLarge("manifest"));
    }
    let mut total = 0u64;
    for (_, bytes) in files {
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            return Err(Problem::TooLarge("entry"));
        }
        total += bytes.len() as u64;
    }
    if total > MAX_TOTAL_BYTES {
        return Err(Problem::TooLarge("total"));
    }
    if !paths_are_safe(
        files.iter().map(|(path, _)| path.as_str()),
        cfg!(windows),
        false,
    ) {
        return Err(Problem::Damaged("unsafe or duplicate path"));
    }
    let mut deflater = flate2::Compress::new(flate2::Compression::default(), false);
    let entries: Vec<Encoded> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
        .chain(std::iter::once((MANIFEST_NAME, manifest)))
        .map(|(name, data)| Encoded::new(name, data, &mut deflater))
        .collect();
    let out = assemble(&entries, dos_time(stamp));
    if out.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(Problem::TooLarge("archive"));
    }
    Ok(out)
}

/// Lay out encoded entries in the format above. Sizes and offsets must fit
/// in 32 bits, which [`write_archive`]'s limits guarantee.
pub(super) fn assemble(entries: &[Encoded], (time, date): (u16, u16)) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let flags = if entry.name.is_ascii() { 0 } else { UTF8_FLAG };
        let needed: u16 = if entry.method == DEFLATE { 20 } else { 10 };
        let offset = out.len() as u32;
        let fields = |buf: &mut Vec<u8>| {
            put16(buf, needed);
            put16(buf, flags);
            put16(buf, entry.method);
            put16(buf, time);
            put16(buf, date);
            put32(buf, entry.crc);
            put32(buf, entry.stored.len() as u32);
            put32(buf, entry.size as u32);
            put16(buf, entry.name.len() as u16);
            put16(buf, 0);
        };
        put32(&mut out, LOCAL_SIG);
        fields(&mut out);
        out.extend_from_slice(entry.name.as_bytes());
        out.extend_from_slice(&entry.stored);
        put32(&mut central, CENTRAL_SIG);
        put16(&mut central, MADE_BY);
        fields(&mut central);
        put16(&mut central, 0); // comment
        put16(&mut central, 0); // disk
        put16(&mut central, 0); // internal attributes
        put32(&mut central, REGULAR_FILE);
        put32(&mut central, offset);
        central.extend_from_slice(entry.name.as_bytes());
    }
    let count = entries.len() as u64;
    let cd_offset = out.len() as u64;
    let cd_size = central.len() as u64;
    out.extend_from_slice(&central);
    let zip64 = count >= 0xFFFF;
    if zip64 {
        let record = out.len() as u64;
        put32(&mut out, ZIP64_EOCD_SIG);
        put64(&mut out, (ZIP64_EOCD_LEN - 12) as u64);
        put16(&mut out, (MADE_BY & 0xFF00) | 45);
        put16(&mut out, 45);
        put32(&mut out, 0);
        put32(&mut out, 0);
        put64(&mut out, count);
        put64(&mut out, count);
        put64(&mut out, cd_size);
        put64(&mut out, cd_offset);
        put32(&mut out, ZIP64_LOCATOR_SIG);
        put32(&mut out, 0);
        put64(&mut out, record);
        put32(&mut out, 1);
    }
    let short_count = if zip64 { 0xFFFF } else { count as u16 };
    put32(&mut out, EOCD_SIG);
    put16(&mut out, 0);
    put16(&mut out, 0);
    put16(&mut out, short_count);
    put16(&mut out, short_count);
    put32(&mut out, cd_size as u32);
    put32(&mut out, cd_offset as u32);
    put16(&mut out, 0);
    out
}

fn put16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

// ---------------------------------------------------------------- reader

/// Bounded positional reads over an archive of known length: a whole
/// archive in memory, or an open file (listing reads only its tail,
/// directory and manifest).
pub(super) trait Source {
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Problem>;
}

impl Source for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Problem> {
        let end = offset
            .checked_add(len)
            .filter(|end| *end <= Source::len(self))
            .ok_or(Problem::Damaged("read past the end"))?;
        Ok(self[offset as usize..end as usize].to_vec())
    }
}

/// An open archive file and the length it had when opened.
pub(super) struct FileSource {
    pub(super) file: std::fs::File,
    pub(super) len: u64,
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, Problem> {
        if offset.checked_add(len).is_none_or(|end| end > self.len) {
            return Err(Problem::Damaged("read past the end"));
        }
        let io = |error: io::Error| Problem::Io(error.kind());
        self.file.seek(SeekFrom::Start(offset)).map_err(io)?;
        let mut out = Vec::with_capacity(len as usize);
        (&mut self.file)
            .take(len)
            .read_to_end(&mut out)
            .map_err(io)?;
        if out.len() as u64 != len {
            return Err(Problem::Damaged("archive shorter than recorded"));
        }
        Ok(out)
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

/// One central-directory record.
#[derive(Clone, Debug)]
pub(super) struct Entry {
    pub(super) name: String,
    version_needed: u16,
    flags: u16,
    method: u16,
    time: u16,
    date: u16,
    crc: u32,
    compressed: u64,
    pub(super) size: u64,
    offset: u64,
}

/// The parsed central directory.
pub(super) struct Directory {
    pub(super) entries: Vec<Entry>,
    cd_offset: u64,
}

/// Parse the end records and the whole central directory, with bounds and
/// consistency checks, reading only the archive's tail and directory.
pub(super) fn read_directory(src: &mut impl Source) -> Result<Directory, Problem> {
    let size = src.len();
    if size > MAX_ARCHIVE_BYTES {
        return Err(Problem::TooLarge("archive"));
    }
    let eocd_at = size
        .checked_sub(EOCD_LEN as u64)
        .ok_or(Problem::Damaged("no end record"))?;
    let eocd = src.read_at(eocd_at, EOCD_LEN as u64)?;
    if u32_at(&eocd, 0) != EOCD_SIG {
        return Err(Problem::Damaged("no end record"));
    }
    if u16_at(&eocd, 4) != 0 || u16_at(&eocd, 6) != 0 || u16_at(&eocd, 20) != 0 {
        return Err(Problem::Unsupported("multi-disk or commented archive"));
    }
    let short_count = u16_at(&eocd, 8);
    if u16_at(&eocd, 10) != short_count {
        return Err(Problem::Damaged("end record counts"));
    }
    let cd_size = u64::from(u32_at(&eocd, 12));
    let cd_offset = u64::from(u32_at(&eocd, 16));
    let (count, cd_end) = if short_count == 0xFFFF {
        // ZIP64 end record + locator, only for the entry count (R6).
        let locator_at = eocd_at
            .checked_sub(ZIP64_LOCATOR_LEN as u64)
            .ok_or(Problem::Damaged("no ZIP64 locator"))?;
        let record_at = locator_at
            .checked_sub(ZIP64_EOCD_LEN as u64)
            .ok_or(Problem::Damaged("no ZIP64 end record"))?;
        let locator = src.read_at(locator_at, ZIP64_LOCATOR_LEN as u64)?;
        if u32_at(&locator, 0) != ZIP64_LOCATOR_SIG
            || u32_at(&locator, 4) != 0
            || u64_at(&locator, 8) != record_at
            || u32_at(&locator, 16) != 1
        {
            return Err(Problem::Damaged("ZIP64 locator"));
        }
        let record = src.read_at(record_at, ZIP64_EOCD_LEN as u64)?;
        let count = u64_at(&record, 24);
        if u32_at(&record, 0) != ZIP64_EOCD_SIG
            || u64_at(&record, 4) != (ZIP64_EOCD_LEN - 12) as u64
            || u32_at(&record, 16) != 0
            || u32_at(&record, 20) != 0
            || u64_at(&record, 32) != count
            || count < 0xFFFF
            || u64_at(&record, 40) != cd_size
            || u64_at(&record, 48) != cd_offset
        {
            return Err(Problem::Damaged("ZIP64 end record"));
        }
        (count, record_at)
    } else {
        (u64::from(short_count), eocd_at)
    };
    if count == 0 {
        return Err(Problem::Damaged("empty archive"));
    }
    if count > MAX_ENTRIES {
        return Err(Problem::TooLarge("entry count"));
    }
    if cd_offset.checked_add(cd_size) != Some(cd_end) {
        return Err(Problem::Damaged("central directory bounds"));
    }
    let cd = src.read_at(cd_offset, cd_size)?;
    let mut entries = Vec::with_capacity(count as usize);
    let mut names = std::collections::HashSet::new();
    let mut at = 0usize;
    for _ in 0..count {
        let fixed = cd
            .get(at..at + CENTRAL_LEN)
            .ok_or(Problem::Damaged("central record bounds"))?;
        if u32_at(fixed, 0) != CENTRAL_SIG {
            return Err(Problem::Damaged("central record signature"));
        }
        let made_by = u16_at(fixed, 4);
        let flags = u16_at(fixed, 8);
        let method = u16_at(fixed, 10);
        let name_len = usize::from(u16_at(fixed, 28));
        let extra_len = u16_at(fixed, 30);
        let comment_len = u16_at(fixed, 32);
        let compressed = u32_at(fixed, 20);
        let size = u32_at(fixed, 24);
        let offset = u32_at(fixed, 42);
        if flags & !UTF8_FLAG != 0 {
            return Err(Problem::Unsupported("zip flags"));
        }
        if method != STORED && method != DEFLATE {
            return Err(Problem::Unsupported("compression method"));
        }
        if extra_len != 0 || comment_len != 0 || u16_at(fixed, 34) != 0 {
            return Err(Problem::Unsupported("zip extra fields"));
        }
        if compressed == u32::MAX || size == u32::MAX || offset == u32::MAX {
            return Err(Problem::Unsupported("per-entry ZIP64"));
        }
        if made_by >> 8 != 3 || u32_at(fixed, 38) >> 16 & 0o170_000 != 0o100_000 {
            return Err(Problem::Unsupported("not a regular file"));
        }
        if name_len == 0 || name_len > MAX_NAME_BYTES {
            return Err(Problem::Damaged("entry name length"));
        }
        let raw = cd
            .get(at + CENTRAL_LEN..at + CENTRAL_LEN + name_len)
            .ok_or(Problem::Damaged("central record bounds"))?;
        let name = std::str::from_utf8(raw)
            .ok()
            .filter(|name| (flags & UTF8_FLAG != 0) == !name.is_ascii())
            .ok_or(Problem::Damaged("entry name encoding"))?;
        if !names.insert(name.to_owned()) {
            return Err(Problem::Damaged("duplicate entry"));
        }
        entries.push(Entry {
            name: name.to_owned(),
            version_needed: u16_at(fixed, 6),
            flags,
            method,
            time: u16_at(fixed, 12),
            date: u16_at(fixed, 14),
            crc: u32_at(fixed, 16),
            compressed: u64::from(compressed),
            size: u64::from(size),
            offset: u64::from(offset),
        });
        at += CENTRAL_LEN + name_len;
    }
    if at as u64 != cd_size {
        return Err(Problem::Damaged("central directory size"));
    }
    if entries.last().map(|entry| entry.name.as_str()) != Some(MANIFEST_NAME)
        || entries
            .iter()
            .filter(|entry| entry.name == MANIFEST_NAME)
            .count()
            != 1
    {
        return Err(Problem::Damaged("manifest is not the one last entry"));
    }
    Ok(Directory { entries, cd_offset })
}

/// One entry's bytes: local header equal to the central record, data inside
/// the entry region, bounded inflate that must end exactly at the recorded
/// sizes, CRC.
fn read_entry(
    src: &mut impl Source,
    entry: &Entry,
    cd_offset: u64,
    limit: u64,
) -> Result<Vec<u8>, Problem> {
    if entry.size > limit {
        return Err(Problem::TooLarge("entry"));
    }
    let header = src.read_at(entry.offset, LOCAL_LEN as u64)?;
    let name_len = entry.name.len() as u64;
    let data_at = entry.offset + LOCAL_LEN as u64 + name_len;
    let data_end = data_at
        .checked_add(entry.compressed)
        .filter(|end| *end <= cd_offset)
        .ok_or(Problem::Damaged("entry bounds"))?;
    if u32_at(&header, 0) != LOCAL_SIG
        || u16_at(&header, 4) != entry.version_needed
        || u16_at(&header, 6) != entry.flags
        || u16_at(&header, 8) != entry.method
        || u16_at(&header, 10) != entry.time
        || u16_at(&header, 12) != entry.date
        || u32_at(&header, 14) != entry.crc
        || u64::from(u32_at(&header, 18)) != entry.compressed
        || u64::from(u32_at(&header, 22)) != entry.size
        || u64::from(u16_at(&header, 26)) != name_len
        || u16_at(&header, 28) != 0
    {
        return Err(Problem::Damaged("local header"));
    }
    if src.read_at(entry.offset + LOCAL_LEN as u64, name_len)? != entry.name.as_bytes() {
        return Err(Problem::Damaged("local name"));
    }
    let data = src.read_at(data_at, data_end - data_at)?;
    let bytes = if entry.method == STORED {
        if entry.compressed != entry.size {
            return Err(Problem::Damaged("stored sizes"));
        }
        data
    } else {
        inflate(&data, entry.size)?
    };
    let mut crc = flate2::Crc::new();
    crc.update(&bytes);
    if crc.sum() != entry.crc {
        return Err(Problem::Damaged("entry CRC"));
    }
    Ok(bytes)
}

/// Raw inflate of exactly `input` into exactly `size` bytes; anything else
/// (more output, a stream that ends early or late, trailing input) is
/// damage. The output buffer is bounded by `size`, which the caller bounded.
fn inflate(input: &[u8], size: u64) -> Result<Vec<u8>, Problem> {
    let mut inflater = flate2::Decompress::new(false);
    let mut out = Vec::with_capacity(size as usize + 1);
    loop {
        let (consumed, produced) = (inflater.total_in(), inflater.total_out());
        let status = inflater
            .decompress_vec(
                &input[consumed as usize..],
                &mut out,
                flate2::FlushDecompress::Finish,
            )
            .map_err(|_| Problem::Damaged("deflate stream"))?;
        if out.len() as u64 > size {
            return Err(Problem::Damaged("entry larger than recorded"));
        }
        if status == flate2::Status::StreamEnd {
            break;
        }
        if inflater.total_in() == consumed && inflater.total_out() == produced {
            return Err(Problem::Damaged("deflate stream"));
        }
    }
    if inflater.total_in() != input.len() as u64 || out.len() as u64 != size {
        return Err(Problem::Damaged("deflate sizes"));
    }
    Ok(out)
}

/// The listing check (R7, item 7): directory, the manifest entry and its
/// checksum, and the manifest's file list against the directory's names and
/// sizes. Never reads a payload entry.
pub(super) fn read_listing(src: &mut impl Source) -> Result<ZipManifest, Problem> {
    let directory = read_directory(src)?;
    let manifest_entry = directory.entries.last().expect("checked non-empty");
    let manifest = parse_manifest(&read_entry(
        src,
        manifest_entry,
        directory.cd_offset,
        MAX_MANIFEST_BYTES,
    )?)?;
    check_listing(&directory, &manifest)?;
    Ok(manifest)
}

fn check_listing(directory: &Directory, manifest: &ZipManifest) -> Result<(), Problem> {
    let payload = &directory.entries[..directory.entries.len() - 1];
    if payload.len() != manifest.files.len()
        || !payload
            .iter()
            .zip(&manifest.files)
            .all(|(entry, file)| entry.name == file.path && entry.size == file.len)
    {
        return Err(Problem::Damaged("manifest does not match the entries"));
    }
    if !paths_are_safe(
        manifest.files.iter().map(|file| file.path.as_str()),
        cfg!(windows),
        false,
    ) {
        return Err(Problem::Damaged("unsafe or duplicate path"));
    }
    Ok(())
}

/// A fully verified archive: its manifest and every payload file's bytes,
/// in manifest order.
pub(super) struct Verified {
    pub(super) manifest: ZipManifest,
    pub(super) files: Vec<Vec<u8>>,
}

/// Full verification (restore, the unchanged-launch skip, the prune keeper):
/// the listing check, entries contiguous from offset 0 up to the central
/// directory, every entry decompressed within the limits, its CRC, length
/// and SHA-256 against the manifest.
pub(super) fn verify(src: &mut impl Source) -> Result<Verified, Problem> {
    let directory = read_directory(src)?;
    let mut expected = 0u64;
    let mut total = 0u64;
    for entry in &directory.entries {
        if entry.offset != expected {
            return Err(Problem::Damaged("entries are not contiguous"));
        }
        expected = entry
            .offset
            .checked_add(LOCAL_LEN as u64 + entry.name.len() as u64)
            .and_then(|end| end.checked_add(entry.compressed))
            .ok_or(Problem::Damaged("entry bounds"))?;
        if entry.name != MANIFEST_NAME {
            total = total
                .checked_add(entry.size)
                .ok_or(Problem::TooLarge("total"))?;
        }
    }
    if expected != directory.cd_offset {
        return Err(Problem::Damaged("entries are not contiguous"));
    }
    if total > MAX_TOTAL_BYTES {
        return Err(Problem::TooLarge("total"));
    }
    let (payload, manifest_entry) = directory.entries.split_at(directory.entries.len() - 1);
    let manifest = parse_manifest(&read_entry(
        src,
        &manifest_entry[0],
        directory.cd_offset,
        MAX_MANIFEST_BYTES,
    )?)?;
    check_listing(&directory, &manifest)?;
    let mut files = Vec::with_capacity(payload.len());
    for (entry, file) in payload.iter().zip(&manifest.files) {
        let bytes = read_entry(src, entry, directory.cd_offset, MAX_ENTRY_BYTES)?;
        if bytes.len() as u64 != file.len || format!("{:x}", Sha256::digest(&bytes)) != file.sha256
        {
            return Err(Problem::Damaged("payload does not match the manifest"));
        }
        files.push(bytes);
    }
    Ok(Verified { manifest, files })
}

#[cfg(test)]
pub(super) mod tests;
