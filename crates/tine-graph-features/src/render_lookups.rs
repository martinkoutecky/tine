//! Corpus lookups for export macros, each built once per `RenderGraph` on first
//! use (I-15): a `{{embed}}`, `{{query}}` or `{{namespace}}` then costs a hash or
//! range probe, not a corpus scan. Each index costs O(pages) or O(blocks) once.
//! Entries are page indexes and child paths into the one corpus the graph holds.

use std::cell::OnceCell;
use std::collections::HashMap;
use tine_core::doc::{DocBlock, Document};
use tine_core::model::PageKind;
use tine_core::{Corpus, CorpusPage};

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

#[derive(Default)]
pub(super) struct Lookups {
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
    pub(super) fn block<'a>(
        &self,
        corpus: &'a Corpus,
        uuid: &str,
    ) -> Option<(&'a CorpusPage, &'a DocBlock)> {
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
        let page = &corpus.pages[*page];
        let (first, rest) = path.split_first()?;
        let mut block = &page.document.roots[*first];
        for i in rest {
            block = &block.children[*i];
        }
        Some((page, block))
    }

    /// The document of the page with this exact name and kind (last file wins).
    pub(super) fn doc<'a>(
        &self,
        corpus: &'a Corpus,
        name: &str,
        kind: PageKind,
    ) -> Option<&'a Document> {
        let docs = self.docs.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            (corpus.pages.iter().enumerate())
                .map(|(i, page)| ((page.name.clone(), page.kind), i))
                .collect()
        });
        let page = docs.get(&(name.to_owned(), kind))?;
        Some(corpus.pages[*page].document.as_ref())
    }

    /// The first page, in corpus order, with `name`'s Logseq page identity (`refs::page_key`).
    pub(super) fn doc_named<'a>(&self, corpus: &'a Corpus, name: &str) -> Option<&'a Document> {
        let by_name = self.by_name.get_or_init(|| {
            count_page_probes(corpus.pages.len());
            let mut map = HashMap::new();
            for (i, page) in corpus.pages.iter().enumerate() {
                map.entry(tine_core::refs::page_key(&page.name)).or_insert(i);
            }
            map
        });
        let page = by_name.get(&tine_core::refs::page_key(name))?;
        Some(corpus.pages[*page].document.as_ref())
    }

    /// Distinct page names starting with `prefix`, sorted: a range of one sorted list.
    pub(super) fn names_under<'a>(&self, corpus: &'a Corpus, prefix: &str) -> Vec<&'a str> {
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
            .collect()
    }
}
