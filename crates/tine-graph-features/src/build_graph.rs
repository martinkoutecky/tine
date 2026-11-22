//! The private build graph every published snapshot is projected from
//! (`docs/contracts/quick-share.md`, "What a share holds"; CM step 2, census
//! Q13): a graph holding only the exported documents, as they were reviewed,
//! under the selecting view's config. Every answer in a snapshot
//! (backlinks, aliases, icons, block reference counts, baked queries and the
//! `answers` tables) is then the live answerer's answer *over the exported
//! pages*: an unexported page has no influence at all, so "not in this export"
//! is indistinguishable from absent and nothing can name, relate or reveal it.
//!
//! The graph is built in memory from the selecting view
//! ([`WholeGraph::subgraph`]) over the exported documents, each serialized
//! only while the loader reads it; nothing is read from or written to disk.
//! Cost: one serialization and parse of each exported page with the initial
//! index capture.
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::sync::Arc;
use tine_core::model::GraphMeta;
use tine_core::{Corpus, CorpusPage};
use tine_store::WholeGraph;

/// Build the private graph of `pages`, selected from `view` (with its config
/// and page times), then call `f` with its whole-graph view, its graph
/// metadata (the effective journal formats) and its corpus, in the order of
/// `pages`.
///
/// Refuses with `refused` when the build graph does not read back as exactly
/// the given pages (by path). In-scope threat: malformed input or a parser
/// round-trip defect, where a reviewed document serializes to bytes the store
/// reads as another page set; publishing then would silently drop or add a
/// page. Recovery: the export says so and writes nothing.
pub(crate) fn with_build_graph<T>(
    view: &WholeGraph,
    pages: &[CorpusPage],
    refused: &str,
    f: impl FnOnce(&WholeGraph, &GraphMeta, Corpus) -> io::Result<T>,
) -> io::Result<T> {
    let files = (pages.iter())
        .map(|page| (page.id.as_str().to_owned(), Arc::clone(&page.document)))
        .collect();
    let (graph, meta) = view.subgraph(files);
    let mut corpus = graph.corpus(None, true);
    let wanted: BTreeSet<_> = pages.iter().map(|p| p.id.as_str()).collect();
    let got: BTreeSet<_> = corpus.pages.iter().map(|p| p.id.as_str()).collect();
    if wanted != got || corpus.pages.len() != pages.len() {
        return Err(crate::publish_query::refusal(refused));
    }
    let order: HashMap<_, _> = pages
        .iter()
        .enumerate()
        .map(|(at, p)| (p.id.as_str().to_owned(), at))
        .collect();
    corpus.pages.sort_by_key(|p| order[p.id.as_str()]);
    f(&graph, &meta, corpus)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    /// The build graph keeps each exported page's file time, so a query
    /// sorted by recency orders the export as the app orders the graph.
    #[test]
    fn the_build_graph_keeps_each_page_time() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("pages")).unwrap();
        for (name, secs) in [("Old", 1_000_000_000), ("New", 1_700_000_000)] {
            let path = dir.path().join(format!("pages/{name}.md"));
            std::fs::write(&path, "- x\n").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
                .unwrap();
        }
        let store = tine_store::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let view = store.whole_graph().unwrap();
        let pages = view.corpus(None, true).pages;
        let mut live = view.page_modified();
        live.sort();
        let mut built = super::with_build_graph(&view, &pages, "refused", |graph, _, _| {
            Ok(graph.page_modified())
        })
        .unwrap();
        built.sort();
        assert_eq!(built, live);
        assert_eq!(built.len(), 2);
        store.close();
    }

    /// The build graph names and classifies pages by the selecting view's
    /// config, not by a `config.edn` changed on disk since (here a journal
    /// title format the view has not taken yet).
    #[test]
    fn the_build_graph_uses_the_view_config() {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["pages", "journals", "logseq"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("journals/2026_11_22.md"), "- day\n").unwrap();
        std::fs::write(dir.path().join("pages/Page.md"), "- page\n").unwrap();
        let options = tine_store::OpenOptions {
            approved_external_assets: None,
            watch: tine_store::WatchMode::Poll,
            launch_checkpoint: None,
        };
        let store = tine_store::Store::open(dir.path(), options).unwrap().0;
        let view = store.whole_graph().unwrap();
        let pages = view.corpus(None, true).pages;
        let kinds = |pages: &[tine_core::CorpusPage]| {
            let mut kinds: Vec<_> = (pages.iter())
                .map(|p| format!("{} {} {:?}", p.id.as_str(), p.name, p.kind))
                .collect();
            kinds.sort();
            kinds
        };
        std::fs::write(
            dir.path().join("logseq/config.edn"),
            "{:journal/page-title-format \"yyyy-MM-dd\"}\n",
        )
        .unwrap();
        let built = super::with_build_graph(&view, &pages, "refused", |_, _, corpus| {
            Ok(kinds(&corpus.pages))
        })
        .unwrap();
        assert_eq!(built, kinds(&pages));
        store.close();
    }
}
