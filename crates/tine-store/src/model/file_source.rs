//! Where a graph reads its text files: the disk, or a fixed set of files held
//! in memory: an export's build graph holds page documents, each serialized
//! only when the loader opens it ([`crate::WholeGraph::subgraph`]); a browser
//! graph holds file texts ([`crate::WholeGraph::from_files`]). Only the read
//! path goes through it (listing, naming, page reads, the parse); saves, transactions, the watcher and assets are
//! disk-only, and a memory graph is never opened by a `Store`, so it never
//! reaches them. The two sources answer the same calls with the same
//! errors, so every loader step above them is the one the disk load runs.
use super::Graph;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tine_core::config::Config;
use tine_core::date::JournalFormat;

/// The root of every memory graph: an absolute path no read leaves (a read
/// that bypassed the source would find nothing there).
pub(crate) const MEMORY_ROOT: &str = "/tine-memory-graph";

#[derive(Clone, Default)]
pub(crate) enum FileSource {
    #[default]
    Disk,
    Memory(Arc<Files>),
}

/// One memory file's content: its bytes, or a page document serialized when
/// it is opened (so a build graph never holds every page's text at once).
pub(crate) enum Content {
    Bytes(Arc<Vec<u8>>),
    Document(Arc<tine_core::doc::Document>),
}

/// A reader's view of shared bytes (no copy).
struct SharedBytes(Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// One memory file: its content and the modification time it carries (the
/// source file's, for an export), which recency queries read.
pub(crate) type MemoryFile = (String, Content, Option<SystemTime>);

/// The longest file name the reference volume holds (POSIX `NAME_MAX`).
const NAME_MAX: usize = 255;

/// A memory graph's tree (MEMORY-SOURCE-MODEL Q1): node 0 is the root
/// directory; a path is resolved by one walk of its components.
pub(crate) struct Files {
    root: PathBuf,
    nodes: Vec<Node>,
}

enum Node {
    File(Content, Option<SystemTime>),
    /// Children by name, in the order they were given (the listing's); each
    /// name held once. Boxed, so a file's node stays small.
    Dir(Box<IndexMap<Box<str>, usize>>),
}

/// A memory file name: one the reference volume (case-sensitive POSIX)
/// holds, and that no platform separator splits.
fn canonical_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name != "."
        && name != ".."
        && !name.contains('\0')
        && !name.chars().any(std::path::is_separator)
}

/// Whether `path` is spelled as a directory: ending in `/` or `/.`.
fn names_a_directory(path: &Path) -> bool {
    (path.as_os_str().to_str()).is_some_and(|raw| raw.ends_with('/') || raw.ends_with("/."))
}

impl Files {
    /// `files` are graph-relative keys, contents and times, under `root`.
    /// A key is canonical names joined by single `/` ([`canonical_name`]);
    /// any other spelling, a key given twice and a key that is also another
    /// key's directory are refused (`InvalidInput`). O(total key bytes).
    pub(crate) fn new(
        root: &Path,
        files: impl IntoIterator<Item = MemoryFile>,
    ) -> io::Result<Files> {
        let mut out = Files {
            root: root.to_path_buf(),
            nodes: vec![Node::Dir(Box::default())],
        };
        let invalid = |rel: &str, why: &str| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{why}: {rel:?}"))
        };
        for (rel, content, modified) in files {
            let mut file = Some(Node::File(content, modified));
            let mut names = rel.split('/').peekable();
            let mut at = 0;
            while let Some(name) = names.next() {
                if !canonical_name(name) {
                    return Err(invalid(&rel, "not a graph-relative file path"));
                }
                let last = names.peek().is_none();
                at = match (out.child(at, name), last) {
                    (None, false) => out.add(at, name, Node::Dir(Box::default())),
                    (None, true) => out.add(at, name, file.take().expect("one file per key")),
                    (Some(i), false) if out.is_dir(i) => i,
                    (Some(i), true) if !out.is_dir(i) => {
                        return Err(invalid(&rel, "a path is given twice"))
                    }
                    (Some(_), _) => {
                        return Err(invalid(&rel, "a path is both a file and a directory"))
                    }
                };
            }
        }
        Ok(out)
    }

    fn is_dir(&self, at: usize) -> bool {
        matches!(self.nodes[at], Node::Dir(..))
    }

    fn child(&self, at: usize, name: &str) -> Option<usize> {
        match &self.nodes[at] {
            Node::Dir(children) => children.get(name).copied(),
            Node::File(..) => None,
        }
    }

    fn add(&mut self, at: usize, name: &str, node: Node) -> usize {
        let id = self.nodes.len();
        self.nodes.push(node);
        let Node::Dir(children) = &mut self.nodes[at] else {
            unreachable!("a child is added to a directory")
        };
        children.insert(name.into(), id);
        id
    }

    /// The node `path` names, by one walk from the root, with the errors the
    /// reference gives: a path through a file, or a file spelled as a
    /// directory (ending in `/` or `/.`), is not a directory; a name longer
    /// than [`NAME_MAX`] is an invalid file name; else not found.
    /// O(path bytes).
    fn node(&self, path: &Path) -> io::Result<usize> {
        let rel = path.strip_prefix(&self.root).map_err(|_| not_found(path))?;
        let mut at = 0;
        for part in rel.components() {
            let Node::Dir(children) = &self.nodes[at] else {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("{} is under a file", path.display()),
                ));
            };
            let std::path::Component::Normal(name) = part else {
                return Err(not_found(path));
            };
            let name = name.to_str().ok_or_else(|| not_found(path))?;
            if name.len() > NAME_MAX {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidFilename,
                    format!("a name in {} is too long", path.display()),
                ));
            }
            at = *children.get(name).ok_or_else(|| not_found(path))?;
        }
        if !self.is_dir(at) && names_a_directory(path) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is not a directory", path.display()),
            ));
        }
        Ok(at)
    }

    /// The file at `path`: a directory there is not one.
    fn file(&self, path: &Path) -> io::Result<(&Content, Option<SystemTime>)> {
        self.file_at(self.node(path)?, path)
    }

    fn file_at(&self, at: usize, path: &Path) -> io::Result<(&Content, Option<SystemTime>)> {
        match &self.nodes[at] {
            Node::File(content, modified) => Ok((content, *modified)),
            Node::Dir(..) => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("{} is a directory", path.display()),
            )),
        }
    }

    /// A parse read of the file at `path` ([`super::read_parse_input_in`]):
    /// held bytes are checked where they are and answered as a view of
    /// them; a document's serialization is the text (MEMORY-SOURCE-MODEL Q3:
    /// no copy of the file's bytes).
    fn parse_text(&self, path: &Path) -> io::Result<Cow<'_, str>> {
        let (content, _) = self.file(path)?;
        #[cfg(feature = "test-faults")]
        crate::cost_counters::full_read();
        match content {
            Content::Bytes(bytes) => super::validate_parse_text(bytes, path).map(Cow::Borrowed),
            Content::Document(document) => {
                let text = serialized(document, path);
                super::validate_parse_text(text.as_bytes(), path)?;
                Ok(Cow::Owned(text))
            }
        }
    }
}

/// A page document as its file's text.
fn serialized(document: &tine_core::doc::Document, path: &Path) -> String {
    match tine_core::model::Format::from_path(path) {
        tine_core::model::Format::Org => tine_core::org::serialize_org(document),
        _ => tine_core::doc::serialize(document),
    }
}

pub(crate) enum EntryKind {
    File,
    Dir,
    Other,
}

/// One directory entry. A disk entry keeps its `DirEntry`, whose metadata
/// needs no file open on Windows (the launch listing's stamps); a memory
/// entry its node.
pub(crate) struct DirItem {
    pub(crate) path: PathBuf,
    pub(crate) kind: EntryKind,
    origin: Origin,
}

enum Origin {
    Disk(fs::DirEntry),
    Memory(usize),
}

/// An opened file: its reader and its length when opened.
pub(crate) struct Opened {
    pub(crate) reader: Box<dyn io::Read + Send>,
    pub(crate) len: u64,
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} is not in this graph", path.display()),
    )
}

impl FileSource {
    /// The entries of `dir`, read as the caller iterates; an entry whose kind
    /// cannot be read is an error for that entry's path (a whole-directory
    /// failure is the outer error).
    pub(crate) fn read_dir<'a>(
        &'a self,
        dir: &Path,
    ) -> io::Result<Box<dyn Iterator<Item = Result<DirItem, (PathBuf, io::Error)>> + 'a>> {
        let dir = dir.to_path_buf();
        match self {
            FileSource::Disk => Ok(Box::new(fs::read_dir(&dir)?.map(move |entry| {
                let entry = entry.map_err(|error| (dir.to_path_buf(), error))?;
                let path = entry.path();
                let kind = match entry.file_type() {
                    Ok(kind) if kind.is_file() => EntryKind::File,
                    Ok(kind) if kind.is_dir() => EntryKind::Dir,
                    Ok(_) => EntryKind::Other,
                    Err(error) => return Err((path, error)),
                };
                Ok(DirItem {
                    path,
                    kind,
                    origin: Origin::Disk(entry),
                })
            }))),
            FileSource::Memory(files) => match &files.nodes[files.node(&dir)?] {
                Node::Dir(children) => Ok(Box::new(children.iter().map(move |(name, id)| {
                    Ok(DirItem {
                        path: dir.join(&**name),
                        kind: if files.is_dir(*id) {
                            EntryKind::Dir
                        } else {
                            EntryKind::File
                        },
                        origin: Origin::Memory(*id),
                    })
                }))),
                Node::File(..) => Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("{} is not a directory", dir.display()),
                )),
            },
        }
    }

    /// A listed entry's stamp: a disk entry's metadata, or a memory file's
    /// carried time.
    pub(crate) fn stamp(&self, item: &DirItem) -> Option<crate::watch::Stamp> {
        match self {
            FileSource::Disk => {
                let Origin::Disk(entry) = &item.origin else {
                    return None;
                };
                crate::watch::stamp_from_metadata(&entry.metadata().ok()?)
            }
            FileSource::Memory(files) => {
                let Origin::Memory(at) = item.origin else {
                    return None;
                };
                let (_, modified) = files.file_at(at, &item.path).ok()?;
                Some(crate::watch::Stamp::in_memory(modified))
            }
        }
    }

    pub(crate) fn open(&self, path: &Path) -> io::Result<Opened> {
        match self {
            FileSource::Disk => {
                let file = fs::File::open(path)?;
                Ok(Opened {
                    len: file.metadata()?.len(),
                    reader: Box::new(file),
                })
            }
            FileSource::Memory(files) => Ok(match files.file(path)?.0 {
                Content::Bytes(bytes) => Opened {
                    len: bytes.len() as u64,
                    reader: Box::new(io::Cursor::new(SharedBytes(Arc::clone(bytes)))),
                },
                Content::Document(document) => {
                    let bytes = serialized(document, path).into_bytes();
                    Opened {
                        len: bytes.len() as u64,
                        reader: Box::new(io::Cursor::new(bytes)),
                    }
                }
            }),
        }
    }

    /// What `path` is, without following a symlink (`symlink_metadata`: a
    /// spelling ending in `/` or `/.` must name a directory).
    pub(crate) fn lstat(&self, path: &Path) -> io::Result<EntryKind> {
        match self {
            FileSource::Disk => {
                let metadata = fs::symlink_metadata(path)?;
                Ok(if metadata.is_file() {
                    EntryKind::File
                } else if metadata.is_dir() {
                    EntryKind::Dir
                } else {
                    EntryKind::Other
                })
            }
            FileSource::Memory(files) => Ok(if files.is_dir(files.node(path)?) {
                EntryKind::Dir
            } else {
                EntryKind::File
            }),
        }
    }

    /// Whether `path` is a regular file of the graph at `root`: not a
    /// symlink, and on disk inside the root once its ancestors resolve.
    pub(crate) fn graph_file(&self, root: &Path, path: &Path) -> io::Result<bool> {
        match self {
            FileSource::Disk => {
                let metadata = fs::symlink_metadata(path)?;
                Ok(metadata.is_file() && super::path_stays_within_root(root, path))
            }
            FileSource::Memory(files) => Ok(!files.is_dir(files.node(path)?)),
        }
    }

    /// When `path` was created, if the filesystem says. A memory file has
    /// no creation after the graph was built.
    pub(crate) fn created(&self, path: &Path) -> Option<SystemTime> {
        match self {
            FileSource::Disk => fs::symlink_metadata(path)
                .and_then(|meta| meta.created())
                .ok(),
            FileSource::Memory(_) => None,
        }
    }

    /// A parse read of `path` from memory, if this is a memory source.
    pub(crate) fn memory_parse_text(&self, path: &Path) -> Option<io::Result<Cow<'_, str>>> {
        match self {
            FileSource::Disk => None,
            FileSource::Memory(files) => Some(files.parse_text(path)),
        }
    }

    /// A file's modification time (a memory file's carried one).
    pub(crate) fn modified(&self, path: &Path) -> Option<SystemTime> {
        match self {
            FileSource::Disk => fs::metadata(path).and_then(|meta| meta.modified()).ok(),
            FileSource::Memory(files) => files.file(path).ok()?.1,
        }
    }
}

impl Graph {
    /// A graph of `files` held in memory under the config among them
    /// (`WholeGraph::from_files`), read and refused as a disk open reads and
    /// refuses its own ([`Graph::check_layout`]; a memory graph has no links
    /// for the disk's containment check).
    pub(crate) fn in_memory_with_config(files: Vec<MemoryFile>) -> io::Result<Graph> {
        let root = PathBuf::from(MEMORY_ROOT);
        let files = Files::new(&root, files)?;
        let graph = Self::open_from(root, FileSource::Memory(Arc::new(files)));
        graph.check_layout()?;
        Ok(graph)
    }

    /// A graph of `files` (graph-relative path, document, modification time)
    /// held in memory under `config`: the load, the naming and every page
    /// read take them from there ([`FileSource::Memory`]). Such a graph is
    /// only read (`WholeGraph::subgraph`); nothing writes it.
    pub(crate) fn in_memory(
        files: Vec<MemoryFile>,
        config: Config,
        journal_format: JournalFormat,
    ) -> io::Result<Graph> {
        let root = PathBuf::from(MEMORY_ROOT);
        let files = Files::new(&root, files)?;
        Ok(Self::empty_with_config(
            root,
            FileSource::Memory(Arc::new(files)),
            config,
            journal_format,
        ))
    }
}

impl Graph {
    /// [`read_parse_bytes_in`] this graph's [`FileSource`].
    pub(crate) fn read_parse_bytes_sourced(&self, path: &Path) -> io::Result<(Vec<u8>, u64)> {
        super::read_parse_bytes_in(&self.source, path)
    }

    /// [`read_parse_input_in`] this graph's [`FileSource`].
    pub(crate) fn read_parse_input(&self, path: &Path) -> io::Result<Cow<'_, str>> {
        super::read_parse_input_in(&self.source, path)
    }
}
