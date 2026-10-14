//! All pages backlink metadata over one graph snapshot.
use crate::store::WholeGraph;
use std::collections::{HashMap, HashSet};
use tine_core::config::Config;
use tine_core::doc::DocBlock;
use tine_core::model::ReferenceKind;
use tine_core::reference_evidence::{has_occurrence_prepared, ReferenceNeedles};
use tine_core::{refs, Corpus};

impl WholeGraph {
    /// Number of distinct referring blocks per page, keyed by graph-relative
    /// path. Explicit links, namespace parents and page properties follow the
    /// reference evidence rules; duplicate links within a block count once.
    /// Cost O(P + B + R), with no file reads or persisted records.
    /// Review measurement: ~4.4 ms warm median on 14,694 blocks / 1,075 pages
    /// (31 release runs; metadata alone ~0.187 ms). Includes namespace expansion
    /// and page-header projection. Read separately, coalesced only in All pages;
    /// incremental maintenance remains a follow-up.
    pub fn page_backlink_counts(&self) -> Vec<(String, usize)> {
        page_backlink_counts(&self.corpus(), &self.config.config)
    }
}

pub(super) fn page_backlink_counts(corpus: &Corpus, config: &Config) -> Vec<(String, usize)> {
    fn collect(block: &DocBlock, config: &Config, counts: &mut HashMap<String, usize>) {
        let mut names = HashSet::new();
        for reference in block.projection().reference_source().explicit {
            for name in refs::namespace_refs(&reference.name).map(refs::page_key) {
                let keys = [name.clone()];
                let needles = ReferenceNeedles::new(&keys);
                // Shared evidence rules verify one edge; narrowing avoids
                // rescanning every other link in the same block.
                let mut one = block.projection().reference_source();
                one.explicit = std::slice::from_ref(reference);
                if has_occurrence_prepared(
                    block.raw(),
                    one,
                    &needles,
                    ReferenceKind::Explicit,
                    config,
                ) {
                    names.insert(name);
                }
            }
        }
        for name in names {
            *counts.entry(name).or_default() += 1;
        }
        for child in &block.children {
            collect(child, config, counts);
        }
    }
    let mut counts = HashMap::new();
    let parse = tine_core::query::atom::ParseConfig::from_config(config);
    for page in &corpus.pages {
        if let Some(mut block) =
            tine_core::page_properties::document_page_property_block(&page.document)
        {
            block.set_property_ref_config(&parse);
            collect(&block, config, &mut counts);
        }
        for block in &page.document.roots {
            collect(block, config, &mut counts);
        }
    }
    corpus
        .pages
        .iter()
        .map(|page| {
            (
                page.id.to_string(),
                *counts.get(&refs::page_key(&page.name)).unwrap_or(&0),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn referring_blocks_count_once_including_properties_and_self_but_not_code() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::create_dir(root.path().join("journals")).unwrap();
        std::fs::write(root.path().join("pages/Target.md"), "- [[Target]]\n").unwrap();
        std::fs::write(
            root.path().join("pages/Source.md"),
            "link:: [[Target]]\n\n- [[Target]] [[Target]]\n  - #Target\n- `[[Target]]`\n",
        )
        .unwrap();
        let (store, _, _) = crate::Store::open(root.path(), Default::default()).unwrap();
        let counts: HashMap<_, _> = store
            .whole_graph()
            .unwrap()
            .page_backlink_counts()
            .into_iter()
            .collect();
        assert_eq!(counts.get("pages/Target.md"), Some(&4));
        assert_eq!(counts.get("pages/Source.md"), Some(&0));
    }
}
