//! CM3-1 / model §5.3: protect stationary journal identities per occurrence.
use super::*;
use tine_core::reference_evidence::visit_reference_targets;
use tine_core::refs::{namespace_refs, page_key, renamed_page_name};

impl WholeGraph {
    /// Journals whose claim key or directly declared alias would be lost from
    /// an owner occurrence's namespace closure by `old` → `new`. Another
    /// occurrence retaining that key does not excuse the loss. Fixed task
    /// markers/priorities and non-owner files do not contribute risks.
    /// Names are sorted and unique. Uses only this view's resident projections,
    /// with no file I/O: O(candidate block references) to collect the lost
    /// keys, then per lost key the claimant index and the alias relation
    /// (both resident). Only a date-shaped lost key (a renamed date-named
    /// prefix, rare) scans the journal owners' declared aliases, since the
    /// alias relation keys spellings, not days.
    pub fn journal_rewrite_risks(&self, old: &str, new: &str) -> Vec<String> {
        let journal = &self.journal_format;
        let key = |name: &str| {
            journal
                .parse(name.trim())
                .map_or_else(|| page_key(name), |day| page_key(&journal.title(day)))
        };
        let owners = self.graph.real_page_owners();
        let is_owner = |entry: &PageEntry| {
            owners
                .get(&page_key(&entry.name))
                .is_some_and(|(path, _)| *path == entry.path)
        };
        let old_key = page_key(old);
        let ascii_old = old_key.is_ascii();
        let renames = HashMap::from([(old_key.clone(), new.to_owned())]);
        let parse_config = tine_core::query::atom::ParseConfig::from_config(self.graph.config());
        let candidates = self.graph.reference_candidate_pages(
            &[old_key.clone()],
            tine_core::model::ReferenceKind::Explicit,
        );
        // 1. The keys some owner occurrence loses from its closure.
        let mut lost = std::collections::BTreeSet::new();
        let mut check = |block: &tine_core::doc::DocBlock| {
            for reference in block.projection().reference_source().explicit {
                // Cheapest test first: most references on a candidate page do
                // not name `old` or a namespace member of it. One key instead
                // of `renamed_page_name`'s key per prefix. Exact only for an
                // ASCII `old` key: a prefix whose key is ASCII keys to a prefix
                // of the whole name's key (trimming and boundary slashes only
                // shorten the prefix's key; no character lowercases to ASCII
                // differently by position), so the test never skips a renamed
                // reference. A non-ASCII key (e.g. a final sigma) skips it.
                if ascii_old && !page_key(&reference.name).starts_with(old_key.as_str()) {
                    continue;
                }
                let Some(rewritten) = renamed_page_name(&reference.name, &renames) else {
                    continue;
                };
                if matches!(reference.rule, "explicit_task_marker" | "explicit_priority")
                    || !visit_reference_targets(reference, self.graph.config(), |_| true)
                {
                    continue;
                }
                // A prefix kept byte for byte is retained (its folded key is
                // the same); fold date spellings (a date parse) only for the
                // others.
                let after: Vec<&str> = namespace_refs(&rewritten).collect();
                let mut after_folded: Option<Vec<String>> = None;
                for before in namespace_refs(&reference.name) {
                    if after.contains(&before) {
                        continue;
                    }
                    let folded = key(before);
                    let after_folded = after_folded
                        .get_or_insert_with(|| namespace_refs(&rewritten).map(&key).collect());
                    if !after_folded.contains(&folded) {
                        lost.insert(folded);
                    }
                }
            }
        };
        for (entry, doc) in &candidates.pages {
            if !is_owner(entry) {
                continue;
            }
            // The page-property header and the preamble prose, as every
            // reference question reads them (one parse, the graph's property
            // reference policy).
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
        if lost.is_empty() {
            return Vec::new();
        }
        // 2. Which of them a journal owns: its claim key, or an alias a
        // journal owner declares directly.
        let mut risks = std::collections::BTreeSet::new();
        let mut dated = Vec::new();
        for k in &lost {
            for claimant in self.claimants.get(&(true, k.clone())).into_iter().flatten() {
                if claimant.kind == PageKind::Journal {
                    risks.insert(claimant.name.clone());
                }
            }
            if journal.parse(k).is_some() {
                dated.push(k.as_str());
                continue;
            }
            for path in self.graph.alias_owner_paths(k, None).into_iter().flatten() {
                let declarer = self
                    .graph
                    .pages
                    .positions
                    .get(path.as_str())
                    .and_then(|&slot| self.graph.pages.get(slot));
                if let Some((entry, _)) = declarer {
                    if entry.kind == PageKind::Journal && is_owner(entry) {
                        risks.insert(entry.name.clone());
                    }
                }
            }
        }
        if !dated.is_empty() {
            self.graph.with_pages(|pages| {
                for (entry, doc) in pages.iter().filter(|(e, _)| e.kind == PageKind::Journal) {
                    if is_owner(entry)
                        && crate::query::document_alias_spellings(doc)
                            .iter()
                            .any(|alias| dated.contains(&key(alias).as_str()))
                    {
                        risks.insert(entry.name.clone());
                    }
                }
            });
        }
        risks.into_iter().collect()
    }
}
