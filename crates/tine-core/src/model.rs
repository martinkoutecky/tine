//! Pure graph data types and file-name classification helpers. File I/O lives
//! in `tine-store`; these values can be serialized for clients or exports.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Encode a page title as a reversible Windows-safe file stem using the
/// graph's configured Logseq format. Existing file paths are never recoded.
pub fn encode_page_name(name: &str, fmt: crate::config::FileNameFormat) -> String {
    use crate::config::FileNameFormat;
    let trailing_safe = name.trim_end_matches([' ', '.']).len();
    let mut escaped = String::with_capacity(name.len());
    for (offset, character) in name.char_indices() {
        let encode = character == '%'
            || character <= '\u{1f}'
            || character == '\u{7f}'
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' | '#'
            )
            || (character == '.'
                && (fmt == FileNameFormat::Legacy || offset == 0 || offset >= trailing_safe))
            || (character == ' ' && offset >= trailing_safe);
        if encode {
            let mut bytes = [0_u8; 4];
            for byte in character.encode_utf8(&mut bytes).as_bytes() {
                push_percent_byte(&mut escaped, *byte);
            }
        } else {
            escaped.push(character);
        }
    }
    let mut encoded = match fmt {
        FileNameFormat::Legacy => escaped.replace('/', "%2F"),
        FileNameFormat::TripleLowbar => escaped
            .replace("___", "%5F%5F%5F")
            .replace("_/", "%5F/")
            .replace("/_", "/%5F")
            .replace('/', "___"),
    };
    let device = encoded
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if reserved {
        let first_len = encoded.chars().next().map(char::len_utf8).unwrap_or(0);
        let mut safe = String::with_capacity(encoded.len() + 2);
        for byte in &encoded.as_bytes()[..first_len] {
            push_percent_byte(&mut safe, *byte);
        }
        safe.push_str(&encoded[first_len..]);
        encoded = safe;
    }
    encoded
}

fn push_percent_byte(output: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    output.push('%');
    output.push(char::from(HEX[usize::from(byte >> 4)]));
    output.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

/// Decode a page filename according to the graph's Logseq naming format.
/// Legacy dots become namespace separators before percent decoding.
/// Cost O(stem bytes); malformed percent escapes are preserved.
pub fn decode_page_name(stem: &str, fmt: crate::config::FileNameFormat) -> String {
    let encoded = match fmt {
        crate::config::FileNameFormat::Legacy => stem.replace('.', "/"),
        crate::config::FileNameFormat::TripleLowbar => stem.replace("___", "/"),
    };
    let bytes = encoded.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let nibble = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                b'A'..=b'F' => Some(byte - b'A' + 10),
                _ => None,
            };
            if let (Some(high), Some(low)) = (nibble(bytes[index + 1]), nibble(bytes[index + 2])) {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// The first nonempty page title in a document preamble. Content after the
/// first Markdown bullet or Org headline is block content, not page identity.
/// Cost O(preamble bytes); malformed or empty titles leave filename identity.
pub fn page_title_from_preamble(content: &str, format: Format) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ")
            || trimmed == "-"
            || (format == Format::Org && trimmed.starts_with("* "))
        {
            break;
        }
        if let Some((key, value)) = crate::doc::parse_property_line(line) {
            if key.eq_ignore_ascii_case("title") && !value.trim().is_empty() {
                return Some(value.trim().to_owned());
            }
        }
        if format == Format::Org {
            let directive = trimmed
                .split_once(':')
                .and_then(|(key, value)| key.eq_ignore_ascii_case("#+title").then_some(value));
            let drawer = trimmed
                .strip_prefix(':')
                .and_then(|rest| rest.split_once(':'))
                .and_then(|(key, value)| key.eq_ignore_ascii_case("title").then_some(value));
            if let Some(title) = directive
                .or(drawer)
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                return Some(title.to_owned());
            }
        }
    }
    None
}

/// Whether a page file is a journal or an ordinary page.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    /// Date-based journal page.
    Journal,
    /// Ordinary named page.
    Page,
}

/// On-disk file format of a page. Markdown (`.md` or `.markdown`) is the default; Logseq org
/// graphs use `.org`. Existing extensions are matched without case sensitivity.
/// A graph may mix the two — format is decided per file by
/// extension, never graph-wide (matching OG, which stores `:block/format` per
/// page). The graph's `:preferred-format` only chooses the extension for NEW
/// files through `Config::preferred_format`.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Markdown page file.
    #[default]
    Md,
    /// Org page file.
    Org,
}

impl Format {
    /// Format of a page file by its extension (`.org` → Org, else Md).
    pub fn from_path(p: &Path) -> Format {
        match p.extension().and_then(|e| e.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("org") => Format::Org,
            _ => Format::Md,
        }
    }
    /// File extension (no dot) for this format.
    pub fn ext(self) -> &'static str {
        match self {
            Format::Md => "md",
            Format::Org => "org",
        }
    }
}

/// If `stem` is a sync tool's conflict copy of another file, return the base file
/// stem it shadows. Recognises Syncthing
/// (`name.sync-conflict-YYYYMMDD-HHMMSS-XXXXXXX`) and Dropbox
/// (`name (conflicted copy …)` / `name (<user>'s conflicted copy …)`).
///
/// A conflict copy is NOT a real page — it must be kept out of the page list and
/// the `(kind,name)` cache (otherwise it shows as a garbage page and its shared
/// `id::` values churn the id space), yet remain loadable by path for the
/// conflict-merge UI. So this is threaded through the *listing* sites, never
/// through `is_page_file`/`entry_for_path`/`resolve_rel` (which the merge UI's
/// path-addressed load relies on).
pub fn sync_conflict_base(stem: &str) -> Option<&str> {
    if let Some(i) = stem.find(".sync-conflict-") {
        return Some(&stem[..i]);
    }
    // Dropbox: "<base> (conflicted copy …)" or "<base> (<user>'s conflicted copy …)".
    if let Some(i) = stem.find(" (") {
        if stem[i..].contains("conflicted copy") {
            return Some(&stem[..i]);
        }
    }
    None
}

/// Whether `stem` names a sync-tool conflict copy (see [`sync_conflict_base`]).
pub fn is_sync_conflict(stem: &str) -> bool {
    sync_conflict_base(stem).is_some()
}

/// Whether `path`'s file stem names a sync-tool conflict copy — the `Path`-level
/// convenience used by the watcher (which works in paths, not stems).
pub fn path_is_sync_conflict(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(is_sync_conflict)
}

/// Opaque graph-root-relative file identity, such as `pages/Example.md` or
/// `logseq/config.edn`, including assets whose approved target may be outside
/// the graph root. `Store::file_id` takes an area-relative name and adds its
/// configured area prefix. Constructing one from a string does not
/// validate it; the store revalidates identities when used.
#[deny(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileId(String);
impl From<String> for FileId {
    fn from(wire: String) -> Self {
        Self(wire)
    }
}
impl FileId {
    /// Borrow the unvalidated graph-root-relative identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Graph-root-relative, slash-separated page file identity, such as
/// `pages/Example.md`. String constructors do not validate it; store calls
/// revalidate before accessing disk. Equality compares the literal path
/// string; it does not canonicalize case or Unicode filesystem aliases.
/// Compare identities, not display names.
#[deny(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PageId(String);
impl From<String> for PageId {
    fn from(wire: String) -> Self {
        Self(wire)
    }
}
impl From<&str> for PageId {
    fn from(wire: &str) -> Self {
        Self::from(wire.to_owned())
    }
}
impl From<PageId> for String {
    fn from(id: PageId) -> Self {
        id.0
    }
}
impl PartialEq<str> for PageId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}
impl PartialEq<&str> for PageId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}
impl PartialEq<String> for PageId {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}
impl PartialEq<PageId> for String {
    fn eq(&self, other: &PageId) -> bool {
        *self == other.0
    }
}
impl std::fmt::Display for PageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::ops::Deref for PageId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}
impl AsRef<str> for PageId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
impl PageId {
    /// Treat this physical page identity as a file identity.
    pub fn file(&self) -> FileId {
        FileId::from(self.0.clone())
    }
    /// Borrow the unvalidated graph-relative identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Whether this identity string is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Byte length of the identity string.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Page-list entry. A referenced name with no file can be virtual; then
/// `rel_path` is `None` and `path` is empty. Physical twins have separate entries.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageEntry {
    /// Decoded page name or journal title.
    pub name: String,
    /// Journal or ordinary page.
    pub kind: PageKind,
    /// Sort key `yyyymmdd` for journals; `None` for ordinary pages.
    pub date_key: Option<i64>,
    /// Graph-root-relative slash-separated identity for opening a physical
    /// claimant; `None` for a virtual reference-only name.
    #[serde(rename = "path", default, with = "optional_page_path")]
    pub rel_path: Option<PageId>,
    #[serde(skip)]
    /// Local filesystem path from the listing, omitted from serialized values.
    /// Revalidate the `rel_path` through `Store::path_for_os_handoff` before
    /// opening it in another process; this field is not a handoff guarantee.
    pub path: PathBuf,
}

impl PageEntry {
    /// Slash-separated relative path, or an empty string for a virtual entry.
    pub fn rel_path_str(&self) -> &str {
        self.rel_path.as_ref().map_or("", PageId::as_str)
    }
}

mod optional_page_path {
    use super::PageId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<PageId>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .as_ref()
            .map_or("", PageId::as_str)
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<PageId>, D::Error> {
        let wire = String::deserialize(deserializer)?;
        Ok((!wire.is_empty()).then(|| PageId::from(wire)))
    }
}

/// Editable block tree node and derived display facets.
#[deny(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockDto {
    /// Runtime block identity; a persisted `id::` remains in `raw`. Structural
    /// edits can change this identity across graph publications, so reacquire
    /// it after a page changes. A new block may use an empty id; saves derive
    /// identity from the physical page and structural sibling-index path, not
    /// this field or the raw text.
    pub id: String,
    /// Raw block text, including properties. Page saves serialize this body;
    /// derived facets and `breadcrumb` do not add text. The `page_property`
    /// flag does not suppress serialization of this raw body.
    pub raw: String,
    /// Whether child blocks are collapsed in the outline. This display facet
    /// alone does not write `collapsed::`; edit `raw` to persist that property.
    #[serde(default)]
    pub collapsed: bool,
    /// Ordered child blocks.
    #[serde(default)]
    pub children: Vec<BlockDto>,
    /// Ancestor first-lines (page-relative path) for search/reference results;
    /// empty for normal page loads. Lets the UI show a "parent › child" trail.
    #[serde(default)]
    pub breadcrumb: Vec<String>,
    /// Synthetic, read-only result row representing references from the source
    /// page's property pre-block rather than an editable outline block. Do not
    /// put such a result row in an editable `PageDto`: save would serialize its
    /// `raw` as an outline block because this flag is only result metadata.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_property: bool,
    // --- M1: block-header facets, computed ONCE off the lsdoc projection (the one
    // grammar source) and shipped so the frontend never re-derives them with its
    // own scanner. Derived (not authoritative — `raw` round-trips); the frontend
    // recomputes locally only for the block it is actively editing. Omitted from the
    // wire when empty to keep the payload small (most blocks have no marker/dates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Derived task marker, if present.
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Derived priority, if present.
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Derived heading level, if present.
    pub heading_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Derived scheduled date, if present.
    pub scheduled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Derived deadline, if present.
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Derived tags.
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    /// Derived property names and values.
    pub properties: Vec<(String, String)>,
}

/// A group of blocks from one source page — used for both Linked References
/// (backlinks) and `{{query}}` results.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefGroup {
    /// Source page name.
    pub page: String,
    /// Source page kind.
    pub kind: PageKind,
    /// Matching blocks or projected subtrees.
    pub blocks: Vec<BlockDto>,
    /// Result-only source evidence keyed by block id. Empty for ordinary query
    /// groups and when deserializing older values without this field; never
    /// crosses the block write boundary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ReferenceBlockEvidence>,
}

/// One backlink root whose visible subtree and co-reference facets can be
/// fetched when the Linked References filter opens. Ordinary backlink results
/// remain shallow until this data is requested.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterTarget {
    /// Source page name.
    pub page: String,
    /// Source page kind.
    pub kind: PageKind,
    /// Runtime structural root block identity, not a persisted `id::` value.
    pub block_id: String,
}

/// One backlink root with projected text and co-reference facets.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterEntry {
    /// Source page name.
    pub page: String,
    /// Source page kind.
    pub kind: PageKind,
    /// Runtime structural root block identity, not a persisted `id::` value.
    pub block_id: String,
    /// Projected visible text.
    pub text: String,
    /// Co-reference facet names.
    pub facets: Vec<String>,
    /// Whether the projection omitted content.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Bounded backlink-filter data; inspect `truncated` for omitted entries.
#[deny(missing_docs)]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacklinkFilterContext {
    /// Returned root entries.
    pub entries: Vec<BacklinkFilterEntry>,
    /// Whether entries were omitted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Cache-friendly bounded result metadata. The groups stay behind one `Arc` so
/// routine frontend refreshes can reuse the generation-scoped native result
/// without a deep clone while preserving the construction ceiling's outcome.
#[deny(missing_docs)]
#[derive(Debug, Clone)]
pub struct BoundedRefGroups {
    /// Returned groups.
    pub groups: Arc<Vec<RefGroup>>,
    /// Total matching rows before truncation.
    pub total: usize,
    /// Whether a construction limit was exceeded.
    pub exceeded: bool,
}

/// A deliberately bounded block-reference hover preview. Ordinary query,
/// reference, and batched-resolution results carry shallow block identities;
/// callers that genuinely need a subtree must ask for one explicitly and give
/// it node and byte budgets to bound the returned outline.
// The preview can be multiplied across an IPC bridge, so keep both budgets.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockPreview {
    /// Source page and projected subtree.
    pub group: RefGroup,
    /// Number of nodes omitted after either construction budget was reached.
    pub truncated: usize,
}

/// Explicit or plain-text reference evidence.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    /// Parsed link, tag, or embed.
    Explicit,
    /// Unlinked text mention.
    Plain,
}

/// UTF-16 span in matching block text.
#[deny(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceSpan {
    /// UTF-16 code-unit offsets into the matching `BlockDto.raw`.
    pub start: usize,
    /// Exclusive end offset.
    pub end: usize,
}

/// One matched reference and its canonical target.
#[deny(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceOccurrence {
    /// Source spelling matched.
    pub matched_name: String,
    /// Canonical target page name.
    pub canonical: String,
    /// Parsed or plain-text reference.
    pub kind: ReferenceKind,
    /// Match position.
    pub span: ReferenceSpan,
    /// Matching rule identifier.
    pub rule: String,
}

/// Source evidence for one block in a reference result.
#[deny(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceBlockEvidence {
    /// Source block identity.
    pub block_id: String,
    /// Bounded matched occurrences.
    pub occurrences: Vec<ReferenceOccurrence>,
    /// Total parser-owned matches before the bounded evidence cap.
    #[serde(default)]
    pub total: usize,
    /// Whether occurrences were omitted by the evidence cap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDiagnosticTrace {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
    pub occurrences: Vec<ReferenceOccurrence>,
    pub included_linked: bool,
    pub included_unlinked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceDiagnostics {
    pub engine_version: String,
    pub target: String,
    pub traces: Vec<ReferenceDiagnosticTrace>,
}

/// A named template (a block with `template:: <name>`) and the blocks to insert.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDto {
    /// Template name as written by `template::`. Callers match a configured
    /// default journal template by exact, case-sensitive string equality.
    pub name: String,
    /// Blocks to insert.
    pub blocks: Vec<BlockDto>,
    /// Page the template's defining block lives on (so the UI can jump to edit it).
    pub page: String,
    /// Kind of that page (journal/page), for navigation.
    pub kind: PageKind,
}

/// An orphaned asset file (no block references it) — surfaced so the user can
/// review + trash unused media. `size` in bytes; `modified` is the file's
/// last-modified time as Unix seconds (≈ when it entered the graph), or `None`
/// if the filesystem doesn't report it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub modified: Option<u64>,
}

/// Count + total bytes of recoverable asset trash. `count`/`bytes` are asset
/// entries only; the other counters are protected non-asset recovery files that
/// share `logseq/.tine-trash` for backward compatibility.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TrashStats {
    pub count: u64,
    pub bytes: u64,
    pub pages: u64,
    pub journals: u64,
    pub conflicts: u64,
    pub other: u64,
}

/// One file participating in a journal-day conflict: its on-disk filename, a
/// graph-root-relative path (so the UI can navigate straight to THIS file even
/// when it shares a date with the canonical one, #21), a one-line content
/// preview, and whether its name is the canonical date stem (`yyyy_MM_dd`, the
/// one normally kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalFile {
    pub name: String,
    pub path: String,
    pub preview: String,
    pub canonical: bool,
}

/// A journal day that resolves to more than one file (e.g. a canonical
/// `2026_06_26.org` plus a title-named `Friday, 26-06-2026.org`, or a `.md`+`.org`
/// twin). These can't be auto-merged, so they're surfaced for the user to
/// reconcile (delete the redundant one / copy content across).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalConflict {
    pub title: String,
    pub files: Vec<JournalFile>,
}

/// A sync-tool conflict copy left in the graph (Syncthing/Dropbox) — a
/// `*.sync-conflict-*.md` (or Dropbox `(conflicted copy)`) file that shadows a
/// real page. Surfaced so the user can review + reconcile it instead of it
/// rotting as a garbage page. See [`sync_conflict_base`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflict {
    /// Graph-root-relative path of the conflict copy file.
    pub path: String,
    /// Display name of the page it shadows (decoded page name / journal title).
    pub base_name: String,
    /// Graph-root-relative path of the winning (base) file, if it still exists.
    pub base_path: Option<String>,
    /// Kind of the shadowed page (journal/page).
    pub kind: PageKind,
    /// The device/timestamp suffix from the conflict filename (best-effort label).
    pub tag: String,
    /// One-line content preview of the conflict copy.
    pub preview: String,
}

/// Editable page tree. For a new page, the caller builds this value and uses
/// `SaveBase::CreateNew`; the caller applies any journal template before saving.
/// For saves, the target `PageId` controls file identity and format. The
/// pre-block and block raw text/children supply content; metadata fields do
/// not write an implicit `title::` property.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDto {
    /// Display name decoded from the file claim when loaded. On save this
    /// field does not rename the target or inject a `title::` property.
    pub name: String,
    /// Journal or ordinary page. The target file area determines save identity.
    pub kind: PageKind,
    /// Display title; not serialized as a `title::` property by the store.
    pub title: String,
    /// Raw page-property pre-block (if any).
    pub pre_block: Option<String>,
    /// Ordered root blocks.
    pub blocks: Vec<BlockDto>,
    /// Raw-byte `FileRev` string of the on-disk content when loaded. The store's
    /// guarded save uses the separate `SaveBase` argument, not this field;
    /// callers should pass the revision they edited from as that base.
    /// `None` for a page with no file yet.
    #[serde(default)]
    pub rev: Option<String>,
    /// On-disk format for editor display. The save serializer uses the target
    /// file extension, even if this field disagrees. New pages default to Markdown.
    #[serde(default)]
    pub format: Format,
    /// True when a loaded Org page cannot round-trip byte-for-byte, for editor
    /// display. Save rechecks current disk bytes; changing this flag cannot
    /// bypass a read-only refusal or make an editable file read-only.
    #[serde(default)]
    pub read_only: bool,
    /// True for bundled in-app Guide pages. `Store::save` rejects this DTO
    /// before disk access even if the caller supplies a real `PageId`.
    #[serde(default)]
    pub guide: bool,
}
/// Effective graph settings returned when opening a store.
#[deny(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    /// Canonical graph root for display and OS handoff.
    pub root: String,
    /// "now" (LATER/NOW) or "todo" (TODO/DOING) — display form of the
    /// graph configuration's `Workflow`, not an independent edit setting.
    pub preferred_workflow: String,
    /// Configured keyboard shortcuts.
    pub shortcuts: std::collections::HashMap<String, String>,
    /// First day of week in Logseq numbering (0=Monday … 6=Sunday); the
    /// frontend converts it to the date picker's JavaScript weekday index.
    pub start_of_week: u32,
    /// Extra property keys to hide from the rendered properties area.
    pub block_hidden_properties: Vec<String>,
    /// Template name for the caller to apply to a new, empty journal page (if
    /// configured); match it exactly against `TemplateDto.name`. The store
    /// does not insert its body on save.
    pub default_journal_template: Option<String>,
    /// Favorited page names (read from config.edn `:favorites`).
    pub favorites: Vec<String>,
    /// Effective journal title format (`:journal/page-title-format`, default
    /// `MMM do, yyyy`) — so the frontend formats "today" to match the backend.
    pub journal_page_title_format: String,
    /// Effective journal filename format (`:journal/file-name-format`, default
    /// `yyyy_MM_dd`).
    pub journal_file_name_format: String,
    /// Format new pages/journals are created in (`"md"` or `"org"`), from
    /// `:preferred-format`. The frontend uses it to label the toggle and pick the
    /// new-page extension. This string reflects the graph configuration's
    /// `Format`; a page DTO's `format` reflects its physical file extension.
    pub preferred_format: String,
    /// User-defined `:macros {"name" "template"}` — the frontend substitutes
    /// `$1..$N` args into the template and renders the result as markdown.
    pub macros: std::collections::HashMap<String, String>,
    /// `:feature/enable-timetracking?` effective value; default true.
    pub enable_timetracking: bool,
    /// `:ui/show-brackets?` effective value; default true.
    pub show_brackets: bool,
    /// `:shortcut/doc-mode-enter-for-new-block?` effective value; default false.
    pub doc_mode_enter_for_new_block: bool,
    /// `:editor/logical-outdenting?` effective value; default false.
    pub logical_outdenting: bool,
    /// `:logbook/settings :with-second-support?` effective value; default true.
    pub logbook_with_second_support: bool,
    /// `:logbook/settings :enabled-in-timestamped-blocks` effective value.
    pub logbook_enabled_in_timestamped_blocks: bool,
    /// `:logbook/settings :enabled-in-all-blocks` effective value.
    pub logbook_enabled_in_all_blocks: bool,
    /// Tine-owned graph-local flag: whether this graph has already seen the
    /// one-time in-app Guide announcement.
    pub guide_announced: bool,
}
pub fn block_dto_estimated_bytes(block: &BlockDto) -> usize {
    block.id.len()
        + block.raw.len()
        + block.breadcrumb.iter().map(String::len).sum::<usize>()
        + block.tags.iter().map(String::len).sum::<usize>()
        + block
            .properties
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
        + block
            .children
            .iter()
            .map(block_dto_estimated_bytes)
            .sum::<usize>()
        + 128
}

/// Conservative owned-memory estimate for a result payload. Tauri commands use
/// this before serialization as a second guard beside the row cap; derived
/// caches use the same accounting so transport and retention budgets cannot
/// drift apart.
pub fn ref_groups_estimated_bytes(groups: &[RefGroup]) -> usize {
    groups
        .iter()
        .map(|group| {
            group.page.len()
                + group
                    .blocks
                    .iter()
                    .map(block_dto_estimated_bytes)
                    .sum::<usize>()
                + group
                    .evidence
                    .iter()
                    .map(|evidence| {
                        evidence.block_id.len()
                            + evidence
                                .occurrences
                                .iter()
                                .map(|occurrence| {
                                    occurrence.matched_name.len()
                                        + occurrence.canonical.len()
                                        + occurrence.rule.len()
                                        + std::mem::size_of::<ReferenceOccurrence>()
                                })
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                + std::mem::size_of::<RefGroup>()
        })
        .sum()
}
impl GraphMeta {
    /// Rebuild display metadata after a config change. Pass the canonical
    /// graph-root string returned by `Store::open` (or its canonical path),
    /// the current config, and a `JournalFormat` built from that config.
    pub fn from_config(
        root: String,
        config: &crate::config::Config,
        journal_format: &crate::date::JournalFormat,
    ) -> Self {
        Self {
            root,
            preferred_workflow: match config.preferred_workflow {
                crate::config::Workflow::Todo => "todo".into(),
                crate::config::Workflow::Now => "now".into(),
            },
            shortcuts: config.shortcuts.clone(),
            start_of_week: config.start_of_week,
            block_hidden_properties: config.block_hidden_properties.clone(),
            default_journal_template: config.default_journal_template.clone(),
            favorites: config.favorites.clone(),
            journal_page_title_format: journal_format.title_format().to_string(),
            journal_file_name_format: journal_format.file_format().to_string(),
            preferred_format: config.preferred_format.ext().to_string(),
            macros: config.macros.clone(),
            enable_timetracking: config.enable_timetracking,
            show_brackets: config.show_brackets,
            doc_mode_enter_for_new_block: config.doc_mode_enter_for_new_block,
            logical_outdenting: config.logical_outdenting,
            logbook_with_second_support: config.logbook.with_second_support,
            logbook_enabled_in_timestamped_blocks: config.logbook.enabled_in_timestamped_blocks,
            logbook_enabled_in_all_blocks: config.logbook.enabled_in_all_blocks,
            guide_announced: config.guide_announced,
        }
    }
}
