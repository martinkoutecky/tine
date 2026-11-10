//! Which reference-only names a rename renames (small-1109c r2): the rename's
//! rewriter renames every reference to a namespace member of a renamed page
//! by the prefix rule (`refs::renamed_page_name`), also when no file claims
//! that member, so its identity moves with the rename. The rename report
//! lists them (`RenameReport::renamed`) and the window follows that list.
use super::*;
use tine_core::refs::{namespace_refs, page_key, renamed_page_name};

impl WholeGraph {
    /// Every name strictly under `old/` that a reference in this view spells,
    /// itself or as a namespace prefix of a longer reference (a fileless
    /// namespace parent), with the name `renames` (old page key → new name,
    /// the rename's lookup) gives it: `(old spelling, new name)`, once per
    /// page key, in key order. File-claimed names are included too; the
    /// caller keeps the ones it did not already report. Task-marker and
    /// priority evidence is no rewritable text and does not count. Uses only
    /// this view's resident projections, with no file I/O: O(candidate block
    /// references × namespace depth), the walk `journal_rewrite_risks` makes.
    pub fn renamed_reference_names(
        &self,
        old: &str,
        renames: &HashMap<String, String>,
    ) -> Vec<(String, String)> {
        let old_key = page_key(old);
        let under = format!("{old_key}/");
        let ascii_old = old_key.is_ascii();
        let parse_config = tine_core::query::atom::ParseConfig::from_config(self.graph.config());
        let candidates = self.graph.reference_candidate_pages(
            &[old_key.clone()],
            tine_core::model::ReferenceKind::Explicit,
        );
        let mut found = std::collections::BTreeMap::new();
        let mut check = |block: &tine_core::doc::DocBlock| {
            for reference in block.projection().reference_source().explicit {
                // As in `journal_rewrite_risks`: one key test first, exact
                // for an ASCII `old` key.
                if ascii_old && !page_key(&reference.name).starts_with(old_key.as_str()) {
                    continue;
                }
                if matches!(reference.rule, "explicit_task_marker" | "explicit_priority") {
                    continue;
                }
                for name in namespace_refs(&reference.name) {
                    let key = page_key(name);
                    if !key.starts_with(&under) || found.contains_key(&key) {
                        continue;
                    }
                    if let Some(to) = renamed_page_name(name, renames) {
                        found.insert(key, (name.to_owned(), to.into_owned()));
                    }
                }
            }
        };
        for (entry, doc) in &candidates.pages {
            if let Some(pre) = &doc.pre_block {
                for block in crate::query::pre_blocks_with_config(entry, pre, &parse_config) {
                    check(&block);
                }
            }
            let mut pending: Vec<_> = doc.roots.iter().collect();
            while let Some(block) = pending.pop() {
                check(block);
                pending.extend(&block.children);
            }
        }
        found.into_values().collect()
    }
}
