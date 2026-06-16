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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDto {
    pub id: String,
    pub raw: String,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub children: Vec<BlockDto>,
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
}

pub struct Graph {
    pub root: PathBuf,
    pub config: Config,
    /// In-memory cache of every parsed page, keyed implicitly by position.
    /// Built once on first whole-graph query and kept in sync by edits, so
    /// search / backlinks / `{{query}}` scan memory instead of re-reading and
    /// re-parsing the entire tree on every keystroke. `None` = not yet built.
    cache: RwLock<Option<Vec<(PageEntry, Document)>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub root: String,
    pub journals_dir: String,
    pub pages_dir: String,
    /// "now" (LATER/NOW) or "todo" (TODO/DOING) — drives the task cycle.
    pub preferred_workflow: String,
    pub shortcuts: std::collections::HashMap<String, String>,
}

impl Graph {
    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let config = fs::read_to_string(root.join("logseq").join("config.edn"))
            .map(|s| Config::parse(&s))
            .unwrap_or_default();
        Graph { root, config, cache: RwLock::new(None) }
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
        }
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
        let mut js: Vec<PageEntry> = list_md(&self.journals_path(), PageKind::Journal)
            .into_iter()
            .filter(|e| e.date_key.is_some())
            .collect();
        js.sort_by_key(|e| std::cmp::Reverse(e.date_key.unwrap_or(0)));
        js
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
                .unwrap_or_else(|| self.journals_path().join(format!("{name}.md"))),
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

    /// Load a page by name; returns `None` if it doesn't exist on disk.
    pub fn load_named(&self, name: &str, kind: PageKind) -> io::Result<Option<PageDto>> {
        match self.find_entry(name, kind) {
            Some(entry) => Ok(Some(self.load_page(&entry)?)),
            None => Ok(None),
        }
    }

    /// Load a page by entry. Served from the in-memory cache so block uuids are
    /// stable and consistent with queries / refs / the sidebar. Falls back to a
    /// disk parse for a page not yet in the cache (e.g. just created externally).
    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        let cached = self.with_pages(|pages| {
            pages
                .iter()
                .find(|(e, _)| e.kind == entry.kind && e.name.eq_ignore_ascii_case(&entry.name))
                .map(|(e, d)| page_dto(e, d))
        });
        if let Some(dto) = cached {
            return Ok(dto);
        }
        let content = fs::read_to_string(&entry.path)?;
        let mut doc = doc::parse(&content);
        for b in &mut doc.roots {
            assign_uuids(b);
        }
        Ok(page_dto(entry, &doc))
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
    }

    /// Drop one page from the cache after deleting its file.
    fn cache_remove(&self, name: &str, kind: PageKind) {
        let mut guard = self.cache.write().unwrap();
        if let Some(pages) = guard.as_mut() {
            pages.retain(|(e, _)| !(e.kind == kind && e.name.eq_ignore_ascii_case(name)));
        }
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
                if *cached == newdoc {
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
    pub fn save_page(&self, page: &PageDto) -> io::Result<()> {
        let path = self.path_for(&page.name, page.kind);
        if let (Ok(disk), Some(cached)) =
            (fs::read_to_string(&path), self.cached_doc(&page.name, page.kind))
        {
            // Compare disk against the cached doc *normalized through the same
            // serialize→parse round-trip the file went through*. The cache after
            // our own save holds the in-memory (DTO-derived) doc, whose `raw` can
            // differ trivially from what `parse(serialize(doc))` yields; comparing
            // the raw cached doc would then flag our own previous write as an
            // external edit on the next save (the spurious "changed on disk").
            let opts = doc::SerializeOpts::detect(Some(&disk));
            let cached_norm = doc::parse(&doc::serialize_with(&cached, &opts));
            if doc::parse(&disk) != cached_norm {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "conflict"));
            }
        }
        self.write_page(page)
    }

    /// Save a page unconditionally (the user chose "keep mine" over a conflict).
    pub fn force_save_page(&self, page: &PageDto) -> io::Result<()> {
        self.write_page(page)
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
        // Keep the search/backlinks cache in sync without a full rebuild.
        let entry = self.find_entry(&page.name, page.kind).unwrap_or(PageEntry {
            name: page.name.clone(),
            kind: page.kind,
            date_key: None,
            path,
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
    }
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
