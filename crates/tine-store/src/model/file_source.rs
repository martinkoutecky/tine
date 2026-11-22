//! Where a graph reads its text files: the disk, or a fixed set of files held
//! in memory ([`crate::WholeGraph::from_pages`]: an export's build graph, and
//! the browser). Only the read path goes through it (listing, naming, page
//! reads, the parse); saves, transactions, the watcher and assets are
//! disk-only, and a memory graph is never opened by a `Store`, so it never
//! reaches them. The two sources answer the same calls with the same
//! errors, so every loader step above them is the one the disk load runs.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The root of every memory graph: an absolute path no read leaves (a read
/// that bypassed the source would find nothing there).
pub(crate) const MEMORY_ROOT: &str = "/tine-memory-graph";

#[derive(Clone, Default)]
pub(crate) enum FileSource {
    #[default]
    Disk,
    Memory(Arc<Files>),
}

/// A memory graph's files by absolute path, and each directory's children
/// (a file or a directory), so a listing walks them as it walks the disk.
pub(crate) struct Files {
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    children: BTreeMap<PathBuf, Vec<(PathBuf, bool)>>,
}

impl Files {
    /// `files` are graph-relative paths and their bytes, under `root`.
    pub(crate) fn new(root: &Path, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Files {
        let mut out = Files {
            files: BTreeMap::new(),
            children: BTreeMap::from([(root.to_path_buf(), Vec::new())]),
        };
        for (rel, bytes) in files {
            let path = root.join(rel);
            let mut child = (path.clone(), false);
            for dir in path.ancestors().skip(1) {
                let known = out.children.contains_key(dir);
                let siblings = out.children.entry(dir.to_path_buf()).or_default();
                if !siblings.contains(&child) {
                    siblings.push(child);
                }
                if known || dir == root {
                    break;
                }
                child = (dir.to_path_buf(), true);
            }
            out.files.insert(path, bytes.into());
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
    /// The entries of `dir`; an entry whose kind cannot be read is an error
    /// for that entry's path (a whole-directory failure is the outer error).
    pub(crate) fn read_dir(
        &self,
        dir: &Path,
    ) -> io::Result<Vec<Result<DirItem, (PathBuf, io::Error)>>> {
        match self {
            FileSource::Disk => Ok(fs::read_dir(dir)?
                .map(|entry| {
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
                })
                .collect()),
            FileSource::Memory(files) => {
                Ok((files.children.get(dir).ok_or_else(|| not_found(dir))?)
                    .iter()
                    .map(|(path, dir)| {
                        Ok(DirItem {
                            path: path.clone(),
                            kind: if *dir {
                                EntryKind::Dir
                            } else {
                                EntryKind::File
                            },
                            disk: None,
                        })
                    })
                    .collect())
            }
        }
    }

    /// A listed entry's stamp; a memory file has none (it never changes).
    pub(crate) fn stamp(&self, item: &DirItem) -> Option<crate::watch::Stamp> {
        let metadata = item.disk.as_ref()?.metadata().ok()?;
        crate::watch::stamp_from_metadata(&metadata)
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
                let bytes = files.files.get(path).ok_or_else(|| not_found(path))?;
                Ok(Opened {
                    len: bytes.len() as u64,
                    reader: Box::new(io::Cursor::new(Arc::clone(bytes))),
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

    /// A file's modification time; a memory file has none.
    pub(crate) fn modified(&self, path: &Path) -> Option<std::time::SystemTime> {
        match self {
            FileSource::Disk => fs::metadata(path).and_then(|meta| meta.modified()).ok(),
            FileSource::Memory(_) => None,
        }
    }
}
