//! Pure DTOs that cross the Tauri IPC boundary, and file-name classification
//! helpers. The graph itself (`Graph`, all file I/O) lives in `tine-store`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    Journal,
    Page,
}

/// On-disk file format of a page. Markdown (`.md`) is the default; Logseq org
/// graphs use `.org`. A graph may mix the two — format is decided per file by
/// extension, never graph-wide (matching OG, which stores `:block/format` per
/// page). The graph's `:preferred-format` only chooses the extension for NEW
/// files (see [`Graph::preferred_format`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Md,
    Org,
}

impl Format {
    /// Format of a page file by its extension (`.org` → Org, else Md).
    pub fn from_path(p: &Path) -> Format {
        match p.extension().and_then(|e| e.to_str()) {
            Some("org") => Format::Org,
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

/// Lightweight entry for the page list / sidebar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageEntry {
    pub name: String,
    pub kind: PageKind,
    /// Sort key `yyyymmdd` for journals; `None` for ordinary pages.
    pub date_key: Option<i64>,
    /// Graph-root-relative path exposed to the frontend so duplicate basenames
    /// can be opened by file, not by ambiguous `(kind,name)`.
    #[serde(rename = "path", default)]
    pub rel_path: String,
    #[serde(skip)]
    pub path: PathBuf,
}

/// A block as sent to / received from the frontend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlockDto {
    pub id: String,
    pub raw: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<BlockDto>,
    /// Ancestor first-lines (page-relative path) for search/reference results;
    /// empty for normal page loads. Lets the UI show a "parent › child" trail.
    #[serde(default)]
    pub breadcrumb: Vec<String>,
    /// Synthetic, read-only result row representing references from the source
    /// page's property pre-block rather than an editable outline block.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_property: bool,
    // --- M1: block-header facets, computed ONCE off the lsdoc projection (the one
    // grammar source) and shipped so the frontend never re-derives them with its
    // own scanner. Derived (not authoritative — `raw` round-trips); the frontend
    // recomputes locally only for the block it is actively editing. Omitted from the
    // wire when empty to keep the payload small (most blocks have no marker/dates).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<(String, String)>,
}

/// A group of blocks from one source page — used for both Linked References
/// (backlinks) and `{{query}}` results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefGroup {
    pub page: String,
    pub kind: PageKind,
    pub blocks: Vec<BlockDto>,
    /// Result-only source evidence keyed by block id. Empty for ordinary query
    /// groups and older callers; never crosses the block write boundary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ReferenceBlockEvidence>,
}

/// One backlink root whose visible subtree can be searched and whose OG-style
/// co-reference facets came from the cached lsdoc projection. This is fetched
/// only when the Linked References filter opens; ordinary backlink DTOs remain
/// shallow so their lazy-loading and bridge cost do not change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterTarget {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacklinkFilterEntry {
    pub page: String,
    pub kind: PageKind,
    pub block_id: String,
    pub text: String,
    pub facets: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BacklinkFilterContext {
    pub entries: Vec<BacklinkFilterEntry>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// Cache-friendly bounded result metadata. The groups stay behind one `Arc` so
/// routine frontend refreshes can reuse the generation-scoped native result
/// without a deep clone while preserving the construction ceiling's outcome.
#[derive(Debug, Clone)]
pub struct BoundedRefGroups {
    pub groups: Arc<Vec<RefGroup>>,
    pub total: usize,
    pub exceeded: bool,
}

/// A deliberately bounded block-reference hover preview. Ordinary query,
/// reference, and batched-resolution results carry shallow block identities;
/// callers that genuinely need a subtree must ask for one explicitly and give
/// it node and byte budgets so an outline cannot be multiplied across the IPC
/// bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockPreview {
    pub group: RefGroup,
    /// Number of nodes omitted after either construction budget was reached.
    pub truncated: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Explicit,
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceSpan {
    /// UTF-16 code-unit offsets into the matching `BlockDto.raw`.
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceOccurrence {
    pub matched_name: String,
    pub canonical: String,
    pub kind: ReferenceKind,
    pub span: ReferenceSpan,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceBlockEvidence {
    pub block_id: String,
    pub occurrences: Vec<ReferenceOccurrence>,
    /// Total parser-owned matches before the bounded evidence cap.
    #[serde(default)]
    pub total: usize,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateDto {
    pub name: String,
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

/// A full page as sent to / received from the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageDto {
    pub name: String,
    pub kind: PageKind,
    pub title: String,
    /// Raw page-property pre-block (if any).
    pub pre_block: Option<String>,
    pub blocks: Vec<BlockDto>,
    /// Hash of the on-disk file content when this page was loaded — the editor's
    /// baseline. Sent back on save so we conflict against the version the editor
    /// actually loaded (not the mutable cache, which the watcher can advance).
    /// `None` for a page with no file yet.
    #[serde(default)]
    pub rev: Option<String>,
    /// On-disk format of this page (markdown vs org), so the editor renders org
    /// inline syntax and shows the right bullet. New pages default to markdown.
    #[serde(default)]
    pub format: Format,
    /// True for an org page Tine can't round-trip byte-for-byte: the editor shows
    /// it but disables editing, so Tine never rewrites (and risks corrupting) it.
    #[serde(default)]
    pub read_only: bool,
    /// Graph-root-relative path of the file this page was loaded from
    /// (`journals/2026_06_26.org`), forward-slashed. Echoed back on save so a page
    /// pinned to a SPECIFIC file — a duplicate-day stray that shares a `(kind,name)`
    /// with the canonical file — saves to its own file instead of being re-resolved
    /// by name to the canonical one (#21). Empty for a brand-new page with no file
    /// yet; then save resolves the path by name, exactly as before.
    #[serde(default)]
    pub path: String,
    /// True for bundled in-app Guide pages. Guide pages are ephemeral/read-only
    /// virtual pages and must never be persisted into the user's graph by the
    /// normal save/writeback path.
    #[serde(default)]
    pub guide: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub root: String,
    pub journals_dir: String,
    pub pages_dir: String,
    /// "now" (LATER/NOW) or "todo" (TODO/DOING) — drives the task cycle.
    pub preferred_workflow: String,
    pub shortcuts: std::collections::HashMap<String, String>,
    /// First day of week for the date picker (0=Sunday … 6=Saturday).
    pub start_of_week: u32,
    /// Extra property keys to hide from the rendered properties area.
    pub block_hidden_properties: Vec<String>,
    /// Template name applied to a new, empty journal page (if configured).
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
    /// new-page extension.
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
    pub fn from_config(
        root: String,
        config: &crate::config::Config,
        journal_format: &crate::date::JournalFormat,
    ) -> Self {
        Self {
            root,
            journals_dir: config.journals_dir.clone(),
            pages_dir: config.pages_dir.clone(),
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
