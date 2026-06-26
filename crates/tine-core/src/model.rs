//! Graph model: opening a graph directory, listing/loading/saving pages, and
//! the DTOs that cross the Tauri IPC boundary.
//!
//! For M0/M1 the canonical state is the on-disk files; Rust loads a page into a
//! [`PageDto`] tree and writes it back from one. The frontend owns the live
//! editing tree (see plan). UUIDs are assigned fresh per load and only
//! persisted as `id::` once a block is referenced (a later milestone).

use crate::config::Config;
use crate::date::{JournalDate, JournalFormat};
use crate::doc::{self, DocBlock, Document};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Whether `path` is a page file Tine reads (markdown or org).
fn is_page_file(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("md") | Some("org"))
}

/// Error for an ambiguous page that exists as both a `.md` and a `.org` file.
/// Deliberately NOT the `AlreadyExists`/"conflict" signal, so the UI surfaces it
/// as a plain error (a toast) instead of a keep-mine/use-disk conflict prompt.
fn twin_error(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Other,
        format!(
            "\"{name}\" exists as both a .md and a .org file — remove one (e.g. in Logseq) to edit it in Tine"
        ),
    )
}

/// Parse a page file's bytes into a [`Document`] using the parser for its
/// format (org headlines vs markdown bullets), chosen by the path's extension.
fn parse_doc(path: &Path, content: &str) -> Document {
    match Format::from_path(path) {
        Format::Md => doc::parse(content),
        Format::Org => crate::org::parse_org(content),
    }
}

/// Lightweight entry for the page list / sidebar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageEntry {
    pub name: String,
    pub kind: PageKind,
    /// Sort key `yyyymmdd` for journals; `None` for ordinary pages.
    pub date_key: Option<i64>,
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
}

/// A group of blocks from one source page — used for both Linked References
/// (backlinks) and `{{query}}` results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefGroup {
    pub page: String,
    pub kind: PageKind,
    pub blocks: Vec<BlockDto>,
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

/// Count + total bytes of the recoverable asset trash (`logseq/.tine-trash`).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TrashStats {
    pub count: u64,
    pub bytes: u64,
}

/// One file participating in a journal-day conflict: its on-disk filename, a
/// one-line content preview, and whether its name is the canonical date stem
/// (`yyyy_MM_dd`, the one normally kept).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalFile {
    pub name: String,
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
}

pub struct Graph {
    pub root: PathBuf,
    pub config: Config,
    /// Journal date formats (filename + title) resolved from `config.edn`, used to
    /// recognize journal files in the user's format and render new ones. Built once
    /// at open (config changes need a reopen, as in OG).
    pub journal_format: JournalFormat,
    /// In-memory cache of every parsed page, keyed implicitly by position.
    /// Built once on first whole-graph query and kept in sync by edits, so
    /// search / backlinks / `{{query}}` scan memory instead of re-reading and
    /// re-parsing the entire tree on every keystroke. `None` = not yet built.
    // `Arc<Document>` so a cache snapshot or a save's scoped-invalidation copy is
    // an O(1) refcount bump, not a deep clone of the whole page (see cache_upsert).
    cache: RwLock<Option<Vec<(PageEntry, Arc<Document>)>>>,
    /// Bumped on every cache mutation (upsert/remove). The lock-free cache build
    /// captures this before reading disk and rebuilds if a mutation raced it
    /// (which would otherwise install stale content over a concurrent save).
    cache_gen: std::sync::atomic::AtomicU64,
    /// Serializes whole-graph cache builds so a racing warmup/search/query parses
    /// the graph ONCE, not once per caller. Held only during the build (not the
    /// cache lock), so it never blocks readers of an already-built cache.
    build_lock: std::sync::Mutex<()>,
    /// Cached `alias:: → canonical` pairs, derived from the page cache. Rebuilt
    /// lazily and dropped whenever the page cache mutates (the only time aliases
    /// can change). Avoids re-scanning the whole graph for aliases on every page
    /// load / backlink lookup.
    alias_cache: RwLock<Option<Vec<(String, String)>>>,
    /// `block uuid / id:: → page name` hint, derived from the page cache and keyed
    /// by `cache_gen` so it self-invalidates on any cache mutation (same pattern as
    /// `alias_cache`). Lets `((uuid))` ref / embed resolution jump straight to the
    /// owning page instead of walking every block of every page. A stale hint is
    /// harmless: resolution falls back to a full scan when the block isn't found.
    block_index: RwLock<Option<(u64, std::collections::HashMap<String, String>)>>,
    /// Memoized results of the pervasive whole-graph scans (run_query / backlinks /
    /// unlinked_refs), keyed by `(cache_gen, today)` so it self-invalidates on ANY
    /// cache mutation and on a date rollover (relative-date queries depend on
    /// today). Lets a re-render, a second component showing the same query, or
    /// navigating back to a page recompute nothing; never serves a stale result.
    derived_cache: RwLock<Option<DerivedCache>>,
    /// Memoized `list_pages()` (the journals//pages/ directory scan), keyed by
    /// cache_gen — which bumps on every page create/delete/rename (Tine or watcher)
    /// — so quick-switch / [[ ]] autocomplete don't re-read both dirs on every
    /// keystroke. An externally-created page not yet seen by the watcher is at most
    /// one watcher tick (≤3s) stale here.
    page_list_cache: RwLock<Option<(u64, Vec<PageEntry>)>>,
    /// `path → content_rev` of the bytes Tine last wrote to each page file,
    /// recorded *before* the write lands on disk. The file watcher reads files
    /// outside the cache lock, so during the window between a save's atomic rename
    /// and its `cache_upsert` it can read disk-ahead-of-cache and mistake Tine's
    /// own write for an external change. This lets the watcher recognize the exact
    /// bytes we wrote and suppress that false positive (the parse-cache comparison
    /// alone races that window). See `write_page` / `sync_file_content`.
    recent_writes: std::sync::Mutex<std::collections::HashMap<PathBuf, String>>,
    /// `key(kind,name) → content_rev` of the on-disk bytes the cached page's
    /// `Document` was parsed from. Invariant: an entry exists IFF the page is in
    /// the cache, and `disk_revs[key] == content_rev(current disk bytes)` ⟹ the
    /// cached doc reflects disk (is fresh). Lets `sync_file_content` skip the
    /// parse→serialize→parse freshness comparison when a file is unchanged — the
    /// common case on every page navigation and most watcher polls. A missing or
    /// mismatched entry always falls through to the correct parse-compare path, so
    /// the worst a desync can cause is redundant work, never a stale serve.
    disk_revs: RwLock<std::collections::HashMap<String, String>>,
    /// All page names referenced anywhere — `[[link]]`/`#tag`/`#[[..]]` plus
    /// `tags::`/`alias::` property values — in their as-written display case,
    /// keyed by `cache_gen`. Like OG, a page that is only referenced (never given
    /// its own file) still "exists" — this lets quick-switch / `[[ ]]`/`#`
    /// autocomplete surface such a page instead of offering a misleading
    /// "Create …". Built from the page cache, and only when it's already warm
    /// (never force-built on a keystroke); empty until then.
    referenced_names_cache: RwLock<Option<(u64, Vec<String>)>>,
    /// Per-resolved-path write locks. The same page file has TWO in-process
    /// writers — the editor (`save_page`/`write_page`) and the PDF highlight path
    /// (`write_highlights`, for an `hls__` page) — and a rename rewrites many
    /// files at once. Holding the per-path lock across the whole
    /// read→conflict-check→write→`cache_upsert` makes same-page writes serialize,
    /// so they can't clobber each other or leave a stale self-write marker.
    /// Lock order is ALWAYS page_lock → cache → disk_revs; never the reverse.
    page_locks: std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<std::sync::Mutex<()>>>>,
}

/// Cache key for `disk_revs` (and any other (kind,name)-keyed side table):
/// case-insensitive name, scoped by kind so a page and a journal of the same
/// title never collide.
fn rev_key(kind: PageKind, name: &str) -> String {
    format!("{kind:?}\u{1}{}", name.to_ascii_lowercase())
}

/// Gen+today-tagged cache of derived scan results. Reset wholesale whenever the
/// tag no longer matches — so every entry is always consistent with the current
/// graph state (no per-entry invalidation to get wrong).
struct DerivedCache {
    gen: u64,
    today: i64,
    // `Arc<Vec<RefGroup>>` so serving a memoized result (every dataRev re-render)
    // is a refcount bump, not a deep clone of every matched block (see derived_memo).
    results: std::collections::HashMap<String, Arc<Vec<RefGroup>>>,
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
}

impl Graph {
    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let config = fs::read_to_string(root.join("logseq").join("config.edn"))
            .map(|s| Config::parse(&s))
            .unwrap_or_default();
        let journal_format = JournalFormat::new(
            config.journal_file_name_format.as_deref(),
            config.journal_page_title_format.as_deref(),
        );
        Graph {
            root,
            config,
            journal_format,
            cache: RwLock::new(None),
            cache_gen: std::sync::atomic::AtomicU64::new(0),
            build_lock: std::sync::Mutex::new(()),
            alias_cache: RwLock::new(None),
            block_index: RwLock::new(None),
            derived_cache: RwLock::new(None),
            page_list_cache: RwLock::new(None),
            recent_writes: std::sync::Mutex::new(std::collections::HashMap::new()),
            disk_revs: RwLock::new(std::collections::HashMap::new()),
            referenced_names_cache: RwLock::new(None),
            page_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The write lock for a resolved page path (see `page_locks`). Returns an
    /// `Arc` the caller holds (`let _g = lock.lock().unwrap();`) for the critical
    /// section. The `page_locks` map mutex is released before the per-page lock is
    /// taken, so callers never serialize on the map. Opportunistically prunes
    /// entries no caller still holds (strong_count == 1) to bound growth.
    fn page_lock(&self, path: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
        let mut map = self.page_locks.lock().unwrap();
        if map.len() >= 64 {
            map.retain(|_, v| std::sync::Arc::strong_count(v) > 1);
        }
        map.entry(path.to_path_buf())
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone()
    }

    pub fn meta(&self) -> GraphMeta {
        GraphMeta {
            root: self.root.display().to_string(),
            journals_dir: self.config.journals_dir.clone(),
            pages_dir: self.config.pages_dir.clone(),
            preferred_workflow: match self.config.preferred_workflow {
                crate::config::Workflow::Todo => "todo".into(),
                crate::config::Workflow::Now => "now".into(),
            },
            shortcuts: self.config.shortcuts.clone(),
            start_of_week: self.config.start_of_week,
            block_hidden_properties: self.config.block_hidden_properties.clone(),
            default_journal_template: self.config.default_journal_template.clone(),
            favorites: self.config.favorites.clone(),
            journal_page_title_format: self.journal_format.title_format().to_string(),
            journal_file_name_format: self.journal_format.file_format().to_string(),
            preferred_format: self.config.preferred_format.ext().to_string(),
        }
    }

    /// Current cache generation — bumped on every cache-mutating page change, and
    /// the key that memoized queries/backlinks/derived results invalidate against.
    /// Exposed for observability and tests (e.g. asserting a no-op save doesn't
    /// needlessly invalidate everything).
    pub fn cache_generation(&self) -> u64 {
        self.cache_gen.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn journals_path(&self) -> PathBuf {
        self.root.join(&self.config.journals_dir)
    }

    pub fn pages_path(&self) -> PathBuf {
        self.root.join(&self.config.pages_dir)
    }

    /// The format (`Md`/`Org`) new pages and journals are created in, from
    /// `config.edn`'s `:preferred-format`. Existing files keep their own format.
    pub fn preferred_format(&self) -> Format {
        self.config.preferred_format
    }

    /// List all pages and journals in the graph.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((g, entries)) = self.page_list_cache.read().unwrap().as_ref() {
            if *g == gen {
                return entries.clone();
            }
        }
        let mut entries = Vec::new();
        entries.extend(list_md(&self.journals_path(), PageKind::Journal, &self.journal_format));
        entries.extend(list_md(&self.pages_path(), PageKind::Page, &self.journal_format));
        *self.page_list_cache.write().unwrap() = Some((gen, entries.clone()));
        entries
    }

    /// Page names referenced anywhere in the graph — inline `[[link]]`/`#tag`/
    /// `#[[..]]` plus `tags::`/`alias::` property values (block- and page-level) —
    /// display case preserved, deduped case-insensitively. These are the pages
    /// that "exist" by reference even without a file of their own (OG semantics),
    /// so autocomplete/quick-switch can offer them instead of "Create …".
    ///
    /// Computed from the whole-graph cache and memoized by `cache_gen`. If the
    /// cache isn't warm yet it returns empty and memoizes nothing — we never force
    /// a full-graph parse from here (this runs on autocomplete keystrokes).
    pub fn referenced_page_names(&self) -> Vec<String> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((g, names)) = self.referenced_names_cache.read().unwrap().as_ref() {
            if *g == gen {
                return names.clone();
            }
        }
        let guard = self.cache.read().unwrap();
        let Some(pages) = guard.as_ref() else {
            return Vec::new(); // cache not warm — don't force a parse, don't memoize
        };
        fn add(seen: &mut std::collections::HashMap<String, String>, name: String) {
            if !name.is_empty() {
                seen.entry(name.to_lowercase()).or_insert(name);
            }
        }
        // `tags::` / `alias::` property values are page references in OG too —
        // comma-separated, written bare or as `[[..]]`/`#..` — so a page named
        // only in a `tags::`/`alias::` list still "exists". Strip any wrapping
        // down to the page name. (Line-based, like DocBlock::property.)
        fn add_property_refs(seen: &mut std::collections::HashMap<String, String>, text: &str) {
            for line in text.lines() {
                let Some((k, v)) = crate::doc::parse_property_line(line) else { continue };
                if !(k.eq_ignore_ascii_case("tags") || k.eq_ignore_ascii_case("alias")) {
                    continue;
                }
                for val in v.split(',') {
                    let t = val.trim();
                    let t = t.strip_prefix("[[").and_then(|x| x.strip_suffix("]]")).unwrap_or(t);
                    add(seen, t.strip_prefix('#').unwrap_or(t).trim().to_string());
                }
            }
        }
        fn visit(b: &DocBlock, seen: &mut std::collections::HashMap<String, String>) {
            for name in crate::refs::page_refs(&b.raw) {
                add(seen, name);
            }
            add_property_refs(seen, &b.raw); // block-level tags::/alias::
            for c in &b.children {
                visit(c, seen);
            }
        }
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (_, doc) in pages.iter() {
            if let Some(pre) = &doc.pre_block {
                add_property_refs(&mut seen, pre); // page-level tags::/alias::
            }
            for b in &doc.roots {
                visit(b, &mut seen);
            }
        }
        drop(guard);
        let names: Vec<String> = seen.into_values().collect();
        *self.referenced_names_cache.write().unwrap() = Some((gen, names.clone()));
        names
    }

    /// Journals sorted newest-first.
    pub fn journals_desc(&self) -> Vec<PageEntry> {
        // Prefer the warmed whole-graph cache — its PageEntry list is kept current
        // by cache_upsert/cache_remove, so we avoid a directory read + parse on
        // every infinite-scroll feed append. Fall back to scanning the dir while
        // the cache isn't built yet.
        let raw: Vec<PageEntry> = match self.cache.read().unwrap().as_ref() {
            Some(pages) => pages
                .iter()
                .filter(|(e, _)| e.kind == PageKind::Journal && e.date_key.is_some())
                .map(|(e, _)| e.clone())
                .collect(),
            None => list_md(&self.journals_path(), PageKind::Journal, &self.journal_format)
                .into_iter()
                .filter(|e| e.date_key.is_some())
                .collect(),
        };
        // Dedup by date: a day with more than one file (e.g. a leftover
        // title-named duplicate of a `yyyy_MM_dd` file) must appear ONCE in the
        // feed, not twice — both files resolve to the same page name, so without
        // this the same day renders twice (loaded from whichever file path_for
        // picks). Keep the canonical date-stem file (the one saves resolve to);
        // the stray stays visible via journal_conflicts() for reconciliation.
        let is_canonical = |e: &PageEntry| {
            e.path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| JournalDate::from_file_stem(s).is_some())
        };
        let mut by_date: std::collections::BTreeMap<i64, PageEntry> =
            std::collections::BTreeMap::new();
        for e in raw {
            let Some(key) = e.date_key else { continue };
            use std::collections::btree_map::Entry;
            match by_date.entry(key) {
                Entry::Vacant(v) => {
                    v.insert(e);
                }
                Entry::Occupied(mut o) => {
                    if is_canonical(&e) && !is_canonical(o.get()) {
                        o.insert(e);
                    }
                }
            }
        }
        let mut js: Vec<PageEntry> = by_date.into_values().collect();
        js.sort_by_key(|e| std::cmp::Reverse(e.date_key.unwrap_or(0)));
        js
    }

    /// Journal `date_key`s (yyyymmdd) whose page has real content — i.e. at
    /// least one block with a non-empty, non-property line. Drives the calendar
    /// picker's empty/non-empty day marking. Served from the cache.
    pub fn journal_content_days(&self) -> Vec<i64> {
        self.with_pages(|pages| {
            pages
                .iter()
                .filter(|(e, _)| e.kind == PageKind::Journal)
                .filter_map(|(e, d)| e.date_key.filter(|_| doc_has_content(&d.roots)))
                .collect()
        })
    }

    /// One-time recovery: a journal that was saved under its display title
    /// ("Jun 18th, 2026.md") instead of its date stem ("2026_06_18.md") can't be
    /// parsed back to a date, so it drops out of the feed and the day looks
    /// empty. Rename such files to their stem — but only when the stem file
    /// doesn't already exist (never clobber/merge). Returns how many were fixed.
    pub fn migrate_journal_filenames(&self) -> usize {
        let dir = self.journals_path();
        let Ok(rd) = fs::read_dir(&dir) else { return 0 };
        let mut n = 0;
        for e in rd.flatten() {
            let p = e.path();
            // Both formats — an org graph's title-named journals are `.org`.
            let ext = match p.extension().and_then(|x| x.to_str()) {
                Some(e @ ("md" | "org")) => e,
                _ => continue,
            };
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if JournalDate::from_file_stem(stem).is_some() {
                continue; // already a plausible date stem (yyyy_MM_dd / yyyy-MM-dd) — leave it
            }
            // A title-named ("Jun 18th, 2026.md", "Thursday, 25-06-2026.org") or
            // otherwise non-stem journal file: normalize it to the graph's filename
            // format so it round-trips with OG and is recognized in the feed.
            let Some(d) = self.journal_format.parse(stem) else { continue }; // not a date
            let want = self.journal_format.file_stem(d);
            if want == stem {
                continue; // already in the graph's filename format
            }
            let target = dir.join(format!("{want}.{ext}"));
            if target.exists() {
                continue; // don't clobber an existing stem file
            }
            if fs::rename(&p, &target).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Journal days that resolve to more than one file — the migration leaves these
    /// alone (it never clobbers), so they're reported for the user to reconcile.
    /// Each file gets a one-line preview and a `canonical` flag (date-stem name).
    pub fn journal_conflicts(&self) -> Vec<JournalConflict> {
        let dir = self.journals_path();
        let Ok(rd) = fs::read_dir(&dir) else { return Vec::new() };
        let mut by_date: std::collections::BTreeMap<i64, Vec<(String, PathBuf, bool)>> =
            std::collections::BTreeMap::new();
        for e in rd.flatten() {
            let p = e.path();
            let ext = match p.extension().and_then(|x| x.to_str()) {
                Some(x @ ("md" | "org")) => x.to_string(),
                _ => continue,
            };
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            // A date-stem file is canonical; otherwise try to parse its title.
            let canonical = JournalDate::from_file_stem(stem).is_some();
            let date = JournalDate::from_file_stem(stem).or_else(|| self.journal_format.parse(stem));
            if let Some(d) = date {
                by_date.entry(d.ordinal_key()).or_default().push((format!("{stem}.{ext}"), p, canonical));
            }
        }
        let mut out = Vec::new();
        for (key, files) in by_date {
            if files.len() < 2 {
                continue;
            }
            let date = JournalDate::from_ordinal(key);
            let mut jfiles: Vec<JournalFile> = files
                .into_iter()
                .map(|(name, path, canonical)| {
                    let preview = fs::read_to_string(&path)
                        .ok()
                        .and_then(|c| {
                            c.lines()
                                .map(|l| l.trim_start_matches(|ch| ch == '*' || ch == '-' || ch == ' ' || ch == '\t').trim().to_string())
                                .find(|l| !l.is_empty())
                        })
                        .map(|l| l.chars().take(80).collect::<String>())
                        .unwrap_or_default();
                    JournalFile { name, preview, canonical }
                })
                .collect();
            // Canonical first (the keeper), then alphabetical.
            jfiles.sort_by(|a, b| b.canonical.cmp(&a.canonical).then_with(|| a.name.cmp(&b.name)));
            out.push(JournalConflict { title: self.journal_format.title(date), files: jfiles });
        }
        out
    }

    /// Raw contents of ONE journal file (by exact filename) — lets the UI show a
    /// duplicate day's individual files (which can't be navigated to separately,
    /// as pages are keyed by date) so the user can inspect before reconciling.
    pub fn read_journal_file(&self, name: &str) -> io::Result<String> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad journal file name"));
        }
        fs::read_to_string(self.journals_path().join(name))
    }

    /// Move ONE journal file (by its exact filename) to the recoverable trash —
    /// the affordance for reconciling a duplicate day. Refuses a path separator so
    /// it can't reach outside `journals/`.
    pub fn trash_journal_file(&self, name: &str) -> io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad journal file name"));
        }
        let src = self.journals_path().join(name);
        if !src.is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such journal file"));
        }
        let trash = self.root.join("logseq").join(".tine-trash");
        let dest = trash.join(format!("{}__{name}", trash_stamp()));
        if fs::create_dir_all(&trash).is_err() || fs::rename(&src, &dest).is_err() {
            fs::remove_file(&src)?;
        }
        Ok(())
    }

    /// Resolve a page name to a file path. Journals match by date title;
    /// pages match by filename stem.
    fn path_for(&self, name: &str, kind: PageKind) -> PathBuf {
        let pref = self.preferred_format();
        match kind {
            PageKind::Journal => self
                .journals_desc()
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
                .map(|e| e.path)
                .unwrap_or_else(|| {
                    // New journal: name it by its date stem in the graph's filename
                    // format ("2026_06_18.org"), not the display title — a
                    // title-named file can't be parsed back to a date, so
                    // journals_desc would drop it and the day would look empty. The
                    // extension follows the graph's :preferred-format.
                    let stem = self
                        .journal_format
                        .parse(name)
                        .map(|d| self.journal_format.file_stem(d))
                        .unwrap_or_else(|| name.to_string());
                    self.journals_path().join(format!("{stem}.{}", pref.ext()))
                }),
            PageKind::Page => {
                // Resolve to an EXISTING file (any format) so a save updates it in
                // place rather than creating a second file in the other extension;
                // a brand-new page is created in the graph's preferred format.
                // Cheap `exists()` probes (the common hit needs one), no dir scan.
                let enc = encode_page_name(name);
                let dir = self.pages_path();
                let primary = dir.join(format!("{enc}.{}", pref.ext()));
                if primary.exists() {
                    return primary;
                }
                let alt_ext = if pref == Format::Org { "md" } else { "org" };
                let alt = dir.join(format!("{enc}.{alt_ext}"));
                if alt.exists() {
                    return alt;
                }
                primary
            }
        }
    }

    /// Whether BOTH a `.md` and a `.org` file exist for the same logical page —
    /// an ambiguous identity, since Tine keys pages by `(kind, name)`. Writes
    /// (save/rename/delete) are refused on such a page so a save can't serve one
    /// twin's content with the other's baseline and clobber the wrong file. This
    /// is an interim guard; the full fix is path/format in page identity (#21).
    /// `.org` is probed first so a markdown-only graph short-circuits after one
    /// stat. A journal whose name doesn't parse to a date stem isn't guarded.
    fn has_twin(&self, name: &str, kind: PageKind) -> bool {
        let (dir, stem) = match kind {
            PageKind::Page => (self.pages_path(), Some(encode_page_name(name))),
            PageKind::Journal => (
                self.journals_path(),
                self.journal_format.parse(name).map(|d| self.journal_format.file_stem(d)),
            ),
        };
        match stem {
            Some(s) => {
                dir.join(format!("{s}.org")).exists() && dir.join(format!("{s}.md")).exists()
            }
            None => false,
        }
    }

    /// Find a page/journal entry by display name.
    pub fn find_entry(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        let dir = match kind {
            PageKind::Journal => self.journals_path(),
            PageKind::Page => self.pages_path(),
        };
        list_md(&dir, kind, &self.journal_format)
            .into_iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Load a page by name; returns `None` if it doesn't exist on disk. Falls
    /// back to alias resolution (`alias::`) for named pages.
    pub fn load_named(&self, name: &str, kind: PageKind) -> io::Result<Option<PageDto>> {
        // A file that vanished between listing and load (external delete) reports
        // NotFound from load_page — map it to "no page" rather than an error, so
        // the page is treated as absent (never resurrected) and the get_page
        // contract (Ok(None) = doesn't exist) holds.
        let load = |entry: &PageEntry| -> io::Result<Option<PageDto>> {
            match self.load_page(entry) {
                Ok(dto) => Ok(Some(dto)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        };
        if let Some(entry) = self.find_entry(name, kind) {
            return load(&entry);
        }
        if kind == PageKind::Page {
            let tnorm = crate::refs::normalize(name);
            if let Some((_, canon)) = self.page_aliases().into_iter().find(|(a, _)| *a == tnorm) {
                if let Some(entry) = self.find_entry(&canon, kind) {
                    return load(&entry);
                }
            }
        }
        Ok(None)
    }

    /// Alias → canonical-page-name pairs (for the UI to resolve links/navigation).
    pub fn page_aliases(&self) -> Vec<(String, String)> {
        if let Some(a) = self.alias_cache.read().unwrap().as_ref() {
            return a.clone();
        }
        let aliases = crate::query::page_aliases(self);
        *self.alias_cache.write().unwrap() = Some(aliases.clone());
        aliases
    }

    /// The page that owns a block uuid / `id::`, via a `cache_gen`-keyed index, or
    /// `None` if unknown. A hint only — callers must verify (the index can lag a
    /// concurrent edit). O(graph) to (re)build once per cache change, then O(1).
    pub fn block_page_hint(&self, uuid: &str) -> Option<String> {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        if let Some((idx_gen, map)) = self.block_index.read().unwrap().as_ref() {
            if *idx_gen == gen {
                return map.get(uuid).cloned();
            }
        }
        fn walk_idx(
            blocks: &[DocBlock],
            name: &str,
            m: &mut std::collections::HashMap<String, String>,
        ) {
            for b in blocks {
                if !b.uuid.is_empty() {
                    m.entry(b.uuid.clone()).or_insert_with(|| name.to_string());
                }
                if let Some(id) = b.property("id") {
                    if !id.is_empty() {
                        m.entry(id).or_insert_with(|| name.to_string());
                    }
                }
                walk_idx(&b.children, name, m);
            }
        }
        let map = self.with_pages(|pages| {
            let mut m = std::collections::HashMap::new();
            for (entry, doc) in pages {
                walk_idx(&doc.roots, &entry.name, &mut m);
            }
            m
        });
        let result = map.get(uuid).cloned();
        *self.block_index.write().unwrap() = Some((gen, map));
        result
    }

    /// A page DTO from the cache ONLY if the cache is already built — never
    /// triggers a (synchronous, whole-graph) build. `None` on a cold cache or a
    /// page not yet cached, so latency-path callers can parse just one file.
    fn peek_cached_page(&self, entry: &PageEntry) -> Option<PageDto> {
        let guard = self.cache.read().unwrap();
        guard
            .as_ref()?
            .iter()
            .find(|(e, _)| e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name))
            .map(|(e, d)| page_dto(e, d))
    }

    /// Load a page by entry. Served from the in-memory cache so block uuids are
    /// stable and consistent with queries / refs / the sidebar. Falls back to a
    /// disk parse for a page not yet in the cache (e.g. just created externally).
    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        // Reconcile any external change into the cache FIRST. Otherwise a stale
        // cache (an edit the 3s watcher hasn't folded in yet) would be served as
        // the editor's content while the rev below reflects the NEW disk bytes —
        // and the editor's save would then clobber the external edit with the rev
        // matching. sync_file is a no-op when the cache already matches disk.
        // Read the file ONCE: reconcile the cache against it, derive the save
        // baseline (rev) from the SAME bytes (so rev and the served content can't
        // disagree via a write landing between two reads), and — on a cache miss —
        // parse it below.
        let read = fs::read_to_string(&entry.path);
        if let Ok(content) = &read {
            self.sync_file_content(&entry.path, content, false);
        } else if read.as_ref().err().is_some_and(|e| e.kind() == io::ErrorKind::NotFound) {
            // The file is gone (external delete) but may still sit in the warm
            // cache. Serving that cached copy below — with rev = None — would make
            // it a null-baseline page, so a later edit + save would treat it as
            // brand-new and silently RESURRECT the externally-deleted file. Evict
            // the stale entry and report NotFound; callers treat the page as
            // absent (the feed skips it, get_page returns None).
            self.forget_file(&entry.path);
            return Err(read.unwrap_err());
        }
        let rev = read.as_ref().ok().map(|s| content_rev(s));
        // Serve from the cache if it's ALREADY built, but never trigger a build
        // here: a cold-cache `with_pages` would synchronously parse the entire
        // graph just to return one page, making first paint scale with graph size
        // (and defeating the background warm). On a cold cache, parse only this
        // file; `warm_cache_async` builds the rest. (Non-ref blocks then get fresh
        // uuids that may differ from the warm cache until the page is reloaded —
        // benign: id:: ref targets are stable, and live-ref views fall back to a
        // read-only render for an unmatched uuid, never losing edits.)
        if let Some(mut dto) = self.peek_cached_page(entry) {
            if let Ok(c) = &read {
                dto.read_only = read_only_org(&entry.path, c);
            }
            dto.rev = rev;
            return Ok(dto);
        }
        // Cache miss: parse the bytes we already read (propagate the original read
        // error if it failed).
        let content = read?;
        let mut doc = parse_doc(&entry.path, &content);
        assign_doc_uuids(&mut doc.roots);
        let mut dto = page_dto(entry, &doc);
        dto.read_only = read_only_org(&entry.path, &content);
        dto.rev = rev;
        Ok(dto)
    }

    /// Read and parse a page file into a [`Document`].
    pub fn read_document(&self, entry: &PageEntry) -> io::Result<Document> {
        let content = fs::read_to_string(&entry.path)?;
        Ok(parse_doc(&entry.path, &content))
    }

    /// Read+parse every page from disk (skipping unreadable files). Used to
    /// build the in-memory cache.
    fn load_all_pages(&self) -> Vec<(PageEntry, Document, String)> {
        self.list_pages()
            .into_iter()
            .filter_map(|e| {
                let content = fs::read_to_string(&e.path).ok()?;
                let rev = content_rev(&content);
                let mut d = parse_doc(&e.path, &content);
                assign_doc_uuids(&mut d.roots);
                Some((e, d, rev))
            })
            .collect()
    }

    /// Install a freshly-built whole-graph snapshot atomically: the parsed pages
    /// into the cache, their on-disk revs into `disk_revs`. Cache set BEFORE
    /// disk_revs so a reader never observes a fresh rev paired with a stale cache.
    fn install_built(&self, built: Vec<(PageEntry, Document, String)>) {
        let revs: std::collections::HashMap<String, String> =
            built.iter().map(|(e, _, r)| (rev_key(e.kind, &e.name), r.clone())).collect();
        let pages: Vec<(PageEntry, Arc<Document>)> =
            built.into_iter().map(|(e, d, _)| (e, Arc::new(d))).collect();
        // Publish cache + revs atomically under the cache lock (cache → disk_revs
        // order), so no reader observes a fresh rev paired with a stale cache.
        let mut guard = self.cache.write().unwrap();
        *guard = Some(pages);
        *self.disk_revs.write().unwrap() = revs;
        drop(guard);
    }

    /// Run `f` over every parsed page, building the cache on first use. The
    /// borrow is held for the duration of `f`, so callers get references without
    /// cloning the documents.
    pub fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Arc<Document>)]) -> T) -> T {
        if let Some(pages) = self.cache.read().unwrap().as_ref() {
            return f(pages);
        }
        // Single-flight build: serialize builders on `build_lock` (NOT the cache
        // lock) so the whole-graph parse happens once, not once per racing caller,
        // and so we never hold the cache write lock during the slow parse.
        use std::sync::atomic::Ordering;
        let _bl = self.build_lock.lock().unwrap();
        if self.cache.read().unwrap().is_none() {
            loop {
                let gen0 = self.cache_gen.load(Ordering::Acquire);
                let built = self.load_all_pages();
                // If a save/remove raced our read (its cache mutation no-op'd
                // because the cache was still None), its disk write is already
                // done — rebuild so we don't install a stale snapshot. We hold
                // build_lock, so no other builder competes.
                if self.cache_gen.load(Ordering::Acquire) == gen0 {
                    self.install_built(built);
                    break;
                }
            }
        }
        drop(_bl);
        f(self.cache.read().unwrap().as_ref().unwrap())
    }

    /// Eagerly build the page cache (call once after opening, off the hot path).
    pub fn warm_cache(&self) {
        use std::sync::atomic::Ordering;
        if self.cache.read().unwrap().is_some() {
            return; // already built (e.g. by a query) — nothing to warm
        }
        // Build PACED and WITHOUT holding build_lock during the parse: on a
        // thermally throttled laptop the warm would otherwise peg a core in one
        // burst right after launch, competing with first scrolling/typing/the
        // first agenda query. Parse into a LOCAL vec in small chunks with a brief
        // yield between them, so the load is spread out and an on-demand
        // `with_pages` (a user query) can still take build_lock and build fast
        // without waiting on our sleeps. If it wins, we discard our work.
        let gen0 = self.cache_gen.load(Ordering::Acquire);
        let entries = self.list_pages();
        let mut built: Vec<(PageEntry, Document, String)> = Vec::with_capacity(entries.len());
        // Record each file's mtime BEFORE reading it, so a re-stat before install
        // catches any external edit that landed during the paced parse (external
        // writers don't bump cache_gen, so the gen check below can't see them).
        let mut mtimes: Vec<(PathBuf, Option<std::time::SystemTime>)> = Vec::with_capacity(entries.len());
        for (i, e) in entries.into_iter().enumerate() {
            let mtime = fs::metadata(&e.path).and_then(|m| m.modified()).ok();
            if let Ok(content) = fs::read_to_string(&e.path) {
                let rev = content_rev(&content);
                let mut d = parse_doc(&e.path, &content);
                assign_doc_uuids(&mut d.roots);
                mtimes.push((e.path.clone(), mtime));
                built.push((e, d, rev));
            }
            if i % 24 == 23 {
                if self.cache.read().unwrap().is_some() {
                    return; // a query built the cache while we parsed
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        // If any built file changed during the paced parse, our snapshot may be
        // stale and the watcher might not yet baseline-track it — discard and let
        // the next on-demand build read fresh. (A false positive just rebuilds.)
        if mtimes
            .iter()
            .any(|(p, m)| fs::metadata(p).and_then(|md| md.modified()).ok() != *m)
        {
            return;
        }
        // Install only if nobody else built it and no Tine save/remove raced our
        // reads (its cache mutation would have no-op'd against the None cache, so
        // its disk write must be folded in by a rebuild — defer to the next
        // on-demand build rather than install a stale snapshot).
        let _bl = self.build_lock.lock().unwrap();
        if self.cache.read().unwrap().is_none() && self.cache_gen.load(Ordering::Acquire) == gen0 {
            self.install_built(built);
        }
    }

    /// Discard the cache; it rebuilds on the next whole-graph query. Use when an
    /// external change may have touched many files.
    pub fn invalidate_cache(&self) {
        let mut guard = self.cache.write().unwrap();
        *guard = None;
        self.disk_revs.write().unwrap().clear(); // under the cache lock (cache → disk_revs)
        // Bump the generation AFTER discarding the cache (under the cache lock), so
        // a reader that loads the new gen then reads the cache sees None (and
        // rebuilds from disk) rather than the stale pre-invalidation content — same
        // gen-after-content ordering as cache_upsert. The gen-keyed block index
        // then rebuilds against fresh content too.
        self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
        drop(guard);
        *self.alias_cache.write().unwrap() = None;
        *self.block_index.write().unwrap() = None;
    }

    /// Update one page in the cache after we write it (no full rebuild). A no-op
    /// if the cache hasn't been built yet. `disk_rev` is `content_rev` of the
    /// exact on-disk bytes `doc` was produced from (the freshness key — see
    /// `disk_revs`).
    fn cache_upsert(&self, entry: PageEntry, mut doc: Document, disk_rev: String) {
        // Fill uuids for any block that lacks one (e.g. PDF-highlight writes);
        // blocks saved from the frontend already carry their ids, which are kept.
        assign_doc_uuids(&mut doc.roots);
        // Only the alias map needs dropping when an `alias::` was added/changed/
        // removed — invalidating on every save would make a normal edit an O(P)
        // alias rescan on the next navigation.
        let new_has_alias = doc_has_alias(&doc);
        let mut alias_touched = new_has_alias;
        let key = rev_key(entry.kind, &entry.name);
        let doc = Arc::new(doc);
        // Keep the new content + identity for the scoped derived-cache pass below
        // (the original is moved into the cache slot; this clone is a refcount bump).
        let evict_doc = Arc::clone(&doc);
        let evict_entry = entry.clone();
        let mut is_new_page = false;
        let mut guard = self.cache.write().unwrap();
        let cache_built = guard.is_some();
        if let Some(pages) = guard.as_mut() {
            match pages.iter_mut().find(|(e, _)| {
                e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name)
            }) {
                Some(slot) => {
                    alias_touched = new_has_alias || doc_has_alias(&slot.1);
                    slot.1 = doc;
                }
                None => {
                    is_new_page = true;
                    pages.push((entry, doc));
                }
            }
            // Update disk_revs WHILE STILL HOLDING the cache write lock, so the
            // cached doc and its freshness rev are published atomically and can
            // never diverge across concurrent same-page writers (e.g. an editor
            // save racing a PDF write_highlights on an hls__ page). If they could
            // diverge, the sync_file_content fast-path could match disk against a
            // rev that isn't the cached doc's and serve a stale doc. Lock order is
            // always cache → disk_revs; readers never hold disk_revs while taking
            // the cache lock, so this nesting can't deadlock. Sets only when the
            // page is actually cached (preserves "entry exists IFF cached").
            self.disk_revs.write().unwrap().insert(key, disk_rev);
        }
        // Bump cache_gen AFTER publishing the new content (and disk_revs), still
        // under the cache write lock. A reader loads cache_gen (Acquire) then takes
        // the cache read lock; because the bump (Release) happens-after the slot
        // write and before the lock is dropped, observing the new gen guarantees
        // the new doc is visible. So any derived result computed at gen G reflects
        // every edit whose gen is <= G — it can never be a stale whole-graph scan
        // that reads the OLD doc yet gets tagged (and served) at the fresh gen.
        // (Bumping FIRST left a window where the gen was new but the doc still old.)
        // The bump is unconditional — even on a cold cache (no slot to update) — so
        // a concurrent lock-free with_pages build still detects the race and retries.
        let newgen = self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release) + 1;
        drop(guard);
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
        // Scoped query/backlink invalidation (#52): a content edit to one page
        // can't change a derived result the page doesn't participate in, so keep
        // those (advancing their generation) and recompute only the entries this
        // page is in or now matches. An alias change, a new page, or a cold cache
        // has graph-wide effects → drop everything. Guarded by the differential
        // fuzz oracle in tests/derived_cache_fuzz.rs.
        let scoped = cache_built && !alias_touched && !is_new_page;
        self.scope_derived_invalidation(&evict_entry, &evict_doc, newgen, scoped);
    }

    /// See `cache_upsert`. When `scoped`, evict only derived entries the edited
    /// page (`entry`, `doc`) participates in and re-tag the survivors to `newgen`;
    /// otherwise drop the whole derived cache.
    fn scope_derived_invalidation(&self, entry: &PageEntry, doc: &Document, newgen: u64, scoped: bool) {
        // Resolve aliases BEFORE taking the derived lock (page_aliases may take the
        // cache lock); never hold derived while taking cache.
        let aliases = if scoped { self.page_aliases() } else { Vec::new() };
        let today = crate::date::JournalDate::today().ordinal_key();
        // Hold the derived write lock across the WHOLE prune+re-tag. This is
        // deliberately atomic: the keep/evict test (page_affects_*) is re-evaluated
        // against whatever entry is CURRENTLY in the map, so a result a concurrent
        // query inserted (possibly from an older page-doc) is re-judged and evicted
        // if this edit affects it — never kept on pointer identity. Combined with
        // the gen-after-content bump (cache_upsert), an entry that survives the
        // prune is provably unaffected by this edit and consistent at `newgen`.
        // (An earlier version evaluated off the lock and kept entries by Arc
        // ptr_eq; that could bless a stale concurrent recompute — reverted.)
        let mut g = self.derived_cache.write().unwrap();
        let Some(dc) = g.as_mut() else { return };
        if !scoped || dc.today != today {
            *g = None; // full invalidate (alias/page-set/cold-cache, or day rollover)
            return;
        }
        let pname = &entry.name;
        dc.results.retain(|key, result| {
            // Evict iff this page is already in the result OR now matches the key's
            // predicate; keep (still correct) otherwise.
            if result.iter().any(|grp| grp.page.eq_ignore_ascii_case(pname)) {
                return false;
            }
            let affects = match key.split_once('\0') {
                Some(("b", t)) => crate::query::page_affects_backlinks(&aliases, t, doc),
                Some(("u", t)) => crate::query::page_affects_unlinked(t, doc),
                Some(("q", s)) => crate::query::page_affects_query(s, entry, doc),
                _ => true, // unknown key shape → evict to stay safe
            };
            !affects
        });
        dc.gen = newgen; // survivors are valid for the post-bump generation
    }

    /// Drop one page from the cache after deleting its file.
    fn cache_remove(&self, name: &str, kind: PageKind) {
        // A page delete is a page-set change (affects namespaces, exists-by-ref,
        // every backlink/query) — drop the whole derived cache.
        *self.derived_cache.write().unwrap() = None;
        let mut guard = self.cache.write().unwrap();
        let mut alias_touched = false;
        if let Some(pages) = guard.as_mut() {
            if let Some((_, d)) =
                pages.iter().find(|(e, _)| e.kind == kind && e.name.eq_ignore_ascii_case(name))
            {
                alias_touched = doc_has_alias(d);
            }
            pages.retain(|(e, _)| !(e.kind == kind && e.name.eq_ignore_ascii_case(name)));
            // Drop the rev under the cache lock (same cache → disk_revs order as
            // cache_upsert) so the two never diverge.
            self.disk_revs.write().unwrap().remove(&rev_key(kind, name));
        }
        // Bump AFTER the removal is published (under the cache lock), so a reader
        // that loads the new gen is guaranteed to see the page gone — see the
        // gen-after-content note in cache_upsert.
        self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
        drop(guard);
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
    }

    /// Memoize a derived whole-graph scan result, keyed by `(cache_gen, today)` +
    /// `key`. On a tag mismatch the whole cache is dropped, so a hit is always
    /// consistent with the current graph. `compute` runs with NO lock held (it
    /// takes the cache read lock itself), so it can't deadlock against `with_pages`.
    fn derived_memo(
        &self,
        key: String,
        compute: impl FnOnce() -> Vec<RefGroup>,
    ) -> Arc<Vec<RefGroup>> {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        let today = crate::date::JournalDate::today().ordinal_key();
        {
            let g = self.derived_cache.read().unwrap();
            if let Some(dc) = g.as_ref() {
                if dc.gen == gen && dc.today == today {
                    if let Some(r) = dc.results.get(&key) {
                        return Arc::clone(r);
                    }
                }
            }
        }
        let result = Arc::new(compute());
        let mut g = self.derived_cache.write().unwrap();
        match g.as_mut() {
            Some(dc) if dc.gen == gen && dc.today == today => {
                dc.results.insert(key, Arc::clone(&result));
            }
            _ => {
                let mut results = std::collections::HashMap::new();
                results.insert(key, Arc::clone(&result));
                *g = Some(DerivedCache { gen, today, results });
            }
        }
        result
    }

    /// Backlinks for a page: blocks across the graph that reference it,
    /// grouped by source page. Delegates to the query module (memoized).
    pub fn backlinks(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("b\0{}", crate::refs::normalize(target)), || {
            crate::query::backlinks(self, target)
        })
    }

    /// Evaluate a `{{query ...}}` body over the graph (memoized).
    pub fn run_query(&self, query_src: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("q\0{query_src}"), || crate::query::run_query(self, query_src))
    }

    /// Evaluate an advanced (datalog-subset) query, returning the matched groups
    /// plus which clauses ran vs were ignored. Not memoized (invoked on demand).
    pub fn run_advanced_query(
        &self,
        query_src: &str,
        current_page: Option<&str>,
    ) -> crate::query::AdvancedResult {
        crate::query::run_advanced_query(self, query_src, current_page)
    }

    /// Unlinked references: plain-text mentions of a page that aren't links
    /// (memoized).
    pub fn unlinked_refs(&self, target: &str) -> Arc<Vec<RefGroup>> {
        self.derived_memo(format!("u\0{}", crate::refs::normalize(target)), || {
            crate::query::unlinked_refs(self, target)
        })
    }

    /// Export the whole graph to static HTML under `<root>/publish/`.
    pub fn publish_html(&self) -> io::Result<(String, usize)> {
        crate::publish::publish_graph(self)
    }

    /// Rename a page, OG-style. Moves its file to the new name and rewrites every
    /// reference across pages AND journals — inline `[[old]]`/`#old`, the page's
    /// OWN self/sibling refs, and bare `tags:: old` property refs — and CASCADES
    /// to the whole `old/*` namespace subtree (each `old/child` page moves to
    /// `new/child`, its refs rewritten), matching Logseq's `rename-namespace-pages!`.
    /// Journals can't be renamed (their name is their date). Transactional: locks
    /// every touched file, re-verifies each is unchanged since collection, commits,
    /// and rolls back every write on any failure. Aborts (no change) if a target
    /// name already exists or a touched file changed under us.
    pub fn rename_page(&self, old: &str, new: &str) -> io::Result<()> {
        let old = old.trim();
        let new = new.trim();
        if new.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty name"));
        }
        if old.is_empty() || old.eq_ignore_ascii_case(new) {
            return Ok(()); // nothing to do (case-only rename is intentionally a no-op)
        }
        // M1: refuse to rename an ambiguous page (both .md and .org on disk) — which
        // twin moves, and which content is authoritative, is undecidable here.
        if self.has_twin(old, PageKind::Page) || self.has_twin(new, PageKind::Page) {
            return Err(twin_error(old));
        }
        let old_n = crate::refs::normalize(old);
        let ns_prefix = format!("{old_n}/");
        let skip = old.chars().count();
        let entries = self.list_pages();

        // Phase 0a — the rename SET: the page itself plus every file-backed
        // namespace descendant (`old/*`). Each contributes a file move and an
        // (old_name -> new_name) ref-rewrite pair applied graph-wide. We only match
        // the exact name or the `old/` prefix (never a bare substring), so renaming
        // `work` -> `work1` turns `work/log` into `work1/log`, not `work1/work1log`.
        let mut rename_pairs: Vec<(String, String)> = Vec::new();
        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut primary_is_file = false;
        for entry in &entries {
            if entry.kind != PageKind::Page {
                continue; // journals aren't namespaced pages; their refs still get rewritten in 0b
            }
            let en = crate::refs::normalize(&entry.name);
            let is_primary = en == old_n;
            if !is_primary && !en.starts_with(&ns_prefix) {
                continue;
            }
            let new_name = if is_primary {
                new.to_string()
            } else {
                // replace the `old` prefix, preserving the descendant's own casing
                let suffix: String = entry.name.chars().skip(skip).collect();
                format!("{new}{suffix}")
            };
            // Keep the page's own format on rename (an .org page stays .org).
            let new_path = self.pages_path().join(format!(
                "{}.{}",
                encode_page_name(&new_name),
                Format::from_path(&entry.path).ext()
            ));
            if new_path != entry.path && new_path.exists() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "target page exists"));
            }
            if is_primary {
                primary_is_file = true;
            }
            rename_pairs.push((entry.name.clone(), new_name));
            moves.push((entry.path.clone(), new_path));
        }
        // A page can exist only via references (no file of its own); still rewrite
        // refs to it.
        if !primary_is_file {
            rename_pairs.push((old.to_string(), new.to_string()));
        }

        // Phase 0b — compute every file edit (inline refs + bare `tags::`), across
        // pages AND journals. A moved page's OWN content is rewritten too (self /
        // sibling refs) and lands at its new path.
        struct Edit {
            src: PathBuf,
            dst: PathBuf,
            orig: String,
            new_content: String,
            base_rev: String,
            is_move: bool,
        }
        let move_dst: std::collections::HashMap<PathBuf, PathBuf> = moves.into_iter().collect();
        let mut edits: Vec<Edit> = Vec::new();
        for entry in &entries {
            let Ok(content) = fs::read_to_string(&entry.path) else { continue };
            let is_org = Format::from_path(&entry.path) == Format::Org;
            let mut updated = content.clone();
            for (o, n) in &rename_pairs {
                updated = crate::refs::rename_refs(&updated, o, n, is_org);
                updated = crate::refs::rename_tags_property(&updated, o, n, is_org);
            }
            // H1: a rename must never rewrite a read-only (non-round-tripping) .org
            // file. Abort the whole rename (all-or-nothing) so the user resolves it
            // in Logseq first. A pure file move with no content change (updated ==
            // content) is still allowed — it preserves bytes exactly.
            if is_org && updated != content && !crate::org::org_editable(&content) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "cannot rename: {} is a read-only .org file (does not round-trip)",
                        entry.path.display()
                    ),
                ));
            }
            match move_dst.get(&entry.path) {
                Some(dst) => edits.push(Edit {
                    src: entry.path.clone(),
                    dst: dst.clone(),
                    base_rev: content_rev(&content),
                    orig: content,
                    new_content: updated,
                    is_move: true,
                }),
                None if updated != content => edits.push(Edit {
                    src: entry.path.clone(),
                    dst: entry.path.clone(),
                    base_rev: content_rev(&content),
                    orig: content,
                    new_content: updated,
                    is_move: false,
                }),
                None => {}
            }
        }
        if edits.is_empty() {
            return Ok(()); // page doesn't exist / nothing references it
        }

        // Phase 1 — lock every touched path (src + move dst), sorted + deduped
        // (deadlock-free against a single-page save, which only ever holds ONE lock).
        let mut lock_paths: Vec<PathBuf> = Vec::new();
        for e in &edits {
            lock_paths.push(e.src.clone());
            if e.is_move {
                lock_paths.push(e.dst.clone());
            }
        }
        lock_paths.sort();
        lock_paths.dedup();
        let locks: Vec<_> = lock_paths.iter().map(|p| self.page_lock(p)).collect();
        let _guards: Vec<_> = locks.iter().map(|l| l.lock().unwrap()).collect();

        // Phase 2 — re-verify nothing changed under us since Phase 0; abort (no
        // change) on any mismatch (an external editor / Syncthing pull landed).
        for e in &edits {
            if e.is_move && e.dst != e.src && e.dst.exists() {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "target page exists"));
            }
            if content_rev(&fs::read_to_string(&e.src).unwrap_or_default()) != e.base_rev {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
            }
        }

        // Phase 3 — commit, tracking writes for rollback. For a move, write the new
        // file BEFORE removing the old one, so a crash mid-rename duplicates a page
        // rather than losing it.
        let mut written: Vec<&Edit> = Vec::new();
        let result: io::Result<()> = (|| {
            for e in &edits {
                self.note_self_write(&e.dst, content_rev(&e.new_content));
                if e.is_move && e.dst != e.src {
                    if let Some(parent) = e.dst.parent() {
                        fs::create_dir_all(parent)?;
                    }
                }
                atomic_write(&e.dst, e.new_content.as_bytes())?;
                if e.is_move && e.dst != e.src {
                    fs::remove_file(&e.src)?;
                }
                written.push(e);
            }
            Ok(())
        })();
        if let Err(err) = result {
            // Roll back in reverse, and drop the self-write markers for bytes that
            // won't survive the rollback so they can't later suppress a real
            // external change (M1).
            for e in written.iter().rev() {
                if e.is_move && e.dst != e.src {
                    let _ = fs::remove_file(&e.dst);
                    self.recent_writes.lock().unwrap().remove(&e.dst);
                    self.note_self_write(&e.src, content_rev(&e.orig));
                    let _ = atomic_write(&e.src, e.orig.as_bytes());
                } else {
                    self.note_self_write(&e.dst, content_rev(&e.orig));
                    let _ = atomic_write(&e.dst, e.orig.as_bytes());
                }
            }
            self.invalidate_cache();
            return Err(err);
        }
        self.invalidate_cache();
        Ok(())
    }

    /// Delete a page/journal file. Rather than unlinking, the file is moved to a
    /// graph-local trash (`logseq/.tine-trash/`, outside journals//pages/ so it's
    /// never re-loaded) — so a delete that races an unseen external edit, or a
    /// simple misclick, is recoverable. Best-effort: falls back to removal if the
    /// trash move fails.
    pub fn delete_page(&self, name: &str, kind: PageKind) -> io::Result<()> {
        // M1: with both a .md and a .org twin, "which file?" is ambiguous — refuse
        // rather than trash an arbitrary one.
        if self.has_twin(name, kind) {
            return Err(twin_error(name));
        }
        if let Some(entry) = self.find_entry(name, kind) {
            let trash = self.root.join("logseq").join(".tine-trash");
            let fname = entry.path.file_name().and_then(|s| s.to_str()).unwrap_or("page.md");
            let dest = trash.join(format!("{}__{fname}", trash_stamp()));
            if fs::create_dir_all(&trash).is_err() || fs::rename(&entry.path, &dest).is_err() {
                fs::remove_file(&entry.path)?;
            }
        }
        self.cache_remove(name, kind);
        Ok(())
    }

    /// Full-text search across all blocks.
    pub fn search(&self, query: &str, limit: usize) -> Vec<RefGroup> {
        crate::query::search(self, query, limit)
    }

    /// Fuzzy page-name matches for the quick switcher.
    pub fn quick_switch(&self, query: &str, limit: usize) -> Vec<PageEntry> {
        crate::query::quick_switch(self, query, limit)
    }

    /// All `template:: <name>` templates across the graph, with the blocks to
    /// insert (ids and template properties stripped).
    pub fn templates(&self) -> Vec<TemplateDto> {
        crate::query::templates(self)
    }

    /// Resolve a `((uuid))` block reference.
    pub fn resolve_block(&self, uuid: &str) -> Option<RefGroup> {
        crate::query::resolve_block(self, uuid)
    }

    /// Resolve many block references in one call (for a page full of `((uuid))`
    /// refs / embeds) — one IPC instead of N. The uuid index is built once and
    /// reused across the batch.
    pub fn resolve_blocks(&self, uuids: &[String]) -> Vec<Option<RefGroup>> {
        uuids.iter().map(|u| crate::query::resolve_block(self, u)).collect()
    }

    /// The graph's `logseq/custom.css`, if present (for user theming).
    pub fn custom_css(&self) -> String {
        std::fs::read_to_string(self.root.join("logseq").join("custom.css")).unwrap_or_default()
    }

    /// Property keys (with their distinct values) used across the graph, for the
    /// query builder's property-filter autocomplete. Excludes internal/metadata
    /// properties (id, collapsed, hl-*, …).
    pub fn property_facets(&self) -> Vec<(String, Vec<String>)> {
        crate::query::property_facets(self)
    }

    // ---- Assets & PDF highlights ----

    pub fn assets_path(&self) -> PathBuf {
        self.root.join("assets")
    }

    /// Top-level `assets/` files that NO block references — orphans the user may
    /// want to trash. Tine never auto-deletes assets (a deleted block keeps its
    /// media as a safety net), so this is the discovery half of "find unused
    /// media". Conservative: scans every block's `raw` + page `pre_block` for any
    /// `assets/<name>` mention; skips subdirectories (PDF area-image stores) and
    /// `.edn`/dotfiles (sidecars, not media) so nothing in use is ever flagged.
    pub fn orphan_assets(&self) -> Vec<AssetInfo> {
        let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
        self.with_pages(|pages| {
            for (_e, doc) in pages {
                if let Some(pre) = &doc.pre_block {
                    collect_asset_refs(pre, &mut referenced);
                }
                for b in &doc.roots {
                    collect_block_asset_refs(b, &mut referenced);
                }
            }
        });
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(self.assets_path()) else { return out };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if !ft.is_file() {
                continue; // skip subdirs (PDF area-image stores, tied to a PDF)
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
            // Sidecars/hidden files aren't user media; never flag them as orphans.
            if name.starts_with('.') || name.ends_with(".edn") {
                continue;
            }
            if referenced.contains(name) {
                continue;
            }
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            out.push(AssetInfo { name: name.to_string(), size, modified });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Move an asset file to `logseq/.tine-trash` (recoverable), never a hard
    /// delete by default. Refuses any name with a path separator (top-level
    /// assets only) so it can't reach outside `assets/`.
    pub fn trash_asset(&self, name: &str) -> io::Result<()> {
        if name.is_empty() || name.contains('/') || name.contains('\\') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bad asset name"));
        }
        let src = self.assets_path().join(name);
        if !src.is_file() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such asset"));
        }
        let trash = self.root.join("logseq").join(".tine-trash");
        let dest = trash.join(format!("{}__{name}", trash_stamp()));
        if fs::create_dir_all(&trash).is_err() || fs::rename(&src, &dest).is_err() {
            fs::remove_file(&src)?;
        }
        Ok(())
    }

    /// File count + total bytes currently in the asset trash (`logseq/.tine-trash`).
    /// Lets the UI show "Empty trash (N)" without listing the directory itself.
    pub fn asset_trash_stats(&self) -> TrashStats {
        let trash = self.root.join("logseq").join(".tine-trash");
        let mut stats = TrashStats::default();
        if let Ok(rd) = fs::read_dir(&trash) {
            for entry in rd.flatten() {
                if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    stats.count += 1;
                    stats.bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        stats
    }

    /// Permanently delete everything in the asset trash. Returns the number of
    /// files removed. The trash lives under `logseq/.tine-trash`, so we only ever
    /// touch that directory — never `assets/`, `pages/`, or `journals/`.
    pub fn empty_asset_trash(&self) -> io::Result<u64> {
        let trash = self.root.join("logseq").join(".tine-trash");
        let mut removed = 0;
        match fs::read_dir(&trash) {
            Ok(rd) => {
                for entry in rd.flatten() {
                    let path = entry.path();
                    let ok = match entry.file_type() {
                        Ok(ft) if ft.is_dir() => fs::remove_dir_all(&path).is_ok(),
                        _ => fs::remove_file(&path).is_ok(),
                    };
                    if ok {
                        removed += 1;
                    }
                }
                Ok(removed)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Read raw bytes of an asset (e.g. a PDF) for the viewer.
    pub fn read_asset(&self, name: &str) -> io::Result<Vec<u8>> {
        fs::read(self.assets_path().join(name))
    }

    /// Write raw bytes (e.g. a pasted image) into `assets/`, returning the
    /// stored filename (de-duplicated if it already exists).
    pub fn save_asset(&self, name: &str, bytes: &[u8]) -> io::Result<String> {
        use std::io::Write;
        let assets = self.assets_path();
        fs::create_dir_all(&assets)?;
        let (final_name, mut file) = reserve_asset(&assets, name)?;
        file.write_all(bytes)?;
        Ok(final_name)
    }

    /// Copy a file into `assets/`, returning the stored filename. De-duplicates
    /// against existing assets (never overwrites one already referenced by notes).
    pub fn import_asset(&self, src: &Path, name: Option<&str>) -> io::Result<String> {
        // Desired stored name (a timestamped name from the frontend), else the
        // source basename. `reserve_asset` still dedups same-name collisions.
        let name = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => src
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad source filename"))?
                .to_string(),
        };
        let assets = self.assets_path();
        fs::create_dir_all(&assets)?;
        // Reserve the name (creating an empty placeholder so no concurrent writer
        // can claim it), then copy the source over our own placeholder.
        let (final_name, _file) = reserve_asset(&assets, &name)?;
        drop(_file);
        fs::copy(src, assets.join(&final_name))?;
        Ok(final_name)
    }

    /// Write a cropped area-highlight image to OG's file-graph layout:
    /// `assets/<key>/<page>_<id>_<stamp>.png` (`<stamp>` = the `js/Date.now()`
    /// epoch-ms integer also stored in the highlight's `:content {:image …}`).
    /// Returns the assets-relative path.
    ///
    /// **Non-dedup on purpose:** the filename IS the stable link from the `.edn`
    /// entry to the file, so a re-save must overwrite in place rather than rename
    /// on collision (which `reserve_asset` would do, breaking the link).
    pub fn write_pdf_area_image(
        &self,
        pdf_filename: &str,
        page: i64,
        id: &str,
        stamp: i64,
        bytes: &[u8],
    ) -> io::Result<String> {
        let key = crate::pdf::asset_key(pdf_filename);
        let dir = self.assets_path().join(&key);
        fs::create_dir_all(&dir)?;
        let name = format!("{page}_{id}_{stamp}.png");
        atomic_write(&dir.join(&name), bytes)?;
        Ok(format!("{key}/{name}"))
    }

    /// Read highlights for a PDF from `assets/<key>.edn`.
    ///
    /// If the OG-compatible key's file is absent but a file under Tine's old
    /// `legacy_asset_key` exists, read that instead (it is migrated forward to
    /// the new key on the next `write_highlights`). This keeps highlights made
    /// by pre-launch Tine builds from disappearing after the key change.
    pub fn read_highlights(&self, pdf_filename: &str) -> Vec<crate::pdf::Highlight> {
        let key = crate::pdf::asset_key(pdf_filename);
        let s = fs::read_to_string(self.assets_path().join(format!("{key}.edn")))
            .ok()
            .or_else(|| {
                let legacy = crate::pdf::legacy_asset_key(pdf_filename);
                (legacy != key)
                    .then(|| fs::read_to_string(self.assets_path().join(format!("{legacy}.edn"))).ok())
                    .flatten()
            });
        s.map(|s| crate::pdf::parse_highlights(&s)).unwrap_or_default()
    }

    /// Persist highlights: write `assets/<key>.edn` and the `hls__<key>` page.
    /// `base_ids` are the highlight ids the editor LOADED (its baseline) — used for
    /// a 3-way merge so a highlight the user deleted is honored while one added
    /// externally (e.g. by OG between load and write) is still preserved.
    pub fn write_highlights(
        &self,
        pdf_filename: &str,
        label: &str,
        highlights: &[crate::pdf::Highlight],
        base_ids: &[String],
    ) -> io::Result<()> {
        let key = crate::pdf::asset_key(pdf_filename);
        // Legacy (pre-launch) key. When it differs and only the legacy files
        // exist, we read those as the baseline and migrate them to the new key
        // below — so the key change never strands existing highlights.
        let legacy_key = crate::pdf::legacy_asset_key(pdf_filename);
        let legacy_edn = (legacy_key != key)
            .then(|| self.assets_path().join(format!("{legacy_key}.edn")));
        let legacy_page = (legacy_key != key)
            .then(|| self.pages_path().join(format!("{}.md", crate::pdf::hls_page_name(&legacy_key))));
        // Serialize against an editor save of the SAME `hls__` page (see
        // `page_locks`): hold the page lock across the .edn merge AND the page
        // read→merge→write→cache_upsert, so the two writers can't clobber each
        // other or trip a false self-write conflict.
        let page_path = self.pages_path().join(format!("{}.md", crate::pdf::hls_page_name(&key)));
        let lock = self.page_lock(&page_path);
        let _guard = lock.lock().unwrap();
        fs::create_dir_all(self.assets_path())?;
        let edn_path = self.assets_path().join(format!("{key}.edn"));
        // 3-way merge against the on-disk set: keep our current highlights, plus
        // any disk highlight that is an EXTERNAL addition (id not in our baseline
        // and not already present). A highlight we deliberately deleted (in the
        // baseline, absent from current) is NOT resurrected. Prefer the new-key
        // file; fall back to the legacy-key file (migrating it forward).
        let have: std::collections::HashSet<&str> = highlights.iter().map(|h| h.id.as_str()).collect();
        let base: std::collections::HashSet<&str> = base_ids.iter().map(|s| s.as_str()).collect();
        let mut merged: Vec<crate::pdf::Highlight> = highlights.to_vec();
        let existing_edn = fs::read_to_string(&edn_path)
            .ok()
            .or_else(|| legacy_edn.as_ref().and_then(|p| fs::read_to_string(p).ok()));
        if let Some(s) = &existing_edn {
            for h in crate::pdf::parse_highlights(s) {
                if !have.contains(h.id.as_str()) && !base.contains(h.id.as_str()) {
                    merged.push(h);
                }
            }
        }
        let edn = crate::pdf::write_highlights(&merged);
        atomic_write(&edn_path, edn.as_bytes())?;

        // Upsert into the existing hls page, preserving note children by id.
        // (`page_path` + its lock were taken at the top of this fn.) Prefer the
        // new-key page; fall back to the legacy-key page so its user notes are
        // carried over during migration.
        let existing_raw = fs::read_to_string(&page_path)
            .ok()
            .or_else(|| legacy_page.as_ref().and_then(|p| fs::read_to_string(p).ok()));
        let existing = existing_raw.as_deref().map(doc::parse);
        let page_doc = crate::pdf::merge_hls_page(existing.as_ref(), pdf_filename, label, &merged);
        let mut page_md = doc::serialize(&page_doc);
        // Preserve the notes page's CRLF endings (see write_page), so re-saving a
        // highlight doesn't flip a Windows-edited hls__ page to LF.
        if existing_raw.as_deref().is_some_and(|e| e.contains("\r\n")) && !page_md.contains('\r') {
            page_md = page_md.replace('\n', "\r\n");
        }
        fs::create_dir_all(self.pages_path())?;
        // The hls page is a normal watched page. Record this write so the file
        // watcher recognizes it as ours — otherwise saving a highlight trips a
        // false "changed on disk" on the notes page (see write_page /
        // sync_file_content). The .edn lives under assets/ which isn't watched.
        let page_rev = content_rev(&page_md);
        self.note_self_write(&page_path, page_rev.clone());
        atomic_write(&page_path, page_md.as_bytes())?;
        // The hls page is a real page; reflect it in the search cache.
        let name = crate::pdf::hls_page_name(&key);
        let entry = self.find_entry(&name, PageKind::Page).unwrap_or(PageEntry {
            name,
            kind: PageKind::Page,
            date_key: None,
            path: page_path.clone(),
        });
        self.cache_upsert(entry, page_doc, page_rev.clone());
        // Drop the self-write marker now the write is published (see write_page):
        // bound it to its write window so it can't linger and later suppress a real
        // external change. Remove only if it's still ours.
        {
            let mut recent = self.recent_writes.lock().unwrap();
            if recent.get(&page_path).is_some_and(|r| *r == page_rev) {
                recent.remove(&page_path);
            }
        }
        // Migrate-on-write: the new-key artifacts now durably carry everything
        // from the legacy-key files (highlights above, user notes via
        // `merge_hls_page`), so remove the stale legacy files to avoid leaving a
        // duplicate hls page. Best-effort — a failure just leaves a harmless
        // orphan, never lost data.
        if let Some(p) = &legacy_edn {
            let _ = fs::remove_file(p);
        }
        if let Some(p) = &legacy_page {
            if p.exists() {
                let _ = fs::remove_file(p);
                self.cache_remove(&crate::pdf::hls_page_name(&legacy_key), PageKind::Page);
            }
        }
        Ok(())
    }

    /// Map an on-disk `.md` path to its page entry (journal or page), or None if
    /// it isn't in the graph's journals/pages dirs.
    pub fn entry_for_path(&self, path: &Path) -> Option<PageEntry> {
        if !is_page_file(path) {
            return None;
        }
        let stem = path.file_stem().and_then(|s| s.to_str())?;
        let parent = path.parent()?;
        if parent == self.journals_path() {
            let (name, date_key) = match self.journal_format.parse(stem) {
                Some(d) => (self.journal_format.title(d), Some(d.ordinal_key())),
                None => (stem.to_string(), None),
            };
            Some(PageEntry { name, kind: PageKind::Journal, date_key, path: path.to_path_buf() })
        } else if parent == self.pages_path() {
            Some(PageEntry {
                name: decode_page_name(stem),
                kind: PageKind::Page,
                date_key: None,
                path: path.to_path_buf(),
            })
        } else {
            None
        }
    }

    /// Record that Tine just wrote content with rev `rev` to `path`, so the file
    /// watcher recognizes the write as ours (see `sync_file_content`). The map is
    /// consumed on first match; this hard cap is a backstop so a write that the
    /// watcher never observes (file deleted before the next poll, watcher idle)
    /// can't leak across a long session. Clearing only reopens the tiny
    /// rename→cache_upsert race for genuinely in-flight writes — harmless.
    fn note_self_write(&self, path: &Path, rev: String) {
        let mut recent = self.recent_writes.lock().unwrap();
        if recent.len() >= 1024 {
            recent.clear();
        }
        recent.insert(path.to_path_buf(), rev);
    }

    /// Reconcile a (possibly externally-changed) file with the in-memory cache.
    /// Returns the entry only if its parsed content actually differs from the
    /// cache (i.e. a real external change) — Tine's own writes keep the cache in
    /// sync, so they return None. No-op if the cache hasn't been built yet.
    pub fn sync_file(&self, path: &Path) -> Option<PageEntry> {
        let content = fs::read_to_string(path).ok()?;
        // The watcher consumes the self-write marker (one-shot) so the map stays
        // bounded to in-flight writes.
        self.sync_file_content(path, &content, true)
    }

    /// Reconcile the cache for `path` given its already-read `content` — so a
    /// caller that has just read the file (e.g. load_page) doesn't read it twice.
    /// `consume_self_write`: whether a match on the self-write marker REMOVES it.
    /// The watcher passes true (bounding); load_page passes false — load_page can
    /// run in the rename→cache_upsert window and must not steal the marker out
    /// from under the watcher, which would turn the watcher's later poll into a
    /// false "changed on disk".
    fn sync_file_content(&self, path: &Path, content: &str, consume_self_write: bool) -> Option<PageEntry> {
        let entry = self.entry_for_path(path)?;
        // Our own write: if the bytes on disk are exactly what Tine last wrote
        // here, this is not an external change — suppress it even if the parse
        // cache hasn't folded in the write yet (the rename→cache_upsert gap the
        // watcher can read into). The cache comparison below alone races that gap.
        let disk_rev = content_rev(content);
        // The self-write marker exists ONLY to stop the WATCHER raising a false
        // "changed on disk" during the rename→cache_upsert window, so only the
        // watcher (consume_self_write) consults it. load_page must NOT short-
        // circuit here: in that window the cache is still pre-write, so returning
        // early would serve a STALE cached doc with the fresh disk rev (a later
        // save could then clobber disk). load_page instead falls through to the
        // disk_revs fast path / parse-reconcile below and serves content matching
        // the exact bytes it just read.
        if consume_self_write {
            let mut recent = self.recent_writes.lock().unwrap();
            if recent.get(path).is_some_and(|r| *r == disk_rev) {
                recent.remove(path);
                return None;
            }
        }
        // Fast freshness check (B1): if the cache for this page already reflects
        // these exact disk bytes, there's nothing to reconcile — skip the parse +
        // serialize→parse normalization comparison below. Read disk_revs WHILE
        // HOLDING cache.read(): cache_upsert publishes the cache slot and its rev
        // together under cache.write(), so taking the cache read lock here makes
        // the reader mutually exclusive with that writer and guarantees a
        // consistent (cache, rev) pair — without it, the reader could observe a
        // slot already updated to a new doc while its rev hadn't been inserted yet
        // (separate lock), match the stale rev against disk, and serve the wrong
        // doc. Lock order cache → disk_revs matches every writer, so no deadlock;
        // the guard is dropped before the reconcile path below re-locks the cache.
        // A missing/mismatched entry falls through to the exact comparison, so this
        // can only ever save work, never serve stale content.
        {
            let _cache_guard = self.cache.read().unwrap();
            if self
                .disk_revs
                .read()
                .unwrap()
                .get(&rev_key(entry.kind, &entry.name))
                .is_some_and(|r| *r == disk_rev)
            {
                return None;
            }
        }
        let newdoc = parse_doc(path, content);
        {
            let guard = self.cache.read().unwrap();
            let Some(cache) = guard.as_ref() else {
                // Cache not built yet: nothing to reconcile in the parse cache, but
                // the page SET may have changed (this could be a newly-created
                // file). Drop the page-list memo so list_pages — and the eventual
                // warm build that reads it — re-scan the dir and include it.
                drop(guard);
                *self.page_list_cache.write().unwrap() = None;
                return None;
            };
            if let Some((_, cached)) = cache
                .iter()
                .find(|(e, _)| e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name))
            {
                // Compare CONTENT, not the in-memory uuids: cached blocks carry
                // generated uuids (assigned at cache build / upsert), while a
                // fresh `parse` leaves them empty for non-ref-target blocks, so a
                // direct `cached == newdoc` would never match and would flag every
                // one of Tine's own writes as an external change. Normalize the
                // cached doc through the same serialize→parse round-trip the file
                // went through (both sides then have empty uuids) and compare.
                let cached_norm = match Format::from_path(path) {
                    Format::Md => {
                        let opts = doc::SerializeOpts::detect(Some(content));
                        doc::parse(&doc::serialize_with(cached, &opts))
                    }
                    Format::Org => crate::org::parse_org(&crate::org::serialize_org_detect(
                        cached,
                        Some(content),
                    )),
                };
                if cached_norm == newdoc {
                    return None; // unchanged / our own write
                }
            }
        }
        self.cache_upsert(entry.clone(), newdoc, disk_rev);
        Some(entry)
    }

    /// Drop a file deleted on disk from the cache; returns the entry if it was
    /// cached (so the UI can react).
    pub fn forget_file(&self, path: &Path) -> Option<PageEntry> {
        let entry = self.entry_for_path(path)?;
        let was_cached = {
            let guard = self.cache.read().unwrap();
            guard.as_ref().is_some_and(|c| {
                c.iter().any(|(e, _)| e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name))
            })
        };
        self.cache_remove(&entry.name, entry.kind);
        was_cached.then_some(entry)
    }

    /// Save a page, refusing to clobber an external change. If the file on disk
    /// no longer matches what Tine last knew (another app or a Syncthing pull
    /// wrote it), returns an `AlreadyExists` "conflict" error WITHOUT writing,
    /// so the caller can surface it and keep the in-memory edits.
    pub fn save_page(&self, page: &PageDto, base_rev: Option<&str>) -> io::Result<String> {
        // M1: refuse to write an ambiguous page (both .md and .org on disk) — we
        // can't tell which file the editor's content belongs to.
        if self.has_twin(&page.name, page.kind) {
            return Err(twin_error(&page.name));
        }
        let path = self.path_for(&page.name, page.kind);
        // Serialize against any other writer of THIS page (a PDF highlight write
        // of the same `hls__` page, or another save) for the whole
        // read→conflict-check→write→cache_upsert, so neither can clobber the other
        // or steal its self-write marker (see `page_locks`).
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        // Single read of the current file (the conflict baseline AND the
        // formatting source AND, with the written content, the returned rev) —
        // avoids re-reading the file 2-3× per save, which is felt on NFS.
        let existing: Option<String> = match fs::read_to_string(&path) {
            Ok(disk_s) => {
                // The file must still match the exact bytes the editor loaded
                // (`base_rev`); if it changed underneath us (external edit /
                // Syncthing pull), refuse to clobber. `base_rev == None` means the
                // editor believed the page was new, so any existing file is an
                // external creation → conflict.
                if !base_rev.is_some_and(|rev| content_rev(&disk_s) == rev) {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
                Some(disk_s)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // The file is gone. If the editor had a baseline (page existed at
                // load), it was deleted externally — DON'T silently resurrect it.
                if base_rev.is_some() {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
                None
            }
            Err(e) => return Err(e), // hard error (permission, I/O) — don't write blind
        };
        // recheck = true: re-verify the file hasn't changed on disk in the instant
        // before the write, to narrow the inherent race against a NON-cooperating
        // external writer (OG/Syncthing) that doesn't take our page lock.
        // M2: write to the SAME path we locked + read the baseline from — never
        // re-resolve `path_for` under the lock (an `exists()`-probe could otherwise
        // pick a different extension if a twin appears mid-save).
        self.write_page(page, &path, existing.as_deref(), true)
    }

    /// Save a page unconditionally (the user chose "keep mine" over a conflict).
    pub fn force_save_page(&self, page: &PageDto) -> io::Result<String> {
        if self.has_twin(&page.name, page.kind) {
            return Err(twin_error(&page.name)); // M1: ambiguous identity — refuse
        }
        let path = self.path_for(&page.name, page.kind);
        let lock = self.page_lock(&path);
        let _guard = lock.lock().unwrap();
        let existing = fs::read_to_string(&path).ok();
        // recheck = false: "keep mine" overwrites unconditionally. Same locked path
        // is threaded into write_page (M2) so a forced save can't land on a twin.
        self.write_page(page, &path, existing.as_deref(), false)
    }

    /// Write a page to `path` (already resolved + locked by the caller), reproducing
    /// `existing`'s formatting, and return the new on-disk content rev (computed from
    /// what was written — no extra read).
    fn write_page(
        &self,
        page: &PageDto,
        path: &Path,
        existing: Option<&str>,
        recheck: bool,
    ) -> io::Result<String> {
        // (A new journal's `path` was named by `path_for` using the graph's
        // `:journal/file-name-format` — so custom-format graphs create the correct
        // file for the day instead of a misplaced default-named duplicate.)
        let doc = Document {
            pre_block: page.pre_block.clone(),
            roots: page.blocks.iter().map(dto_to_doc).collect(),
        };
        // Own the caller's resolved+locked path (M2: never re-resolve path_for here).
        let path = path.to_path_buf();
        let content = match Format::from_path(&path) {
            Format::Md => {
                // Reproduce the existing file's formatting (trailing newline,
                // post-property blank line, indent) so an unchanged save is
                // byte-identical and edits produce a minimal diff — critical to
                // avoid Syncthing churn against Logseq.
                let opts = doc::SerializeOpts::detect(existing);
                let mut content = doc::serialize_with(&doc, &opts);
                // A5: if the ONLY difference from disk is whitespace trivia the
                // serializer doesn't round-trip byte-exactly (post-property
                // blank-line count, empty-bullet spelling `- ` vs `-`, indented
                // blank continuation lines), keep the existing bytes verbatim.
                // `doc::parse` collapses exactly this trivia (and ignores uuids),
                // so equal parses ⟹ the user changed nothing of substance → don't
                // rewrite (avoids needless Syncthing churn). Adopting the disk
                // bytes makes `changed` below false and the returned rev the
                // on-disk rev — so every downstream path stays correct.
                if let Some(e) = existing {
                    if e != content && doc::parse(e) == doc::parse(&content) {
                        content = e.to_string();
                    }
                }
                // CRLF preservation: if the existing file uses Windows line
                // endings, emit them too — so a real edit produces a minimal diff
                // instead of flipping every line LF (Syncthing churn vs a Windows
                // editor). No-op saves already kept the existing bytes verbatim
                // (A5 above), so guard against double-converting. New files = LF.
                if existing.is_some_and(|e| e.contains("\r\n")) && !content.contains('\r') {
                    content = content.replace('\n', "\r\n");
                }
                content
            }
            Format::Org => {
                // Corruption firewall: never write a .org file Tine cannot
                // reproduce byte-for-byte. Such a page is served read-only (the
                // editor blocks edits), but defend the write path too — a stale
                // editor or a direct save must not rewrite it. The org serializer
                // is itself byte-exact (no trivia dance / CRLF rewrite needed):
                // the block bodies carry their verbatim text, including any `\r`.
                if let Some(e) = existing {
                    if !crate::org::org_editable(e) {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "org file is read-only (does not round-trip)",
                        ));
                    }
                }
                crate::org::serialize_org_detect(&doc, existing)
            }
        };
        let rev = content_rev(&content);
        // No-op save: identical bytes already on disk (e.g. focus/blur with no real
        // edit, or a forced flush of an unchanged page). Skip the write, the
        // watcher record, AND — crucially — the cache update below.
        let changed = existing != Some(content.as_str());
        if changed {
            // Record what we're about to write BEFORE it lands, so the file watcher
            // (which reads files outside the cache lock) recognizes these exact
            // bytes as our own write during the window between the atomic rename
            // and the `cache_upsert` below — otherwise it reads disk-ahead-of-cache
            // and flags Tine's own save as an external change (false conflict).
            self.note_self_write(&path, rev.clone());
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            // A3: last-moment recheck. We hold this page's lock, so no other Tine
            // writer can have touched the file since our baseline read — but a
            // non-cooperating external writer (OG/Syncthing) might have. Re-read and
            // confirm the file still matches the bytes we were authorized against
            // (`existing`); if it changed, abort WITHOUT writing and drop our marker
            // so the watcher still sees the external change. `force_save_page` skips
            // this (recheck=false) so "keep mine" overwrites unconditionally.
            if recheck {
                let now = fs::read_to_string(&path).ok();
                let still_matches = match (now.as_deref(), existing) {
                    (Some(n), Some(e)) => n == e,
                    (None, None) => true,
                    _ => false,
                };
                if !still_matches {
                    let mut recent = self.recent_writes.lock().unwrap();
                    if recent.get(&path).is_some_and(|r| *r == rev) {
                        recent.remove(&path);
                    }
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
                }
            }
            atomic_write(&path, content.as_bytes())?;
        }
        // Touch the cache only when the bytes changed, or the page isn't in an
        // already-built cache yet (fold a cold page in). A no-op save of an
        // already-cached page MUST NOT call cache_upsert: it bumps `cache_gen`,
        // which keys every memoized query/backlink/derived result — so an unchanged
        // re-save would force a whole-graph requery on every open dashboard.
        let need_cache_update = changed || {
            let guard = self.cache.read().unwrap();
            guard.as_ref().is_some_and(|pages| {
                !pages
                    .iter()
                    .any(|(e, _)| e.kind == page.kind && e.name.eq_ignore_ascii_case(&page.name))
            })
        };
        if need_cache_update {
            // For a brand-new journal, derive its date_key from the name so it's
            // recognized as a dated journal by `journals_desc` (which reads this
            // cache) — otherwise today's freshly-created page would be missing.
            let entry = self.find_entry(&page.name, page.kind).unwrap_or_else(|| {
                let date_key = if page.kind == PageKind::Journal {
                    crate::date::JournalDate::from_title(&page.name).map(|d| d.ordinal_key())
                } else {
                    None
                };
                PageEntry { name: page.name.clone(), kind: page.kind, date_key, path: path.clone() }
            });
            // H4: for org, the on-disk bytes are authoritative. If the user typed a
            // structural marker (a column-0 `* ` line, or an unbalanced #+BEGIN_)
            // into a block body, `content` re-parses to a DIFFERENT tree than the
            // frontend `doc` — cache what's actually on disk so the next load shows
            // the real structure instead of a cache that silently disagrees. Common
            // case: structures match → keep `doc` (block uuids stay stable). Markdown
            // continuation lines are indented, so they can't re-read differently.
            let cache_doc = if Format::from_path(&path) == Format::Org {
                let reparsed = crate::org::parse_org(&content);
                if reparsed == doc {
                    doc
                } else {
                    let mut d = reparsed;
                    assign_doc_uuids(&mut d.roots);
                    d
                }
            } else {
                doc
            };
            self.cache_upsert(entry, cache_doc, rev.clone());
        }
        // Drop the self-write marker now that the write is published. It only had
        // to cover the window between the atomic write above and the cache_upsert
        // just done; disk_revs now suppresses the watcher. Removing it here (rather
        // than leaving it for the watcher to consume) bounds the marker to its
        // write window, so it can never outlive this save and later suppress a real
        // external change that happens to restore these exact bytes (e.g. a
        // delete+recreate, or a cold-cache save). Remove only if it's still OURS — a
        // concurrent same-path writer may have replaced it.
        if changed {
            let mut recent = self.recent_writes.lock().unwrap();
            if recent.get(&path).is_some_and(|r| *r == rev) {
                recent.remove(&path);
            }
        }
        // The new baseline rev = hash of exactly what's now on disk (the content we
        // serialized, or the identical existing bytes on a no-op) — no re-read.
        Ok(rev)
    }
}

/// Atomically reserve a unique filename in `assets/` for `name`, de-duplicating
/// against existing files by appending `_1`, `_2`, … to the stem. Unlike a plain
/// `exists()` check followed by a write, this CREATES the file exclusively
/// (`create_new`), so a concurrent writer (OG Logseq, or another asset op) that
/// races between the name check and our write can't claim the same name and get
/// silently overwritten — whoever loses the create retries the next candidate.
/// Returns the chosen name and the open (empty) file handle.
fn reserve_asset(assets: &Path, name: &str) -> io::Result<(String, fs::File)> {
    let create_new = |n: &str| {
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(assets.join(n))
    };
    match create_new(name) {
        Ok(f) => return Ok((name.to_string(), f)),
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
        _ => {}
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    let mut i = 1;
    loop {
        let candidate = format!("{stem}_{i}{ext}");
        match create_new(&candidate) {
            Ok(f) => return Ok((candidate, f)),
            Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
            _ => i += 1,
        }
    }
}

fn list_md(dir: &Path, kind: PageKind, fmt: &JournalFormat) -> Vec<PageEntry> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let path = entry.path();
        if !is_page_file(&path) {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let (name, date_key) = match kind {
            PageKind::Journal => match fmt.parse(stem) {
                Some(d) => (fmt.title(d), Some(d.ordinal_key())),
                None => (stem.to_string(), None),
            },
            PageKind::Page => (decode_page_name(stem), None),
        };
        out.push(PageEntry { name, kind, date_key, path });
    }
    out
}

/// True if any block in the subtree has a non-empty line that isn't a `key::`
/// property line — i.e. the page is more than an empty/placeholder bullet.
fn doc_has_content(blocks: &[DocBlock]) -> bool {
    blocks.iter().any(|b| {
        b.raw
            .lines()
            .any(|l| !l.trim().is_empty() && crate::doc::parse_property_line(l).is_none())
            || doc_has_content(&b.children)
    })
}

/// Assign uuids across a whole document's roots, sharing ONE `seen` set so a
/// persisted `id::` duplicated across blocks (copy-paste of raw text, or a sync
/// conflict) doesn't make two nodes share a uuid. The first occurrence keeps the
/// id (so `((id))` refs still resolve); later duplicates get a fresh internal
/// uuid. The raw `id::` line is left untouched — we only fix in-memory identity,
/// which otherwise collides in the frontend's global byId map and can duplicate
/// or drop a block's content on save.
pub fn assign_doc_uuids(roots: &mut [DocBlock]) {
    let mut seen = std::collections::HashSet::new();
    for b in roots {
        assign_uuids_rec(b, &mut seen);
    }
}

fn assign_uuids_rec(b: &mut DocBlock, seen: &mut std::collections::HashSet<String>) {
    if b.uuid.is_empty() {
        b.uuid = match b.property("id") {
            // Reuse the persisted id only if no earlier block already claimed it.
            Some(id) if !id.is_empty() && !seen.contains(&id) => id,
            _ => Uuid::new_v4().to_string(),
        };
    }
    seen.insert(b.uuid.clone());
    for c in &mut b.children {
        assign_uuids_rec(c, seen);
    }
}

/// Convert a parsed (cached) block to a DTO, carrying its stable uuid as the id.
pub fn block_to_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: if b.uuid.is_empty() { Uuid::new_v4().to_string() } else { b.uuid.clone() },
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: b.children.iter().map(block_to_dto).collect(),
        breadcrumb: Vec::new(),
    }
}

/// Convert a frontend DTO subtree back to a doc block, preserving the frontend's
/// block id as the node uuid so the cache and the frontend agree on identity.
fn dto_to_doc(b: &BlockDto) -> DocBlock {
    DocBlock {
        raw: b.raw.clone(),
        children: b.children.iter().map(dto_to_doc).collect(),
        uuid: b.id.clone(),
        proj: std::sync::OnceLock::new(),
    }
}

/// Build a page DTO from a cached document. `read_only` is left false here (the
/// on-disk bytes aren't known at this point); `load_page` sets it from the file
/// it reads.
fn page_dto(entry: &PageEntry, doc: &Document) -> PageDto {
    PageDto {
        name: entry.name.clone(),
        kind: entry.kind,
        title: entry.name.clone(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
        format: Format::from_path(&entry.path),
        read_only: false,
    }
}

/// Whether a page should load read-only: an org file whose on-disk bytes don't
/// round-trip through Tine's org parser/serializer, so Tine must never rewrite
/// it (lest it corrupt the user's graph). Markdown pages are always editable.
fn read_only_org(path: &Path, content: &str) -> bool {
    Format::from_path(path) == Format::Org && !crate::org::org_editable(content)
}

/// Whether a document declares an `alias::` anywhere — used to decide if a save
/// must invalidate the cached alias map (most saves don't touch aliases).
fn has_alias_prop(text: &str) -> bool {
    // Case-insensitive (Logseq accepts `Alias::`); ASCII-lowercase is enough and
    // cheaper than full to_lowercase. Over-matching content is harmless (it just
    // invalidates the alias cache slightly more often); MISSING `Alias::` would
    // leave it stale, which is the bug we're avoiding.
    text.to_ascii_lowercase().contains("alias::")
}
fn doc_has_alias(doc: &Document) -> bool {
    doc.pre_block.as_deref().is_some_and(has_alias_prop) || doc.roots.iter().any(block_has_alias)
}
fn block_has_alias(b: &DocBlock) -> bool {
    has_alias_prop(&b.raw) || b.children.iter().any(block_has_alias)
}

/// Stable (deterministic, seed-free) content hash — FNV-1a/64 as hex. Used as a
/// per-load baseline so a save can detect that the file changed underneath the
/// editor. Deterministic so a rev returned from one save matches the next read.
pub fn content_rev(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Logseq encodes some characters in page filenames. We handle the common
/// namespace separator (`/` <-> `___`, the `:triple-lowbar` default).
fn encode_page_name(name: &str) -> String {
    name.replace('/', "___")
}

fn decode_page_name(stem: &str) -> String {
    stem.replace("___", "/")
}

/// A unique-ish label (epoch millis + process-local sequence) for trashed files,
/// so deleting two pages with the same name doesn't collide in the trash.
/// Collect every `assets/<name>` reference in `text` into `into`. Captures both
/// markdown (`![](../assets/x.png)`, `[f](../assets/x.pdf)`) and org
/// (`[[file:../assets/x.png]]`) forms — the name runs from after `assets/` to the
/// next markup closer (`)`/`]`/quote/etc.) or line break. Crucially it does NOT
/// stop at a space, so a referenced filename containing spaces is matched in full
/// (mis-truncating it would make `orphan_assets` flag a file that IS in use). The
/// first path segment is added too, so a PDF area-image ref (`assets/<key>/p.png`)
/// marks `<key>` as in use.
fn collect_asset_refs(text: &str, into: &mut std::collections::HashSet<String>) {
    let mut rest = text;
    while let Some(i) = rest.find("assets/") {
        let after = &rest[i + "assets/".len()..];
        let end = after
            .find(|c: char| matches!(c, ')' | ']' | '"' | '\'' | '<' | '>' | '|' | '\n' | '\r' | '\t'))
            .unwrap_or(after.len());
        let name = &after[..end];
        if !name.is_empty() {
            into.insert(name.to_string());
            if let Some(seg) = name.split('/').next() {
                if seg != name {
                    into.insert(seg.to_string());
                }
            }
        }
        rest = &after[end..];
    }
}

fn collect_block_asset_refs(b: &DocBlock, into: &mut std::collections::HashSet<String>) {
    collect_asset_refs(&b.raw, into);
    for c in &b.children {
        collect_block_asset_refs(c, into);
    }
}

fn trash_stamp() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Atomic write: write to a temp file in the same directory, then rename. The
/// temp name is unique per write (pid + sequence) so two concurrent writers to
/// the same path (e.g. an autosave and a highlight/rename rewrite) can't truncate
/// each other's temp; the rename is still atomic. The temp is removed if the
/// write fails, so a unique name never leaks an orphan behind.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("page");
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{fname}.{}.{seq}.tmp", std::process::id()));
    let res = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp); // never leave a temp behind on failure
    } else {
        // Persist the rename itself: fsync the directory so a crash right after the
        // write can't lose the new directory entry (the rename) on some
        // filesystems. Best-effort — not all platforms allow fsync on a dir.
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assign_doc_uuids_dedups_duplicate_ids() {
        // Two blocks persisting the SAME id:: (copy-paste of raw text, or a sync
        // conflict) must NOT end up sharing a uuid — that collides in the
        // frontend's global byId map and duplicates/drops content on save.
        let mut roots = vec![
            DocBlock::new("first\nid:: dup-1234"),
            DocBlock::new("second\nid:: dup-1234"),
        ];
        assign_doc_uuids(&mut roots);
        assert_eq!(roots[0].uuid, "dup-1234", "first occurrence keeps the id");
        assert_ne!(roots[1].uuid, "dup-1234", "duplicate gets a fresh uuid");
        assert!(!roots[1].uuid.is_empty());

        // A nested duplicate is caught too.
        let mut parent = DocBlock::new("p\nid:: x");
        parent.children.push(DocBlock::new("c\nid:: x"));
        assign_doc_uuids(std::slice::from_mut(&mut parent));
        assert_eq!(parent.uuid, "x");
        assert_ne!(parent.children[0].uuid, "x");
    }

    #[test]
    fn reserve_asset_avoids_overwrite() {
        let dir = std::env::temp_dir().join(format!("tine-asset-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Each reserve CREATES the file (exclusively), so the next reserve of the
        // same name is forced onto a fresh suffix — no manual writes needed, and a
        // racing writer can never be handed an already-taken name.
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper.pdf");
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_1.pdf");
        assert_eq!(reserve_asset(&dir, "paper.pdf").unwrap().0, "paper_2.pdf");
        // Extensionless names work too.
        assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES");
        assert_eq!(reserve_asset(&dir, "NOTES").unwrap().0, "NOTES_1");
        // Every reserved name is a real, distinct file on disk.
        for n in ["paper.pdf", "paper_1.pdf", "paper_2.pdf", "NOTES", "NOTES_1"] {
            assert!(dir.join(n).exists(), "{n} reserved");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn quick_switch_includes_referenced_pages() {
        // A page referenced by `#tag` / `[[link]]` but with no file of its own
        // still "exists" (OG semantics) and must show up in quick-switch — that's
        // what lets `#`/`[[ ]]` autocomplete say "#thistag" rather than a
        // misleading "Create #thistag" when the tag is already used elsewhere.
        let dir = std::env::temp_dir().join(format!("tine-refpages-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        fs::write(
            dir.join("pages").join("notes.md"),
            "- uses #thistag and [[Some Page]]\n",
        )
        .unwrap();
        // A page whose page-properties carry tags::/alias:: (OG autolinks these as
        // page references, bare or bracketed).
        fs::write(
            dir.join("pages").join("paper.md"),
            "tags:: ProjectX, [[Linear IP]]\nalias:: LP Survey\n- body\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        g.warm_cache(); // referenced names come from the whole-graph cache

        let has = |q: &str, name: &str| {
            g.quick_switch(q, 8).iter().any(|e| e.name.eq_ignore_ascii_case(name))
        };
        assert!(has("thistag", "thistag"), "referenced #thistag should appear");
        assert!(has("some page", "Some Page"), "referenced [[Some Page]] should appear");
        // tags:: values (bare and bracketed) and alias:: values count too.
        assert!(has("projectx", "ProjectX"), "bare tags:: value should appear");
        assert!(has("linear ip", "Linear IP"), "bracketed tags:: value should appear");
        assert!(has("lp survey", "LP Survey"), "alias:: value should appear");
        // Neither filed nor referenced → not offered (so autocomplete still says
        // "Create" for a genuinely new name).
        assert!(!has("nonexistent", "nonexistent"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tine-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::create_dir_all(dir.join("pages")).unwrap();
        dir
    }

    #[test]
    fn migrate_recovers_title_named_org_journals() {
        // Regression: changing :journal/page-title-format while a stale in-memory
        // format was still active saved new journals under their title
        // ("Thursday, 25-06-2026.org") instead of the date stem, so they dropped
        // out of the feed. A reopen + migrate (now .org-aware) must recover them.
        let dir = scratch("journal-migrate-org");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(dir.join("journals").join("Thursday, 25-06-2026.org"), "* bla\n").unwrap();
        // A canonical file for another day must be left untouched.
        fs::write(dir.join("journals").join("2026_06_24.org"), "* prior\n").unwrap();

        let g = Graph::open(&dir);
        assert_eq!(g.migrate_journal_filenames(), 1, "exactly the title-named file renamed");
        assert!(dir.join("journals").join("2026_06_25.org").exists(), "renamed to date stem");
        assert!(!dir.join("journals").join("Thursday, 25-06-2026.org").exists(), "old name gone");
        assert!(dir.join("journals").join("2026_06_24.org").exists(), "canonical file untouched");

        // It's now recognized in the feed listing (name via the title format).
        let names: Vec<String> = Graph::open(&dir).journals_desc().into_iter().map(|e| e.name).collect();
        assert!(names.iter().any(|n| n == "Thursday, 25-06-2026"), "listed: {names:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journal_conflicts_reports_duplicate_days() {
        let dir = scratch("journal-conflicts");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        // Same day, two files (canonical stem + title-named) — a conflict.
        fs::write(dir.join("journals").join("2026_06_26.org"), "* canonical content\n").unwrap();
        fs::write(dir.join("journals").join("Friday, 26-06-2026.org"), "* stray content\n").unwrap();
        // A clean day with one file — not a conflict.
        fs::write(dir.join("journals").join("2026_06_24.org"), "* fine\n").unwrap();

        let conflicts = Graph::open(&dir).journal_conflicts();
        assert_eq!(conflicts.len(), 1, "exactly one conflicted day: {conflicts:?}");
        let c = &conflicts[0];
        assert_eq!(c.title, "Friday, 26-06-2026");
        assert_eq!(c.files.len(), 2);
        // Canonical (date-stem) file sorts first and is flagged; preview is the body line.
        assert_eq!(c.files[0].name, "2026_06_26.org");
        assert!(c.files[0].canonical);
        assert_eq!(c.files[0].preview, "canonical content");
        assert!(!c.files[1].canonical);
        assert_eq!(c.files[1].name, "Friday, 26-06-2026.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn journals_desc_dedups_duplicate_day_to_canonical() {
        // The feed must show a day ONCE even when two files resolve to it — else
        // the same day renders twice (loaded from whichever file path_for picks).
        let dir = scratch("journal-dedup");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:preferred-format \"Org\"\n :journal/page-title-format \"EEEE, dd-MM-yyyy\"}\n",
        )
        .unwrap();
        fs::write(dir.join("journals").join("2026_06_26.org"), "* real day\n").unwrap();
        fs::write(dir.join("journals").join("Friday, 26-06-2026.org"), "* stray\n").unwrap();
        fs::write(dir.join("journals").join("2026_06_24.org"), "* other day\n").unwrap();

        let js = Graph::open(&dir).journals_desc();
        assert_eq!(js.len(), 2, "one entry per day: {:?}", js.iter().map(|e| &e.name).collect::<Vec<_>>());
        // The deduped 26th keeps the canonical date-stem file (what saves resolve to).
        let day26 = js.iter().find(|e| e.name == "Friday, 26-06-2026").expect("26th present");
        assert_eq!(day26.path.file_name().unwrap().to_str().unwrap(), "2026_06_26.org");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_transaction_moves_file_and_rewrites_refs() {
        let dir = scratch("rename");
        fs::write(dir.join("pages").join("Alpha.md"), "- alpha body\n").unwrap();
        fs::write(dir.join("pages").join("Other.md"), "- see [[Alpha]] here\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_page("Alpha", "Beta").unwrap();
        // The page file moved (content preserved) and the old file is gone.
        assert!(!dir.join("pages").join("Alpha.md").exists());
        assert_eq!(fs::read_to_string(dir.join("pages").join("Beta.md")).unwrap(), "- alpha body\n");
        // Every reference was rewritten across the graph.
        let other = fs::read_to_string(dir.join("pages").join("Other.md")).unwrap();
        assert!(other.contains("[[Beta]]"), "ref rewritten to [[Beta]]");
        assert!(!other.contains("[[Alpha]]"), "no stale [[Alpha]] left");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_page_lists_loads_edits_and_round_trips() {
        let dir = scratch("org-page");
        let src = "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block\n";
        fs::write(dir.join("pages").join("Org Notes.org"), src).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();

        // Listed, recognized as an org page.
        let entry = g
            .list_pages()
            .into_iter()
            .find(|e| e.name == "Org Notes")
            .expect("org page listed");
        assert_eq!(Format::from_path(&entry.path), Format::Org);

        // Loaded: format=org, editable, headlines decomposed into blocks.
        let dto = g.load_named("Org Notes", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.format, Format::Org);
        assert!(!dto.read_only);
        assert_eq!(dto.blocks.len(), 2);
        assert_eq!(dto.blocks[0].raw, "TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>");
        assert_eq!(dto.blocks[1].raw, "second block");

        // No-op save leaves the file byte-identical (no churn).
        let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap(), src);

        // Edit a block and save → file updated, still org, byte-faithful.
        let mut edited = dto.clone();
        edited.blocks[1].raw = "second block edited".into();
        g.save_page(&edited, Some(&rev)).unwrap();
        let on_disk = fs::read_to_string(dir.join("pages").join("Org Notes.org")).unwrap();
        assert_eq!(
            on_disk,
            "* TODO Buy milk\nSCHEDULED: <2026-06-25 Thu>\n* second block edited\n"
        );
        // No stray .md twin was created.
        assert!(!dir.join("pages").join("Org Notes.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_journal_recognized_and_listed() {
        let dir = scratch("org-journal");
        fs::write(dir.join("journals").join("2026_06_24.org"), "* woke up\n* TODO ship\n").unwrap();
        let g = Graph::open(&dir);
        let j = g
            .journals_desc()
            .into_iter()
            .find(|e| e.kind == PageKind::Journal)
            .expect("org journal listed");
        assert_eq!(Format::from_path(&j.path), Format::Org);
        assert!(j.date_key.is_some(), "journal date parsed from .org stem");
        let dto = g.load_page(&j).unwrap();
        assert_eq!(dto.blocks.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_round_trip_org_is_read_only_and_save_refused() {
        let dir = scratch("org-ro");
        // Skipped heading level (`*` then `***`) cannot be reproduced from tree
        // depth → not round-trip safe → must load read-only and refuse writes.
        let src = "* a\n*** c\n";
        fs::write(dir.join("pages").join("Weird.org"), src).unwrap();
        let g = Graph::open(&dir);
        let dto = g.load_named("Weird", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.format, Format::Org);
        assert!(dto.read_only, "non-round-tripping org loads read-only");
        // Even a forced save must refuse (defense in depth) and leave bytes intact.
        let err = g.force_save_page(&dto).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(), src);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn twin_md_org_refuses_writes() {
        // M1: a page that exists as BOTH Foo.md and Foo.org is ambiguous — save,
        // force-save, rename, and delete must all refuse (no clobber of either).
        let dir = scratch("org-twin");
        fs::write(dir.join("pages").join("Foo.md"), "- md body\n").unwrap();
        fs::write(dir.join("pages").join("Foo.org"), "* org body\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let page = PageDto {
            name: "Foo".into(),
            kind: PageKind::Page,
            title: "Foo".into(),
            pre_block: None,
            blocks: vec![BlockDto { id: "x".into(), raw: "edited".into(), ..Default::default() }],
            rev: None,
            format: Format::Md,
            read_only: false,
        };
        assert!(g.save_page(&page, None).is_err(), "save refused on twin");
        assert!(g.force_save_page(&page).is_err(), "force_save refused on twin");
        assert!(g.rename_page("Foo", "Bar").is_err(), "rename refused on twin");
        assert!(g.delete_page("Foo", PageKind::Page).is_err(), "delete refused on twin");
        // Both files are byte-intact (nothing was written/moved/trashed).
        assert_eq!(fs::read_to_string(dir.join("pages").join("Foo.md")).unwrap(), "- md body\n");
        assert_eq!(fs::read_to_string(dir.join("pages").join("Foo.org")).unwrap(), "* org body\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn readonly_org_unchanged_does_not_reconcile() {
        // L2 check: an UNCHANGED read-only (non-round-tripping) .org file must not
        // spuriously reconcile (bump cache_gen) on a watcher tick — the disk_revs
        // fast path + structural normalize-compare should both treat it as "ours".
        let dir = scratch("org-ro-l2");
        let src = "* a\n*** c\n"; // skipped heading level → read-only
        let path = dir.join("pages").join("RO.org");
        fs::write(&path, src).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        // Confirm it loaded read-only.
        let dto = g.load_named("RO", PageKind::Page).unwrap().unwrap();
        assert!(dto.read_only);
        let gen0 = g.cache_generation();
        // Two watcher reconciles of the unchanged file must be no-ops.
        g.sync_file(&path);
        g.sync_file(&path);
        assert_eq!(g.cache_generation(), gen0, "unchanged read-only org reconciled spuriously");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn orphan_assets_lists_only_unreferenced_media() {
        let dir = scratch("orphans");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        // Referenced by blocks (kept): an image, a pdf, a spaced-name clip.
        fs::write(assets.join("used.png"), b"x").unwrap();
        fs::write(assets.join("paper.pdf"), b"x").unwrap();
        fs::write(assets.join("my clip.mp4"), b"x").unwrap();
        // Not referenced (orphans).
        fs::write(assets.join("stray.png"), b"x").unwrap();
        fs::write(assets.join("old_video.webm"), b"x").unwrap();
        // Sidecars / non-media — never flagged.
        fs::write(assets.join("paper.edn"), b"{}").unwrap();
        fs::create_dir_all(assets.join("paper")).unwrap(); // PDF area-image dir
        fs::write(assets.join("paper").join("1_a_2.png"), b"x").unwrap();
        fs::write(
            dir.join("pages").join("P.md"),
            "- ![](../assets/used.png)\n- [paper](../assets/paper.pdf)\n- ![](../assets/my clip.mp4)\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        let orphans: Vec<String> = g.orphan_assets().into_iter().map(|a| a.name).collect();
        assert_eq!(orphans, vec!["old_video.webm".to_string(), "stray.png".to_string()]);
        // Trash one → it moves out of assets/ into the recoverable trash.
        g.trash_asset("stray.png").unwrap();
        assert!(!assets.join("stray.png").exists());
        assert!(dir.join("logseq").join(".tine-trash").exists());
        // A name with a separator is refused (can't escape assets/).
        assert!(g.trash_asset("../pages/P.md").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_asset_trash_clears_trashed_files() {
        let dir = scratch("empty-trash");
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        fs::write(assets.join("junk1.png"), b"xx").unwrap(); // 2 bytes
        fs::write(assets.join("junk2.png"), b"yyy").unwrap(); // 3 bytes
        let g = Graph::open(&dir);
        g.trash_asset("junk1.png").unwrap();
        g.trash_asset("junk2.png").unwrap();
        let s = g.asset_trash_stats();
        assert_eq!(s.count, 2, "two files in trash");
        assert_eq!(s.bytes, 5, "2 + 3 bytes preserved through the move");
        assert_eq!(g.empty_asset_trash().unwrap(), 2, "both removed");
        assert_eq!(g.asset_trash_stats().count, 0, "trash empty afterwards");
        // Emptying a never-created trash is a no-op, not an error.
        let dir2 = scratch("empty-trash-missing");
        assert_eq!(Graph::open(&dir2).empty_asset_trash().unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }

    #[test]
    fn import_asset_uses_given_name() {
        let dir = scratch("import-name");
        let src = dir.join("source.png");
        fs::write(&src, b"img").unwrap();
        let g = Graph::open(&dir);
        let saved = g.import_asset(&src, Some("source_20260626_120000.png")).unwrap();
        assert_eq!(saved, "source_20260626_120000.png");
        assert!(dir.join("assets").join(&saved).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_aborts_on_readonly_org_referrer() {
        // H1: a rename must NOT rewrite a read-only (non-round-tripping) .org file.
        let dir = scratch("org-rename-ro");
        fs::write(dir.join("pages").join("Alpha.md"), "- alpha\n").unwrap();
        // `* a\n*** c` skips a heading level → not round-trip-safe → read-only.
        let ro = "* a\n*** c referencing [[Alpha]]\n";
        fs::write(dir.join("pages").join("Weird.org"), ro).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let err = g.rename_page("Alpha", "Beta").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        // All-or-nothing: neither file moved/changed.
        assert!(dir.join("pages").join("Alpha.md").exists(), "rename rolled back");
        assert_eq!(fs::read_to_string(dir.join("pages").join("Weird.org")).unwrap(), ro);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_org_skips_refs_in_src_block() {
        // H2 end-to-end: renaming a page leaves a `[[Old]]` literal inside an org
        // src block untouched while rewriting a real ref outside it.
        let dir = scratch("org-rename-src");
        fs::write(dir.join("pages").join("Old.md"), "- old body\n").unwrap();
        let org = "* note\nsee [[Old]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n";
        fs::write(dir.join("pages").join("Ref.org"), org).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        g.rename_page("Old", "New").unwrap();
        let got = fs::read_to_string(dir.join("pages").join("Ref.org")).unwrap();
        assert_eq!(got, "* note\nsee [[New]]\n#+BEGIN_SRC clojure\n\"[[Old]]\"\n#+END_SRC\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn org_save_with_typed_headline_caches_disk_tree() {
        // H4: typing a column-0 `* ` line into a block body makes the saved bytes
        // re-parse to a DIFFERENT tree; the cache must reflect what's on disk, not
        // the (now-stale) frontend doc — so reads after the save see the real shape.
        let dir = scratch("org-h4");
        fs::write(dir.join("pages").join("P.org"), "* one\n* two\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let dto = g.load_named("P", PageKind::Page).unwrap().unwrap();
        assert_eq!(dto.blocks.len(), 2);
        // Edit block 0's body to contain a column-0 headline marker.
        let mut edited = dto.clone();
        edited.blocks[0].raw = "one\n* injected".into();
        let rev = g.save_page(&edited, dto.rev.as_deref()).unwrap();
        // Disk now has THREE headlines.
        let disk = fs::read_to_string(dir.join("pages").join("P.org")).unwrap();
        assert_eq!(disk, "* one\n* injected\n* two\n");
        // A fresh load (served from cache) must reflect the 3-block disk structure,
        // not the 2-block frontend doc that produced it.
        let again = g.load_named("P", PageKind::Page).unwrap().unwrap();
        assert_eq!(again.blocks.len(), 3, "cache reflects disk structure after H4 reparse");
        assert_eq!(again.rev.as_deref(), Some(rev.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_page_uses_preferred_format_org() {
        let dir = scratch("org-pref");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(dir.join("logseq").join("config.edn"), "{:preferred-format \"Org\"}\n").unwrap();
        let g = Graph::open(&dir);
        assert_eq!(g.preferred_format(), Format::Org);
        // Create a brand-new page via save (no baseline) — it must land as .org.
        let page = PageDto {
            name: "Fresh".into(),
            kind: PageKind::Page,
            title: "Fresh".into(),
            pre_block: None,
            blocks: vec![BlockDto { id: "x".into(), raw: "hello org".into(), ..Default::default() }],
            rev: None,
            format: Format::Org,
            read_only: false,
        };
        g.save_page(&page, None).unwrap();
        assert!(dir.join("pages").join("Fresh.org").exists(), "new page created as .org");
        assert!(!dir.join("pages").join("Fresh.md").exists());
        assert_eq!(fs::read_to_string(dir.join("pages").join("Fresh.org")).unwrap(), "* hello org\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_skips_rewrite_when_only_whitespace_trivia_differs() {
        // The file has an empty bullet written `- ` (trailing space); the
        // serializer would re-emit it as `-`. A5: a load→save with no real edit
        // must NOT rewrite the file (no Syncthing churn) and must not bump the
        // cache generation — the parsed structure is identical.
        let dir = scratch("noop");
        let path = dir.join("pages").join("A.md");
        let original = "- a\n- \n"; // second bullet: dash + trailing space
        fs::write(&path, original).unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        let entry = g.find_entry("A", PageKind::Page).unwrap();
        let dto = g.load_page(&entry).unwrap();
        let gen_before = g.cache_generation();
        let rev = g.save_page(&dto, dto.rev.as_deref()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original, "bytes left untouched");
        assert_eq!(rev, content_rev(original), "returned rev is the on-disk rev");
        assert_eq!(g.cache_generation(), gen_before, "no cache_gen bump on a trivia-only no-op");
        let _ = fs::remove_dir_all(&dir);
    }

    fn mkhl(id: &str, page: i64, text: Option<&str>) -> crate::pdf::Highlight {
        let r = crate::pdf::Rect { top: 1.0, left: 2.0, width: 3.0, height: 4.0 };
        crate::pdf::Highlight {
            id: id.into(),
            page,
            position: crate::pdf::Position { page, bounding: r.clone(), rects: vec![r] },
            color: "yellow".into(),
            text: text.map(String::from),
            image: None,
        }
    }

    #[test]
    fn write_highlights_migrates_legacy_key_forward() {
        // Old Tine wrote highlight files under a lowercase+underscore key
        // (`my_paper`); the OG-compatible key for "My Paper.pdf" is "My Paper". A
        // read must find the legacy file, and the next write must migrate the
        // artifacts to the new key (removing the stale legacy ones).
        let dir = scratch("hlmig");
        let pdf = "My Paper.pdf";
        let legacy_key = crate::pdf::legacy_asset_key(pdf); // "my_paper"
        let new_key = crate::pdf::asset_key(pdf); // "My Paper"
        assert_ne!(legacy_key, new_key);
        let assets = dir.join("assets");
        fs::create_dir_all(&assets).unwrap();
        let h1 = mkhl("11111111-1111-1111-1111-111111111111", 3, Some("legacy text"));
        fs::write(assets.join(format!("{legacy_key}.edn")), crate::pdf::write_highlights(&[h1.clone()]))
            .unwrap();
        let legacy_page = crate::pdf::hls_page_document(pdf, "My Paper", &[h1.clone()]);
        fs::write(dir.join("pages").join(format!("hls__{legacy_key}.md")), doc::serialize(&legacy_page))
            .unwrap();

        let g = Graph::open(&dir);
        g.warm_cache();
        // Read-fallback: the legacy file is found under the new-key lookup.
        let read = g.read_highlights(pdf);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, h1.id);

        // Write H1 + a newly-added H2 (editor baseline = [H1]).
        let h2 = mkhl("22222222-2222-2222-2222-222222222222", 4, Some("new text"));
        g.write_highlights(pdf, "My Paper", &[h1.clone(), h2.clone()], &[h1.id.clone()]).unwrap();

        // New-key artifacts exist with both highlights; the legacy ones are gone.
        let new_edn = assets.join(format!("{new_key}.edn"));
        assert!(new_edn.exists(), "new-key edn written");
        let migrated = crate::pdf::parse_highlights(&fs::read_to_string(&new_edn).unwrap());
        assert_eq!(migrated.len(), 2, "both highlights carried forward");
        assert!(dir.join("pages").join(format!("hls__{new_key}.md")).exists(), "new hls page");
        assert!(!assets.join(format!("{legacy_key}.edn")).exists(), "legacy edn removed");
        assert!(
            !dir.join("pages").join(format!("hls__{legacy_key}.md")).exists(),
            "legacy hls page removed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_highlight_write_is_not_seen_as_external() {
        // Repro for the "someone else edited the note" warning when deleting a
        // highlight while its hls__ page is open: the hls page write (and the
        // delete-rewrite) must be recognized as Tine's OWN write by the watcher,
        // not flagged as an external change.
        let dir = scratch("hldel");
        let g = Graph::open(&dir);
        g.warm_cache();
        let h1 = mkhl("aaaaaaaa-0000-0000-0000-000000000001", 1, Some("one"));
        let h2 = mkhl("bbbbbbbb-0000-0000-0000-000000000002", 2, Some("two"));
        let page_path = dir.join("pages").join("hls__paper.md");
        g.write_highlights("paper.pdf", "Paper", &[h1.clone(), h2.clone()], &[]).unwrap();
        assert!(g.sync_file(&page_path).is_none(), "initial highlight write looked external");
        // Delete h2 (write just h1; baseline = both) — the rewrite must also be ours.
        g.write_highlights("paper.pdf", "Paper", &[h1.clone()], &[h1.id.clone(), h2.id.clone()])
            .unwrap();
        assert!(g.sync_file(&page_path).is_none(), "delete-rewrite looked external (false conflict)");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_pdf_area_image_uses_og_layout() {
        let dir = scratch("areaimg");
        let g = Graph::open(&dir);
        let rel = g.write_pdf_area_image("My Paper.pdf", 7, "abc-id", 1659920114630, &[1, 2, 3, 4]).unwrap();
        // OG layout: assets/<key>/<page>_<id>_<stamp>.png with the OG-compatible key.
        assert_eq!(rel, "My Paper/7_abc-id_1659920114630.png");
        let p = dir.join("assets").join("My Paper").join("7_abc-id_1659920114630.png");
        assert_eq!(fs::read(&p).unwrap(), vec![1, 2, 3, 4]);
        let _ = fs::remove_dir_all(&dir);
    }

    fn jdto(name: &str) -> PageDto {
        PageDto {
            name: name.into(),
            kind: PageKind::Journal,
            title: name.into(),
            pre_block: None,
            blocks: vec![BlockDto {
                id: String::new(),
                raw: "hi".into(),
                collapsed: false,
                children: vec![],
                breadcrumb: vec![],
            }],
            rev: None,
            format: Format::Md,
            read_only: false,
        }
    }

    #[test]
    fn custom_journal_format_creates_in_user_format() {
        let dir = scratch("jfmt");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:journal/file-name-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        let g = Graph::open(&dir);
        // A custom filename format now creates today's journal at the CORRECT path
        // (the user's format) — not a misplaced default `yyyy_MM_dd` duplicate.
        g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
        assert!(dir.join("journals").join("2026-06-24.md").exists());
        assert!(!dir.join("journals").join("2026_06_24.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_format_journal_files_load_and_display() {
        // THE reported bug: a graph whose journal files use a non-default format
        // must still load — the files are recognized and titled in the user's
        // page-title-format.
        let dir = scratch("jfmt-load");
        fs::create_dir_all(dir.join("logseq")).unwrap();
        fs::create_dir_all(dir.join("journals")).unwrap();
        fs::write(
            dir.join("logseq").join("config.edn"),
            "{:journal/file-name-format \"dd-MM-yyyy\" :journal/page-title-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        // A real journal file in the user's dd-MM-yyyy filename format.
        fs::write(dir.join("journals").join("24-06-2026.md"), "- hi\n").unwrap();
        let g = Graph::open(&dir);
        let js = g.journals_desc();
        assert_eq!(js.len(), 1, "custom-format journal must be recognized (was dropped before)");
        assert_eq!(js[0].date_key, Some(20260624));
        assert_eq!(js[0].name, "2026-06-24", "title rendered in :journal/page-title-format");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_journal_format_creates_journal() {
        let dir = scratch("jfmt-default");
        // No config.edn → defaults → creation proceeds as before.
        let g = Graph::open(&dir);
        g.save_page(&jdto("Jun 24th, 2026"), None).unwrap();
        assert!(dir.join("journals").join("2026_06_24.md").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn advanced_query_runs_supported_subset_flags_rest() {
        let dir = scratch("adv");
        fs::write(dir.join("journals").join("2026_06_20.md"), "- TODO ship it\n- DONE done\n").unwrap();
        fs::write(dir.join("pages").join("Note.md"), "- TODO not a journal\n").unwrap();
        let g = Graph::open(&dir);
        g.warm_cache();
        // (task ?b #{"TODO"}) maps to the existing Task predicate.
        let r = g.run_advanced_query(r#"[:find (pull ?b [*]) :where (task ?b #{"TODO"})]"#, None);
        assert!(r.supported);
        assert!(r.ran.contains(&"task".to_string()));
        let total: usize = r.groups.iter().map(|grp| grp.blocks.len()).sum();
        assert_eq!(total, 2, "both TODO blocks match");
        // A clause outside the subset (a raw [?e :a ?v] join) → nothing supported.
        let u = g.run_advanced_query("[:find ?b :where [?b :block/foo ?v]]", None);
        assert!(!u.supported);
        assert!(u.groups.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
