//! The whole-graph views of a graph held in memory: an export's build graph
//! (`subgraph`, under a live view's config) and the browser's graph
//! (`from_files`, under its own `config.edn`), both published as a store's
//! first publication is.
use super::{ConfigState, GraphRev, Snapshot, WholeGraph};
use crate::model::file_source::Content;
use crate::model::Graph;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, RwLock};
use tine_core::date::JournalFormat;

impl WholeGraph {
    /// The whole-graph view of only `pages` (graph-relative path and document)
    /// under this view's effective config, journal formats and page times:
    /// what this view would answer if the graph held just these pages. It is
    /// built in memory by the store's own launch pass and first publication,
    /// with no filesystem access; an export's build graph. Its graph metadata
    /// names the root `/tine-memory-graph`. Cost: one serialization and parse
    /// of each page, one page's bytes held at a time, and the initial index
    /// capture, as a store open has, plus one lookup per page in this view's
    /// page times. Refuses (`InvalidInput`) a path that is not graph-relative
    /// or is both a file and a directory.
    pub fn subgraph(
        &self,
        pages: Vec<(String, Arc<tine_core::doc::Document>)>,
    ) -> io::Result<(WholeGraph, tine_core::model::GraphMeta)> {
        let times = self.graph.observed_page_mtimes();
        let files = pages
            .into_iter()
            .map(|(rel, document)| {
                let time = times.get(&rel).copied();
                (rel, Content::Document(document), time)
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
        Ok((view, meta))
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
    pub fn from_files(files: Vec<(String, Vec<u8>)>) -> io::Result<(WholeGraph, String)> {
        let files = (files.into_iter())
            .map(|(rel, bytes)| (rel, Content::Bytes(Arc::new(bytes)), None))
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
