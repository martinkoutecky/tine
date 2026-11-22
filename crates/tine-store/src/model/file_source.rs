//! Where a graph reads its text files: the disk, or a fixed set of files held
//! in memory: an export's build graph holds page documents, each serialized
//! only when the loader opens it ([`crate::WholeGraph::subgraph`]); a browser
//! graph holds file texts ([`crate::WholeGraph::from_files`]). Only the read
//! path goes through it (listing, naming, page reads, the parse); saves, transactions, the watcher and assets are
//! disk-only, and a memory graph is never opened by a `Store`, so it never
//! reaches them. The two sources answer the same calls with the same
//! errors, so every loader step above them is the one the disk load runs.
use super::Graph;
use std::collections::HashMap;
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
    Bytes(Arc<[u8]>),
    Document(Arc<tine_core::doc::Document>),
}

/// One memory file: its content and the modification time it carries (the
/// source file's, for an export), which recency queries read.
pub(crate) type MemoryFile = (String, Content, Option<SystemTime>);

/// A memory graph's files by absolute path, and each directory's children
/// (a file or a directory), so a listing walks them as it walks the disk.
pub(crate) struct Files {
    files: HashMap<PathBuf, (Content, Option<SystemTime>)>,
    children: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
}

impl Files {
    /// `files` are graph-relative paths, contents and times, under `root`.
    /// A repeated path keeps its last content. Refuses a path that is not
    /// relative and plain (`InvalidInput`: no directory holds it), and one
    /// that is both a file and a directory (no directory holds both).
    /// O(files × depth).
    pub(crate) fn new(
        root: &Path,
        files: impl IntoIterator<Item = MemoryFile>,
    ) -> io::Result<Files> {
        let mut out = Files {
            files: HashMap::new(),
            children: HashMap::from([(root.to_path_buf(), Vec::new())]),
        };
        let invalid = |rel: &str, why: &str| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{why}: {rel:?}"))
        };
        for (rel, content, modified) in files {
            let plain = !rel.is_empty()
                && !rel.contains('\\')
                && (Path::new(&rel).components())
                    .all(|c| matches!(c, std::path::Component::Normal(_)));
            if !plain {
                return Err(invalid(&rel, "not a graph-relative file path"));
            }
            let path = root.join(&rel);
            if out.children.contains_key(&path)
                || (path.ancestors().skip(1)).any(|dir| out.files.contains_key(dir))
            {
                return Err(invalid(&rel, "a path is both a file and a directory"));
            }
            if out
                .files
                .insert(path.clone(), (content, modified))
                .is_some()
            {
                continue;
            }
            // Each file, and each directory when first seen, joins its
            // parent's children once.
            let mut child = (path.clone(), false);
            for dir in path.ancestors().skip(1) {
                let known = out.children.contains_key(dir);
                out.children
                    .entry(dir.to_path_buf())
                    .or_default()
                    .push(child);
                if known {
                    break;
                }
                child = (dir.to_path_buf(), true);
            }
        }
        Ok(out)
    }

    /// The error a disk read of `path` gives when no file is there: through a
    /// file ancestor it is not a directory, else not found.
    fn absent(&self, path: &Path) -> io::Error {
        if (path.ancestors().skip(1)).any(|dir| self.files.contains_key(dir)) {
            return io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("{} is under a file", path.display()),
            );
        }
        not_found(path)
    }
}

pub(crate) enum EntryKind {
    File,
    Dir,
    Other,
}

/// One directory entry. A disk entry keeps its `DirEntry`, whose metadata
/// needs no file open on Windows (the launch listing's stamps).
pub(crate) struct DirItem {
    pub(crate) path: PathBuf,
    pub(crate) kind: EntryKind,
    disk: Option<fs::DirEntry>,
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
                    disk: Some(entry),
                })
            }))),
            FileSource::Memory(files) => {
                let children = files.children.get(&dir).ok_or_else(|| {
                    if files.files.contains_key(&dir) {
                        io::Error::new(io::ErrorKind::NotADirectory, "not a directory")
                    } else {
                        files.absent(&dir)
                    }
                })?;
                Ok(Box::new(children.iter().map(|(path, dir)| {
                    Ok(DirItem {
                        path: path.clone(),
                        kind: if *dir {
                            EntryKind::Dir
                        } else {
                            EntryKind::File
                        },
                        disk: None,
                    })
                })))
            }
        }
    }

    /// A listed entry's stamp: a disk entry's metadata, or a memory file's
    /// carried time.
    pub(crate) fn stamp(&self, item: &DirItem) -> Option<crate::watch::Stamp> {
        match self {
            FileSource::Disk => {
                let metadata = item.disk.as_ref()?.metadata().ok()?;
                crate::watch::stamp_from_metadata(&metadata)
            }
            FileSource::Memory(files) => {
                let (_, modified) = files.files.get(&item.path)?;
                Some(crate::watch::Stamp::in_memory(*modified))
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
            FileSource::Memory(files) => {
                if files.children.contains_key(path) {
                    return Err(io::Error::new(
                        io::ErrorKind::IsADirectory,
                        format!("{} is a directory", path.display()),
                    ));
                }
                let (content, _) = files.files.get(path).ok_or_else(|| files.absent(path))?;
                Ok(match content {
                    Content::Bytes(bytes) => Opened {
                        len: bytes.len() as u64,
                        reader: Box::new(io::Cursor::new(Arc::clone(bytes))),
                    },
                    Content::Document(document) => {
                        let bytes = match tine_core::model::Format::from_path(path) {
                            tine_core::model::Format::Org => {
                                tine_core::org::serialize_org(document)
                            }
                            _ => tine_core::doc::serialize(document),
                        }
                        .into_bytes();
                        Opened {
                            len: bytes.len() as u64,
                            reader: Box::new(io::Cursor::new(bytes)),
                        }
                    }
                })
            }
        }
    }

    /// What `path` is, without following a symlink.
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
            FileSource::Memory(files) if files.files.contains_key(path) => Ok(EntryKind::File),
            FileSource::Memory(files) if files.children.contains_key(path) => Ok(EntryKind::Dir),
            FileSource::Memory(files) => Err(files.absent(path)),
        }
    }

    /// A file's modification time (a memory file's carried one).
    /// Whether `path` is a regular file of the graph at `root`: not a
    /// symlink, and on disk inside the root once its ancestors resolve.
    pub(crate) fn graph_file(&self, root: &Path, path: &Path) -> io::Result<bool> {
        match self {
            FileSource::Disk => {
                let metadata = fs::symlink_metadata(path)?;
                Ok(metadata.is_file() && super::path_stays_within_root(root, path))
            }
            FileSource::Memory(files) if files.files.contains_key(path) => Ok(true),
            FileSource::Memory(files) if files.children.contains_key(path) => Ok(false),
            FileSource::Memory(files) => Err(files.absent(path)),
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

    pub(crate) fn modified(&self, path: &Path) -> Option<SystemTime> {
        match self {
            FileSource::Disk => fs::metadata(path).and_then(|meta| meta.modified()).ok(),
            FileSource::Memory(files) => files.files.get(path)?.1,
        }
    }
}

impl Graph {
    /// A graph of `files` held in memory under the config among them
    /// (`WholeGraph::from_files`), read as a disk open reads its own and
    /// refused for the configured directories' spelling as a disk open is
    /// (a memory graph has no symlinks for the disk's containment check).
    pub(crate) fn in_memory_with_config(files: Vec<MemoryFile>) -> io::Result<Graph> {
        let root = PathBuf::from(MEMORY_ROOT);
        let files = Files::new(&root, files)?;
        let graph = Self::open_from(root, FileSource::Memory(Arc::new(files)));
        super::managed_dir_spelling(&graph.config.journals_dir, "journals")?;
        super::managed_dir_spelling(&graph.config.pages_dir, "pages")?;
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
    pub(crate) fn read_parse_input(&self, path: &Path) -> io::Result<String> {
        super::read_parse_input_in(&self.source, path)
    }
}
