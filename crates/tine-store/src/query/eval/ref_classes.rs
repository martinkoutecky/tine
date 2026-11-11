//! What a reference leaf's operand names (D-20261019-Q8, conformance S10 and
//! S12): `[[A]]`, `refs`, `#A`, TQL `ref("A")` and `direct_ref("A")` match a
//! reference to ANY name of A's alias group, and a reference that spells the
//! date of a journal in that group. The names come from the store's one
//! answer ([`crate::query::reference_class`], the Linked References panel's
//! names); nothing here decides which names belong together.
//!
//! Two builds, one answer:
//! - per execution (`EvalCache::spell_ref_classes`, `spelled`): over the
//!   snapshot being executed, with a journal day's referenced spellings as
//!   keys, so evaluation and the page narrowing compare keys only;
//! - per plan (`Plan::ref_classes`, read by the memo carry and the
//!   mounted-query touch test): days stay days, tested per reference with
//!   the one date answer behind the `JournalFormat::day_spelling_prefilter`
//!   digit check, so the test reads the page alone. A plan is carried to a
//!   later generation only when no alias and no page name moved
//!   (`ReadSnapshot::carry_from`, `change_summary`'s full refresh). Not
//!   persisted: a plan restored from the launch checkpoint rebuilds them on
//!   first use.
//!
//! Each is O(operands * component); the spelled build adds
//! `referenced_day_keys`' O(R) digit checks per journal day reached.
//!
//! Declared different questions (I-12): block `tag("x")` (TQL, `Rel::Tags`)
//! compares the literal tag like `(page-tags x)` (KI2: tags stay literal), and
//! `name` / `page` leaves never expand an alias (Q8); they fold a date
//! spelling in `eval_page_name`.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tine_core::date::{JournalDate, JournalFormat};
use tine_core::query::ir::{Attr, CmpOp, Filter, Leaf, Rel, Value};
use tine_core::refs;

use crate::model::ReadSnapshot;

/// The names one operand denotes: page keys, and journal days whose every
/// spelling counts (`yyyymmdd` ordinals).
pub(crate) struct RefClass {
    keys: HashSet<String>,
    /// The same keys as a list, for the per-page narrowing probe
    /// (`PageFacts::may_reference`), built once rather than per page.
    list: Vec<String>,
    days: Vec<i64>,
    real: crate::query::RealPageOwners,
    /// Per day, the day of month and the year mod 100 in decimal: the digit
    /// runs every accepted spelling of that day contains
    /// (`JournalFormat::may_spell_day`), so most names skip the date parse.
    needles: Vec<Box<dyn Fn(&str) -> bool + Send + Sync>>,
}

impl RefClass {
    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> {
        self.list.iter()
    }

    pub(crate) fn key_list(&self) -> &[String] {
        &self.list
    }

    pub(crate) fn has_days(&self) -> bool {
        !self.days.is_empty()
    }

    /// Whether the reference key `name` (a page key) is a name of this class.
    pub(crate) fn holds(&self, name: &str, day: impl FnOnce(&str) -> Option<i64>) -> bool {
        self.keys.contains(name)
            || (self.may_spell_a_day(name)
                && !self
                    .real
                    .opening_entry(name)
                    .is_some_and(|entry| entry.kind == tine_core::model::PageKind::Page)
                // Days are used only by conservative plan touch/memo probes.
                // A normalized key can lose a configured literal's case;
                // execution uses exact authored spelling keys with no days.
                && day(name).is_none_or(|day| self.days.contains(&day)))
    }

    fn may_spell_a_day(&self, name: &str) -> bool {
        self.needles.iter().any(|may_spell| may_spell(name))
    }
}

/// Every reference operand of one plan, by its page key, and by the operand
/// as written so the per-block lookup needs no key fold.
#[derive(Default)]
pub(crate) struct RefClasses {
    by_key: HashMap<String, Arc<RefClass>>,
    by_text: HashMap<String, Arc<RefClass>>,
    real: Option<crate::query::RealPageOwners>,
}

impl RefClasses {
    pub(crate) fn ordinary_claim(&self, key: &str) -> bool {
        self.real
            .as_ref()
            .and_then(|real| real.opening_entry(key))
            .is_some_and(|entry| entry.kind == tine_core::model::PageKind::Page)
    }
    /// The empty table: every operand compares literally. Only plans with no
    /// reference leaf, or test plans built without a graph, read it.
    pub(crate) fn none() -> &'static RefClasses {
        static NONE: std::sync::OnceLock<RefClasses> = std::sync::OnceLock::new();
        NONE.get_or_init(RefClasses::default)
    }

    /// `spelled`: for one execution over `graph` (see
    /// [`crate::query::reference_class`]): a journal day's referenced
    /// spellings join the keys and no day test remains. Otherwise (a plan's
    /// own classes, read by the memo carry and the mounted-query touch test)
    /// the days stay days, so the test is a function of the page alone.
    pub(crate) fn build(graph: &ReadSnapshot, filter: &Filter, spelled: bool) -> RefClasses {
        let mut operands = Vec::new();
        collect(filter, false, &mut operands);
        let mut by_key: HashMap<String, Arc<RefClass>> = HashMap::new();
        let mut by_text = HashMap::new();
        for operand in operands {
            let key = refs::page_key(&operand);
            if let Some(class) = by_key.get(&key) {
                by_text.insert(operand, Arc::clone(class));
                continue;
            }
            let (keys, days) = crate::query::reference_class(graph, &operand, spelled);
            let mut keys: HashSet<String> = keys.into_iter().collect();
            keys.insert(key.clone());
            let mut list: Vec<String> = keys.iter().cloned().collect();
            list.sort();
            let needles = days
                .iter()
                .map(|&day| {
                    let day = JournalDate {
                        year: (day / 10000) as i32,
                        month: (day / 100 % 100) as u32,
                        day: (day % 100) as u32,
                    };
                    Box::new(JournalFormat::day_spelling_prefilter(day))
                        as Box<dyn Fn(&str) -> bool + Send + Sync>
                })
                .collect();
            let class = Arc::new(RefClass {
                keys,
                list,
                days,
                real: graph.real_page_owners(),
                needles,
            });
            by_text.insert(operand, Arc::clone(&class));
            by_key.insert(key, class);
        }
        RefClasses {
            by_key,
            by_text,
            real: Some(graph.real_page_owners()),
        }
    }

    /// Whether a page named by one of `keys` (page keys of names that spell a
    /// journal day) appearing or leaving can change a class: one holds a day,
    /// or names one of them (an ordinary page named like a date keeps its own
    /// name only, `holds`). O(operands * keys).
    pub(crate) fn reshaped_by(&self, keys: &[String]) -> bool {
        self.by_key
            .values()
            .any(|class| class.has_days() || keys.iter().any(|key| class.keys.contains(key)))
    }

    /// The class of operand `name`; `None` when it was not collected.
    pub(crate) fn class(&self, name: &str) -> Option<&RefClass> {
        if self.by_key.is_empty() {
            return None;
        }
        self.by_text
            .get(name)
            .or_else(|| self.by_key.get(&refs::page_key(name)))
            .map(|class| &**class)
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        let texts = self
            .by_text
            .keys()
            .fold(0usize, |sum, text| sum.saturating_add(text.capacity() + 48));
        self.by_key.iter().fold(texts, |sum, (key, class)| {
            class.keys.iter().fold(
                sum.saturating_add(key.capacity() + 64 + class.days.capacity() * 8),
                |sum, name| sum.saturating_add(name.capacity() * 2 + 56),
            )
        })
    }
}

/// Whether `filter` has a reference operand, whose class (the alias group and
/// journal days, see [`RefClasses::build`]) is read from page names.
pub(crate) fn has_reference_operands(filter: &Filter) -> bool {
    let mut operands = Vec::new();
    collect(filter, false, &mut operands);
    !operands.is_empty()
}

/// The text operands of `name = …` element tests under a reference relation.
fn collect(filter: &Filter, in_ref: bool, out: &mut Vec<String>) {
    match filter {
        Filter::And { items } | Filter::Or { items } => {
            for item in items {
                collect(item, in_ref, out);
            }
        }
        Filter::Not { inner } => collect(inner, in_ref, out),
        Filter::Leaf { leaf } => match leaf {
            Leaf::Rel { rel, pred, .. } => {
                collect(pred, matches!(rel, Rel::Refs | Rel::DirectRefs), out)
            }
            Leaf::Near { seeds, .. } => collect(seeds, false, out),
            Leaf::Attr {
                attr: Attr::Name,
                op: CmpOp::Eq,
                value: Value::Text { text },
            } if in_ref => out.push(text.clone()),
            _ => {}
        },
        _ => {}
    }
}
