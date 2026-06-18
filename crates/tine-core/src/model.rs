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
    /// Cached `alias:: → canonical` pairs, derived from the page cache. Rebuilt
    /// lazily and dropped whenever the page cache mutates (the only time aliases
    /// can change). Avoids re-scanning the whole graph for aliases on every page
    /// load / backlink lookup.
    alias_cache: RwLock<Option<Vec<(String, String)>>>,
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
        Graph { root, config, cache: RwLock::new(None), alias_cache: RwLock::new(None) }
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
        if let Some(start) = content.find(":favorites") {
            // Replace the existing (single-level) `:favorites [...]`.
            if let Some(br) = content[start..].find('[') {
                let abs = start + br;
                if let Some(end_rel) = content[abs..].find(']') {
                    let end = abs + end_rel + 1;
                    content.replace_range(start..end, &format!(":favorites {vec_str}"));
                }
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

    pub fn journals_path(&self) -> PathBuf {
        self.root.join(&self.config.journals_dir)
    }

    pub fn pages_path(&self) -> PathBuf {
        self.root.join(&self.config.pages_dir)
    }

    /// List all pages and journals in the graph.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        let mut entries = Vec::new();
        entries.extend(list_md(&self.journals_path(), PageKind::Journal));
        entries.extend(list_md(&self.pages_path(), PageKind::Page));
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
        if let Some(entry) = self.find_entry(name, kind) {
            return Ok(Some(self.load_page(&entry)?));
        }
        if kind == PageKind::Page {
            let tnorm = crate::refs::normalize(name);
            if let Some((_, canon)) = self.page_aliases().into_iter().find(|(a, _)| *a == tnorm) {
                if let Some(entry) = self.find_entry(&canon, kind) {
                    return Ok(Some(self.load_page(&entry)?));
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

    /// Load a page by entry. Served from the in-memory cache so block uuids are
    /// stable and consistent with queries / refs / the sidebar. Falls back to a
    /// disk parse for a page not yet in the cache (e.g. just created externally).
    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        // The editor's save-baseline is the hash of the actual on-disk bytes at
        // load time (read regardless of cache hit — load is not the hot path).
        let rev = fs::read_to_string(&entry.path).ok().map(|s| content_rev(&s));
        let cached = self.with_pages(|pages| {
            pages
                .iter()
                .find(|(e, _)| e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name))
                .map(|(e, d)| page_dto(e, d))
        });
        if let Some(mut dto) = cached {
            dto.rev = rev;
            return Ok(dto);
        }
        let content = fs::read_to_string(&entry.path)?;
        let mut doc = doc::parse(&content);
        for b in &mut doc.roots {
            assign_uuids(b);
        }
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
    fn load_all_pages(&self) -> Vec<(PageEntry, Document)> {
        self.list_pages()
            .into_iter()
            .filter_map(|e| {
                self.read_document(&e).ok().map(|mut d| {
                    for b in &mut d.roots {
                        assign_uuids(b);
                    }
                    (e, d)
                })
            })
            .collect()
    }

    /// Run `f` over every parsed page, building the cache on first use. The
    /// borrow is held for the duration of `f`, so callers get references without
    /// cloning the documents.
    pub fn with_pages<T>(&self, f: impl FnOnce(&[(PageEntry, Document)]) -> T) -> T {
        if let Some(pages) = self.cache.read().unwrap().as_ref() {
            return f(pages);
        }
        // Build under the write lock (re-checking in case of a race).
        let mut guard = self.cache.write().unwrap();
        if guard.is_none() {
            *guard = Some(self.load_all_pages());
        }
        f(guard.as_ref().unwrap())
    }

    /// Eagerly build the page cache (call once after opening, off the hot path).
    pub fn warm_cache(&self) {
        self.with_pages(|_| ());
    }

    /// Discard the cache; it rebuilds on the next whole-graph query. Use when an
    /// external change may have touched many files.
    pub fn invalidate_cache(&self) {
        *self.cache.write().unwrap() = None;
        *self.alias_cache.write().unwrap() = None;
    }

    /// Update one page in the cache after we write it (no full rebuild). A no-op
    /// if the cache hasn't been built yet.
    fn cache_upsert(&self, entry: PageEntry, mut doc: Document) {
        // Fill uuids for any block that lacks one (e.g. PDF-highlight writes);
        // blocks saved from the frontend already carry their ids, which are kept.
        for b in &mut doc.roots {
            assign_uuids(b);
        }
        let mut guard = self.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            match pages.iter_mut().find(|(e, _)| {
                e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name)
            }) {
                Some(slot) => slot.1 = doc,
                None => pages.push((entry, doc)),
            }
        }
        *self.alias_cache.write().unwrap() = None; // a saved page may change aliases
    }

    /// Drop one page from the cache after deleting its file.
    fn cache_remove(&self, name: &str, kind: PageKind) {
        let mut guard = self.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            pages.retain(|(e, _)| !(e.kind == kind && e.name.eq_ignore_ascii_case(name)));
        }
        *self.alias_cache.write().unwrap() = None;
    }

    /// Backlinks for a page: blocks across the graph that reference it,
    /// grouped by source page. Delegates to the query module.
    pub fn backlinks(&self, target: &str) -> Vec<RefGroup> {
        crate::query::backlinks(self, target)
    }

    /// Evaluate a `{{query ...}}` body over the graph.
    pub fn run_query(&self, query_src: &str) -> Vec<RefGroup> {
        crate::query::run_query(self, query_src)
    }

    /// Unlinked references: plain-text mentions of a page that aren't links.
    pub fn unlinked_refs(&self, target: &str) -> Vec<RefGroup> {
        crate::query::unlinked_refs(self, target)
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

    /// Delete a page/journal file.
    pub fn delete_page(&self, name: &str, kind: PageKind) -> io::Result<()> {
        if let Some(entry) = self.find_entry(name, kind) {
            fs::remove_file(&entry.path)?;
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
        let assets = self.assets_path();
        fs::create_dir_all(&assets)?;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.to_string(), String::new()),
        };
        let mut final_name = name.to_string();
        let mut i = 1;
        while assets.join(&final_name).exists() {
            final_name = format!("{stem}_{i}{ext}");
            i += 1;
        }
        fs::write(assets.join(&final_name), bytes)?;
        Ok(final_name)
    }

    /// Copy a file into `assets/`, returning the stored filename.
    pub fn import_asset(&self, src: &Path) -> io::Result<String> {
        let name = src
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad source filename"))?
            .to_string();
        let assets = self.assets_path();
        fs::create_dir_all(&assets)?;
        fs::copy(src, assets.join(&name))?;
        Ok(name)
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
    pub fn write_highlights(
        &self,
        pdf_filename: &str,
        label: &str,
        highlights: &[crate::pdf::Highlight],
    ) -> io::Result<()> {
        let key = crate::pdf::asset_key(pdf_filename);
        fs::create_dir_all(self.assets_path())?;
        let edn = crate::pdf::write_highlights(highlights);
        atomic_write(&self.assets_path().join(format!("{key}.edn")), edn.as_bytes())?;

        // Upsert into the existing hls page, preserving note children by id.
        let page_path = self.pages_path().join(format!("{}.md", crate::pdf::hls_page_name(&key)));
        let existing = fs::read_to_string(&page_path).ok().map(|s| doc::parse(&s));
        let page_doc = crate::pdf::merge_hls_page(existing.as_ref(), pdf_filename, label, highlights);
        let page_md = doc::serialize(&page_doc);
        fs::create_dir_all(self.pages_path())?;
        atomic_write(&page_path, page_md.as_bytes())?;
        // The hls page is a real page; reflect it in the search cache.
        let name = crate::pdf::hls_page_name(&key);
        let entry = self.find_entry(&name, PageKind::Page).unwrap_or(PageEntry {
            name,
            kind: PageKind::Page,
            date_key: None,
            path: page_path,
        });
        self.cache_upsert(entry, page_doc);
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

    /// Reconcile a (possibly externally-changed) file with the in-memory cache.
    /// Returns the entry only if its parsed content actually differs from the
    /// cache (i.e. a real external change) — Tine's own writes keep the cache in
    /// sync, so they return None. No-op if the cache hasn't been built yet.
    pub fn sync_file(&self, path: &Path) -> Option<PageEntry> {
        let entry = self.entry_for_path(path)?;
        let content = fs::read_to_string(path).ok()?;
        let newdoc = doc::parse(&content);
        {
            let guard = self.cache.read().unwrap();
            let cache = guard.as_ref()?; // cache not built -> nothing to reconcile
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
                let opts = doc::SerializeOpts::detect(Some(&content));
                let cached_norm = doc::parse(&doc::serialize_with(cached, &opts));
                if cached_norm == newdoc {
                    return None; // unchanged / our own write
                }
            }
        }
        self.cache_upsert(entry.clone(), newdoc);
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

    /// The cached parsed Document for a page (what Tine believes is on disk), if
    /// the cache is built and has it.
    fn cached_doc(&self, name: &str, kind: PageKind) -> Option<Document> {
        let guard = self.cache.read().unwrap();
        guard
            .as_ref()?
            .iter()
            .find(|(e, _)| e.kind == kind && e.name.eq_ignore_ascii_case(name))
            .map(|(_, d)| d.clone())
    }

    /// Save a page, refusing to clobber an external change. If the file on disk
    /// no longer matches what Tine last knew (another app or a Syncthing pull
    /// wrote it), returns an `AlreadyExists` "conflict" error WITHOUT writing,
    /// so the caller can surface it and keep the in-memory edits.
    pub fn save_page(&self, page: &PageDto, base_rev: Option<&str>) -> io::Result<String> {
        let path = self.path_for(&page.name, page.kind);
        if let Ok(disk_s) = fs::read_to_string(&path) {
            // A file exists. It must still match the exact bytes the editor loaded
            // (`base_rev`); if it changed underneath us (external edit / Syncthing
            // pull — possibly already folded into the cache), refuse to clobber.
            // `base_rev == None` means the editor believed the page was new, so any
            // existing file is an external creation and is likewise a conflict.
            let unchanged = base_rev.is_some_and(|rev| content_rev(&disk_s) == rev);
            if !unchanged {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
            }
        }
        self.write_page(page)?;
        Ok(self.disk_rev(&path))
    }

    /// Save a page unconditionally (the user chose "keep mine" over a conflict).
    pub fn force_save_page(&self, page: &PageDto) -> io::Result<String> {
        let path = self.path_for(&page.name, page.kind);
        self.write_page(page)?;
        Ok(self.disk_rev(&path))
    }

    /// Rev of the file now on disk (the new baseline after a write).
    fn disk_rev(&self, path: &Path) -> String {
        fs::read_to_string(path).map(|s| content_rev(&s)).unwrap_or_default()
    }

    fn write_page(&self, page: &PageDto) -> io::Result<()> {
        let doc = Document {
            pre_block: page.pre_block.clone(),
            roots: page.blocks.iter().map(dto_to_doc).collect(),
        };
        let path = self.path_for(&page.name, page.kind);
        // Reproduce the existing file's formatting (trailing newline, post-property
        // blank line, indent) so an unchanged save is byte-identical and edits
        // produce a minimal diff — critical to avoid Syncthing churn against Logseq.
        let existing = fs::read_to_string(&path).ok();
        let opts = doc::SerializeOpts::detect(existing.as_deref());
        let content = doc::serialize_with(&doc, &opts);
        // No-op save: identical bytes already on disk (e.g. focus/blur with no
        // real edit). Skip the write entirely — keep the cache current though.
        if existing.as_deref() != Some(content.as_str()) {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            atomic_write(&path, content.as_bytes())?;
        }
        // Keep the search/backlinks cache in sync without a full rebuild. For a
        // brand-new journal, derive its date_key from the name so it's recognized
        // as a dated journal by `journals_desc` (which reads from this cache) —
        // otherwise today's freshly-created page would be missing from the feed.
        let entry = self.find_entry(&page.name, page.kind).unwrap_or_else(|| {
            let date_key = if page.kind == PageKind::Journal {
                crate::date::JournalDate::from_title(&page.name).map(|d| d.ordinal_key())
            } else {
                None
            };
            PageEntry { name: page.name.clone(), kind: page.kind, date_key, path }
        });
        self.cache_upsert(entry, doc);
        Ok(())
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

/// Assign a stable uuid to every block that lacks one (called when a document
/// enters the cache). Prefers the persisted `id::` so a referenced block's node
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

/// identity and its `((ref))` target coincide; otherwise generates one. Existing
/// (non-empty) uuids are preserved — this is what carries the frontend's block
/// ids through a save so identity is stable across edits.
pub fn assign_uuids(b: &mut DocBlock) {
    if b.uuid.is_empty() {
        b.uuid = b
            .property("id")
            .unwrap_or_else(|| Uuid::new_v4().to_string());
    }
    for c in &mut b.children {
        assign_uuids(c);
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

/// Atomic write: write to a temp file in the same directory, then rename.
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("page")
    ));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}
