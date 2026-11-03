//! `WholeGraph::corpus(Some(name))`: the pages claiming one name, from the
//! view's name table, so a single-page render (print / PDF) never copies or
//! walks every page (cost-model audit 2026-11-02, row 5; I-13, D-10). A
//! parameter on the existing corpus question, not a sibling (SURFACE.txt
//! rule 3). The printed page's embedded blocks come from `WholeGraph::blocks`,
//! which answers from the exact block-owner index.
use super::*;
use tine_core::CorpusPage;

impl WholeGraph {
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
    /// a set-aside duplicate-day file and an `id::` held by two pages.
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
        ] {
            fs::write(root.path().join(file), text).unwrap();
        }
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        let view = store.whole_graph_reconciled().unwrap();
        let corpus = view.corpus(None);
        for name in ["Twin", "twin", "TWIN", "Other", "Oct 2nd, 2026", "Absent"] {
            let key = tine_core::refs::page_key(name);
            let scanned: Vec<_> = (corpus.pages.iter())
                .filter(|page| tine_core::refs::page_key(&page.name) == key)
                .map(|page| page.id.as_str().to_owned())
                .collect();
            let keyed: Vec<_> = (view.corpus(Some(name)).pages.into_iter())
                .map(|page| page.id.as_str().to_owned())
                .collect();
            assert_eq!(keyed, scanned, "{name}");
        }
        let first = corpus
            .pages
            .iter()
            .find(|page| {
                page.document
                    .roots
                    .iter()
                    .any(|b| b.property("id").as_deref() == Some(id))
                    || page.document.roots.iter().any(|b| {
                        b.children.iter().any(|c| {
                            c.children
                                .iter()
                                .any(|d| d.property("id").as_deref() == Some(id))
                        })
                    })
            })
            .unwrap();
        // Print finds an embedded block's page through `blocks`: the
        // exact owner index names the page a corpus scan finds first.
        let group = view.blocks(&[id.to_owned()]).unwrap().remove(0).unwrap();
        assert_eq!(
            (group.page.as_str(), group.kind),
            (first.name.as_str(), first.kind)
        );
        assert!(group.blocks[0].raw.contains(&format!("id:: {id}")));
        assert!(view
            .blocks(&["6553f1a0-0000-4000-8000-0000000000ff".to_owned()])
            .unwrap()[0]
            .is_none());
        store.close();
    }
}
