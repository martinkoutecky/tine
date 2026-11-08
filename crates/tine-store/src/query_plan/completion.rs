//! Incremental literal page completion. Retain ALL matches, never just the
//! displayed top-k: an extension can promote a previously hidden candidate.
//! The ordinary page evaluator remains the differential oracle.
use super::*;
use pages::{best_page_match, push_page, PageCandidate, ScoredPage};
use std::sync::{
    atomic::{AtomicBool, Ordering as AtomicOrdering},
    Arc, Mutex, Weak,
};

const MAX_SESSIONS: usize = 8;
type Baseline = Option<(String, String, String)>;

#[derive(Default)]
pub(crate) struct Registry(Mutex<Vec<Weak<Session>>>);
pub(crate) struct CompletionCache {
    sessions: Mutex<Vec<Arc<Session>>>,
    registry: Arc<Registry>,
}

struct Session {
    baseline: Baseline,
    session: Option<String>,
    transliteration: bool,
    remove_accents: bool,
    state: Mutex<State>,
    closed: AtomicBool,
}

#[derive(Default)]
struct State {
    query: Option<String>,
    expansion: Option<String>,
    files: Arc<Vec<PageEntry>>,
    pool: Vec<Candidate>,
}

struct Candidate {
    /// Physical entries borrow the snapshot's existing materialized list.
    page: Option<usize>,
    /// Alias spelling, virtual name, or None for the physical page's name.
    name: Option<String>,
    tie: String,
}

impl CompletionCache {
    #[cfg(test)]
    pub(crate) fn retained_candidates(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.state.lock().unwrap().pool.len())
            .sum()
    }
    pub(crate) fn new(owner: &crate::model::CompletionSessions) -> Self {
        Self {
            sessions: Mutex::new(Vec::new()),
            registry: Arc::clone(&owner.ranks),
        }
    }
    /// Share bounded session state only when the changed pages leave its
    /// baseline-adjusted name universe and PageEntry metadata unchanged.
    pub(crate) fn carry(&self, unchanged: impl Fn(&Baseline) -> bool) -> Self {
        Self {
            sessions: Mutex::new(
                self.sessions
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|s| !s.closed.load(AtomicOrdering::Relaxed) && unchanged(&s.baseline))
                    .cloned()
                    .collect(),
            ),
            registry: Arc::clone(&self.registry),
        }
    }

    pub(crate) fn release(&self, baseline: &Baseline, session: Option<&str>) {
        let owners: Vec<_> = {
            let mut registry = self.registry.0.lock().unwrap();
            let mut owners = Vec::new();
            registry.retain(|weak| {
                let Some(owner) = weak.upgrade() else {
                    return false;
                };
                if owner.baseline == *baseline && owner.session.as_deref() == session {
                    owner.closed.store(true, AtomicOrdering::Relaxed);
                    owners.push(owner);
                    false
                } else {
                    true
                }
            });
            owners
        };
        for owner in owners {
            *owner.state.lock().unwrap() = State::default();
        }
        self.sessions
            .lock()
            .unwrap()
            .retain(|s| !s.closed.load(AtomicOrdering::Relaxed));
    }

    pub(crate) fn execute(
        &self,
        plan: &QueryPlan,
        graph: &ReadSnapshot,
        session: Option<&str>,
    ) -> Vec<(PageEntry, ObjectiveMatchClass)> {
        let held = {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.retain(|s| !s.closed.load(AtomicOrdering::Relaxed));
            let at = sessions.iter().position(|s| {
                s.baseline == plan.editor_reference_baseline
                    && s.session.as_deref() == session
                    && s.transliteration == plan.transliteration
                    && s.remove_accents == plan.remove_accents
            });
            let held = at.map(|at| sessions.remove(at)).unwrap_or_else(|| {
                Arc::new(Session {
                    baseline: plan.editor_reference_baseline.clone(),
                    session: session.map(str::to_owned),
                    transliteration: plan.transliteration,
                    remove_accents: plan.remove_accents,
                    state: Mutex::new(State::default()),
                    closed: AtomicBool::new(false),
                })
            });
            if sessions.len() == MAX_SESSIONS {
                sessions.remove(0);
            }
            sessions.push(Arc::clone(&held));
            if at.is_none() {
                let mut registry = self.registry.0.lock().unwrap();
                registry.retain(|s| s.strong_count() > 0);
                registry.push(Arc::downgrade(&held));
            }
            held
        };
        let mut state = held.state.lock().unwrap();
        if held.closed.load(AtomicOrdering::Relaxed) {
            return Vec::new();
        }
        let branch = &plan.branches[0];
        let QueryExpr::Text(pred) = &branch.predicate else {
            unreachable!("literal completion")
        };
        let expansion = plan.transliteration_queries.get(&pred.clause_id).cloned();
        // Appending a combining mark can rewrite NFC, lowercase final sigma
        // can change, and accent policy can preserve marks. Check folded
        // prefixes, not raw input prefixes. Expansion has its own fold.
        let narrows = state.query.as_ref().is_some_and(|q| {
            pred.value.starts_with(q)
                && (q.is_empty()
                    || expansion.as_ref().is_none_or(|new| {
                        state
                            .expansion
                            .as_ref()
                            .is_some_and(|old| new.starts_with(old))
                    }))
        });
        if !narrows {
            (state.files, state.pool) = candidates(graph, plan);
        }
        let files = Arc::clone(&state.files);
        let mut heap = BinaryHeap::new();
        let mut index = 0;
        state.pool.retain(|candidate| {
            let name = candidate
                .name
                .as_deref()
                .unwrap_or_else(|| &files[candidate.page.unwrap()].name);
            let Some((base, class, matched_text, _)) =
                best_page_match(plan, &branch.predicate, name, &[])
            else {
                return false;
            };
            push_page(
                &mut heap,
                branch.limit,
                ScoredPage {
                    from_content: false,
                    score: base - name.len() as i32,
                    match_class: class,
                    matched_text,
                    matched_alias: None,
                    tie_key: candidate.tie.clone(),
                    candidate: PageCandidate::File(index),
                    content_text: None,
                },
            );
            index += 1;
            true
        });
        state.query = Some(pred.value.clone());
        state.expansion = expansion;
        let mut winners = heap.into_vec();
        winners.sort();
        winners
            .into_iter()
            .map(|winner| {
                let PageCandidate::File(index) = winner.candidate else {
                    unreachable!()
                };
                let candidate = &state.pool[index];
                let page = if let Some(index) = candidate.page {
                    let mut page = files[index].clone();
                    if let Some(name) = &candidate.name {
                        page.name.clone_from(name);
                    }
                    page
                } else {
                    PageEntry {
                        name: candidate.name.as_ref().unwrap().clone(),
                        kind: PageKind::Page,
                        date_key: None,
                        rel_path: None,
                        path: Default::default(),
                    }
                };
                (page, winner.match_class)
            })
            .collect()
    }
}

/// Same traversal and duplicate authority as execute_pages: physical names,
/// first physical owner of each alias, namespace parents, referenced names.
/// No evidence, page-property hydration or full-result sort for completion.
fn candidates(graph: &ReadSnapshot, plan: &QueryPlan) -> (Arc<Vec<PageEntry>>, Vec<Candidate>) {
    let files = graph.page_list_arc();
    let mut have: HashSet<String> = files.iter().map(|p| page_key(&p.name)).collect();
    let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
    for (alias, _, owner) in graph.page_alias_spellings_with_owners() {
        aliases.entry(owner).or_default().push(alias);
    }
    let mut pool = Vec::new();
    for (index, page) in files.iter().enumerate() {
        pool.push(Candidate {
            page: Some(index),
            name: None,
            tie: page.rel_path_str().to_owned(),
        });
        for alias in aliases.get(page.rel_path_str()).into_iter().flatten() {
            let key = page_key(alias);
            if have.insert(key.clone()) {
                pool.push(Candidate {
                    page: Some(index),
                    name: Some(alias.clone()),
                    tie: format!("{key}:{}", page.rel_path_str()),
                });
            }
        }
    }
    have.extend(aliases.values().flatten().map(|a| page_key(a)));
    let referenced = graph.referenced_names_for_completion(
        plan.editor_reference_baseline
            .as_ref()
            .map(|(page, block, original)| (page.as_str(), block.as_str(), original.as_str())),
    );
    let parents = graph.completion_namespace_parents(&files, &referenced);
    for name in parents
        .iter()
        .copied()
        .chain(referenced.iter().map(|n| n.as_ref()))
    {
        let key = page_key(name);
        if !have.contains(&key) {
            pool.push(Candidate {
                page: None,
                name: Some(name.to_owned()),
                tie: key,
            });
        }
    }
    (files, pool)
}
