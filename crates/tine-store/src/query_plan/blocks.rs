//! Block ranking and ordered admission for a friendly search.
use super::*;

pub(super) fn execute_blocks(
    plan: &QueryPlan,
    graph: &impl GraphRead,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    graph.with_pages(|pages| {
        let mut heap = BinaryHeap::new();
        let mut has_more = false;
        let sort = friendly_sort_fields(plan.block_view.as_ref());
        let selection_limit = if sort.is_empty() {
            branch.limit
        } else {
            usize::MAX
        };
        let mut index = 0usize;
        for (entry, doc) in pages {
            if cancelled() {
                return None;
            }
            if let Some(scope) = &plan.page_scope {
                let selected = match scope.path.as_deref() {
                    Some(path) => entry.rel_path_str() == path,
                    None => {
                        entry.kind == scope.page_kind && refs::same_page(&entry.name, &scope.name)
                    }
                };
                if !selected {
                    continue;
                }
            }
            let mut ancestors = Vec::new();
            walk_blocks(&doc.roots, &mut ancestors, &mut |block, path| {
                if cancelled() {
                    return false;
                }
                let candidate_index = index;
                index = index.saturating_add(1);
                let visible = block.visible_text();
                let lower = block.visible_folded(plan.remove_accents);
                if let Some(relevance) = block_relevance(plan, &branch.predicate, visible, lower) {
                    has_more |= heap.len() >= selection_limit;
                    let retain = heap.len() < selection_limit
                        || heap.peek().is_some_and(|worst: &ScoredBlock<'_>| {
                            relevance.cmp_quality(&worst.relevance) == Ordering::Greater
                                || (relevance.cmp_quality(&worst.relevance) == Ordering::Equal
                                    && (entry.rel_path_str(), candidate_index)
                                        < (worst.page.rel_path_str(), worst.index))
                        });
                    if retain {
                        push_block(
                            &mut heap,
                            selection_limit,
                            ScoredBlock {
                                relevance,
                                index: candidate_index,
                                page: entry,
                                block,
                                breadcrumb: path
                                    .iter()
                                    .map(|ancestor| crate::query::crumb_line(ancestor))
                                    .collect(),
                            },
                        );
                    }
                }
                true
            });
            if cancelled() {
                return None;
            }
        }
        let mut winners = heap.into_vec();
        if sort.is_empty() {
            winners.sort_by(|a, b| {
                b.relevance.cmp_quality(&a.relevance).then_with(|| {
                    (a.page.rel_path_str(), a.index).cmp(&(b.page.rel_path_str(), b.index))
                })
            });
        } else {
            let ascending: Vec<bool> = sort.iter().map(|(_, asc)| *asc).collect();
            let mut decorated: Vec<_> = winners
                .into_iter()
                .map(|winner| {
                    let keys = sort
                        .iter()
                        .map(|(field, _)| {
                            crate::query::exec::block_sort_decor(
                                field,
                                winner.page,
                                winner.block,
                                || 0,
                            )
                        })
                        .collect::<Vec<_>>();
                    (keys, winner)
                })
                .collect();
            decorated.sort_by(|a, b| {
                tine_core::query::sort::compare_sort_decorations(&a.0, &b.0, &ascending).then_with(
                    || {
                        (a.1.page.rel_path_str(), a.1.index)
                            .cmp(&(b.1.page.rel_path_str(), b.1.index))
                    },
                )
            });
            winners = decorated.into_iter().map(|(_, winner)| winner).collect();
            has_more = winners.len() > branch.limit;
            winners.truncate(branch.limit);
        }
        Some((
            winners
                .into_iter()
                .map(|winner| {
                    let visible = winner.block.visible_text();
                    let lower = winner.block.visible_folded(plan.remove_accents);
                    let matched = eval_ranked_block_expr(plan, &branch.predicate, visible, lower)
                        .expect("rank and evidence evaluators must agree");
                    // Search hits are result identities, not independent copies
                    // of their entire descendant trees. The source page owns the
                    // hierarchy and live consumers hydrate it once per page.
                    let mut dto = tine_core::projection::block_to_shallow_dto(winner.block);
                    dto.breadcrumb = winner.breadcrumb;
                    QueryHit::Block {
                        page: winner.page.name.clone(),
                        kind: winner.page.kind,
                        path: winner.page.rel_path.clone().unwrap(),
                        block: dto,
                        display_text: visible.to_owned(),
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
