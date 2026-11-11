//! `WholeGraph::corpus(Some(name))`: the pages claiming one name, from the
//! view's name table, so a single-page render (print / PDF) never copies or
//! walks every page (cost-model audit 2026-11-02, row 5; I-13, D-10). A
//! parameter on the existing corpus question, not a sibling (SURFACE.txt
//! rule 3). The printed page's embedded blocks come from `WholeGraph::blocks`,
//! which answers from the exact block-owner index.
use super::*;
use tine_core::CorpusPage;

impl<'a> From<Option<&'a str>> for CorpusScope<'a> {
    fn from(name: Option<&'a str>) -> Self {
        Self::Page(name)
    }
}

impl WholeGraph {
    /// Return an owned `Corpus` of pages in this view for evaluation: every
    /// page for `None`; for `Some(name)`, the parsed pages whose Logseq
    /// identity (`refs::page_key`) is `name`'s, ordinary pages and journals
    /// alike, in page order (what filtering the whole corpus by that key
    /// gives). `active_only` selects semantic documents; false retains every
    /// physical claimant for mutation and safety checks. Cost O(P)
    /// for `None`, O(C log P) for C claimants of the name; parsed documents
    /// are shared while the result is held.
    /// BlockIdCandidates uses the existing block index: O(log I + candidate pages),
    /// with no I/O. Callers verify the persisted-ID predicate within those documents.
    pub fn corpus<'a>(
        &self,
        scope: impl Into<CorpusScope<'a>>,
        active_only: bool,
    ) -> tine_core::Corpus {
        let name = match scope.into() {
            CorpusScope::Page(name) => name,
            CorpusScope::BlockIdCandidates(uuid) => {
                return tine_core::Corpus {
                    pages: self
                        .graph
                        .block_id_candidate_pages(uuid)
                        .into_iter()
                        .filter(|(entry, _)| {
                            !active_only
                                || self
                                    .graph
                                    .pages
                                    .positions
                                    .contains_key(entry.rel_path_str())
                        })
                        .filter_map(|(entry, document)| {
                            Some(tine_core::CorpusPage {
                                id: entry.rel_path.clone()?,
                                name: entry.name.clone(),
                                kind: entry.kind,
                                document: Arc::clone(document),
                            })
                        })
                        .collect(),
                };
            }
        };
        if let Some(name) = name {
            let mut corpus = self.corpus_named(name);
            if active_only {
                corpus
                    .pages
                    .retain(|page| self.graph.pages.positions.contains_key(page.id.as_str()));
            }
            return corpus;
        }
        #[cfg(feature = "test-faults")]
        crate::cost_counters::corpus();
        let pages = {
            let pages = if active_only {
                &self.graph.pages
            } else {
                &self.graph.physical_pages
            };
            pages
                .iter()
                .filter_map(|(entry, document)| {
                    Some(tine_core::CorpusPage {
                        id: entry.rel_path.clone()?,
                        name: entry.name.clone(),
                        kind: entry.kind,
                        document: Arc::clone(document),
                    })
                })
                .collect()
        };
        tine_core::Corpus { pages }
    }

    /// See [`WholeGraph::corpus`]: the parsed pages whose `page_key` is
    /// `name`'s, in page order.
    pub(super) fn corpus_named(&self, name: &str) -> tine_core::Corpus {
        let key = tine_core::refs::page_key(name);
        let mut found: Vec<(usize, CorpusPage)> = Vec::new();
        for journal in [false, true] {
            let Some(claimants) = self.claimants.get(&(journal, key.clone())) else {
                continue;
            };
            for claimant in claimants {
                let Some((slot, (entry, document))) =
                    self.graph.parsed_page(claimant.rel_path_str())
                else {
                    continue;
                };
                let Some(id) = entry.rel_path.clone() else {
                    continue;
                };
                if tine_core::refs::page_key(&entry.name) != key
                    || found.iter().any(|(known, _)| *known == slot)
                {
                    continue;
                }
                found.push((
                    slot,
                    CorpusPage {
                        id,
                        name: entry.name.clone(),
                        kind: entry.kind,
                        document: Arc::clone(document),
                    },
                ));
            }
        }
        found.sort_by_key(|(slot, _)| *slot);
        #[cfg(test)]
        {
            let scanned: Vec<_> = (self.graph.pages_in_order())
                .filter(|(entry, _)| {
                    entry.rel_path.is_some() && tine_core::refs::page_key(&entry.name) == key
                })
                .map(|(entry, _)| entry.rel_path_str().to_owned())
                .collect();
            let keyed: Vec<_> = found
                .iter()
                .map(|(_, page)| page.id.as_str().to_owned())
                .collect();
            assert_eq!(
                keyed, scanned,
                "corpus(Some(name)) drifted from a corpus scan for {name:?}"
            );
        }
        tine_core::Corpus {
            pages: found.into_iter().map(|(_, page)| page).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keyed answers equal a corpus scan's, with page twins, a journal,
    /// a set-aside duplicate-day file, an `id::` held by two pages, a page
    /// whose `title::` names it apart from its file, and Org pages (one named
    /// by `#+TITLE:`, one by its file).
    #[test]
    fn keyed_corpus_and_block_answer_as_a_corpus_scan() {
        let root = tempfile::tempdir().unwrap();
        for dir in ["pages", "journals", "logseq"] {
            fs::create_dir_all(root.path().join(dir)).unwrap();
        }
        let id = "6553f1a0-0000-4000-8000-0000000000aa";
        for (file, text) in [
            ("pages/Twin.md", format!("- first twin\n  id:: {id}\n")),
            (
                "pages/TWIN.md",
                format!("- second twin\n  - nested\n    id:: {id}\n"),
            ),
            ("pages/Other.md", "- other\n".to_owned()),
            ("journals/2026_10_02.md", "- day\n".to_owned()),
            ("journals/2026-10-02.md", "- stray day\n".to_owned()),
            (
                "pages/old file name.md",
                "title:: Shown Title\n\n- titled\n".to_owned(),
            ),
            ("pages/Plain Org.org", "* org heading\n".to_owned()),
            (
                "pages/org file.org",
                "#+TITLE: Org Titled\n\n* titled org\n".to_owned(),
            ),
        ] {
            fs::write(root.path().join(file), text).unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let view = store.whole_graph_reconciled().unwrap();
        let corpus = view.corpus(None, false);
        for name in [
            "Twin",
            "twin",
            "TWIN",
            "Other",
            "Oct 2nd, 2026",
            "Absent",
            "Shown Title",
            "shown title",
            "old file name",
            "Plain Org",
            "Org Titled",
            "org file",
        ] {
            let key = tine_core::refs::page_key(name);
            let scanned: Vec<_> = (corpus.pages.iter())
                .filter(|page| tine_core::refs::page_key(&page.name) == key)
                .map(|page| page.id.as_str().to_owned())
                .collect();
            let keyed: Vec<_> = (view.corpus(Some(name), false).pages.into_iter())
                .map(|page| page.id.as_str().to_owned())
                .collect();
            assert_eq!(keyed, scanned, "{name}");
        }
        // The fixture means what it says: each renamed page answers to its
        // title only, and both Org pages are found.
        let ids = |name: &str| -> Vec<String> {
            (view.corpus(Some(name), false).pages.into_iter())
                .map(|page| page.id.as_str().to_owned())
                .collect()
        };
        assert_eq!(ids("Shown Title"), ["pages/old file name.md"]);
        assert_eq!(ids("Plain Org"), ["pages/Plain Org.org"]);
        assert_eq!(ids("Org Titled"), ["pages/org file.org"]);
        assert!(ids("old file name").is_empty() && ids("org file").is_empty());
        let active = view.corpus(None, true);
        // The prefix inventory uses the same semantic owners as the active
        // corpus, including journals and duplicate physical claimants.
        for prefix in ["", "t", "Oct", "Absent", "Shown", "old", "Org", "Plain"] {
            let key = tine_core::refs::page_key(prefix);
            let mut expected: Vec<_> = active
                .pages
                .iter()
                .filter(|page| tine_core::refs::page_key(&page.name).starts_with(&key))
                .map(|page| (page.name.clone(), page.id.clone()))
                .collect();
            expected.sort_by_key(|(name, _)| tine_core::refs::page_key(name));
            let inventory = view.inventory(InventoryScope::ActiveFilesUnder(prefix));
            let actual: Vec<_> = inventory
                .0
                .iter()
                .map(|entry| {
                    let Resolved::Existing { id, others } = &entry.target else {
                        panic!("active file target")
                    };
                    assert!(others.is_empty());
                    (entry.name.clone(), id.clone())
                })
                .collect();
            assert_eq!(actual, expected, "prefix {prefix:?}");
        }
        let first = active
            .pages
            .iter()
            .find(|page| {
                page.document
                    .roots
                    .iter()
                    .any(|b| b.property("id").as_deref() == Some(id))
                    || page.document.roots.iter().any(|b| {
                        b.children
                            .iter()
                            .any(|c| c.property("id").as_deref() == Some(id))
                    })
            })
            .unwrap();
        // Print finds an embedded block's page through `blocks`: the
        // exact owner index names the page a corpus scan finds first.
        let group = view
            .blocks(&[id.to_owned()], true)
            .unwrap()
            .remove(0)
            .unwrap();
        assert_eq!(
            (group.page.as_str(), group.kind),
            (first.name.as_str(), first.kind)
        );
        assert!(group.blocks[0].raw.contains(&format!("id:: {id}")));
        assert!(view
            .blocks(&["6553f1a0-0000-4000-8000-0000000000ff".to_owned()], true)
            .unwrap()[0]
            .is_none());
        // A `title::` edit renames the page in place: the next view answers
        // to the new title only, as a scan of that view does.
        fs::write(
            root.path().join("pages/old file name.md"),
            "title:: Retitled\n\n- titled\n",
        )
        .unwrap();
        store.refresh(crate::Depth::Stamps).unwrap();
        let view = store.whole_graph_reconciled().unwrap();
        let corpus = view.corpus(None, false);
        for name in ["Retitled", "Shown Title", "old file name"] {
            let key = tine_core::refs::page_key(name);
            let scanned: Vec<_> = (corpus.pages.iter())
                .filter(|page| tine_core::refs::page_key(&page.name) == key)
                .map(|page| page.id.as_str().to_owned())
                .collect();
            let keyed: Vec<_> = (view.corpus(Some(name), false).pages.into_iter())
                .map(|page| page.id.as_str().to_owned())
                .collect();
            assert_eq!(keyed, scanned, "after retitle: {name}");
        }
        assert_eq!(
            (view.corpus(Some("Retitled"), false).pages.into_iter())
                .map(|page| page.id.as_str().to_owned())
                .collect::<Vec<_>>(),
            ["pages/old file name.md"]
        );
        store.close();
    }
}
