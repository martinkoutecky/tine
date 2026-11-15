//! Page-name, alias, and page-content ranking for one friendly-search plan.
use super::*;

#[derive(Debug)]
pub(super) enum PageCandidate {
    File(usize),
    Referenced(PageEntry),
}

#[derive(Debug)]
pub(super) struct ScoredPage {
    pub(super) from_content: bool,
    pub(super) score: i32,
    pub(super) match_class: ObjectiveMatchClass,
    pub(super) matched_text: String,
    pub(super) matched_alias: Option<String>,
    pub(super) tie_key: String,
    pub(super) candidate: PageCandidate,
    pub(super) content_text: Option<String>,
}

impl PartialEq for ScoredPage {
    fn eq(&self, other: &Self) -> bool {
        self.from_content == other.from_content
            && self.match_class == other.match_class
            && self.score == other.score
            && self.tie_key == other.tie_key
    }
}
impl Eq for ScoredPage {}
impl PartialOrd for ScoredPage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for ScoredPage {
    fn cmp(&self, other: &Self) -> Ordering {
        // Max-heap root is the WORST retained candidate, ready for eviction;
        // "greater" therefore means "worse" and the ascending order is the
        // result order: name matches before content matches, then match
        // class, score (higher is better), tie key.
        self.from_content
            .cmp(&other.from_content)
            .then_with(|| other.match_class.rank().cmp(&self.match_class.rank()))
            .then_with(|| other.score.cmp(&self.score))
            .then_with(|| self.tie_key.cmp(&other.tie_key))
    }
}

/// A page candidate with the authored sort keys of a sorted search (in
/// `ascending`'s directions), so the heap keeps the best `limit` by the sort
/// itself (audit L7s3-02). With no sort, `ascending` is empty and the
/// candidate's own order decides.
pub(super) struct RankedPage<'a> {
    pub(super) page: ScoredPage,
    pub(super) keys: Vec<SortDecor>,
    pub(super) ascending: &'a [bool],
}

impl PartialEq for RankedPage<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for RankedPage<'_> {}
impl PartialOrd for RankedPage<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedPage<'_> {
    /// The result order, as [`ScoredPage`]'s: "less" is better, the heap root
    /// is the worst. The tie key is never reversed by a descending sort key.
    fn cmp(&self, other: &Self) -> Ordering {
        if self.ascending.is_empty() {
            return self.page.cmp(&other.page);
        }
        compare_sort_decorations(&self.keys, &other.keys, self.ascending)
            .then_with(|| self.page.tie_key.cmp(&other.page.tie_key))
    }
}

/// Keep the best `limit` of what is pushed.
pub(super) fn push_page<T: Ord>(heap: &mut BinaryHeap<T>, limit: usize, candidate: T) {
    let full = heap.len() >= limit;
    if full && !heap.peek().is_some_and(|worst| candidate < *worst) {
        return;
    }
    if full {
        *heap.peek_mut().unwrap() = candidate;
    } else {
        heap.push(candidate);
    }
    #[cfg(test)]
    super::PAGE_SELECTION.with(|c| c.set((c.get().0 + 1, c.get().1.max(heap.len()))));
}

/// Match + page relevance in one cached-lowercase pass. AND takes the best
/// positive clause; OR uses the first matching branch, mirroring
/// `Matcher::score_name`.
fn page_base_score(
    plan: &QueryPlan,
    expr: &QueryExpr,
    original: &str,
    lower: &str,
) -> Option<(i32, ObjectiveMatchClass)> {
    match expr {
        QueryExpr::Never => None,
        QueryExpr::Text(pred) if pred.field != TextField::PageName => None,
        QueryExpr::Text(pred) => match pred.mode {
            TextMatchMode::Fuzzy => fuzzy_name_score(lower, &pred.value)
                .map(|(score, class)| {
                    if class == ObjectiveMatchClass::Exact
                        && plan
                            .page_exact
                            .as_ref()
                            .is_some_and(|exact| page_key(original) != *exact)
                    {
                        (1000, ObjectiveMatchClass::Prefix)
                    } else {
                        (score, class)
                    }
                })
                .or_else(|| plan.transliteration_score(original, pred)),
            TextMatchMode::Fragments => {
                plan.fragments_matcher(pred)
                    .first_offset(lower)
                    .map(|first| {
                        if first == 0 {
                            (1000, ObjectiveMatchClass::Prefix)
                        } else {
                            (500, ObjectiveMatchClass::Substring)
                        }
                    })
            }
            TextMatchMode::Words => plan.words_matcher(pred).first_offset(lower).map(|first| {
                if first == 0 {
                    (1000, ObjectiveMatchClass::Prefix)
                } else {
                    (500, ObjectiveMatchClass::Substring)
                }
            }),
            TextMatchMode::Regex => plan
                .regexes
                .get(&pred.clause_id)
                .is_some_and(|regex| regex.is_match(original))
                .then_some((500, ObjectiveMatchClass::Substring)),
            TextMatchMode::Contains | TextMatchMode::Phrase => {
                if lower == pred.value {
                    if plan
                        .page_exact
                        .as_ref()
                        .is_some_and(|exact| page_key(original) != *exact)
                    {
                        Some((1000, ObjectiveMatchClass::Prefix))
                    } else {
                        Some((1500, ObjectiveMatchClass::Exact))
                    }
                } else if lower.starts_with(&pred.value) {
                    Some((1000, ObjectiveMatchClass::Prefix))
                } else if lower.contains(&pred.value) {
                    Some((500, ObjectiveMatchClass::Substring))
                } else {
                    plan.transliteration_score(original, pred)
                }
            }
        },
        QueryExpr::And(children) => {
            let mut score = 0;
            let mut class = ObjectiveMatchClass::Exact;
            for child in children {
                let (child_score, child_class) = page_base_score(plan, child, original, lower)?;
                score = score.max(child_score);
                if child_class.rank() < class.rank() {
                    class = child_class;
                }
            }
            Some((score, class))
        }
        QueryExpr::Or(children) => {
            let mut expansion = None;
            for child in children {
                if let Some(hit) = page_base_score(plan, child, original, lower) {
                    // Preserve the first native branch's existing authority,
                    // even when an earlier alternative matched via expansion.
                    if hit.1 != ObjectiveMatchClass::Transliteration {
                        return Some(hit);
                    }
                    if expansion.is_none() {
                        expansion = Some(hit);
                    }
                }
            }
            expansion
        }
        QueryExpr::Not(child) => {
            (if plan.transliteration {
                page_base_score(plan, child, original, lower).is_none()
            } else {
                !eval_expr_fast(plan, child, TextField::PageName, original, lower)
            })
            // A successful exclusion contributes no positive relevance and
            // therefore must not weaken the class supplied by an AND sibling.
            .then_some((0, ObjectiveMatchClass::Exact))
        }
    }
}

pub(super) fn best_page_match(
    plan: &QueryPlan,
    expr: &QueryExpr,
    page_name: &str,
    aliases: &[String],
) -> Option<(i32, ObjectiveMatchClass, String, Option<String>)> {
    #[cfg(test)]
    COMPLETION_SCORES.with(|count| count.set(count.get() + 1));
    let page_match = page_base_score(plan, expr, page_name, &plan.fold(page_name));
    let mut best = page_match.map(|(score, class)| (score, class, page_name.to_string(), None));
    for alias in aliases {
        let Some((score, class)) = page_base_score(plan, expr, alias, &plan.fold(alias)) else {
            continue;
        };
        let replace = best.as_ref().is_none_or(|(best_score, best_class, _, _)| {
            class.rank() > best_class.rank() || (class == *best_class && score > *best_score)
        });
        if replace {
            best = Some((score, class, alias.clone(), Some(alias.clone())));
        }
    }
    // Upgrade only an outcome that already satisfied the parsed expression.
    // This repairs the objective class for ordinary multi-word titles without
    // bypassing NOT/OR/regex membership semantics for syntax-looking names.
    if let Some(exact) = plan.page_exact.as_deref() {
        if page_match.is_some() && page_key(page_name) == exact {
            return Some((
                1500,
                ObjectiveMatchClass::Exact,
                page_name.to_string(),
                None,
            ));
        }
        if let Some(alias) = aliases.iter().find(|alias| {
            page_key(alias) == exact
                && page_base_score(plan, expr, alias, &plan.fold(alias)).is_some()
        }) {
            return Some((
                1500,
                ObjectiveMatchClass::Exact,
                alias.clone(),
                Some(alias.clone()),
            ));
        }
    }
    best
}

pub(super) fn execute_pages(
    plan: &QueryPlan,
    graph: &ReadSnapshot,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    let file_pages = graph.page_list_arc();
    let include_names = plan.page_match_scope != FriendlyPageMatchScope::Content;
    let include_content = plan.page_match_scope != FriendlyPageMatchScope::Names;
    let mut content_by_path: HashMap<String, (BlockRelevance, String, bool)> = HashMap::new();
    if include_content {
        let block_expr = plan
            .branches
            .iter()
            .find(|branch| branch.target == QueryTarget::Blocks)
            .map(|branch| &branch.predicate)?;
        graph.with_pages(|pages| {
            let config = tine_core::query::atom::ParseConfig::from_config(graph.config());
            for (entry, doc) in pages {
                if cancelled() {
                    break;
                }
                let mut ancestors = Vec::new();
                let pre: Vec<_> = doc
                    .pre_block
                    .as_deref()
                    .filter(|pre| text::pre_block_may_match(plan, block_expr, pre))
                    .map(|pre| text::pre_blocks_with_config(entry, pre, &config))
                    .unwrap_or_default();
                for (is_pre, roots) in [(true, pre.as_slice()), (false, doc.roots.as_slice())] {
                    walk_blocks(roots, &mut ancestors, false, &mut |block, _, is_comment| {
                        if cancelled() {
                            return false;
                        }
                        let text = search_text(block, plan, graph);
                        if is_pre && text.original.chars().all(char::is_whitespace) {
                            return true;
                        }
                        if let Some(relevance) =
                            block_relevance(plan, block_expr, &text.original, &text.folded)
                        {
                            let key = entry.rel_path_str().to_owned();
                            let replace = content_by_path.get(&key).is_none_or(|(best, _, _)| {
                                relevance.cmp_quality(best) == Ordering::Greater
                            });
                            if replace {
                                content_by_path.insert(
                                    key,
                                    (relevance, text.original.into_owned(), is_comment),
                                );
                            }
                        }
                        true
                    });
                }
            }
        });
        if cancelled() {
            return None;
        }
    }
    let mut aliases_by_owner: HashMap<String, Vec<String>> = HashMap::new();
    for (alias, _, owner_rel_path) in if plan.separate_aliases {
        graph.page_alias_spellings_with_owners()
    } else {
        graph.page_aliases_with_owners()
    } {
        aliases_by_owner
            .entry(owner_rel_path)
            .or_default()
            .push(alias);
    }
    let sort = friendly_sort_fields(plan.page_view.as_ref());
    let ascending: Vec<bool> = sort.iter().map(|(_, asc)| *asc).collect();
    // A candidate's sort keys: a file page's from its own preamble (an alias
    // candidate sorts as its owner), a referenced name's from the name alone.
    let ranked = |page: ScoredPage| {
        let keys = match &page.candidate {
            _ if sort.is_empty() => Vec::new(),
            PageCandidate::File(index) => {
                let entry = &file_pages[*index];
                graph.with_pages(|pages| {
                    let properties = (pages.positions.get(entry.rel_path_str()))
                        .and_then(|slot| pages.get(*slot))
                        .map(|(_, doc)| crate::query::page_facets(doc).0)
                        .unwrap_or_default();
                    sort.iter()
                        .map(|(field, _)| {
                            crate::query::exec::page_sort_decor(field, entry, &properties, 0)
                        })
                        .collect()
                })
            }
            PageCandidate::Referenced(entry) => sort
                .iter()
                .map(|(field, _)| crate::query::exec::page_sort_decor(field, entry, &[], 0))
                .collect(),
        };
        RankedPage {
            page,
            keys,
            ascending: &ascending,
        }
    };
    let mut heap = BinaryHeap::new();
    let mut has_more = false;
    let mut completion_names: HashSet<String> =
        file_pages.iter().map(|page| page_key(&page.name)).collect();
    for (index, page) in file_pages.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let aliases = aliases_by_owner
            .get(page.rel_path_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let name_match = include_names
            .then(|| {
                best_page_match(
                    plan,
                    &branch.predicate,
                    &page.name,
                    if plan.separate_aliases { &[] } else { aliases },
                )
            })
            .flatten();
        let content_match = content_by_path.get(page.rel_path_str());
        if let Some((base_score, match_class, matched_text, matched_alias, content_text)) =
            name_match
                .map(|(score, class, text, alias)| (score, class, text, alias, None))
                .or_else(|| {
                    content_match.map(|(relevance, text, _)| {
                        (
                            relevance.score(),
                            ObjectiveMatchClass::Fuzzy,
                            text.clone(),
                            None,
                            Some(text.clone()),
                        )
                    })
                })
        {
            has_more |= heap.len() >= branch.limit;
            push_page(
                &mut heap,
                branch.limit,
                ranked(ScoredPage {
                    from_content: content_text.is_some(),
                    score: base_score - page.name.len() as i32,
                    match_class,
                    matched_text,
                    matched_alias,
                    tie_key: page.rel_path_str().to_owned(),
                    candidate: PageCandidate::File(index),
                    content_text,
                }),
            );
        }
        if plan.separate_aliases {
            for alias in aliases {
                // A physical page wins over an alias with the same identity.
                if !completion_names.insert(page_key(alias)) {
                    continue;
                }
                if let Some((base_score, match_class, matched_text, _)) =
                    best_page_match(plan, &branch.predicate, alias, &[])
                {
                    has_more |= heap.len() >= branch.limit;
                    push_page(
                        &mut heap,
                        branch.limit,
                        ranked(ScoredPage {
                            from_content: false,
                            score: base_score - alias.len() as i32,
                            match_class,
                            matched_text,
                            matched_alias: Some(alias.clone()),
                            tie_key: format!("{}:{}", page_key(alias), page.rel_path_str()),
                            candidate: PageCandidate::File(index),
                            content_text: None,
                        }),
                    );
                }
            }
        }
    }
    let mut have = completion_names;
    // GH #353 / #623: an alias of a file page names THAT page. The owner carries
    // the identity (with `matched_alias` as display context), so the alias text
    // must never also appear as a referenced, path-less page candidate: selecting
    // that phantom row opened a standalone alias-named page instead of the owner.
    // `aliases_by_owner` is the one physical-owner alias inventory above.
    have.extend(
        aliases_by_owner
            .values()
            .flatten()
            .map(|alias| page_key(alias)),
    );
    // The resident referenced-name table answers; only the editing block's
    // page is re-read for its baseline (cost-model audit 2026-11-02, row 2a).
    let referenced_names = if include_names {
        graph.referenced_names_for_completion(
            plan.editor_reference_baseline
                .as_ref()
                .map(|(owner, id, original)| (owner.as_str(), id.as_str(), original.as_str())),
        )
    } else {
        Vec::new()
    };
    // Namespace parents are pages too (OG's page search lists them; the one
    // answerer is `model::namespace_parents`), offered beside the referenced
    // names. They follow the baseline-adjusted names the editor sees.
    let parents = if include_names {
        graph
            .completion_namespace_parents(&file_pages, &referenced_names)
            .into_iter()
            .map(|name| std::borrow::Cow::Owned(name.to_owned()))
            .collect()
    } else {
        Vec::new()
    };
    let parents: Vec<std::borrow::Cow<'_, str>> = parents;
    for name in parents.into_iter().chain(referenced_names) {
        if cancelled() {
            return None;
        }
        let key = page_key(&name);
        if have.contains(&key) {
            continue;
        }
        if let Some((base_score, match_class, matched_text, matched_alias)) =
            best_page_match(plan, &branch.predicate, &name, &[])
        {
            has_more |= heap.len() >= branch.limit;
            let score = base_score - name.len() as i32;
            push_page(
                &mut heap,
                branch.limit,
                ranked(ScoredPage {
                    from_content: false,
                    score,
                    match_class,
                    matched_text,
                    matched_alias,
                    tie_key: tine_core::refs::page_key(&name),
                    candidate: PageCandidate::Referenced(PageEntry {
                        name: name.into_owned(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: None,
                        path: std::path::PathBuf::new(),
                    }),
                    content_text: None,
                }),
            );
        }
    }
    let winners: Vec<ScoredPage> = (heap.into_sorted_vec().into_iter())
        .map(|ranked| ranked.page)
        .collect();
    // Hydrate only admitted physical pages through the shared preamble reader.
    // Search rows do not need the graph-wide block facts or property registry.
    let wanted: HashSet<&str> = winners
        .iter()
        .filter_map(|winner| match winner.candidate {
            PageCandidate::File(index) => Some(file_pages[index].rel_path_str()),
            PageCandidate::Referenced(_) => None,
        })
        .collect();
    let rows = if wanted.is_empty() {
        HashMap::new()
    } else {
        graph.with_pages(|pages| {
            pages
                .iter()
                .filter(|(entry, _)| wanted.contains(entry.rel_path_str()))
                .map(|(entry, doc)| {
                    let (properties, _) = crate::query::page_facets(doc);
                    (
                        entry.rel_path_str().to_owned(),
                        tine_core::query::ir::PageRow {
                            path: entry.rel_path_str().to_owned(),
                            name: entry.name.clone(),
                            kind: entry.kind,
                            journal_day: entry.date_key,
                            properties,
                        },
                    )
                })
                .collect::<HashMap<_, _>>()
        })
    };
    Some((
        winners
            .into_iter()
            .map(|winner| {
                let page = match winner.candidate {
                    PageCandidate::File(index) => file_pages[index].clone(),
                    PageCandidate::Referenced(page) => page,
                };
                let evidence = if let Some(ref text) = winner.content_text {
                    let block_expr = &plan
                        .branches
                        .iter()
                        .find(|b| b.target == QueryTarget::Blocks)
                        .unwrap()
                        .predicate;
                    eval_expr(plan, block_expr, TextField::VisibleContent, text)
                } else {
                    eval_expr(
                        plan,
                        &branch.predicate,
                        TextField::PageName,
                        &winner.matched_text,
                    )
                }
                .map(|matched| matched.evidence)
                .unwrap_or_default();
                QueryHit::Page {
                    is_comment: winner.content_text.is_some()
                        && content_by_path
                            .get(page.rel_path_str())
                            .is_some_and(|(_, _, comment)| *comment),
                    display_text: winner.matched_text,
                    row: page
                        .rel_path
                        .as_ref()
                        .and_then(|_| rows.get(page.rel_path_str()).cloned()),
                    page,
                    evidence,
                    score: winner.score,
                    match_class: winner.match_class,
                    matched_alias: winner.matched_alias,
                }
            })
            .collect(),
        has_more,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn page_search_returns_authored_properties_without_building_graph_query_facts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("pages")).unwrap();
        for i in 0..30 {
            std::fs::write(dir.path().join(format!("pages/P{i}.md")), "- unrelated\n").unwrap();
        }
        std::fs::write(dir.path().join("pages/Hit.md"), "color:: blue\n\n- body\n").unwrap();
        std::fs::write(
            dir.path().join("pages/Hit Org.org"),
            "#+COLOR: red\n* body\n",
        )
        .unwrap();
        let store = crate::Store::open(dir.path(), Default::default())
            .unwrap()
            .0;
        let graph = store.whole_graph().unwrap();
        let request = crate::SearchRequest {
            transliteration: false,
            text: "Hit".into(),
            within: None,
            page_limit: 10,
            block_limit: 0,
            explain: false,
            page_match_scope: None,
            page_view: None,
            block_view: None,
        };
        crate::query::index::BUILT_FACT_PAGES.with(|count| count.set(0));
        let found = graph
            .search(
                &request,
                &crate::Cancel(Arc::new(std::sync::atomic::AtomicBool::new(false))),
            )
            .unwrap();
        let properties: Vec<_> = found
            .hits
            .iter()
            .filter_map(|hit| match hit {
                QueryHit::Page { row: Some(row), .. } => {
                    Some((row.name.as_str(), row.properties.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            properties,
            vec![
                ("Hit", vec![("color".into(), "blue".into())]),
                ("Hit Org", vec![("color".into(), "red".into())]),
            ]
        );
        let work = crate::query::index::BUILT_FACT_PAGES.with(|count| count.get());
        store.close();
        assert_eq!(work, 0,
            "I-13/I-25: page-search winners need their preambles, not graph query facts; exemplar query_plan/pages.rs");
    }
}

#[cfg(test)]
mod page_order_tests {
    use super::*;

    fn page(from_content: bool, class: ObjectiveMatchClass, score: i32, tie: &str) -> ScoredPage {
        ScoredPage {
            from_content,
            score,
            match_class: class,
            matched_text: String::new(),
            matched_alias: None,
            tie_key: tie.to_owned(),
            candidate: PageCandidate::File(0),
            content_text: from_content.then(String::new),
        }
    }

    /// The bounded heap evicts by `Ord` (root = worst) and admits by the same
    /// order (checkpoint-5 L02 B3, I-12). Before, `Ord` ranked a content match
    /// BETTER than a name match while admission and the result sort ranked it
    /// worse, so a full heap evicted the best name match and kept a content
    /// match.
    #[test]
    fn page_heap_keeps_the_best_name_matches_over_content_matches() {
        use ObjectiveMatchClass::*;
        let mut heap = BinaryHeap::new();
        push_page(&mut heap, 2, page(false, Exact, 100, "a"));
        push_page(&mut heap, 2, page(true, Fuzzy, 90, "b"));
        push_page(&mut heap, 2, page(false, Prefix, 50, "c"));
        let mut kept: Vec<_> = heap.into_vec();
        kept.sort();
        let kept: Vec<_> = kept.iter().map(|p| p.tie_key.as_str()).collect();
        assert_eq!(kept, ["a", "c"], "the content match is the one evicted");
    }

    /// Admission and eviction are one order, `Ord` itself: whatever order the
    /// candidates arrive in, a full heap holds exactly the first `limit` of the
    /// sorted pool (checkpoint-5 L02 B3, I-12).
    #[test]
    fn page_heap_keeps_the_first_limit_of_the_sorted_pool_in_any_arrival_order() {
        use ObjectiveMatchClass::*;
        let pool = || {
            let mut all = Vec::new();
            for content in [false, true] {
                for class in [Exact, Prefix, Substring, Fuzzy] {
                    for score in [1, 2] {
                        for tie in ["x", "y"] {
                            all.push(page(content, class, score, tie));
                        }
                    }
                }
            }
            all
        };
        let id = |p: &ScoredPage| {
            (
                p.from_content,
                p.match_class.rank(),
                p.score,
                p.tie_key.clone(),
            )
        };
        let mut sorted = pool();
        sorted.sort();
        let sorted: Vec<_> = sorted.iter().map(id).collect();
        for limit in [1, 3, 8, 31] {
            for rotation in 0..sorted.len() {
                for reverse in [false, true] {
                    let mut arrival = pool();
                    arrival.rotate_left(rotation);
                    if reverse {
                        arrival.reverse();
                    }
                    let mut heap = BinaryHeap::new();
                    for candidate in arrival {
                        push_page(&mut heap, limit, candidate);
                    }
                    let kept: Vec<_> = heap.into_sorted_vec().iter().map(id).collect();
                    assert_eq!(kept, sorted[..limit], "limit {limit} rotation {rotation}");
                }
            }
        }
    }
}
