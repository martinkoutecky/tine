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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub root: String,
    pub journals_dir: String,
    pub pages_dir: String,
}

impl Graph {
    /// Open a graph directory, reading `logseq/config.edn` if present.
    pub fn open(root: impl AsRef<Path>) -> Graph {
        let root = root.as_ref().to_path_buf();
        let config = fs::read_to_string(root.join("logseq").join("config.edn"))
            .map(|s| Config::parse(&s))
            .unwrap_or_default();
        Graph { root, config }
    }

    pub fn meta(&self) -> GraphMeta {
        GraphMeta {
            root: self.root.display().to_string(),
            journals_dir: self.config.journals_dir.clone(),
            pages_dir: self.config.pages_dir.clone(),
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

    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        let content = fs::read_to_string(&entry.path)?;
        let doc = doc::parse(&content);
        Ok(PageDto {
            name: entry.name.clone(),
            kind: entry.kind,
            title: entry.name.clone(),
            pre_block: doc.pre_block.clone(),
            blocks: doc.roots.iter().map(doc_to_dto).collect(),
        })
    }

    pub fn save_page(&self, page: &PageDto) -> io::Result<()> {
        let doc = Document {
            pre_block: page.pre_block.clone(),
            roots: page.blocks.iter().map(dto_to_doc).collect(),
        };
        let content = doc::serialize(&doc);
        let path = self.path_for(&page.name, page.kind);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&path, content.as_bytes())
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

fn doc_to_dto(b: &DocBlock) -> BlockDto {
    BlockDto {
        id: Uuid::new_v4().to_string(),
        raw: b.raw.clone(),
        collapsed: b.collapsed(),
        children: b.children.iter().map(doc_to_dto).collect(),
    }
}

fn dto_to_doc(b: &BlockDto) -> DocBlock {
    DocBlock {
        raw: b.raw.clone(),
        children: b.children.iter().map(dto_to_doc).collect(),
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
