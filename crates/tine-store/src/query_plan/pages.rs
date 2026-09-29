//! Page-name, alias, and page-content ranking for one friendly-search plan.
use super::*;

#[derive(Debug)]
enum PageCandidate {
    File(usize),
    Referenced(PageEntry),
}

#[derive(Debug)]
struct ScoredPage {
    from_content: bool,
    score: i32,
    match_class: ObjectiveMatchClass,
    matched_text: String,
    matched_alias: Option<String>,
    tie_key: String,
    candidate: PageCandidate,
    content_text: Option<String>,
}

impl ScoredPage {
    fn is_better_than(&self, other: &Self) -> bool {
        (!self.from_content && other.from_content)
            || (self.from_content == other.from_content
                && (self.match_class.rank() > other.match_class.rank()
                    || (self.match_class == other.match_class
                        && (self.score > other.score
                            || (self.score == other.score && self.tie_key < other.tie_key)))))
    }
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
        // Max-heap root is the WORST retained candidate, ready for eviction.
        other
            .from_content
            .cmp(&self.from_content)
            .then_with(|| other.match_class.rank().cmp(&self.match_class.rank()))
            .then_with(|| other.score.cmp(&self.score))
            .then_with(|| self.tie_key.cmp(&other.tie_key))
    }
}

fn push_page(heap: &mut BinaryHeap<ScoredPage>, limit: usize, candidate: ScoredPage) {
    if heap.len() < limit {
        heap.push(candidate);
    } else if heap
        .peek()
        .is_some_and(|worst| candidate.is_better_than(worst))
    {
        *heap.peek_mut().unwrap() = candidate;
    }
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
            TextMatchMode::Fuzzy => fuzzy_name_score(lower, &pred.value).map(|(score, class)| {
                if class == ObjectiveMatchClass::Exact
                    && plan
                        .page_exact
                        .as_ref()
                        .is_some_and(|exact| identity_fold(original) != *exact)
                {
                    (1000, ObjectiveMatchClass::Prefix)
                } else {
                    (score, class)
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
                        .is_some_and(|exact| identity_fold(original) != *exact)
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
                    None
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
        QueryExpr::Or(children) => children
            .iter()
            .find_map(|child| page_base_score(plan, child, original, lower)),
        QueryExpr::Not(child) => {
            (!eval_expr_fast(plan, child, TextField::PageName, original, lower))
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
        if page_match.is_some() && identity_fold(page_name) == exact {
            return Some((
                1500,
                ObjectiveMatchClass::Exact,
                page_name.to_string(),
                None,
            ));
        }
        if let Some(alias) = aliases.iter().find(|alias| {
            identity_fold(alias) == exact
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
    graph: &impl GraphRead,
    branch: &QueryBranch,
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<QueryHit>, bool)> {
    if branch.limit == 0 {
        return Some((Vec::new(), false));
    }
    let file_pages = graph.page_list_arc();
    let include_names = plan.page_match_scope != FriendlyPageMatchScope::Content;
    let include_content = plan.page_match_scope != FriendlyPageMatchScope::Names;
    let mut content_by_path: HashMap<String, (BlockRelevance, String)> = HashMap::new();
    if include_content {
        let block_expr = plan
            .branches
            .iter()
            .find(|branch| branch.target == QueryTarget::Blocks)
            .map(|branch| &branch.predicate)?;
        graph.with_pages(|pages| {
            for (entry, doc) in pages {
                if cancelled() {
                    break;
                }
                let mut ancestors = Vec::new();
                walk_blocks(&doc.roots, &mut ancestors, &mut |block, _| {
                    if cancelled() {
                        return false;
                    }
                    let projection = block.projection();
                    let lower = projection.visible_folded(plan.remove_accents);
                    if let Some(relevance) =
                        block_relevance(plan, block_expr, &projection.visible, lower)
                    {
                        let key = entry.rel_path_str().to_owned();
                        let replace = content_by_path.get(&key).is_none_or(|(best, _)| {
                            relevance.cmp_quality(best) == Ordering::Greater
                        });
                        if replace {
                            content_by_path.insert(key, (relevance, projection.visible.clone()));
                        }
                    }
                    true
                });
            }
        });
        if cancelled() {
            return None;
        }
    }
    let mut aliases_by_owner: HashMap<String, Vec<String>> = HashMap::new();
    for (alias, _, owner_rel_path) in graph.page_aliases_with_owners() {
        aliases_by_owner
            .entry(owner_rel_path)
            .or_default()
            .push(alias);
    }
    let mut heap = BinaryHeap::new();
    let mut has_more = false;
    let sort = friendly_sort_fields(plan.page_view.as_ref());
    let selection_limit = if sort.is_empty() {
        branch.limit
    } else {
        usize::MAX
    };
    for (index, page) in file_pages.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let aliases = aliases_by_owner
            .get(page.rel_path_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let name_match = include_names
            .then(|| best_page_match(plan, &branch.predicate, &page.name, aliases))
            .flatten();
        let content_match = content_by_path.get(page.rel_path_str());
        if let Some((base_score, match_class, matched_text, matched_alias, content_text)) =
            name_match
                .map(|(score, class, text, alias)| (score, class, text, alias, None))
                .or_else(|| {
                    content_match.map(|(relevance, text)| {
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
            has_more |= heap.len() >= selection_limit;
            push_page(
                &mut heap,
                selection_limit,
                ScoredPage {
                    from_content: content_text.is_some(),
                    score: base_score - page.name.len() as i32,
                    match_class,
                    matched_text,
                    matched_alias,
                    tie_key: page.rel_path_str().to_owned(),
                    candidate: PageCandidate::File(index),
                    content_text,
                },
            );
        }
    }
    let have: HashSet<String> = file_pages
        .iter()
        .map(|page| identity_fold(&page.name))
        .collect();
    for name in if include_names {
        graph.referenced_page_names()
    } else {
        Vec::new()
    } {
        if cancelled() {
            return None;
        }
        let key = identity_fold(&name);
        if have.contains(&key) {
            continue;
        }
        if let Some((base_score, match_class, matched_text, matched_alias)) =
            best_page_match(plan, &branch.predicate, &name, &[])
        {
            has_more |= heap.len() >= selection_limit;
            let score = base_score - name.len() as i32;
            push_page(
                &mut heap,
                selection_limit,
                ScoredPage {
                    from_content: false,
                    score,
                    match_class,
                    matched_text,
                    matched_alias,
                    tie_key: tine_core::refs::page_key(&name),
                    candidate: PageCandidate::Referenced(PageEntry {
                        name,
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: None,
                        path: std::path::PathBuf::new(),
                    }),
                    content_text: None,
                },
            );
        }
    }
    let mut winners = heap.into_vec();
    if sort.is_empty() {
        winners.sort_by(|a, b| {
            a.from_content
                .cmp(&b.from_content)
                .then_with(|| b.match_class.rank().cmp(&a.match_class.rank()))
                .then_with(|| b.score.cmp(&a.score))
                .then_with(|| a.tie_key.cmp(&b.tie_key))
        });
    } else {
        let wanted: HashSet<&str> = winners
            .iter()
            .filter_map(|winner| match winner.candidate {
                PageCandidate::File(index) => Some(file_pages[index].rel_path_str()),
                PageCandidate::Referenced(_) => None,
            })
            .collect();
        let keys = graph.with_pages(|pages| {
            let index = graph.query_index();
            pages
                .iter()
                .filter(|(entry, _)| wanted.contains(entry.rel_path_str()))
                .map(|(entry, doc)| {
                    let facts = index.facts(entry, doc);
                    (
                        entry.rel_path_str().to_owned(),
                        sort.iter()
                            .map(|(field, _)| {
                                crate::query::exec::page_sort_decor(
                                    field,
                                    entry,
                                    facts.properties(),
                                    0,
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<HashMap<_, _>>()
        });
        let ascending: Vec<bool> = sort.iter().map(|(_, asc)| *asc).collect();
        let mut decorated: Vec<_> = winners
            .into_iter()
            .map(|winner| {
                let key = match &winner.candidate {
                    PageCandidate::File(index) => keys
                        .get(file_pages[*index].rel_path_str())
                        .cloned()
                        .unwrap_or_else(|| {
                            sort.iter()
                                .map(|(field, _)| {
                                    crate::query::exec::page_sort_decor(
                                        field,
                                        &file_pages[*index],
                                        &[],
                                        0,
                                    )
                                })
                                .collect()
                        }),
                    PageCandidate::Referenced(page) => sort
                        .iter()
                        .map(|(field, _)| crate::query::exec::page_sort_decor(field, page, &[], 0))
                        .collect(),
                };
                (key, winner)
            })
            .collect();
        decorated.sort_by(|a, b| {
            tine_core::query::sort::compare_sort_decorations(&a.0, &b.0, &ascending)
                .then_with(|| a.1.tie_key.cmp(&b.1.tie_key))
        });
        winners = decorated.into_iter().map(|(_, winner)| winner).collect();
        has_more = winners.len() > branch.limit;
        winners.truncate(branch.limit);
    }
    // Hydrate only admitted physical pages. The query index owns the authored
    // property projection; matching and row construction share one graph snapshot.
    let wanted: HashSet<&str> = winners
        .iter()
        .filter_map(|winner| match winner.candidate {
            PageCandidate::File(index) => Some(file_pages[index].rel_path_str()),
            PageCandidate::Referenced(_) => None,
        })
        .collect();
    let rows = graph.with_pages(|pages| {
        let index = graph.query_index();
        pages
            .iter()
            .filter(|(entry, _)| wanted.contains(entry.rel_path_str()))
            .map(|(entry, doc)| {
                let facts = index.facts(entry, doc);
                (
                    entry.rel_path_str().to_owned(),
                    tine_core::query::ir::PageRow {
                        path: entry.rel_path_str().to_owned(),
                        name: entry.name.clone(),
                        kind: entry.kind,
                        journal_day: entry.date_key,
                        properties: facts.properties().to_vec(),
                    },
                )
            })
            .collect::<HashMap<_, _>>()
    });
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
