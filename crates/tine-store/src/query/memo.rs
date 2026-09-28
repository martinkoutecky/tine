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
//! entries / 64 MiB, and an answer over 16 MiB is returned but not retained.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use tine_core::date::JournalDate;
use tine_core::doc::Document;
use tine_core::model::{ref_groups_estimated_bytes, BoundedRefGroups, PageEntry, RefGroup};
use tine_core::query::atom::ParseConfig;
use tine_core::query::ir::{QueryResult, QueryRows};
use tine_core::query::statistics::StatisticsResourceLimit;
use tine_core::query::AdvancedResult;

use super::exec::Plan;
use super::index::PageFacts;

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

    fn contains(&self, entry: &PageEntry) -> bool {
        if let Answer::Result(Ok(result)) = self {
            if let QueryRows::Page { pages } = &result.rows {
                return pages.iter().any(|row| row.path == entry.rel_path_str());
            }
        }
        self.groups()
            .iter()
            .any(|group| tine_core::refs::same_page(&group.page, &entry.name))
    }

    fn estimated_bytes(&self) -> usize {
        let rows = match self {
            Answer::Result(Err(_)) => 0,
            Answer::Result(Ok(result)) => match &result.rows {
                QueryRows::Page { pages } => pages
                    .iter()
                    .map(|row| {
                        row.properties
                            .iter()
                            .fold(96 + row.name.len() + row.path.len(), |bytes, (k, v)| {
                                bytes + k.len() + v.len()
                            })
                    })
                    .sum(),
                QueryRows::Block { groups } => ref_groups_estimated_bytes(groups),
            },
            Answer::Groups(groups) => ref_groups_estimated_bytes(&groups.groups),
            Answer::Advanced { result, .. } => ref_groups_estimated_bytes(&result.groups)
                .saturating_add(
                    result
                        .ran
                        .iter()
                        .chain(&result.ignored)
                        .map(String::len)
                        .sum(),
                ),
        };
        rows.saturating_add(4096)
    }
}

#[derive(Clone)]
struct Entry {
    /// `None` when the answer does not depend on the graph (a refused or
    /// unsupported query).
    plan: Option<Arc<Plan>>,
    answer: Answer,
    bytes: usize,
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
    /// `compute` runs with no lock held.
    pub(crate) fn answer(
        &self,
        key: String,
        parse_config: &ParseConfig,
        compute: impl FnOnce() -> (Answer, Option<Arc<Plan>>),
    ) -> Answer {
        let today = JournalDate::today().ordinal_key();
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
        let bytes = answer
            .estimated_bytes()
            .saturating_add(key.len().saturating_mul(2));
        if bytes > MAX_ENTRY_BYTES {
            return answer;
        }
        let mut guard = self.inner.write().unwrap();
        let memo = match guard.as_mut() {
            Some(memo) if memo.today == today && memo.parse_config == *parse_config => memo,
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
            let old_facts = PageFacts::of(entry, before);
            let new_facts = PageFacts::of(entry, after);
            let rows_moved = old_facts.rows_digest() != new_facts.rows_digest();
            memo.entries.retain(|_, cached| {
                let Some(plan) = &cached.plan else {
                    return true;
                };
                !(cached.answer.contains(entry)
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
