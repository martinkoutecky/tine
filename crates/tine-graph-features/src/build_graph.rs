//! The private build graph every published snapshot is projected from
//! (`docs/contracts/quick-share.md`, "What a share holds"; CM step 2, census
//! Q13): a fresh store holding only the exported documents, as they were
//! reviewed, plus the graph's `logseq/config.edn`. Every answer in a snapshot
//! (backlinks, aliases, icons, block reference counts, baked queries and the
//! `answers` tables) is then the live answerer's answer *over the exported
//! pages*: an unexported page has no influence at all, so "not in this export"
//! is indistinguishable from absent and nothing can name, relate or reveal it.
//!
//! The copy is not the user's graph: it is written under a caller-given scratch
//! directory (the app cache; the system temp directory for the CLI) and removed
//! before returning. Nothing here writes the graph. Cost: one serialization of
//! each exported document, one small file write each, and one store open (a
//! full parse of the exported pages); measured in the census Q13 receipt.
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::Path;
use tine_core::model::{FileId, Format, GraphMeta};
use tine_core::{Corpus, CorpusPage};
use tine_store::{OpenOptions, Store, StoreError, WatchMode, WholeGraph};

/// Largest `logseq/config.edn` copied into a build graph.
const CONFIG_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Build the private graph of `pages` (with `store`'s config) under
/// `scratch`, then call `f` with its whole-graph view, its graph metadata (the
/// effective journal formats) and its corpus, in the order of `pages`.
/// The build store is closed and its directory removed before returning,
/// whatever `f` answers; a removal failure is reported as an error (the
/// caller has not written anything yet, so nothing is exported).
///
/// Refuses with `refused` when the build graph does not read back as exactly
/// the given pages (by path). In-scope threat: malformed input or a parser
/// round-trip defect, where a reviewed document serializes to bytes the store
/// reads as another page set; publishing then would silently drop or add a
/// page. Recovery: the export says so and writes nothing.
pub(crate) fn with_build_graph<T>(
    store: &Store,
    pages: &[CorpusPage],
    scratch: &Path,
    refused: &str,
    f: impl FnOnce(&WholeGraph, &GraphMeta, Corpus) -> io::Result<T>,
) -> io::Result<T> {
    let dir = tempfile::Builder::new()
        .prefix("tine-build-graph-")
        .tempdir_in(scratch)?;
    let result = build_and_answer(store, pages, dir.path(), refused, f);
    // Observe the scratch removal: tempfile's destructor ignores a failure,
    // which would silently leave the exported pages and the full config
    // behind in the scratch directory (in-scope threat: disk errors).
    let kept = dir.path().display().to_string();
    match (result, dir.close()) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(io::Error::new(
            error.kind(),
            format!("the export's scratch copy at {kept} could not be removed ({error}); nothing was exported"),
        )),
        (Err(cause), Err(error)) => Err(io::Error::new(
            cause.kind(),
            format!("{cause}; also, the export's scratch copy at {kept} could not be removed ({error})"),
        )),
    }
}

fn build_and_answer<T>(
    store: &Store,
    pages: &[CorpusPage],
    dir: &Path,
    refused: &str,
    f: impl FnOnce(&WholeGraph, &GraphMeta, Corpus) -> io::Result<T>,
) -> io::Result<T> {
    match store.read(
        &FileId::from("logseq/config.edn".to_owned()),
        Some(CONFIG_MAX_BYTES),
    ) {
        Ok((bytes, _)) => {
            std::fs::create_dir_all(dir.join("logseq"))?;
            std::fs::write(dir.join("logseq/config.edn"), bytes)?;
        }
        Err(StoreError::NotFound) => {}
        Err(error) => return Err(io::Error::other(format!("{error:?}"))),
    }
    for page in pages {
        let rel = page.id.as_str();
        let text = match Format::from_path(Path::new(rel)) {
            Format::Org => tine_core::org::serialize_org(&page.document),
            _ => tine_core::doc::serialize(&page.document),
        };
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, text)?;
    }
    let (private, meta, _) = Store::open(
        dir,
        OpenOptions {
            approved_external_assets: None,
            watch: WatchMode::Poll,
            launch_checkpoint: None,
        },
    )
    .map_err(|error| io::Error::other(format!("{error:?}")))?;
    let result = (|| {
        let graph = private
            .whole_graph()
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
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
    })();
    private.close();
    result
}
