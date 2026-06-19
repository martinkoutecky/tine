//! Graph model: opening a graph directory, listing/loading/saving pages, and
//! the DTOs that cross the Tauri IPC boundary.
//!
//! For M0/M1 the canonical state is the on-disk files; Rust loads a page into a
//! [`PageDto`] tree and writes it back from one. The frontend owns the live
//! editing tree (see plan). UUIDs are assigned fresh per load and only
//! persisted as `id::` once a block is referenced (a later milestone).

use crate::config::Config;
use crate::date::JournalDate;
use crate::doc::{self, DocBlock, Document};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    Journal,
    Page,
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
}

pub struct Graph {
    pub root: PathBuf,
    pub config: Config,
    /// In-memory cache of every parsed page, keyed implicitly by position.
    /// Built once on first whole-graph query and kept in sync by edits, so
    /// search / backlinks / `{{query}}` scan memory instead of re-reading and
    /// re-parsing the entire tree on every keystroke. `None` = not yet built.
    cache: RwLock<Option<Vec<(PageEntry, Document)>>>,
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
    results: std::collections::HashMap<String, Vec<RefGroup>>,
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
}

impl Graph {
    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let config = fs::read_to_string(root.join("logseq").join("config.edn"))
            .map(|s| Config::parse(&s))
            .unwrap_or_default();
        Graph {
            root,
            config,
            cache: RwLock::new(None),
            cache_gen: std::sync::atomic::AtomicU64::new(0),
            build_lock: std::sync::Mutex::new(()),
            alias_cache: RwLock::new(None),
            block_index: RwLock::new(None),
            derived_cache: RwLock::new(None),
            page_list_cache: RwLock::new(None),
            recent_writes: std::sync::Mutex::new(std::collections::HashMap::new()),
            disk_revs: RwLock::new(std::collections::HashMap::new()),
        }
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
        }
    }

    /// Current cache generation — bumped on every cache-mutating page change, and
    /// the key that memoized queries/backlinks/derived results invalidate against.
    /// Exposed for observability and tests (e.g. asserting a no-op save doesn't
    /// needlessly invalidate everything).
    pub fn cache_generation(&self) -> u64 {
        self.cache_gen.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Persist the favorites list to config.edn `:favorites [...]`, replacing the
    /// existing vector or inserting one, preserving the rest of the file.
    pub fn set_favorites(&self, names: &[String]) -> io::Result<()> {
        let path = self.root.join("logseq").join("config.edn");
        let mut content = fs::read_to_string(&path).unwrap_or_else(|_| "{}\n".to_string());
        let vec_str = format!(
            "[{}]",
            names
                .iter()
                .map(|n| format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if let Some(start) = find_keyword(&content, ":favorites") {
            // Replace the existing `:favorites [...]` vector. Locate the key
            // EDN-aware (skips strings/comments) and require its value to be a
            // vector; find the matching `]` with an EDN-aware scan so a favorite
            // NAME containing `]` (or a comment inside the vector) can't truncate
            // the replacement and corrupt config.edn.
            let after = start + ":favorites".len();
            let b = content.as_bytes();
            let mut j = after;
            while j < b.len() && matches!(b[j], b' ' | b'\t' | b'\n' | b'\r' | b',') {
                j += 1;
            }
            if j < b.len() && b[j] == b'[' {
                let end = match_close_bracket(&content, j) + 1;
                content.replace_range(start..end, &format!(":favorites {vec_str}"));
            } else {
                // Key present but no vector value — insert one right after it.
                content.insert_str(after, &format!(" {vec_str}"));
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n :favorites {vec_str}\n"));
        } else {
            content = format!("{{:favorites {vec_str}}}\n");
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, content.as_bytes())
    }

    /// Persist the task workflow to config.edn `:preferred-workflow :todo`/`:now`,
    /// replacing the existing keyword value (preserving the rest of the file) or
    /// inserting the key. Mirrors Logseq's own config key so it travels with the
    /// graph. We locate the key on a non-comment line so a commented-out example
    /// `;; :preferred-workflow …` is never edited by mistake.
    pub fn set_preferred_workflow(&self, wf: &str) -> io::Result<()> {
        let kw = if wf == "todo" { ":todo" } else { ":now" };
        let key = ":preferred-workflow";
        let path = self.root.join("logseq").join("config.edn");
        let mut content = fs::read_to_string(&path).unwrap_or_else(|_| "{}\n".to_string());

        if let Some(start) = find_keyword(&content, key) {
            let after = start + key.len();
            // The value is a keyword: first non-whitespace char after the key,
            // expected to start with ':', running to whitespace/`}`/`)`.
            // `find_keyword` (not `find_uncommented`) so a `:preferred-workflow`
            // inside a string literal can't be mistaken for the real key.
            match content[after..].find(|c: char| !c.is_whitespace()) {
                Some(rel) if content[after + rel..].starts_with(':') => {
                    let vstart = after + rel;
                    let vrest = &content[vstart + 1..];
                    let end = vrest
                        .find(|c: char| c.is_whitespace() || c == '}' || c == ')')
                        .unwrap_or(vrest.len());
                    content.replace_range(vstart..vstart + 1 + end, kw);
                }
                // Key present but no keyword value — insert one right after it.
                _ => content.insert_str(after, &format!(" {kw}")),
            }
        } else if let Some(brace) = content.find('{') {
            content.insert_str(brace + 1, &format!("\n :preferred-workflow {kw}\n"));
        } else {
            content = format!("{{:preferred-workflow {kw}}}\n");
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, content.as_bytes())
    }

    /// Persist the new-journal default template to config.edn as
    /// `:default-templates {:journals "Name"}`. `Some` sets/replaces the
    /// `:journals` entry; `None` removes it. Any OTHER keys in
    /// `:default-templates`, the rest of the file, and comments are preserved.
    /// Mirrors Logseq's own config key, so it travels with the graph.
    pub fn set_default_journal_template(&self, name: Option<&str>) -> io::Result<()> {
        let path = self.root.join("logseq").join("config.edn");
        let mut content = fs::read_to_string(&path).unwrap_or_else(|_| "{}\n".to_string());

        // Locate a real (non-string, non-comment) `:default-templates` whose value
        // is a map literal `{ … }`. `find_keyword` skips strings/comments and
        // requires a token boundary, so a `:default-templates` inside a string
        // value can't be mistaken for the key; we then require the next non-blank
        // char to be `{` so a non-map value (e.g. `:default-templates nil`) doesn't
        // make us grab a brace from elsewhere in the file.
        let dt = find_keyword(&content, ":default-templates").and_then(|start| {
            let after = start + ":default-templates".len();
            let b = content.as_bytes();
            let mut j = after;
            while j < b.len() && matches!(b[j], b' ' | b'\t' | b'\n' | b'\r' | b',') {
                j += 1;
            }
            if j >= b.len() || b[j] != b'{' {
                return None; // value isn't a map → don't touch it
            }
            let close = match_close_brace(&content, j); // EDN-aware (skips strings/comments/nesting)
            Some((j, close)) // byte indices of `{` and matching `}`
        });

        match name {
            Some(n) => {
                let v = format!("\"{}\"", n.replace('\\', "\\\\").replace('"', "\\\""));
                match dt {
                    Some((open, close)) => {
                        if let Some(jrel) = find_keyword(&content[open + 1..close], ":journals") {
                            // Replace the value IMMEDIATELY after :journals — whatever
                            // it is (a string, or a non-string token like `nil`). Do NOT
                            // scan for the next quote anywhere in the map: that could land
                            // on a *later* key's string value (e.g. `:journals nil :pages
                            // "P"` would otherwise replace "P").
                            let after = open + 1 + jrel + ":journals".len();
                            match next_value_span(&content, after, close) {
                                Some((vstart, vend, _)) => content.replace_range(vstart..vend, &v),
                                None => content.insert_str(after, &format!(" {v}")),
                            }
                        } else {
                            // Map present but no :journals — insert the pair.
                            let sep = if content[open + 1..close].trim().is_empty() { "" } else { " " };
                            content.insert_str(open + 1, &format!(":journals {v}{sep}"));
                        }
                    }
                    None => {
                        let entry = format!("\n :default-templates {{:journals {v}}}\n");
                        if let Some(brace) = content.find('{') {
                            content.insert_str(brace + 1, &entry);
                        } else {
                            content = format!("{{:default-templates {{:journals {v}}}}}\n");
                        }
                    }
                }
            }
            None => {
                // Remove the `:journals "…"` pair (leaving any other keys + an empty
                // `:default-templates {}`, which parses to "no journal template").
                if let Some((open, close)) = dt {
                    if let Some(jrel) = find_keyword(&content[open + 1..close], ":journals") {
                        let jstart = open + 1 + jrel;
                        let after = jstart + ":journals".len();
                        // End just past the IMMEDIATE value token (string or not),
                        // escape-aware — not the next quote anywhere in the map, which
                        // could belong to a later key.
                        let end = next_value_span(&content, after, close)
                            .map(|(_, vend, _)| vend)
                            .unwrap_or(after);
                        // Swallow trailing separators so we don't leave a gap.
                        let tail: usize = content[end..close]
                            .chars()
                            .take_while(|c| c.is_whitespace() || *c == ',')
                            .map(|c| c.len_utf8())
                            .sum();
                        content.replace_range(jstart..end + tail, "");
                    }
                }
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, content.as_bytes())
    }

    pub fn journals_path(&self) -> PathBuf {
        self.root.join(&self.config.journals_dir)
    }

    pub fn pages_path(&self) -> PathBuf {
        self.root.join(&self.config.pages_dir)
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
        entries.extend(list_md(&self.journals_path(), PageKind::Journal));
        entries.extend(list_md(&self.pages_path(), PageKind::Page));
        *self.page_list_cache.write().unwrap() = Some((gen, entries.clone()));
        entries
    }

    /// Journals sorted newest-first.
    pub fn journals_desc(&self) -> Vec<PageEntry> {
        // Prefer the warmed whole-graph cache — its PageEntry list is kept current
        // by cache_upsert/cache_remove, so we avoid a directory read + parse on
        // every infinite-scroll feed append. Fall back to scanning the dir while
        // the cache isn't built yet.
        let mut js: Vec<PageEntry> = match self.cache.read().unwrap().as_ref() {
            Some(pages) => pages
                .iter()
                .filter(|(e, _)| e.kind == PageKind::Journal && e.date_key.is_some())
                .map(|(e, _)| e.clone())
                .collect(),
            None => list_md(&self.journals_path(), PageKind::Journal)
                .into_iter()
                .filter(|e| e.date_key.is_some())
                .collect(),
        };
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
            if p.extension().and_then(|x| x.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if JournalDate::from_file_stem(stem).is_some() {
                continue; // already a valid date stem
            }
            let Some(d) = JournalDate::from_title(stem) else { continue }; // not a date title
            let target = dir.join(format!("{}.md", d.file_stem()));
            if target.exists() {
                continue; // don't clobber an existing stem file
            }
            if fs::rename(&p, &target).is_ok() {
                n += 1;
            }
        }
        n
    }

    /// Resolve a page name to a file path. Journals match by date title;
    /// pages match by filename stem.
    fn path_for(&self, name: &str, kind: PageKind) -> PathBuf {
        match kind {
            PageKind::Journal => self
                .journals_desc()
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
                .map(|e| e.path)
                .unwrap_or_else(|| {
                    // New journal: name it by its date stem ("2026_06_18.md"), not
                    // the display title — a title-named file ("Jun 18th, 2026.md")
                    // can't be parsed back to a date, so journals_desc would drop
                    // it and the day would look empty.
                    let stem = JournalDate::from_title(name)
                        .map(|d| d.file_stem())
                        .unwrap_or_else(|| name.to_string());
                    self.journals_path().join(format!("{stem}.md"))
                }),
            PageKind::Page => self.pages_path().join(format!("{}.md", encode_page_name(name))),
        }
    }

    /// Find a page/journal entry by display name.
    pub fn find_entry(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        let dir = match kind {
            PageKind::Journal => self.journals_path(),
            PageKind::Page => self.pages_path(),
        };
        list_md(&dir, kind)
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
            dto.rev = rev;
            return Ok(dto);
        }
        // Cache miss: parse the bytes we already read (propagate the original read
        // error if it failed).
        let content = read?;
        let mut doc = doc::parse(&content);
        assign_doc_uuids(&mut doc.roots);
        let mut dto = page_dto(entry, &doc);
        dto.rev = rev;
        Ok(dto)
    }

    /// Read and parse a page file into a [`Document`].
    pub fn read_document(&self, entry: &PageEntry) -> io::Result<Document> {
        let content = fs::read_to_string(&entry.path)?;
        Ok(doc::parse(&content))
    }

    /// Read+parse every page from disk (skipping unreadable files). Used to
    /// build the in-memory cache.
    fn load_all_pages(&self) -> Vec<(PageEntry, Document, String)> {
        self.list_pages()
            .into_iter()
            .filter_map(|e| {
                let content = fs::read_to_string(&e.path).ok()?;
                let rev = content_rev(&content);
                let mut d = doc::parse(&content);
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
        let pages: Vec<(PageEntry, Document)> =
            built.into_iter().map(|(e, d, _)| (e, d)).collect();
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
    pub fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Document)]) -> T) -> T {
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
                let mut d = doc::parse(&content);
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
        // Bump the generation so the gen-keyed block index rebuilds against the
        // fresh content rather than trusting a hint from the discarded cache.
        self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
        let mut guard = self.cache.write().unwrap();
        *guard = None;
        self.disk_revs.write().unwrap().clear(); // under the cache lock (cache → disk_revs)
        drop(guard);
        *self.alias_cache.write().unwrap() = None;
        *self.block_index.write().unwrap() = None;
    }

    /// Update one page in the cache after we write it (no full rebuild). A no-op
    /// if the cache hasn't been built yet. `disk_rev` is `content_rev` of the
    /// exact on-disk bytes `doc` was produced from (the freshness key — see
    /// `disk_revs`).
    fn cache_upsert(&self, entry: PageEntry, mut doc: Document, disk_rev: String) {
        // Signal a mutation so a concurrent lock-free cache build won't install a
        // snapshot that predates this change (see with_pages). Update the cache
        // slot BEFORE disk_revs below, so a reader can never observe a fresh rev
        // paired with a stale cached doc.
        self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
        // Fill uuids for any block that lacks one (e.g. PDF-highlight writes);
        // blocks saved from the frontend already carry their ids, which are kept.
        assign_doc_uuids(&mut doc.roots);
        // Only the alias map needs dropping when an `alias::` was added/changed/
        // removed — invalidating on every save would make a normal edit an O(P)
        // alias rescan on the next navigation.
        let new_has_alias = doc_has_alias(&doc);
        let mut alias_touched = new_has_alias;
        let key = rev_key(entry.kind, &entry.name);
        let mut guard = self.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            match pages.iter_mut().find(|(e, _)| {
                e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name)
            }) {
                Some(slot) => {
                    alias_touched = new_has_alias || doc_has_alias(&slot.1);
                    slot.1 = doc;
                }
                None => pages.push((entry, doc)),
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
        drop(guard);
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
    }

    /// Drop one page from the cache after deleting its file.
    fn cache_remove(&self, name: &str, kind: PageKind) {
        self.cache_gen.fetch_add(1, std::sync::atomic::Ordering::Release);
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
        drop(guard);
        if alias_touched {
            *self.alias_cache.write().unwrap() = None;
        }
    }

    /// Memoize a derived whole-graph scan result, keyed by `(cache_gen, today)` +
    /// `key`. On a tag mismatch the whole cache is dropped, so a hit is always
    /// consistent with the current graph. `compute` runs with NO lock held (it
    /// takes the cache read lock itself), so it can't deadlock against `with_pages`.
    fn derived_memo(&self, key: String, compute: impl FnOnce() -> Vec<RefGroup>) -> Vec<RefGroup> {
        use std::sync::atomic::Ordering;
        let gen = self.cache_gen.load(Ordering::Acquire);
        let today = crate::date::JournalDate::today().ordinal_key();
        {
            let g = self.derived_cache.read().unwrap();
            if let Some(dc) = g.as_ref() {
                if dc.gen == gen && dc.today == today {
                    if let Some(r) = dc.results.get(&key) {
                        return r.clone();
                    }
                }
            }
        }
        let result = compute();
        let mut g = self.derived_cache.write().unwrap();
        match g.as_mut() {
            Some(dc) if dc.gen == gen && dc.today == today => {
                dc.results.insert(key, result.clone());
            }
            _ => {
                let mut results = std::collections::HashMap::new();
                results.insert(key, result.clone());
                *g = Some(DerivedCache { gen, today, results });
            }
        }
        result
    }

    /// Backlinks for a page: blocks across the graph that reference it,
    /// grouped by source page. Delegates to the query module (memoized).
    pub fn backlinks(&self, target: &str) -> Vec<RefGroup> {
        self.derived_memo(format!("b\0{}", crate::refs::normalize(target)), || {
            crate::query::backlinks(self, target)
        })
    }

    /// Evaluate a `{{query ...}}` body over the graph (memoized).
    pub fn run_query(&self, query_src: &str) -> Vec<RefGroup> {
        self.derived_memo(format!("q\0{query_src}"), || crate::query::run_query(self, query_src))
    }

    /// Unlinked references: plain-text mentions of a page that aren't links
    /// (memoized).
    pub fn unlinked_refs(&self, target: &str) -> Vec<RefGroup> {
        self.derived_memo(format!("u\0{}", crate::refs::normalize(target)), || {
            crate::query::unlinked_refs(self, target)
        })
    }

    /// Export the whole graph to static HTML under `<root>/publish/`.
    pub fn publish_html(&self) -> io::Result<(String, usize)> {
        crate::publish::publish_graph(self)
    }

    /// Rename a page: move its file to the new name and rewrite every `[[old]]`
    /// / `#old` reference across the graph. Journals can't be renamed (their
    /// name is their date). Returns an error if `new` already exists.
    pub fn rename_page(&self, old: &str, new: &str) -> io::Result<()> {
        let new = new.trim();
        if new.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty name"));
        }
        if old.eq_ignore_ascii_case(new) {
            return Ok(());
        }
        let old_path = self.pages_path().join(format!("{}.md", encode_page_name(old)));
        let new_path = self.pages_path().join(format!("{}.md", encode_page_name(new)));
        if new_path.exists() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "target page exists"));
        }
        // Move the page's own file (if it has one — a page can exist only as
        // references, with no file yet).
        if old_path.exists() {
            let content = fs::read_to_string(&old_path)?;
            if let Some(parent) = new_path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&new_path, content.as_bytes())?;
            fs::remove_file(&old_path)?;
        }
        // Rewrite references across every page/journal that mentions `old`.
        for entry in self.list_pages() {
            if entry.path == old_path {
                continue; // already moved
            }
            let Ok(content) = fs::read_to_string(&entry.path) else { continue };
            if !crate::refs::references_page(&content, old) {
                continue;
            }
            let updated = crate::refs::rename_refs(&content, old, new);
            if updated != content {
                atomic_write(&entry.path, updated.as_bytes())?;
            }
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
    pub fn import_asset(&self, src: &Path) -> io::Result<String> {
        let name = src
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad source filename"))?
            .to_string();
        let assets = self.assets_path();
        fs::create_dir_all(&assets)?;
        // Reserve the name (creating an empty placeholder so no concurrent writer
        // can claim it), then copy the source over our own placeholder.
        let (final_name, _file) = reserve_asset(&assets, &name)?;
        drop(_file);
        fs::copy(src, assets.join(&final_name))?;
        Ok(final_name)
    }

    /// Read highlights for a PDF from `assets/<key>.edn`.
    pub fn read_highlights(&self, pdf_filename: &str) -> Vec<crate::pdf::Highlight> {
        let key = crate::pdf::asset_key(pdf_filename);
        match fs::read_to_string(self.assets_path().join(format!("{key}.edn"))) {
            Ok(s) => crate::pdf::parse_highlights(&s),
            Err(_) => Vec::new(),
        }
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
        fs::create_dir_all(self.assets_path())?;
        let edn_path = self.assets_path().join(format!("{key}.edn"));
        // 3-way merge against the on-disk set: keep our current highlights, plus
        // any disk highlight that is an EXTERNAL addition (id not in our baseline
        // and not already present). A highlight we deliberately deleted (in the
        // baseline, absent from current) is NOT resurrected.
        let have: std::collections::HashSet<&str> = highlights.iter().map(|h| h.id.as_str()).collect();
        let base: std::collections::HashSet<&str> = base_ids.iter().map(|s| s.as_str()).collect();
        let mut merged: Vec<crate::pdf::Highlight> = highlights.to_vec();
        if let Ok(s) = fs::read_to_string(&edn_path) {
            for h in crate::pdf::parse_highlights(&s) {
                if !have.contains(h.id.as_str()) && !base.contains(h.id.as_str()) {
                    merged.push(h);
                }
            }
        }
        let edn = crate::pdf::write_highlights(&merged);
        atomic_write(&edn_path, edn.as_bytes())?;

        // Upsert into the existing hls page, preserving note children by id.
        let page_path = self.pages_path().join(format!("{}.md", crate::pdf::hls_page_name(&key)));
        let existing = fs::read_to_string(&page_path).ok().map(|s| doc::parse(&s));
        let page_doc = crate::pdf::merge_hls_page(existing.as_ref(), pdf_filename, label, &merged);
        let page_md = doc::serialize(&page_doc);
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
        Ok(())
    }

    /// Map an on-disk `.md` path to its page entry (journal or page), or None if
    /// it isn't in the graph's journals/pages dirs.
    pub fn entry_for_path(&self, path: &Path) -> Option<PageEntry> {
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            return None;
        }
        let stem = path.file_stem().and_then(|s| s.to_str())?;
        let parent = path.parent()?;
        if parent == self.journals_path() {
            let (name, date_key) = match JournalDate::from_file_stem(stem) {
                Some(d) => (d.title(), Some(d.ordinal_key())),
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
        let newdoc = doc::parse(content);
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
                let opts = doc::SerializeOpts::detect(Some(content));
                let cached_norm = doc::parse(&doc::serialize_with(cached, &opts));
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
        let path = self.path_for(&page.name, page.kind);
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
        self.write_page(page, existing.as_deref())
    }

    /// Save a page unconditionally (the user chose "keep mine" over a conflict).
    pub fn force_save_page(&self, page: &PageDto) -> io::Result<String> {
        let path = self.path_for(&page.name, page.kind);
        let existing = fs::read_to_string(&path).ok();
        self.write_page(page, existing.as_deref())
    }

    /// Write a page, reproducing `existing`'s formatting, and return the new
    /// on-disk content rev (computed from what was written — no extra read).
    fn write_page(&self, page: &PageDto, existing: Option<&str>) -> io::Result<String> {
        let doc = Document {
            pre_block: page.pre_block.clone(),
            roots: page.blocks.iter().map(dto_to_doc).collect(),
        };
        let path = self.path_for(&page.name, page.kind);
        // Reproduce the existing file's formatting (trailing newline, post-property
        // blank line, indent) so an unchanged save is byte-identical and edits
        // produce a minimal diff — critical to avoid Syncthing churn against Logseq.
        let opts = doc::SerializeOpts::detect(existing);
        let content = doc::serialize_with(&doc, &opts);
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
            self.cache_upsert(entry, doc, rev.clone());
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

/// A filename under `assets/` that doesn't collide with an existing file: `name`
/// itself if free, else `stem_1.ext`, `stem_2.ext`, … so an import/paste never
/// overwrites an asset already referenced by notes.
/// Byte offset of the first occurrence of `key` on a NON-comment line (the text
/// before any `;` on that line), or None. Config writers use this so a
/// commented-out example like `;; :favorites […]` is never edited by mistake.
/// Index just past the closing quote of an EDN string that opens at byte `open`
/// (a `"`), skipping `\"` / `\\` escapes — so an escaped quote inside the value
/// can't make a range-edit land on the wrong byte and corrupt config.edn. Returns
/// end-of-string if the string is unterminated. (`"` is ASCII, so the returned
/// index is always a char boundary.)
fn edn_str_end(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut i = open + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    s.len()
}

/// Matching close `}` for the map whose `{` is at byte `open`, EDN-aware: skips
/// strings (with escapes), `;` line comments, and nested braces. Returns
/// end-of-string if unbalanced. (`}` is ASCII → returned index is a char boundary.)
fn match_close_brace(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut i = open + 1;
    let mut depth = 1usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = edn_str_end(s, i);
                continue;
            }
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// Matching close `]` for the vector whose `[` is at byte `open`, EDN-aware:
/// skips strings (with escapes), `;` line comments, and nested brackets. Returns
/// end-of-string if unbalanced. (`]` is ASCII → returned index is a char boundary.)
fn match_close_bracket(s: &str, open: usize) -> usize {
    let b = s.as_bytes();
    let mut i = open + 1;
    let mut depth = 1usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = edn_str_end(s, i);
                continue;
            }
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// Byte index of a real `key` keyword inside `map_inner` (a map body), skipping
/// strings + `;` comments, and requiring a token boundary after it. None if absent.
fn find_keyword(map_inner: &str, key: &str) -> Option<usize> {
    let b = map_inner.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i = edn_str_end(map_inner, i);
                continue;
            }
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            _ if map_inner[i..].starts_with(key) => {
                let after = i + key.len();
                let boundary = after >= b.len()
                    || matches!(b[after], b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'{' | b'}' | b',');
                if boundary {
                    return Some(i);
                }
                i = after;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Span `[start, end)` of the value token following byte `from` (skipping leading
/// whitespace/commas) within `..close`, plus whether it is an EDN string. None if
/// there is no value before `close` (e.g. `:journals}`). A string value's end is
/// escape-aware; a non-string token (`nil`, a number, a symbol) ends at the next
/// whitespace/comma/brace/quote. Used to replace/remove the value that belongs to
/// a key without accidentally consuming a later key's value.
fn next_value_span(s: &str, from: usize, close: usize) -> Option<(usize, usize, bool)> {
    let b = s.as_bytes();
    let mut i = from;
    while i < close && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
        i += 1;
    }
    if i >= close {
        return None;
    }
    if b[i] == b'"' {
        return Some((i, edn_str_end(s, i).min(close), true));
    }
    let start = i;
    while i < close && !matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',' | b'{' | b'}' | b'"') {
        i += 1;
    }
    Some((start, i, false))
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

fn list_md(dir: &Path, kind: PageKind) -> Vec<PageEntry> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return out };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let (name, date_key) = match kind {
            PageKind::Journal => match JournalDate::from_file_stem(stem) {
                Some(d) => (d.title(), Some(d.ordinal_key())),
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
    }
}

/// Build a page DTO from a cached document.
fn page_dto(entry: &PageEntry, doc: &Document) -> PageDto {
    PageDto {
        name: entry.name.clone(),
        kind: entry.kind,
        title: entry.name.clone(),
        pre_block: doc.pre_block.clone(),
        blocks: doc.roots.iter().map(block_to_dto).collect(),
        rev: None,
    }
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
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
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
}
