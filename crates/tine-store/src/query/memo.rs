//! The per-snapshot query answer memo, carried to the next snapshot with
//! scoped invalidation (og's shape, one memo for every query entry point).
//!
//! An answer stays valid across an edit of page P unless
//! - P already contributes to it (a returned group or page row is P), or
//! - the plan selects P in its old or new document (evaluated with the
//!   answer's own registry snapshot), or
//! - the plan read the property registry and P's registry rows moved, or
//! - the day rolled over, or the query configuration changed.
//!
//! Per-page evaluation is page-local (refs, tags, properties, task attributes,
//! the page's own name and preamble), so these cover every input. Page
//! additions/removals and alias changes never reach here: the snapshot drops
//! all memos for them (`ReadSnapshot::carry_memos_from`).
//!
//! Nothing is persisted (Unit cost: none on disk); memory is bounded at 64
//! entries / 64 MiB, and an entry over 16 MiB is returned but not retained.
//! Charging covers the complete answer (including statistics/diagnostics/report),
//! plan/filter/registry, compiled-program reservations, contributor sets and keys.
//! Reservations are conservative; exceeding one skips caching, never execution.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, RwLock};

use tine_core::date::JournalDate;
use tine_core::doc::Document;
use tine_core::model::{BoundedRefGroups, PageEntry, RefGroup};
use tine_core::query::atom::ParseConfig;
use tine_core::query::ir::{QueryResult, QueryRows};
use tine_core::query::statistics::StatisticsResourceLimit;
use tine_core::query::AdvancedResult;

use super::exec::Plan;
use super::index::PageFacts;

#[path = "retained.rs"]
pub(super) mod retained;

const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

/// One memoized answer.
#[derive(Clone)]
pub(crate) enum Answer {
    /// `query_run`'s result (a statistics refusal is as deterministic as rows).
    Result(Result<Arc<QueryResult>, StatisticsResourceLimit>),
    /// The legacy block-group bridge (`run_query`, Copy/Export).
    Groups(BoundedRefGroups),
    /// The advanced bridge: its groups plus the support report.
    Advanced {
        result: Arc<AdvancedResult>,
        total: usize,
        exceeded: bool,
    },
}

impl Answer {
    fn groups(&self) -> &[RefGroup] {
        match self {
            Answer::Result(Ok(result)) => match &result.rows {
                QueryRows::Block { groups } => groups,
                QueryRows::Page { .. } => &[],
            },
            Answer::Result(Err(_)) => &[],
            Answer::Groups(groups) => &groups.groups,
            Answer::Advanced { result, .. } => &result.groups,
        }
    }

    /// The pages this answer returns rows from, as the keys `contains`
    /// probes: page-row paths, and `page_key` of every block group's page.
    /// Built once when the answer is retained, so an edit's carry asks one
    /// hash lookup per answer instead of re-folding every group (I-25).
    fn pages(&self) -> HashSet<String> {
        if let Answer::Result(Ok(result)) = self {
            if let QueryRows::Page { pages } = &result.rows {
                return pages.iter().map(|row| row.path.clone()).collect();
            }
        }
        self.groups()
            .iter()
            .map(|group| tine_core::refs::page_key(&group.page))
            .collect()
    }

    fn estimated_bytes(&self) -> usize {
        let payload = match self {
            Answer::Result(Err(_)) => 0,
            Answer::Result(Ok(result)) => retained::serialized_bytes(result.as_ref()),
            Answer::Groups(groups) => retained::serialized_bytes(groups.groups.as_ref()),
            Answer::Advanced { result, .. } => retained::serialized_bytes(result.as_ref()),
        };
        payload.saturating_add(4096)
    }
}

#[derive(Clone)]
struct Entry {
    /// `None` when the answer does not depend on the graph (a refused or
    /// unsupported query).
    plan: Option<Arc<Plan>>,
    answer: Answer,
    /// [`Answer::pages`], shared across carried generations.
    pages: Arc<HashSet<String>>,
    bytes: usize,
}

impl Entry {
    fn contains(&self, entry: &PageEntry) -> bool {
        self.pages.contains(entry.rel_path_str())
            || self.pages.contains(&tine_core::refs::page_key(&entry.name))
    }
}

#[derive(Clone)]
struct Memo {
    today: i64,
    parse_config: ParseConfig,
    entries: HashMap<String, Entry>,
    lru: VecDeque<String>,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct QueryMemo {
    inner: RwLock<Option<Memo>>,
}

impl QueryMemo {
    /// The memoized answer for `key`, or `compute`'s (retained when it fits).
    /// `today` is the day the caller evaluates relative dates against (read
    /// once per query): the answer is filed under that day, so an evaluation
    /// that straddles midnight never lands in the next day's memo. `compute`
    /// runs with no lock held.
    pub(crate) fn answer(
        &self,
        key: String,
        today: JournalDate,
        parse_config: &ParseConfig,
        compute: impl FnOnce() -> (Answer, Option<Arc<Plan>>),
    ) -> Answer {
        let today = today.ordinal_key();
        {
            let mut memo = self.inner.write().unwrap();
            if let Some(memo) = memo.as_mut() {
                if memo.today == today && memo.parse_config == *parse_config {
                    if let Some(entry) = memo.entries.get(&key) {
                        let answer = entry.answer.clone();
                        touch(&mut memo.lru, &key);
                        return answer;
                    }
                }
            }
        }
        let (answer, plan) = compute();
        let pages = Arc::new(answer.pages());
        let bytes = answer
            .estimated_bytes()
            .saturating_add(plan.as_ref().map_or(0, |plan| plan.estimated_bytes()))
            .saturating_add(pages.iter().fold(pages.capacity() * 64, |bytes, page| {
                bytes.saturating_add(page.capacity())
            }))
            .saturating_add(retained::parse_config_bytes(parse_config))
            .saturating_add(key.len().saturating_mul(2));
        if bytes > MAX_ENTRY_BYTES {
            return answer;
        }
        let mut guard = self.inner.write().unwrap();
        let memo = match guard.as_mut() {
            Some(memo) if memo.today == today && memo.parse_config == *parse_config => memo,
            // A different day's memo is not replaced by an answer computed for
            // an older day than it (a query that straddled midnight).
            Some(memo) if memo.today > today => return answer,
            _ => guard.insert(Memo {
                today,
                parse_config: parse_config.clone(),
                entries: HashMap::new(),
                lru: VecDeque::new(),
                bytes: 0,
            }),
        };
        let entry = Entry {
            plan,
            pages,
            answer: answer.clone(),
            bytes,
        };
        if let Some(previous) = memo.entries.insert(key.clone(), entry) {
            memo.bytes = memo.bytes.saturating_sub(previous.bytes);
        }
        memo.bytes = memo.bytes.saturating_add(bytes);
        touch(&mut memo.lru, &key);
        while memo.entries.len() > MAX_ENTRIES || memo.bytes > MAX_BYTES {
            let Some(oldest) = memo.lru.pop_front() else {
                break;
            };
            if let Some(removed) = memo.entries.remove(&oldest) {
                memo.bytes = memo.bytes.saturating_sub(removed.bytes);
            }
        }
        answer
    }

    /// Carry `previous`'s answers into this (new) snapshot's memo, dropping
    /// every answer one of `edits` (entry, old document, new document) can
    /// change. `parse_config` is the new snapshot's.
    pub(crate) fn carry_from(
        &self,
        previous: &QueryMemo,
        parse_config: &ParseConfig,
        edits: &[(PageEntry, Arc<Document>, Arc<Document>)],
    ) {
        let Some(mut memo) = previous.inner.read().unwrap().clone() else {
            return;
        };
        if memo.today != JournalDate::today().ordinal_key() || memo.parse_config != *parse_config {
            return;
        }
        for (entry, before, after) in edits {
            let old_facts = PageFacts::of(entry, before, parse_config);
            let new_facts = PageFacts::of(entry, after, parse_config);
            let rows_moved = old_facts.registry_rows_differ(&new_facts);
            memo.entries.retain(|_, cached| {
                let Some(plan) = &cached.plan else {
                    return true;
                };
                !(cached.contains(entry)
                    || rows_moved && plan.registry().is_some()
                    || plan.touches(entry, before, &old_facts, parse_config)
                    || plan.touches(entry, after, &new_facts, parse_config))
            });
        }
        memo.lru.retain(|key| memo.entries.contains_key(key));
        memo.bytes = memo.entries.values().map(|entry| entry.bytes).sum();
        *self.inner.write().unwrap() = Some(memo);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .is_none_or(|memo| memo.entries.is_empty())
    }

    #[cfg(test)]
    pub(crate) fn cached(&self, key: &str) -> Option<Answer> {
        let memo = self.inner.read().unwrap();
        memo.as_ref()?
            .entries
            .get(key)
            .map(|entry| entry.answer.clone())
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map_or(0, |memo| memo.entries.len())
    }
}

fn touch(lru: &mut VecDeque<String>, key: &str) {
    if let Some(at) = lru.iter().position(|candidate| candidate == key) {
        lru.remove(at);
    }
    lru.push_back(key.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(total: usize) -> Answer {
        Answer::Groups(BoundedRefGroups {
            groups: Arc::new(Vec::new()),
            total,
            exceeded: false,
        })
    }

    fn total(answer: &Answer) -> usize {
        match answer {
            Answer::Groups(groups) => groups.total,
            _ => unreachable!(),
        }
    }

    #[test]
    fn b_query_memo_charges_statistics_and_compiled_plans_without_refusing_answers() {
        use tine_core::query::ir::*;
        let memo = QueryMemo::default();
        let config = ParseConfig::default();
        let query = Query {
            anchor: Anchor::Block,
            filter: Filter::and(
                (0..9)
                    .map(|i| {
                        Filter::attr(
                            Attr::Content,
                            CmpOp::Regex,
                            Value::text(format!("pattern{i}")),
                        )
                    })
                    .collect(),
            ),
            diagnostics: Vec::new(),
            source: Source::Tql {
                original: String::new(),
                og_options: String::new(),
            },
        };
        let plan = Arc::new(Plan::new(
            &query,
            JournalDate::today(),
            false,
            false,
            || Arc::new(tine_core::query::registry::Registry::empty(&config)),
        ));
        let answer = memo.answer("regex".into(), JournalDate::today(), &config, || {
            (groups(9), Some(plan))
        });
        assert_eq!(
            total(&answer),
            9,
            "a cache budget may skip retention, never refuse the answer"
        );
        assert_eq!(
            memo.len(),
            0,
            "I-22: all compiled regex reservations must count toward the memo entry ceiling"
        );
    }

    #[test]
    fn b_query_memo_charges_statistics_even_when_rows_are_omitted() {
        use tine_core::query::ir::*;
        let memo = QueryMemo::default();
        let config = ParseConfig::default();
        let result = QueryResult {
            rows: QueryRows::Page { pages: Vec::new() },
            diagnostics: Vec::new(),
            report: QueryReport::default(),
            total: 0,
            matched_total: None,
            exceeded: true,
            statistics: Some(QueryStatistics {
                count: 10_000,
                aggregates: Vec::new(),
                group_by: Some(Field("owner".into())),
                overall: Vec::new(),
                groups: Some(
                    (0..10_000)
                        .map(|i| QueryStatisticsGroup {
                            key: Some(format!("{i}{}", "x".repeat(500))),
                            count: 1,
                            cells: Vec::new(),
                        })
                        .collect(),
                ),
                grouping_status: QueryStatisticsGroupingStatus::Exact,
            }),
        };
        let answer = memo.answer("stats".into(), JournalDate::today(), &config, || {
            (Answer::Result(Ok(Arc::new(result))), None)
        });
        assert!(
            matches!(answer, Answer::Result(Ok(ref result)) if result.statistics.as_ref().unwrap().groups.as_ref().unwrap().len() == 10_000)
        );
        assert_eq!(
            memo.len(),
            0,
            "I-22: omitted rows can still retain statistics groups; they must be charged"
        );
        memo.answer("benign".into(), JournalDate::today(), &config, || {
            (groups(20_000), None)
        });
        assert_eq!(
            memo.len(),
            1,
            "a large logical count with no retained payload must still be memoized"
        );
    }

    #[test]
    fn b_query_memo_total_budget_evicts_program_reservations_and_reuses_small_plans() {
        use tine_core::query::ir::*;
        let memo = QueryMemo::default();
        let config = ParseConfig::default();
        let query = Query::new(
            Anchor::Block,
            Filter::and(
                (0..6)
                    .map(|i| {
                        Filter::attr(Attr::Content, CmpOp::Regex, Value::text(format!("p{i}")))
                    })
                    .collect(),
            ),
            Source::Builder,
        );
        let plan = Arc::new(Plan::new(
            &query,
            JournalDate::today(),
            false,
            false,
            || unreachable!(),
        ));
        for i in 0..8 {
            memo.answer(i.to_string(), JournalDate::today(), &config, || {
                (groups(i), Some(plan.clone()))
            });
        }
        assert_eq!(
            memo.len(),
            5,
            "I-22: independently charged entries must fit the total 64 MiB memo ceiling"
        );
        assert!(memo.cached("0").is_none());
        assert!(memo.cached("7").is_some());
        let served = memo.answer("7".into(), JournalDate::today(), &config, || {
            panic!("a fitting answer should remain memoized")
        });
        assert_eq!(total(&served), 7);
    }

    /// Reader B (og 14 Q2): an answer evaluated against yesterday — a query
    /// that started before midnight — must not be served as today's.
    #[test]
    fn an_answer_is_filed_under_the_day_it_was_evaluated_for() {
        let memo = QueryMemo::default();
        let config = ParseConfig::default();
        let today = JournalDate::today();
        let yesterday = today.add_days(-1);
        memo.answer("k".into(), yesterday, &config, || (groups(1), None));
        let served = memo.answer("k".into(), today, &config, || (groups(2), None));
        assert_eq!(total(&served), 2, "yesterday's answer was served for today");
        // …and a late straggler for yesterday does not evict today's memo.
        memo.answer("y".into(), yesterday, &config, || (groups(3), None));
        let again = memo.answer("k".into(), today, &config, || (groups(4), None));
        assert_eq!(
            total(&again),
            2,
            "a straggler for yesterday dropped today's memo"
        );
    }
}
