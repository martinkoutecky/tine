//! Corpus lookups for export macros, each built once per `RenderGraph` on first
//! use (I-15): a `{{embed}}`, `{{query}}` or `{{namespace}}` then costs a hash or
//! range probe, not a corpus scan. Each index costs O(pages) or O(blocks) once.
//! Entries are page indexes and child paths into the one corpus the graph holds.
//!
//! A single-page print holds no whole-graph corpus: its lookups are `keyed`
//! and ask the view's name table (`WholeGraph::corpus(Some(name))`) and exact
//! block index (`WholeGraph::blocks`, which names the page a corpus scan finds
//! first) instead, so a printed embed or query costs its target, not the graph
//! (cost-model audit 2026-11-02, row 5). Only `{{namespace}}`, and a block
//! whose subtree exceeds `blocks`' result budget, still copy the corpus once.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tine_core::doc::{DocBlock, Document};
use tine_core::model::PageKind;
use tine_core::{Corpus, CorpusPage};
use tine_store::WholeGraph;

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
    pub(super) fn block(&self) -> &DocBlock {
        let (first, rest) = self.path.split_first().expect("a block path is non-empty");
        let mut block = &self.page.document.roots[*first];
        for i in rest {
            block = &block.children[*i];
        }
        block
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
    let pages = whole.corpus(Some(&group.page), true).pages.into_iter();
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
    /// Keyed mode's page list for `{{namespace}}`, copied on first use.
    all_pages: OnceCell<Corpus>,
    /// Block uuid or `id::` → page index and child-index path from its roots.
    blocks: OnceCell<HashMap<String, BlockAt>>,
    docs: OnceCell<HashMap<(String, PageKind), usize>>,
    by_name: OnceCell<HashMap<String, usize>>,
    /// Page indexes sorted by name, one per distinct name.
    names: OnceCell<Vec<usize>>,
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
                    map.entry(block.uuid.clone()).or_insert_with(at);
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

    /// Distinct page names starting with `prefix`, sorted: a range of one sorted list.
    pub(super) fn names_under(
        &self,
        corpus: &Corpus,
        whole: &WholeGraph,
        prefix: &str,
    ) -> Vec<String> {
        // Keyed mode has no corpus to range over: a namespace listing copies
        // the view's page list once (the one graph-sized walk print keeps).
        let corpus = if self.keyed {
            self.all_pages.get_or_init(|| whole.corpus(None, true))
        } else {
            corpus
        };
        let name = |i: &usize| corpus.pages[*i].name.as_str();
        let names = self.names.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            let mut names: Vec<usize> = (0..corpus.pages.len()).collect();
            names.sort_unstable_by(|a, b| name(a).cmp(name(b)));
            names.dedup_by(|a, b| name(a) == name(b));
            names
        });
        let start = names.partition_point(|i| name(i) < prefix);
        (names[start..].iter().map(name))
            .take_while(|n| n.starts_with(prefix))
            .map(str::to_owned)
            .collect()
    }
}
