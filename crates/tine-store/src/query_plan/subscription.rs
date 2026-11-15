//! Search subscriptions classify only changed parsed documents, with the same
//! compiled name/visible-content predicates as execution. No graph scan or I/O.
use super::*;
use tine_core::doc::Document;

impl QueryPlan {
    /// A matching page can change its displayed properties/order; a document
    /// containing a matching block can change its excerpt, breadcrumb or rank.
    /// Test both sides at the caller so removals invalidate as well as additions.
    pub(crate) fn touches_search_document(
        &self,
        entry: &PageEntry,
        doc: &Document,
        graph: &ReadSnapshot,
    ) -> bool {
        // Newly referenced virtual names are already in this publication's
        // incremental name-key map. Use each side's map for pinyin additions
        // and removals without rebuilding keys or walking the graph.
        let mut plan = self.clone();
        plan.name_keys = Some(std::sync::Arc::clone(&graph.transliteration_names));
        plan.touches_document(entry, doc, graph.config())
    }

    fn touches_document(
        &self,
        entry: &PageEntry,
        doc: &Document,
        config: &tine_core::Config,
    ) -> bool {
        for branch in &self.branches {
            if branch.target == QueryTarget::Pages
                && self.page_match_scope != FriendlyPageMatchScope::Content
                && pages::best_page_match(
                    self,
                    &branch.predicate,
                    &entry.name,
                    &crate::query::document_alias_spellings(doc),
                )
                .is_some()
            {
                return true;
            }
            // Page search also offers referenced names without physical files.
            // Property links may create those names without matching any visible
            // block text, so they need the same name predicate as physical pages.
            if branch.target == QueryTarget::Pages
                && self.page_match_scope != FriendlyPageMatchScope::Content
                && crate::model::collect_document_referenced_names(doc, config)
                    .iter()
                    .any(|name| {
                        pages::best_page_match(self, &branch.predicate, name, &[]).is_some()
                    })
            {
                return true;
            }
            if branch.target == QueryTarget::Blocks {
                let mut matched = false;
                walk_blocks(&doc.roots, &mut Vec::new(), false, &mut |block, _, _| {
                    matched |= block_relevance(
                        self,
                        &branch.predicate,
                        block.visible_text(),
                        block.visible_folded(self.remove_accents),
                    )
                    .is_some();
                    !matched
                });
                if matched {
                    return true;
                }
            }
        }
        false
    }
}
