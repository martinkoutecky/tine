//! The whole-graph views of a graph held in memory: an export's build graph
//! (`subgraph`, under a live view's config) and the browser's graph
//! (`from_files`, under its own `config.edn`), both published as a store's
//! first publication is.
use super::{ConfigState, GraphRev, Snapshot, WholeGraph};
use crate::model::file_source::{serialized, Content};
use crate::model::Graph;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tine_core::date::JournalFormat;

type MemoryFile = (String, Arc<Vec<u8>>);

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
    /// returned root spelling is `/tine-memory-graph`. Cost: one read and
    /// parse of each file and the initial index capture, as a store open has.
    /// Refuses what `Store::open` refuses for the configured directories'
    /// spelling, and (`InvalidInput`) a path that is not graph-relative or is
    /// both a file and a directory; a config read problem is the view's
    /// `config().problem`, as on disk.
    pub fn from_files<T: Into<Arc<Vec<u8>>>>(
        files: Vec<(String, T)>,
    ) -> io::Result<(WholeGraph, String)> {
        let files = files
            .into_iter()
            .map(|(rel, bytes)| (rel, Content::Bytes(bytes.into()), None))
            .collect();
        let graph = Graph::in_memory_with_config(files)?;
        Ok((Self::in_memory(&graph), graph.root.display().to_string()))
    }

    /// A memory graph's view: the store's launch pass and first publication.
    fn in_memory(graph: &Graph) -> WholeGraph {
        graph.install_live_config();
        graph.warm_cache_cancellable(|| false);
        let problem = graph.config_read_problem.clone();
        let config = ConfigState::of(graph, Arc::new(graph.config.clone()), problem);
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
