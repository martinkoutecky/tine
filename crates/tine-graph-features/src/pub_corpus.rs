//! The pages a publication, print or sheet export works on, each with the one
//! name code may use for it.
//!
//! **The question this module answers** (I-12): what is this page called, and
//! is it the page the caller named. A year-less title format (`MM-dd`) makes a
//! day's title (its *label*, `11-02`) a name that other years and an ordinary
//! page share, so a day crosses to the app, is admitted, slugged, looked up,
//! homed and bounded by its *operational name* (its handle, `2026_11_02`, as
//! `InventoryEntry::handle` gives it; CM SPEC §2.1). Every other page's
//! operational name is its name. The label is for display only.
//!
//! [`PubPage`] has no `name` field: [`PubPage::name`] is the operational name
//! and [`PubPage::label`] the label, so code cannot read the label as a name
//! by accident. This file is the only place in publication, render and print
//! that reads `CorpusPage::name` or asks the whole graph for a corpus
//! (`pub_corpus_guard.rs` fails on a site that does).
//!
//! One names-only inventory row per journal page
//! ([`tine_store::InventoryScope::FileNames`]) builds the names: O(day files ·
//! log P), no claimant of a shared label read, once per corpus.
use std::collections::HashSet;
use std::sync::Arc;
use tine_core::doc::Document;
use tine_core::model::{PageId, PageKind};
use tine_core::refs::page_key;
use tine_core::{Corpus, CorpusPage};
use tine_store::{InventoryScope, WholeGraph};

/// Parsed pages and, parallel to them, each one's operational name.
pub struct PubCorpus {
    corpus: Corpus,
    names: Vec<String>,
}

/// One page of a [`PubCorpus`].
#[derive(Clone, Copy)]
pub struct PubPage<'a> {
    /// Physical page identity.
    pub id: &'a PageId,
    /// Journal or ordinary page.
    pub kind: PageKind,
    /// Parsed page document.
    pub document: &'a Arc<Document>,
    operational: &'a str,
    label: &'a str,
}

impl<'a> PubPage<'a> {
    /// The one name code may use: what admits, slugs, keys and looks the page
    /// up, and what the app calls it.
    pub fn name(&self) -> &'a str {
        self.operational
    }

    /// The page's identity among names: `page_key` of its operational name.
    pub fn key(&self) -> String {
        key_of(self.operational)
    }

    /// The page's title as it is shown. Never a key.
    pub fn label(&self) -> &'a str {
        self.label
    }
}

/// The one identity check: names are the same page's when their `page_key`s
/// are equal. A page's own key is [`PubPage::key`].
pub(crate) fn key_of(name: &str) -> String {
    page_key(name)
}

/// The identity of the names a caller asked for: `page_key`, so a spelling
/// that differs from a page's own only by case, surrounding space, boundary
/// slashes or Unicode normalization names that page.
struct Named(HashSet<String>);

impl Named {
    fn of<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        Self(names.into_iter().map(key_of).collect())
    }

    fn admits(&self, operational: &str) -> bool {
        self.0.contains(&key_of(operational))
    }
}

/// The day handle of file `id` (`2026_11_02` under `MM-dd`), when it has one:
/// from its own names-only inventory row, O(log P).
fn handle_of(whole: &WholeGraph, id: &PageId) -> Option<String> {
    let rows = whole.inventory(InventoryScope::FileNames(id));
    rows.0.first()?.handle.clone()
}

/// The operational name of the page `[[link]]` opens, when that is a day whose
/// handle differs from the link (`[[2026-11-02]]` opens `2026_11_02` under
/// `MM-dd`); else the link itself. The link resolves as it opens
/// ([`WholeGraph::resolve`]); a name with no digit is never a day. O(log P + C)
/// for a date-like link, O(name) otherwise.
pub(crate) fn link_name(whole: &WholeGraph, link: &str) -> String {
    if link.bytes().any(|b| b.is_ascii_digit()) {
        if let tine_store::Resolved::Existing { id, .. } = whole.resolve(link, false) {
            if let Some(handle) = handle_of(whole, &id) {
                return handle;
            }
        }
    }
    link.to_owned()
}

impl PubCorpus {
    /// `corpus`, a corpus of `whole`, with its pages' operational names.
    pub(crate) fn new(whole: &WholeGraph, corpus: Corpus) -> Self {
        let names = (corpus.pages.iter())
            .map(|page| {
                let handle = (page.kind == PageKind::Journal)
                    .then(|| handle_of(whole, &page.id))
                    .flatten();
                handle.unwrap_or_else(|| page.name.clone())
            })
            .collect();
        Self { corpus, names }
    }

    /// The pages of `whole`'s view (the active ones, or every physical
    /// claimant when `active_only` is false), in page order.
    pub(crate) fn all(whole: &WholeGraph, active_only: bool) -> Self {
        Self::new(whole, whole.corpus(None, active_only))
    }

    /// The pages with the block-ID candidate `uuid` (`CorpusScope`).
    pub(crate) fn block_candidates(whole: &WholeGraph, uuid: &str) -> Self {
        Self::new(
            whole,
            whole.corpus(tine_store::CorpusScope::BlockIdCandidates(uuid), true),
        )
    }

    /// The one page file `id`, as its own names-only row names it: one
    /// claimant of its label among the label's claimants, none read for another
    /// file. `None` when the file is not a page of the view.
    pub(crate) fn file(whole: &WholeGraph, id: &PageId) -> Option<Self> {
        let label = whole
            .inventory(InventoryScope::FileNames(id))
            .0
            .first()?
            .name
            .clone();
        let mut corpus = Self::new(whole, whole.corpus(Some(label.as_str()), true));
        corpus.retain(|page| page.id == id);
        (!corpus.is_empty()).then_some(corpus)
    }

    /// The pages whose operational name is one of `names` (by page identity),
    /// in the order of `names`, page order within a name: a print's pages, a keyed
    /// lookup's.
    /// A day's handle names no claimant, so its file is found as a link to it
    /// opens and the claimants of its label are filtered. O(C log P) for C
    /// claimants.
    pub(crate) fn named<'a>(whole: &WholeGraph, names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut found = Self::empty();
        for name in names {
            let mut pages = whole.corpus(Some(name), true).pages;
            if pages.is_empty() {
                if let tine_store::Resolved::Existing { id, .. } = whole.resolve(name, false) {
                    if let Some(row) = whole.inventory(InventoryScope::FileNames(&id)).0.first() {
                        pages = whole.corpus(Some(row.name.as_str()), true).pages;
                    }
                }
            }
            let mut these = Self::new(whole, Corpus { pages });
            let named = Named::of([name]);
            these.retain(|page| named.admits(page.name()));
            found.corpus.pages.append(&mut these.corpus.pages);
            found.names.append(&mut these.names);
        }
        found
    }

    /// No pages.
    pub(crate) fn empty() -> Self {
        Self {
            corpus: Corpus::default(),
            names: Vec::new(),
        }
    }

    /// The pages, in order.
    pub(crate) fn pages(&self) -> impl ExactSizeIterator<Item = PubPage<'_>> + Clone {
        (0..self.len()).map(|index| self.page_at(index))
    }

    /// The pages among `names` by page identity; every page for `None`.
    pub(crate) fn pages_among<'a>(
        &'a self,
        names: Option<&[String]>,
    ) -> impl Iterator<Item = PubPage<'a>> {
        let wanted = names.map(|names| Named::of(names.iter().map(String::as_str)));
        self.pages()
            .filter(move |page| wanted.as_ref().is_none_or(|w| w.admits(page.name())))
    }

    /// The page at `index`.
    pub(crate) fn page_at(&self, index: usize) -> PubPage<'_> {
        let (page, operational) = (&self.corpus.pages[index], &self.names[index]);
        PubPage {
            id: &page.id,
            kind: page.kind,
            document: &page.document,
            operational,
            label: &page.name,
        }
    }

    /// The first page.
    pub(crate) fn first(&self) -> Option<PubPage<'_>> {
        self.pages().next()
    }

    /// The first page the caller named, by page identity.
    pub(crate) fn find_named(&self, name: &str) -> Option<PubPage<'_>> {
        let named = Named::of([name]);
        self.pages().find(|page| named.admits(page.name()))
    }

    /// The page at `index`, as a corpus of its own.
    pub(crate) fn only(&self, index: usize) -> Self {
        Self {
            corpus: Corpus {
                pages: vec![self.corpus.pages[index].clone()],
            },
            names: vec![self.names[index].clone()],
        }
    }

    /// How many pages.
    pub(crate) fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether there are no pages.
    pub(crate) fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Keep the pages `keep` accepts, in order.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(PubPage<'_>) -> bool) {
        let marks: Vec<bool> = self.pages().map(&mut keep).collect();
        let mut at = 0;
        self.corpus.pages.retain(|_| {
            at += 1;
            marks[at - 1]
        });
        at = 0;
        self.names.retain(|_| {
            at += 1;
            marks[at - 1]
        });
    }

    /// Order the pages by `key`, stably.
    pub(crate) fn sort_by_key<K: Ord>(&mut self, mut key: impl FnMut(PubPage<'_>) -> K) {
        let mut order: Vec<(K, usize)> = self
            .pages()
            .enumerate()
            .map(|(at, page)| (key(page), at))
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut pages: Vec<Option<CorpusPage>> = std::mem::take(&mut self.corpus.pages)
            .into_iter()
            .map(Some)
            .collect();
        let mut names: Vec<Option<String>> = std::mem::take(&mut self.names)
            .into_iter()
            .map(Some)
            .collect();
        for (_, at) in order {
            self.corpus
                .pages
                .push(pages[at].take().expect("each page once"));
            self.names.push(names[at].take().expect("each name once"));
        }
    }

    /// Drop the comment threads a publication without comments leaves out
    /// (`output_comments::project`).
    pub(crate) fn project_comments(&mut self, include: bool) {
        crate::output_comments::project(&mut self.corpus.pages, include);
    }

    /// Close a query result's rows to the projected body
    /// (`output_comments::close_result`).
    pub(crate) fn close_result(&self, result: &mut tine_core::query::ir::QueryResult) {
        crate::output_comments::close_result(result, &self.corpus.pages);
    }

    /// Carry a query result's reviewed ids onto the same pages' `published`
    /// ids (`output_comments::remap_result_ids`).
    pub(crate) fn remap_result_ids(
        &self,
        result: &mut tine_core::query::ir::QueryResult,
        published: &Self,
    ) {
        crate::output_comments::remap_result_ids(
            result,
            &self.corpus.pages,
            &published.corpus.pages,
        );
    }

    /// [`Corpus::block_ref_counts`] over these pages.
    pub(crate) fn block_ref_counts(&self) -> std::collections::HashMap<String, usize> {
        self.corpus.block_ref_counts()
    }

    /// [`Corpus::asset_refs`] over these pages.
    pub(crate) fn asset_refs(&self) -> HashSet<String> {
        self.corpus.asset_refs()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// `src` with comments and string literals' `//` lines left out: only code
    /// is scanned.
    fn code(src: &str) -> String {
        src.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .map(|line| line.split(" // ").next().unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The file up to its inline `#[cfg(test)] mod name {` unit-test module
    /// (a test module in its own `*_tests.rs` file is not scanned at all).
    fn before_tests(src: &str) -> String {
        let lines: Vec<&str> = src.lines().collect();
        let end = (0..lines.len())
            .find(|&at| {
                lines[at].trim() == "#[cfg(test)]"
                    && lines.get(at + 1).is_some_and(|next| {
                        next.starts_with("mod ") && next.trim_end().ends_with('{')
                    })
            })
            .unwrap_or(lines.len());
        lines[..end].join("\n")
    }

    fn words(src: &str) -> impl Iterator<Item = &str> {
        src.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
    }

    /// What a publication, render or print source may not do: name the plain
    /// corpus types or ask the whole graph for a corpus. Both go through
    /// `PubCorpus`, whose pages have `name()` and `label()` and no `name`.
    fn violations(src: &str) -> Vec<String> {
        let src = code(src);
        let mut found: Vec<String> = words(&src)
            .filter(|word| matches!(*word, "Corpus" | "CorpusPage"))
            .map(|word| format!("the type `{word}`"))
            .collect();
        for call in [".corpus(", ".corpus ("] {
            if src.contains(call) {
                found.push(format!("a `{call}` call on the whole graph"));
            }
        }
        found
    }

    #[test]
    fn publication_sources_read_page_names_only_through_pub_corpus() {
        // The guard sees a violation it is meant to catch.
        assert!(!violations("let c = whole.corpus(None, true);").is_empty());
        assert!(!violations("fn f(p: &CorpusPage) -> &str { &p.name }").is_empty());
        assert!(!violations("let c: Corpus = Default::default();").is_empty());
        assert!(violations("// the Corpus\nlet c = PubCorpus::all(w, true);").is_empty());

        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        for dir in [src.clone(), src.join("publish_query")] {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let in_scope = ["print", "publish", "quick_share", "build_graph", "render"]
                    .iter()
                    .any(|prefix| name.starts_with(prefix));
                if path.is_file() && in_scope && !name.ends_with("_tests.rs") {
                    files.push(path);
                }
            }
        }
        assert!(files.len() >= 9, "the scan found {files:?}");
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            // Unit-test modules built in the same file construct raw corpora.
            let found = violations(&before_tests(&text));
            assert!(found.is_empty(), "{}: {found:?}", path.display());
        }
        // The comment projection reads documents only, never a page's name.
        let comments = std::fs::read_to_string(src.join("output_comments.rs")).unwrap();
        let body = code(&before_tests(&comments));
        assert!(
            !body.contains(".name"),
            "output_comments.rs reads a page name"
        );
    }
}
