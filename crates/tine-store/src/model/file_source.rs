//! Where a graph reads its text files: the disk, or a fixed set of page
//! documents held in memory ([`crate::WholeGraph::subgraph`]: an export's
//! build graph), each serialized only when the loader opens it. Only the read
//! path goes through it (listing, naming, page reads, the parse); saves, transactions, the watcher and assets are
//! disk-only, and a memory graph is never opened by a `Store`, so it never
//! reaches them. The two sources answer the same calls with the same
//! errors, so every loader step above them is the one the disk load runs.
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// The root of every memory graph: an absolute path no read leaves (a read
/// that bypassed the source would find nothing there).
pub(crate) const MEMORY_ROOT: &str = "/tine-memory-graph";

#[derive(Clone, Default)]
pub(crate) enum FileSource {
    #[default]
    Disk,
    Memory(Arc<Files>),
}

/// One memory file: its document and the modification time it carries (the
/// source file's, for an export), which recency queries read.
pub(crate) type MemoryFile = (String, Arc<tine_core::doc::Document>, Option<SystemTime>);

/// A memory graph's files by absolute path, and each directory's children
/// (a file or a directory), so a listing walks them as it walks the disk.
pub(crate) struct Files {
    files: HashMap<PathBuf, (Arc<tine_core::doc::Document>, Option<SystemTime>)>,
    children: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
}

impl Files {
    /// `files` are graph-relative paths, documents and times, under `root`.
    /// A repeated path keeps its last document. O(files × depth).
    pub(crate) fn new(root: &Path, files: impl IntoIterator<Item = MemoryFile>) -> Files {
        let mut out = Files {
            files: HashMap::new(),
            children: HashMap::from([(root.to_path_buf(), Vec::new())]),
        };
        for (rel, document, modified) in files {
            let path = root.join(rel);
            if out
                .files
                .insert(path.clone(), (document, modified))
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
        out
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
                let children = files.children.get(&dir).ok_or_else(|| not_found(&dir))?;
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
                let (document, _) = files.files.get(path).ok_or_else(|| not_found(path))?;
                let bytes = match tine_core::model::Format::from_path(path) {
                    tine_core::model::Format::Org => tine_core::org::serialize_org(document),
                    _ => tine_core::doc::serialize(document),
                }
                .into_bytes();
                Ok(Opened {
                    len: bytes.len() as u64,
                    reader: Box::new(io::Cursor::new(bytes)),
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
            FileSource::Memory(_) => Err(not_found(path)),
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
            FileSource::Memory(_) => Err(not_found(path)),
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
