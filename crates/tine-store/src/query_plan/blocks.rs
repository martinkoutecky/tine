//! Block ranking and ordered admission for a friendly search.
use super::*;

pub(super) fn execute_blocks(
    plan: &QueryPlan,
    graph: &ReadSnapshot,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    graph.with_pages(|pages| {
        let selected = |entry: &PageEntry| {
            plan.page_scope
                .as_ref()
                .is_none_or(|scope| match scope.path.as_deref() {
                    Some(path) => entry.rel_path_str() == path,
                    None => {
                        entry.kind == scope.page_kind && refs::same_page(&entry.name, &scope.name)
                    }
                })
        };
        // OG indexes its pre-block as a block. Retain these projections for the
        // ranked pool's lifetime; graph documents and persisted indexes stay intact.
        let config = tine_core::query::atom::ParseConfig::from_config(graph.config());
        let pre_blocks: Vec<Vec<DocBlock>> = if plan.include_user_properties {
            pages
                .iter()
                .take_while(|_| !cancelled())
                .map(|(entry, doc)| {
                    if !selected(entry) {
                        return Vec::new();
                    }
                    doc.pre_block
                        .as_deref()
                        .filter(|pre| text::pre_block_may_match(plan, &branch.predicate, pre))
                        .map(|pre| text::pre_blocks_with_config(entry, pre, &config))
                        .unwrap_or_default()
                })
                .collect()
        } else {
            Vec::new()
        };
        if cancelled() {
            return None;
        }
        let sort = friendly_sort_fields(plan.block_view.as_ref());
        let ascending: Vec<bool> = sort.iter().map(|(_, asc)| *asc).collect();
        let mut heap = BinaryHeap::new();
        let mut has_more = false;
        let mut index = 0usize;
        for (page_index, (entry, doc)) in pages.iter().enumerate() {
            if cancelled() {
                return None;
            }
            if !selected(entry) {
                continue;
            }
            let mut ancestors = Vec::new();
            let pre = pre_blocks.get(page_index).map(Vec::as_slice).unwrap_or(&[]);
            for (is_pre, roots) in [(true, pre), (false, doc.roots.as_slice())] {
                walk_blocks(
                    roots,
                    &mut ancestors,
                    false,
                    &mut |block, path, is_comment| {
                        if cancelled() {
                            return false;
                        }
                        let candidate_index = index;
                        index = index.saturating_add(1);
                        let text = search_text(block, plan, graph);
                        // New synthetic headers must not consume a window with
                        // invisible metadata. Ordinary outline behavior stays intact.
                        if is_pre && text.original.chars().all(char::is_whitespace) {
                            return true;
                        }
                        if let Some(relevance) =
                            block_relevance(plan, &branch.predicate, &text.original, &text.folded)
                        {
                            has_more |= heap.len() >= branch.limit;
                            // The key is read from the candidate, so every match
                            // pays for it; only a kept one is retained.
                            let keys = sort
                                .iter()
                                .map(|(field, _)| {
                                    crate::query::exec::block_sort_decor(field, entry, block, || 0)
                                })
                                .collect();
                            let candidate = ScoredBlock {
                                relevance,
                                index: candidate_index,
                                page: entry,
                                block,
                                is_comment,
                                keys,
                                ascending: &ascending,
                                ancestors: Vec::new(),
                            };
                            push_block(&mut heap, branch.limit, candidate, path);
                        }
                        true
                    },
                );
            }
            if cancelled() {
                return None;
            }
        }
        let winners = heap.into_sorted_vec();
        Some((
            winners
                .into_iter()
                .map(|winner| {
                    let text = search_text(winner.block, plan, graph);
                    let matched = eval_ranked_block_expr(
                        plan,
                        &branch.predicate,
                        &text.original,
                        &text.folded,
                    )
                    .expect("rank and evidence evaluators must agree");
                    // Search hits are result identities, not independent copies
                    // of their entire descendant trees. The source page owns the
                    // hierarchy and live consumers hydrate it once per page.
                    let mut dto = tine_core::projection::block_to_shallow_dto(winner.block);
                    dto.page_property = winner.block.uuid.text().starts_with("page-property:");
                    dto.page_preamble = winner.block.uuid.text().starts_with("page-preamble:");
                    dto.breadcrumb = winner
                        .ancestors
                        .iter()
                        .map(|ancestor| crate::query::crumb_line(ancestor))
                        .collect();
                    QueryHit::Block {
                        page: winner.page.name.clone(),
                        kind: winner.page.kind,
                        path: winner.page.rel_path.clone().unwrap(),
                        block: dto,
                        is_comment: winner.is_comment,
                        display_text: text.original.into_owned(),
                        evidence: matched.evidence,
                        score: winner.relevance.score(),
                        match_class: winner.relevance.match_class,
                    }
                })
                .collect(),
            has_more,
        ))
    })
}

pub(super) fn text_predicate_relevance(
    plan: &QueryPlan,
    pred: &TextPredicate,
    original: &str,
    lower: &str,
) -> Option<BlockRelevance> {
    let mut leg = 0;
    let (match_class, word_boundary, first_offset, occurrences) = match pred.mode {
        TextMatchMode::Contains | TextMatchMode::Phrase => {
            let mut matches = lower.match_indices(&pred.value);
            let (first, _) = matches.next()?;
            let occurrences = 1 + matches.count();
            let match_class = if lower == pred.value {
                ObjectiveMatchClass::Exact
            } else if first == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            (
                match_class,
                starts_at_word_boundary(lower, first),
                lower[..first].encode_utf16().count(),
                occurrences,
            )
        }
        TextMatchMode::Regex => {
            let regex = plan.regexes.get(&pred.clause_id)?;
            let mut matches = regex.find_iter(original);
            let first = matches.next()?;
            let occurrences = 1 + matches.count();
            let match_class = if first.start() == 0 && first.end() == original.len() {
                ObjectiveMatchClass::Exact
            } else if first.start() == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            (
                match_class,
                starts_at_word_boundary(original, first.start()),
                original[..first.start()].encode_utf16().count(),
                occurrences,
            )
        }
        TextMatchMode::Fragments => {
            let first = plan.fragments_matcher(pred).first_offset(lower)?;
            let match_class = if lower.split_whitespace().eq(pred.value.split_whitespace()) {
                ObjectiveMatchClass::Exact
            } else if first == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            // The picker's ranking before this mode (Fuzzy) counted one
            // occurrence; keep that so equal-quality blocks keep their order.
            let occurrences = 1;
            (
                match_class,
                starts_at_word_boundary(lower, first),
                lower[..first].encode_utf16().count(),
                occurrences,
            )
        }
        TextMatchMode::Words => {
            let first = plan.words_matcher(pred).first_offset(lower)?;
            leg = 1;
            let match_class = if first == 0 {
                ObjectiveMatchClass::Prefix
            } else {
                ObjectiveMatchClass::Substring
            };
            // Whole tokens start at a word boundary by construction.
            (match_class, true, lower[..first].encode_utf16().count(), 1)
        }
        TextMatchMode::Fuzzy => {
            let (_, match_class) = fuzzy_name_score(lower, &pred.value)?;
            let first = lower.find(&pred.value).unwrap_or(0);
            (
                match_class,
                starts_at_word_boundary(lower, first),
                lower[..first].encode_utf16().count(),
                1,
            )
        }
    };
    Some(BlockRelevance {
        leg,
        match_class,
        word_boundary,
        first_offset,
        text_len: ranking_text_len(original),
        occurrences,
        positive: true,
    })
}

#[cfg(test)]
thread_local! {
    static RANK_LENGTH_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn ranking_text_len(original: &str) -> usize {
    #[cfg(test)]
    RANK_LENGTH_BYTES.with(|count| count.set(count.get() + original.len()));
    original.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn graph_search_measures_rank_length_only_for_matching_blocks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        for i in 0..30 {
            std::fs::write(
                dir.path().join(format!("pages/P{i}.md")),
                "- unrelated text without a hit\n",
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("pages/Hit.md"), "- needle 😀\n").unwrap();
        let store = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let graph = store.whole_graph().unwrap();
        let request = crate::SearchRequest {
            transliteration: false,
            text: "needle".into(),
            within: None,
            page_limit: 0,
            block_limit: 10,
            explain: false,
            page_match_scope: None,
            page_view: None,
            block_view: None,
        };
        RANK_LENGTH_BYTES.with(|count| count.set(0));
        let found = graph
            .search(
                &request,
                &crate::Cancel(Arc::new(std::sync::atomic::AtomicBool::new(false))),
            )
            .unwrap();
        assert_eq!(found.hits.len(), 1);
        let work = RANK_LENGTH_BYTES.with(|count| count.get());
        store.close();
        assert_eq!(work, "needle 😀".len(),
            "I-25: rejected graph-search candidates need no rank-length pass; exemplar query_plan/blocks.rs");
    }
}
