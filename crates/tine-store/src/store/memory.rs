//! The whole-graph views of a graph held in memory: an export's build graph
//! (`subgraph`, under a live view's config) and the browser's graph
//! (`from_files`, under its own `config.edn`), both published as a store's
//! first publication is.
use super::snapshot::PendingFiles;
use super::{Change, ChangeKind, ConfigState, FileRev, GraphRev, Origin, Snapshot, WholeGraph};
use crate::model::file_source::{serialized, Content};
use crate::model::Graph;
use crate::{FileId, PageId, PageRefusal};
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use tine_core::date::JournalFormat;
use tine_core::model::PageDto;

/// The graph a view built from files reads. Readers share it through `&`
/// (its caches sit behind locks); only [`WholeGraph::replace_files`], with
/// the one reference, edits it. A reader's panic so leaves no half-made
/// state another reader sees, as with the rest of a view.
#[derive(Clone)]
pub(super) struct MemoryGraph(Arc<Graph>);

impl std::panic::RefUnwindSafe for MemoryGraph {}
impl std::panic::UnwindSafe for MemoryGraph {}

impl MemoryGraph {
    /// The one answer to whether a view can change its graph, which every
    /// edit asks: it was built editable ([`WholeGraph::from_files`]) and no
    /// clone of it shares the graph.
    fn editable(memory: &mut Option<Self>) -> io::Result<&mut Graph> {
        (memory.as_mut())
            .and_then(|memory| Arc::get_mut(&mut memory.0))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "only an unshared, editable graph built from files is changed",
                )
            })
    }
}

type MemoryFile = (String, Arc<Vec<u8>>);

/// A file change and, for a write, the document its bytes serialize.
type PreparedChange = (String, Option<Vec<u8>>, Option<tine_core::doc::Document>);

impl WholeGraph {
    /// The whole-graph view of only `pages` under this view's effective config,
    /// journal formats and file times. The store's launch and first publication
    /// run in memory, with no filesystem access. Each page is serialized and
    /// parsed once. With `retain_sources`, the caller shares the same serialized
    /// bytes the graph reads. Otherwise only one page's text is held at a time.
    /// Refuses invalid memory-graph paths with `InvalidInput`.
    pub fn subgraph(
        &self,
        pages: Vec<(String, Arc<tine_core::doc::Document>)>,
        retain_sources: bool,
    ) -> io::Result<(WholeGraph, tine_core::model::GraphMeta, Vec<MemoryFile>)> {
        let times = self.graph.observed_page_mtimes();
        let mut sources = Vec::new();
        let files = pages
            .into_iter()
            .map(|(path, document)| {
                let modified = times.get(&path).copied();
                let content = if retain_sources {
                    let bytes = Arc::new(serialized(&document, Path::new(&path)).into_bytes());
                    sources.push((path.clone(), Arc::clone(&bytes)));
                    Content::Bytes(bytes)
                } else {
                    Content::Document(document)
                };
                (path, content, modified)
            })
            .collect();
        let graph = Graph::in_memory(
            files,
            tine_core::config::Config::clone(&self.config.config),
            JournalFormat::clone(&self.journal_format),
        )?;
        let view = Self::in_memory(&graph);
        let meta = tine_core::model::GraphMeta::from_config(
            graph.root.display().to_string(),
            &view.config.config,
            &graph.journal_format,
        );
        Ok((view, meta, sources))
    }

    /// The whole-graph view of a graph's text files (graph-relative path and
    /// bytes, `logseq/config.edn` among them), held in memory: what
    /// `Store::open` of a directory holding exactly these files answers, with
    /// no page times. The browser's graph (a published export's pages). The
    /// returned root spelling is `/tine-memory-graph`. `times` gives files
    /// their modification times (a file without one has none), which
    /// `page_modified` and recency order answer from; `None` is a time the
    /// caller was given but cannot represent, admitted as any other and then
    /// no time. Cost: one read and parse of each file and the initial index
    /// capture, as a store open has.
    /// Refuses what `Store::open` refuses for the configured directories'
    /// spelling, and (`InvalidInput`) a path that is not graph-relative or is
    /// both a file and a directory, and a time given twice or for no file; a
    /// config read problem is the view's `config().problem`, as on disk.
    /// Only an `editable` view keeps its graph and the files' bytes, for
    /// [`Self::replace_files`] and [`Self::submit_page`]; a read-only one (a
    /// published export, a quick share) keeps the captured view alone and
    /// refuses both.
    pub fn from_files<T: Into<Arc<Vec<u8>>>>(
        files: Vec<(String, T)>,
        times: Vec<(String, Option<SystemTime>)>,
        editable: bool,
    ) -> io::Result<(WholeGraph, String)> {
        let refused = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
        let mut by_path = HashMap::with_capacity(times.len());
        for (rel, time) in times {
            if by_path.insert(rel.clone(), time).is_some() {
                return Err(refused(format!("a time is given twice: {rel:?}")));
            }
        }
        let files: Vec<_> = (files.into_iter())
            .map(|(rel, bytes)| {
                let modified = by_path.remove(&rel).flatten();
                (rel, Content::Bytes(bytes.into()), modified)
            })
            .collect();
        if let Some(rel) = by_path.keys().next() {
            return Err(refused(format!("a time for no file: {rel:?}")));
        }
        let graph = Graph::in_memory_with_config(files)?;
        let root = graph.root.display().to_string();
        let mut view = Self::in_memory(&graph);
        view.memory = editable.then(|| MemoryGraph(Arc::new(graph)));
        Ok((view, root))
    }

    /// Replace files of a view built by [`Self::from_files`] and make this
    /// view the result: `Some(bytes)` writes a file, with the current time
    /// as its modification time (a disk write's), and `None` deletes one.
    /// Keys follow `from_files`'s rules (MEMORY-SOURCE-MODEL Q1): a new key
    /// that is another key's ancestor or descendant is refused, and a deleted
    /// file's emptied directories go. A file that is graph text under the
    /// resulting config (the open's eligibility check) must pass the parse
    /// input checks (`InvalidData`); other bytes (assets, dotfiles, hidden
    /// directories, conflict copies) are held as an open holds them. The
    /// result must open as `from_files` of the same files opens (layout). A
    /// refusal leaves the view and its files unchanged. A change that can
    /// move what the config read finds (the config file or its iCloud
    /// placeholder, anything under them, a `logseq` file) reads the config
    /// again as an open does. This raw path writes under an unreadable
    /// config too, so the config itself can be repaired. Returns the publication's
    /// [`Change`], as the desktop's watcher publishes one for the same files
    /// (`Origin::External`: its files, the active pages that moved and the
    /// answer deltas), and makes its revision this view's. Cost: one parse of each changed page file
    /// and the incremental capture of the changed paths, as the desktop
    /// watcher's publication of the same files; a config change that moves
    /// what the page build reads parses every page again, as on the desktop.
    pub fn replace_files(&mut self, changes: Vec<(String, Option<Vec<u8>>)>) -> io::Result<Change> {
        let changes = changes.into_iter().map(|(rel, bytes)| (rel, bytes, None));
        self.replace_prepared(changes.collect())
    }

    /// [`Self::replace_files`], where a write may carry the document its
    /// bytes were serialized from (a submit's): that document is published,
    /// so its blocks keep their runtime ids, as the desktop publishes its
    /// own save's. Any other graph-text write is parsed once. A change that
    /// is not graph text keeps its file change and revision and is not
    /// published as a page, as the desktop transaction publishes only pages.
    fn replace_prepared(&mut self, changes: Vec<PreparedChange>) -> io::Result<Change> {
        let graph = MemoryGraph::editable(&mut self.memory)?;
        let (mut writes, mut written) = (Vec::new(), Vec::new());
        for (rel, bytes, document) in changes {
            let bytes = bytes.map(Arc::new);
            writes.push((rel, bytes.clone()));
            written.push((bytes, document));
        }
        let now = tine_core::clock::system_now();
        let (replaced, reread) = graph.replace_memory_files(writes, now)?;
        let graph: &Graph = graph;
        let (mut files, mut published) = (Vec::with_capacity(replaced.len()), HashMap::new());
        for ((bytes, prepared), replaced) in written.into_iter().zip(replaced) {
            let path = graph.root.join(&replaced.rel);
            let kind = match (&bytes, replaced.existed) {
                (None, _) => ChangeKind::Removed,
                (Some(_), true) => ChangeKind::Modified,
                (Some(_), false) => ChangeKind::Created,
            };
            let bytes = bytes.as_deref().map(Vec::as_slice);
            let id = FileId::from(replaced.rel);
            if replaced.page {
                // Published as the desktop publishes its own write: its
                // document (the serializer's, or one parse of the page's
                // text) installed with no comparison.
                let document = prepared.or_else(|| {
                    (bytes.and_then(|bytes| std::str::from_utf8(bytes).ok()))
                        .map(|text| crate::model::parse_doc(&path, text))
                });
                let file_set_changed = kind != ChangeKind::Modified;
                let entry = graph.transaction_publish_page_inner(
                    &path,
                    bytes,
                    document.as_ref(),
                    file_set_changed,
                );
                if let Some(entry) = entry {
                    published.insert(id.clone(), entry);
                }
            }
            files.push((id, kind, bytes.map(FileRev::from_bytes)));
        }
        let old = &self._snapshot;
        let config = Self::config_of(graph);
        // What the page build and indexes read moved (the desktop's rule).
        let config_changed = reread
            && super::checkpoint::config_key(&old.config.config)
                != super::checkpoint::config_key(&config.config);
        let snapshot = Snapshot::capture(
            graph,
            &RwLock::new(config),
            Some(old),
            &PendingFiles::of(&files, graph),
            config_changed,
            false,
            GraphRev(old.rev.0 + 1),
            &published,
            &Default::default(),
        );
        let pages = snapshot.page_changes(Some(old), Origin::External, &files);
        graph.adopt_name_index(
            Some(old.cache_generation),
            snapshot.cache_generation,
            (&snapshot.list, &snapshot.claimants),
        );
        let change = Change {
            graph_rev: snapshot.rev,
            origin: Origin::External,
            files,
            pages,
            answers: snapshot.answers.clone(),
            watch: None,
        };
        let memory = self.memory.take();
        *self = WholeGraph::of(Arc::new(snapshot));
        self.memory = memory;
        Ok(change)
    }

    /// Submit `dto` for the page file `id` of a view built by
    /// [`Self::from_files`]: serialize it over the file's current bytes here
    /// (none: a creation) with the store's one page serializer, as the
    /// desktop page host does, then [`Self::replace_files`] it. Answers the
    /// bytes written and the publication's change, or the page host's
    /// refusal: its serializer's and page-target checks, the desktop
    /// transaction's refusal under an unreadable config (`ConfigUnreadable`,
    /// as `InvalidTarget`, the host's mapping of it), `NotAdmitted` when the
    /// caller's copy is not `current` (the host's answer to a stale
    /// request), and `Failed` for a read-only view or a refused replace. A
    /// refusal writes nothing. The serializer's document is published, so
    /// each submitted block keeps its runtime id. Cost: one serialization
    /// and the parse of the old and new text, as a desktop submit, then the
    /// replace of one file with no further parse.
    pub fn submit_page(
        &mut self,
        id: &PageId,
        dto: &PageDto,
        current: bool,
    ) -> Result<(Vec<u8>, Change), PageRefusal> {
        let failed = |error: io::Error| PageRefusal::Failed {
            message: error.to_string(),
        };
        if !current {
            return Err(PageRefusal::NotAdmitted);
        }
        MemoryGraph::editable(&mut self.memory).map_err(failed)?;
        if let Some(why) = crate::transaction::config_write_failure(self.config()) {
            return Err(why.into());
        }
        let file = id.file();
        let is_page = || self.validated_page(id).is_ok();
        crate::transaction::page_save_target(dto, is_page, &file)?;
        let graph: &Graph = MemoryGraph::editable(&mut self.memory).map_err(failed)?;
        let path = graph.root.join(id.as_str());
        let old = graph.memory_bytes(&path).map_err(failed)?;
        let old = old.as_deref().map(Vec::as_slice);
        let (bytes, document) =
            crate::transaction::serialize_page_at(graph, &path, dto, old, false)?;
        let change = vec![(id.as_str().to_owned(), Some(bytes.clone()), Some(document))];
        let change = self.replace_prepared(change).map_err(failed)?;
        Ok((bytes, change))
    }

    /// This view without its memory graph: a read-only copy (tests keep
    /// the previous view so).
    #[cfg(test)]
    pub(crate) fn clone_without_memory(&self) -> WholeGraph {
        let mut view = self.clone();
        view.memory = None;
        view
    }

    /// A memory graph's current bytes of the file `id` (tests).
    #[cfg(test)]
    pub(crate) fn memory_page_bytes(&self, id: &PageId) -> Option<Vec<u8>> {
        let graph = &self.memory.as_ref().unwrap().0;
        let path = graph.root.join(id.as_str());
        graph
            .memory_bytes(&path)
            .unwrap()
            .map(|bytes| bytes.to_vec())
    }

    /// The config state a memory graph publishes: the config it read.
    fn config_of(graph: &Graph) -> ConfigState {
        let problem = graph.config_read_problem.clone();
        ConfigState::of(graph, Arc::new(graph.config.clone()), problem)
    }

    /// A memory graph's view: the store's launch pass and first publication.
    fn in_memory(graph: &Graph) -> WholeGraph {
        graph.install_live_config();
        graph.warm_cache_cancellable(|| false);
        let config = Self::config_of(graph);
        let snapshot = Snapshot::capture(
            graph,
            &RwLock::new(config),
            None,
            &Default::default(),
            false,
            false,
            GraphRev(0),
            &HashMap::new(),
            &Default::default(),
        );
        WholeGraph::of(Arc::new(snapshot))
    }
}
