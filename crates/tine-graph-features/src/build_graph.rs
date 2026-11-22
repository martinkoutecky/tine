//! The private build graph every published snapshot is projected from
//! (`docs/contracts/quick-share.md`, "What a share holds"; CM step 2, census
//! Q13): a graph holding only the exported documents, as they were reviewed,
//! plus the graph's `logseq/config.edn`. Every answer in a snapshot
//! (backlinks, aliases, icons, block reference counts, baked queries and the
//! `answers` tables) is then the live answerer's answer *over the exported
//! pages*: an unexported page has no influence at all, so "not in this export"
//! is indistinguishable from absent and nothing can name, relate or reveal it.
//!
//! The graph is built in memory by the store's own load
//! ([`WholeGraph::from_pages`]); nothing is written anywhere. Cost: one
//! serialization of each exported document and one parse of the exported
//! pages with the initial index capture.
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::Path;
use tine_core::model::{FileId, Format, GraphMeta};
use tine_core::{Corpus, CorpusPage};
use tine_store::{Store, StoreError, WholeGraph};

/// Largest `logseq/config.edn` copied into a build graph.
const CONFIG_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Build the private graph of `pages` (with `store`'s config), then call `f`
/// with its whole-graph view, its graph metadata (the effective journal
/// formats) and its corpus, in the order of `pages`.
///
/// Refuses as a store open does when the config's pages or journals directory
/// is unsafe (an external editor's change the live store has not accepted).
/// Refuses with `refused` when the build graph does not read back as exactly
/// the given pages (by path). In-scope threat: malformed input or a parser
/// round-trip defect, where a reviewed document serializes to bytes the store
/// reads as another page set; publishing then would silently drop or add a
/// page. Recovery: the export says so and writes nothing.
pub(crate) fn with_build_graph<T>(
    store: &Store,
    pages: &[CorpusPage],
    refused: &str,
    f: impl FnOnce(&WholeGraph, &GraphMeta, Corpus) -> io::Result<T>,
) -> io::Result<T> {
    let mut files = Vec::with_capacity(pages.len() + 1);
    match store.read(
        &FileId::from("logseq/config.edn".to_owned()),
        Some(CONFIG_MAX_BYTES),
    ) {
        Ok((bytes, _)) => files.push(("logseq/config.edn".to_owned(), bytes, None)),
        Err(StoreError::NotFound(_)) => {}
        Err(error) => return Err(io::Error::other(format!("{error:?}"))),
    }
    // Each page keeps its file's time, which recency queries sort by.
    let modified: HashMap<String, i64> = (store.whole_graph())
        .map_err(|error| io::Error::other(format!("{error:?}")))?
        .page_modified()
        .into_iter()
        .collect();
    for page in pages {
        let rel = page.id.as_str();
        let text = match Format::from_path(Path::new(rel)) {
            Format::Org => tine_core::org::serialize_org(&page.document),
            _ => tine_core::doc::serialize(&page.document),
        };
        let time = (modified.get(rel).and_then(|secs| u64::try_from(*secs).ok()))
            .map(|secs| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs));
        files.push((rel.to_owned(), text.into_bytes(), time));
    }
    let (graph, meta) = WholeGraph::from_pages(files)?;
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
        let mut built = super::with_build_graph(&store, &pages, "refused", |graph, _, _| {
            Ok(graph.page_modified())
        })
        .unwrap();
        built.sort();
        assert_eq!(built, live);
        assert_eq!(built.len(), 2);
        store.close();
    }
}
