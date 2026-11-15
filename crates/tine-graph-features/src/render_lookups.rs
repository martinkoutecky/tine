//! Corpus indexes for export embeds and queries, built once per `RenderGraph`
//! on first use (I-15), then answered by hash probes instead of corpus scans.
//! Each corpus index costs O(pages) or O(blocks) once; namespace names use the
//! store's existing identity tree directly.
//! Entries are page indexes and child paths into the one corpus the graph holds.
//!
//! A single-page print holds no whole-graph corpus: its lookups are `keyed`
//! and ask the view's name table (`WholeGraph::corpus(Some(name))`) and exact
//! block index (`WholeGraph::blocks`, which names the page a corpus scan finds
//! first) instead, so a printed embed or query costs its target, not the graph
//! (cost-model audit 2026-11-02, row 5). Namespace names range the store's
//! identity tree in either mode. A block whose subtree exceeds `blocks`'
//! result budget still copies the corpus once.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tine_core::doc::{DocBlock, Document};
use tine_core::model::PageKind;
use tine_core::{Corpus, CorpusPage};
use tine_store::WholeGraph;

#[cfg(test)]
#[path = "render_namespace_tests.rs"]
mod namespace_tests;

#[cfg(test)]
thread_local!(pub(super) static PAGE_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// Cost counter for tests: corpus pages visited to answer macro lookups.
fn count_page_probes(pages: usize) {
    #[cfg(test)]
    PAGE_PROBES.with(|probes| probes.set(probes.get() + pages));
    let _ = pages;
}

/// A page index and the child-index path from that page's roots to one block.
type BlockAt = (usize, Box<[usize]>);

/// A block found by uuid: its page and the child-index path to it.
pub(super) struct BlockHit {
    pub page: CorpusPage,
    path: Box<[usize]>,
}

impl BlockHit {
    pub(super) fn block(&self) -> (&DocBlock, bool) {
        let (first, rest) = self.path.split_first().expect("a block path is non-empty");
        let mut block = &self.page.document.roots[*first];
        let mut comment = false;
        for i in rest {
            block = &block.children[*i];
            comment |= crate::output_comments::is_comment(block, true);
        }
        (block, comment)
    }
}

/// `uuid`'s block from the exact block index: `blocks` names the page a corpus
/// scan finds first (page order, then walk order); the first page of that
/// name and kind holding the block is that page. `Err` when the answer is
/// over `blocks`' result budget (the caller scans instead).
fn keyed_block(whole: &WholeGraph, uuid: &str) -> Result<Option<BlockHit>, ()> {
    fn find(blocks: &[DocBlock], uuid: &str, path: &mut Vec<usize>) -> bool {
        for (index, block) in blocks.iter().enumerate() {
            path.push(index);
            if block.uuid == uuid || block.property("id").as_deref() == Some(uuid) {
                return true;
            }
            if find(&block.children, uuid, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let group = whole
        .blocks(&[uuid.to_owned()], true)
        .map_err(|_| ())?
        .pop()
        .flatten();
    let Some(group) = group else {
        return Ok(None);
    };
    let pages = whole
        .corpus(Some(group.page.as_str()), true)
        .pages
        .into_iter();
    for page in pages.filter(|page| page.name == group.page && page.kind == group.kind) {
        let mut path = Vec::new();
        if find(&page.document.roots, uuid, &mut path) {
            return Ok(Some(BlockHit {
                page,
                path: path.into_boxed_slice(),
            }));
        }
    }
    Ok(None)
}

#[derive(Default)]
pub(super) struct Lookups {
    /// Answer from the view's keyed indexes, not the corpus (single-page print).
    pub keyed: bool,
    /// Keyed mode's fallback for blocks over the exact lookup result budget.
    all_pages: OnceCell<Corpus>,
    /// Block uuid or `id::` → page index and child-index path from its roots.
    blocks: OnceCell<HashMap<String, BlockAt>>,
    docs: OnceCell<HashMap<(String, PageKind), usize>>,
    by_name: OnceCell<HashMap<String, usize>>,
}

impl Lookups {
    /// The first block, in corpus order then pre-order, whose runtime uuid or
    /// `id::` property is `uuid`.
    pub(super) fn block(
        &self,
        corpus: &Corpus,
        whole: &WholeGraph,
        uuid: &str,
    ) -> Option<BlockHit> {
        let corpus = if self.keyed {
            match keyed_block(whole, uuid) {
                Ok(hit) => return hit,
                // Over the block question's result budget: one corpus copy.
                Err(()) => self.all_pages.get_or_init(|| whole.corpus(None, true)),
            }
        } else {
            corpus
        };
        let blocks = self.blocks.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            let mut map = HashMap::new();
            for (page, entry) in corpus.pages.iter().enumerate() {
                let mut stack: Vec<(Vec<usize>, &DocBlock)> = (entry.document.roots.iter())
                    .enumerate()
                    .rev()
                    .map(|(i, block)| (vec![i], block))
                    .collect();
                while let Some((path, block)) = stack.pop() {
                    let at = || (page, path.clone().into_boxed_slice());
                    map.entry(block.uuid.to_string()).or_insert_with(at);
                    if let Some(id) = block.property("id") {
                        map.entry(id).or_insert_with(at);
                    }
                    for (i, child) in block.children.iter().enumerate().rev() {
                        let mut child_path = path.clone();
                        child_path.push(i);
                        stack.push((child_path, child));
                    }
                }
            }
            map
        });
        let (page, path) = blocks.get(uuid)?;
        if path.is_empty() {
            return None;
        }
        Some(BlockHit {
            page: corpus.pages[*page].clone(),
            path: path.clone(),
        })
    }

    /// The document of the page with this exact name and kind (last file wins).
    pub(super) fn doc(
        &self,
        corpus: &Corpus,
        whole: &WholeGraph,
        name: &str,
        kind: PageKind,
    ) -> Option<Arc<Document>> {
        if self.keyed {
            return (whole.corpus(Some(name), true).pages.into_iter())
                .filter(|page| page.name == name && page.kind == kind)
                .last()
                .map(|page| page.document);
        }
        let docs = self.docs.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            (corpus.pages.iter().enumerate())
                .map(|(i, page)| ((page.name.clone(), page.kind), i))
                .collect()
        });
        let page = docs.get(&(name.to_owned(), kind))?;
        Some(Arc::clone(&corpus.pages[*page].document))
    }

    /// The first page, in corpus order, with `name`'s Logseq page identity (`refs::page_key`).
    pub(super) fn doc_named(
        &self,
        corpus: &Corpus,
        whole: &WholeGraph,
        name: &str,
    ) -> Option<Arc<Document>> {
        if self.keyed {
            return (whole.corpus(Some(name), true).pages.into_iter())
                .next()
                .map(|page| page.document);
        }
        let by_name = self.by_name.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            let mut map = HashMap::new();
            for (i, page) in corpus.pages.iter().enumerate() {
                map.entry(tine_core::refs::page_key(&page.name))
                    .or_insert(i);
            }
            map
        });
        let page = by_name.get(&tine_core::refs::page_key(name))?;
        Some(Arc::clone(&corpus.pages[*page].document))
    }

    /// Active stored names under an identity prefix, in displayed-name byte order.
    pub(super) fn names_under(&self, whole: &WholeGraph, prefix: &str) -> Vec<String> {
        let entries = whole.inventory(tine_store::InventoryScope::ActiveFilesUnder(prefix));
        let mut names: Vec<_> = entries.0.iter().map(|entry| entry.name.clone()).collect();
        names.sort_unstable();
        names
    }
}
