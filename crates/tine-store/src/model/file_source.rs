//! Where a graph reads its text files: the disk, or a fixed set of files held
//! in memory. A static export can hold documents and serialize each one when
//! opened. Source snapshots retain the same serialized bytes as the build
//! graph. A browser graph holds file text ([`crate::WholeGraph::from_files`]). Only the read
//! path goes through it (listing, naming, page reads, the parse); saves, transactions, the watcher and assets are
//! disk-only, and a memory graph is never opened by a `Store`, so it never
//! reaches them. An editable browser graph's files change only through
//! [`Graph::replace_memory_files`], all or none. The two sources answer the same calls with the same
//! errors, so every loader step above them is the one the disk load runs.
use super::Graph;
use indexmap::IndexMap;
use std::borrow::Cow;
use std::collections::HashSet;
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

/// The graph-relative config file an open reads.
pub(crate) const CONFIG_FILE: &str = "logseq/config.edn";

/// Whether a change at the key `rel` can move what an open's config read
/// ([`super::read_config_in`]) answers: the key is a path that read looks up
/// (the config file or its iCloud placeholder), lies under one (a child of a
/// directory there), or is one's ancestor (a `logseq` file).
pub(crate) fn affects_config_lookup(rel: &str) -> bool {
    fn nested(inner: &str, outer: &str) -> bool {
        (inner.strip_prefix(outer)).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
    // Keys are spelled with `/` on every platform; a `PathBuf` is not.
    let placeholder = super::config_placeholder(Path::new(CONFIG_FILE));
    let name = (placeholder.file_name().and_then(|name| name.to_str())).expect("a UTF-8 constant");
    let (dir, _) = CONFIG_FILE.rsplit_once('/').expect("a nested constant");
    let placeholder = format!("{dir}/{name}");
    let lookups = [CONFIG_FILE, placeholder.as_str()];
    (lookups.iter()).any(|lookup| nested(rel, lookup) || nested(lookup, rel))
}

#[derive(Clone, Default)]
pub(crate) enum FileSource {
    #[default]
    Disk,
    Memory(Arc<Files>),
}

/// One memory file's content: its bytes, or a page document serialized on read.
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

/// One memory-file change ([`Files::replace`]): a key and the content and
/// time it now holds, or `None` to delete the file.
pub(crate) type MemoryChange = (String, Option<(Content, Option<SystemTime>)>);

/// One key a [`Graph::replace_memory_files`] changed: whether a file was there
/// before, and whether the file there is graph text under the resulting config
/// (the one answer its page validation and its publication read).
pub(crate) struct Replaced {
    pub(crate) rel: String,
    pub(crate) existed: bool,
    pub(crate) page: bool,
}

/// The longest file name the reference volume holds (POSIX `NAME_MAX`).
const NAME_MAX: usize = 255;

/// A memory graph's tree (MEMORY-SOURCE-MODEL Q1): node 0 is the root
/// directory; a path is resolved by one walk of its components.
pub(crate) struct Files {
    root: PathBuf,
    nodes: Vec<Node>,
    /// Nodes a deletion freed, reused before the arena grows.
    free: Vec<usize>,
}

enum Node {
    File(Content, Option<SystemTime>),
    /// Children by name, in the order they were given (the listing's); each
    /// name held once. Boxed, so a file's node stays small.
    Dir(Box<IndexMap<Box<str>, usize>>),
}

impl Node {
    /// What a freed node holds until [`Files::add`] reuses it: no name
    /// reaches it.
    fn freed() -> Node {
        Node::File(Content::Bytes(Arc::default()), None)
    }
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

/// Tine's names in a graph path's spelling: split on the platform's
/// separators (`/`, and on Windows `\` too, as `Path` splits), empty and `.`
/// names dropped. `x//./pages` names `x` and `pages`. On Windows a name
/// holding `:` is refused (`InvalidInput`) before anything is probed: the
/// platform reads `C:` as a drive's current directory and `name:stream` as
/// an alternate data stream, so the name would not be the graph's entry.
pub(crate) fn tine_names(spelling: &str) -> io::Result<impl Iterator<Item = &str> + Clone> {
    if cfg!(windows) && spelling.contains(':') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{spelling:?} has a name with `:`, which Windows reads as a drive or a stream"),
        ));
    }
    Ok((spelling.split(std::path::is_separator)).filter(|name| !name.is_empty() && *name != "."))
}

/// Whether `spelling` is spelled as a directory: ending in a separator or a
/// `.` name (`x/`, `x/.`, `x//./`).
fn spelled_as_directory(mut spelling: &str) -> bool {
    let whole = spelling.len();
    loop {
        let trimmed = spelling.trim_end_matches(std::path::is_separator);
        match trimmed.strip_suffix('.') {
            Some(rest) if rest.is_empty() || rest.ends_with(std::path::is_separator) => {
                spelling = rest
            }
            _ => return trimmed.len() != whole,
        }
    }
}

/// What a lookup of a spelling's names found ([`Files::locate`],
/// [`locate_deepest`]): how many of its `names` exist (`depth`), and what is
/// at the deepest existing prefix (`None`: a directory nothing probed, the
/// root or an ancestor the platform would not answer for).
pub(crate) struct Located<T> {
    depth: usize,
    names: usize,
    at: Option<T>,
}

/// What kind of entry a graph-relative spelling names, decided from its
/// [`Located`] answer alone: the one rule both sources read a path by (Tine's
/// names, [`tine_names`], never the platform's normalization of a joined
/// path; MEMORY-SOURCE-MODEL Q2).
///
/// - All names exist, but the entry is not a directory (`is_dir`) while
///   spelled as one (`as_directory`: `x/`, `x/.`): not a directory. Unix says
///   so itself (ENOTDIR); Windows, and std's join onto a verbatim (`\\?\`)
///   root, drop the trailing `/` and `.`.
/// - The deepest existing prefix is not a directory: the spelling is under a
///   file, not a directory (Unix says ENOTDIR itself; Windows says
///   ERROR_PATH_NOT_FOUND).
/// - Otherwise all names exist (the entry) or not (not found).
///
/// An error of the lookup itself is the answer. Free: no probe, no allocation.
pub(crate) fn spelled_kind<T>(
    as_directory: bool,
    located: io::Result<Located<T>>,
    is_dir: impl Fn(&T) -> bool,
    shown: &Path,
) -> io::Result<T> {
    let not_a_directory = |why: &str| {
        io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} {why}", shown.display()),
        )
    };
    let Located { depth, names, at } = located?;
    match at {
        Some(at) if depth == names && as_directory && !is_dir(&at) => {
            Err(not_a_directory("is not a directory"))
        }
        Some(at) if depth == names => Ok(at),
        Some(at) if !is_dir(&at) => Err(not_a_directory("is under a file")),
        _ => Err(not_found(shown)),
    }
}

/// `root` joined with `names`, built once as one string: the root's own
/// spelling, then each name after the platform separator. Never
/// `PathBuf::push` per name, which re-parses a verbatim (`\\?\`) root on every
/// push. Returns the path and how many names it holds. O(path bytes).
fn spelled_path<'a>(root: &Path, names: impl Iterator<Item = &'a str>) -> (PathBuf, usize) {
    let root = root.as_os_str();
    let mut path = std::ffi::OsString::with_capacity(root.len() + 256);
    path.push(root);
    let ends =
        (root.as_encoded_bytes().last()).is_some_and(|&b| std::path::is_separator(b as char));
    let mut separate = !root.is_empty() && !ends;
    let mut count = 0;
    for name in names {
        #[cfg(test)]
        names_tests::NAME_VISITS.with(|visits| visits.set(visits.get() + 1));
        if separate {
            path.push(std::path::MAIN_SEPARATOR_STR);
        }
        path.push(name);
        (separate, count) = (true, count + 1);
    }
    (PathBuf::from(path), count)
}

/// The disk's lookup of a spelling's `names` names below the graph root:
/// `whole` is the answer for the whole path ([`spelled_path`]); only after
/// its `NotFound`, a binary search for the deepest existing prefix, asking
/// `prefix` for the answer at a depth (the whole path's first that many
/// names). A prefix of an existing path exists, so existence is monotone in
/// the depth: the root (depth 0) is taken to exist and is never asked, the
/// whole path does not. A prefix that is not a directory ends the search
/// (nothing exists below it); one that answers another error settles nothing
/// (the spelling stays not found). Cost: at most ⌈log₂ names⌉ + 1 probes,
/// each of at most `names` components, whatever the platform sends a path
/// as (an SMB open carries the whole share-relative path); no allocation.
fn locate_deepest<T>(
    names: usize,
    whole: io::Result<T>,
    is_dir: impl Fn(&T) -> bool,
    mut prefix: impl FnMut(usize) -> io::Result<T>,
) -> io::Result<Located<T>> {
    match whole {
        Ok(at) => {
            return Ok(Located {
                depth: names,
                names,
                at: Some(at),
            })
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        Err(_) => {}
    }
    // `low` exists (`at` is its answer; `None` for the root), `high` does not.
    let (mut low, mut high, mut at) = (0, names, None);
    while high - low > 1 {
        let depth = low + (high - low) / 2;
        match prefix(depth) {
            Ok(found) => {
                let deeper = is_dir(&found);
                (low, at) = (depth, Some(found));
                if !deeper {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => high = depth,
            Err(_) => {
                return Ok(Located {
                    depth,
                    names,
                    at: None,
                })
            }
        }
    }
    Ok(Located {
        depth: low,
        names,
        at,
    })
}

/// One disk metadata call on `path`: following a link (`metadata`), as a
/// lookup through an ancestor does, or not (`symlink_metadata`).
fn disk_kind(path: &Path, follow: bool) -> io::Result<EntryKind> {
    #[cfg(test)]
    names_tests::DISK_PROBES.with(|count| count.set(count.get() + 1));
    let metadata = if follow {
        fs::metadata(path)?
    } else {
        fs::symlink_metadata(path)?
    };
    Ok(entry_kind(&metadata))
}

fn entry_kind(metadata: &fs::Metadata) -> EntryKind {
    if metadata.is_file() {
        EntryKind::File
    } else if metadata.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::Other
    }
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
            free: Vec::new(),
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
        let id = match self.free.pop() {
            Some(id) => {
                self.nodes[id] = node;
                id
            }
            None => {
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        };
        let Node::Dir(children) = &mut self.nodes[at] else {
            unreachable!("a child is added to a directory")
        };
        children.insert(name.into(), id);
        id
    }

    /// Apply `changes`, all or none, under Q1's key rules: a key is
    /// canonical and named once; no key is another key's ancestor, in the
    /// batch or in the tree (a write neither passes through a file nor
    /// replaces a directory); a deleted key is a file (`NotFound`). Other
    /// refusals are `InvalidInput`. A deleted file's directories that become
    /// empty go: a reference listing holds no empty directory. Returns, per
    /// change in order, its inverse (`None` where the key was absent):
    /// applying them restores the files. O(changed key bytes).
    pub(crate) fn replace(&mut self, changes: Vec<MemoryChange>) -> io::Result<Vec<MemoryChange>> {
        let invalid = |rel: &str, why: &str| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{why}: {rel:?}"))
        };
        let both = "a path is both a file and a directory";
        let (mut keys, mut dirs) = (HashSet::new(), HashSet::new());
        for (rel, change) in &changes {
            if !rel.split('/').all(canonical_name) {
                return Err(invalid(rel, "not a graph-relative file path"));
            }
            if !keys.insert(rel.as_str()) {
                return Err(invalid(rel, "a path is given twice"));
            }
            let (mut at, mut end) = (Some(0), 0);
            let mut names = rel.split('/').peekable();
            while let Some(name) = names.next() {
                let last = names.peek().is_none();
                at = at.and_then(|i| self.child(i, name));
                if at.is_some_and(|i| self.is_dir(i) == last) {
                    return Err(invalid(rel, both));
                }
                end += name.len();
                if !last {
                    dirs.insert(&rel[..end]);
                    end += 1;
                }
            }
            if at.is_none() && change.is_none() {
                return Err(not_found(&self.root.join(rel)));
            }
        }
        if let Some(rel) = keys.iter().find(|key| dirs.contains(*key)) {
            return Err(invalid(rel, both));
        }
        Ok((changes.into_iter())
            .map(|(rel, change)| {
                let before = self.put(&rel, change);
                (rel, before)
            })
            .collect())
    }

    /// Write or delete the file at a key [`Self::replace`] admitted; the
    /// file it held.
    fn put(
        &mut self,
        rel: &str,
        change: Option<(Content, Option<SystemTime>)>,
    ) -> Option<(Content, Option<SystemTime>)> {
        let names: Vec<&str> = rel.split('/').collect();
        let (mut dirs, mut at) = (Vec::with_capacity(names.len()), 0);
        for (depth, name) in names.iter().enumerate() {
            dirs.push(at);
            at = match (self.child(at, name), depth + 1 == names.len()) {
                (Some(i), _) => i,
                (None, false) => self.add(at, name, Node::Dir(Box::default())),
                (None, true) => {
                    let (content, modified) = change.expect("a deleted key exists");
                    self.add(at, name, Node::File(content, modified));
                    return None;
                }
            };
        }
        let deleted = change.is_none();
        let node = match change {
            Some((content, modified)) => Node::File(content, modified),
            None => Node::freed(),
        };
        let Node::File(content, modified) = std::mem::replace(&mut self.nodes[at], node) else {
            unreachable!("an admitted key names a file")
        };
        if deleted {
            self.free.push(at);
            // Unlink the name, then each directory the deletion empties.
            for (depth, &dir) in dirs.iter().enumerate().rev() {
                let Node::Dir(children) = &mut self.nodes[dir] else {
                    unreachable!("a key's ancestors are directories")
                };
                // O(1): the last sibling takes the slot. No answer reads the
                // listing's order (a disk listing has none either).
                children.swap_remove(names[depth]);
                if !children.is_empty() || dir == 0 {
                    break;
                }
                self.nodes[dir] = Node::freed();
                self.free.push(dir);
            }
        }
        Some((content, modified))
    }

    /// The node `path` names, by [`spelled_kind`] over the tree's one walk
    /// ([`Self::locate`]), with the errors the reference gives: a path
    /// through a file, or a file spelled as a directory (ending in `/` or
    /// `/.`), is not a directory; a name longer than [`NAME_MAX`] is an
    /// invalid file name; else not found. O(path bytes).
    fn node(&self, path: &Path) -> io::Result<usize> {
        let rel = path.strip_prefix(&self.root).map_err(|_| not_found(path))?;
        let rel = rel.to_str().ok_or_else(|| not_found(path))?;
        let raw = path.as_os_str().to_str().unwrap_or_default();
        let located = self.locate(tine_names(rel)?, path);
        spelled_kind(
            spelled_as_directory(raw),
            located,
            |&at| self.is_dir(at),
            path,
        )
    }

    /// How deep `names` exist in the tree and the node at the deepest
    /// existing prefix, by one walk from the root: it stops at a missing
    /// name or at a file with names left. A name longer than [`NAME_MAX`]
    /// under a directory is an invalid file name (reported as `path`). One
    /// visit per name walked; the names left are only counted.
    fn locate<'a>(
        &self,
        mut names: impl Iterator<Item = &'a str>,
        path: &Path,
    ) -> io::Result<Located<usize>> {
        let (mut at, mut depth) = (0, 0);
        while let Some(name) = names.next() {
            #[cfg(test)]
            names_tests::NAME_VISITS.with(|count| count.set(count.get() + 1));
            let next = match &self.nodes[at] {
                Node::Dir(_) if name.len() > NAME_MAX => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidFilename,
                        format!("a name in {} is too long", path.display()),
                    ))
                }
                Node::Dir(children) => children.get(name),
                Node::File(..) => None,
            };
            let Some(&next) = next else {
                let names = depth + 1 + names.count();
                return Ok(Located {
                    depth,
                    names,
                    at: Some(at),
                });
            };
            (at, depth) = (next, depth + 1);
        }
        Ok(Located {
            depth,
            names: depth,
            at: Some(at),
        })
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
pub(crate) fn serialized(document: &tine_core::doc::Document, path: &Path) -> String {
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
            FileSource::Disk => Ok(entry_kind(&fs::symlink_metadata(path)?)),
            FileSource::Memory(files) => Ok(if files.is_dir(files.node(path)?) {
                EntryKind::Dir
            } else {
                EntryKind::File
            }),
        }
    }

    /// What the graph-relative spelling `rel` (a configured directory) names
    /// under `root`, by [`spelled_kind`]: the memory tree's one walk
    /// ([`Files::node`]), or on disk [`locate_deepest`] over the path built
    /// once ([`spelled_path`]): one `lstat` of it (a `stat` when spelled as a
    /// directory, so `pages/` through a link still opens), and only after a
    /// `NotFound` a binary search over its prefixes (borrowed, `Path::parent`)
    /// for the deepest that exists: O(d log d) components for depth d.
    pub(crate) fn lstat_spelled(&self, root: &Path, rel: &str) -> io::Result<EntryKind> {
        if let FileSource::Memory(_) = self {
            return self.lstat(&root.join(rel));
        }
        let as_directory = spelled_as_directory(rel);
        let (path, names) = spelled_path(root, tine_names(rel)?);
        #[cfg(test)]
        names_tests::COMPONENTS.with(|count| count.set(count.get() + names));
        let whole = disk_kind(&path, as_directory);
        let is_dir = |kind: &EntryKind| matches!(kind, EntryKind::Dir);
        let located = locate_deepest(names, whole, is_dir, |depth| {
            let prefix = (path.ancestors().nth(names - depth))
                .expect("a prefix of the path built from the names");
            #[cfg(test)]
            names_tests::COMPONENTS.with(|count| count.set(count.get() + depth));
            disk_kind(prefix, true)
        });
        spelled_kind(as_directory, located, is_dir, &path)
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

    /// Replace files of a graph [`Self::in_memory_with_config`] built
    /// ([`Files::replace`]): `Some(bytes)` writes a file with the time `now`,
    /// `None` deletes one. When a change can move what the config read
    /// finds ([`affects_config_lookup`]) the config is read again as an open
    /// reads it. Each change is then decided once: graph text under the
    /// resulting config ([`super::graph_text_eligible`], the open's check)
    /// or not. A graph-text write must pass the parse input checks
    /// (`InvalidData`), read from the given bytes; other bytes are held as an
    /// open holds them and never reach the page validator. The layout is
    /// then checked as an open checks it. All or none: a refusal restores
    /// the files and the config. Returns each change's [`Replaced`] in order
    /// and whether the config was read again. The page cache is not touched:
    /// the caller publishes the graph-text changes.
    pub(crate) fn replace_memory_files(
        &mut self,
        changes: Vec<(String, Option<Arc<Vec<u8>>>)>,
        now: SystemTime,
    ) -> io::Result<(Vec<Replaced>, bool)> {
        let reread = changes.iter().any(|(rel, _)| affects_config_lookup(rel));
        let mut written = Vec::with_capacity(changes.len());
        let changes = (changes.into_iter())
            .map(|(rel, bytes)| {
                written.push(bytes.clone());
                (rel, bytes.map(|bytes| (Content::Bytes(bytes), Some(now))))
            })
            .collect();
        let undo = self.memory_files()?.replace(changes)?;
        let before = reread.then(|| {
            let path = self.root.join(CONFIG_FILE);
            let read = super::read_config_in(&self.source, &path);
            self.set_config_read(read)
        });
        let replaced: Vec<Replaced> = (undo.iter())
            .map(|(rel, before)| Replaced {
                rel: rel.clone(),
                existed: before.is_some(),
                page: super::graph_text_eligible(&self.root, &self.root.join(rel), &self.config),
            })
            .collect();
        let checked = (replaced.iter().zip(&written))
            .filter(|(replaced, _)| replaced.page)
            .filter_map(|(replaced, bytes)| Some((&replaced.rel, bytes.as_ref()?)))
            .try_for_each(|(rel, bytes)| {
                super::validate_parse_text(bytes, &self.root.join(rel)).map(drop)
            })
            .and_then(|()| self.check_layout());
        if let Err(error) = checked {
            if let Some(before) = before {
                self.set_config_read(before);
            }
            let mut inverse = undo;
            inverse.reverse();
            (self.memory_files()?.replace(inverse)).expect("an inverse change applies");
            return Err(error);
        }
        if reread {
            self.install_config(self.config.clone());
        }
        Ok((replaced, reread))
    }

    fn memory_files(&mut self) -> io::Result<&mut Files> {
        match &mut self.source {
            FileSource::Memory(files) => Arc::get_mut(files)
                .ok_or_else(|| io::Error::other("a memory graph's files are shared")),
            FileSource::Disk => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "only a memory graph's files are replaced",
            )),
        }
    }

    /// The bytes a memory graph holds at `path` (a document serialized),
    /// shared; `None` when no file is there. O(1) for held bytes.
    pub(crate) fn memory_bytes(&self, path: &Path) -> io::Result<Option<Arc<Vec<u8>>>> {
        let FileSource::Memory(files) = &self.source else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "not a memory graph",
            ));
        };
        match files.file(path) {
            Ok((Content::Bytes(bytes), _)) => Ok(Some(Arc::clone(bytes))),
            Ok((Content::Document(document), _)) => {
                Ok(Some(Arc::new(serialized(document, path).into_bytes())))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
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

#[cfg(test)]
#[path = "file_source_names_tests.rs"]
mod names_tests;
