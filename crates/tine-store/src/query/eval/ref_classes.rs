//! What a reference leaf's operand names (D-20261019-Q8, conformance S10 and
//! S12): `[[A]]`, `refs`, `#A`, TQL `ref("A")` and `direct_ref("A")` match a
//! reference to ANY name of A's alias group, and a reference that spells the
//! date of a journal in that group. The names come from the store's one
//! answer ([`crate::query::reference_class`], the Linked References panel's
//! names); nothing here decides which names belong together.
//!
//! Built once per plan from the generation the plan runs on: one entry per
//! distinct operand of a reference leaf, O(operands * component). A plan is
//! carried to a later generation only when no alias and no page name moved
//! (`ReadSnapshot::carry_from`, `change_summary`'s full refresh), so the
//! classes it holds stay that generation's. Not persisted: a plan restored
//! from the launch checkpoint rebuilds them on first use.
//!
//! Declared different questions (I-12): block `tag("x")` (TQL, `Rel::Tags`)
//! compares the literal tag like `(page-tags x)` (KI2: tags stay literal), and
//! `name` / `page` leaves never expand an alias (Q8); they fold a date
//! spelling in `eval_page_name`.
use std::collections::{HashMap, HashSet};

use tine_core::query::ir::{Attr, CmpOp, Filter, Leaf, Rel, Value};
use tine_core::refs;

use crate::model::ReadSnapshot;

/// The names one operand denotes: page keys, and journal days whose every
/// spelling counts (`yyyymmdd` ordinals).
#[derive(Default)]
pub(crate) struct RefClass {
    keys: HashSet<String>,
    days: Vec<i64>,
}

impl RefClass {
    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> {
        self.keys.iter()
    }

    pub(crate) fn has_days(&self) -> bool {
        !self.days.is_empty()
    }

    /// Whether the reference key `name` (a page key) is a name of this class.
    pub(crate) fn holds(&self, name: &str, day: impl FnOnce(&str) -> Option<i64>) -> bool {
        self.keys.contains(name)
            || (!self.days.is_empty() && day(name).is_some_and(|day| self.days.contains(&day)))
    }
}

/// Every reference operand of one plan, by its page key.
#[derive(Default)]
pub(crate) struct RefClasses {
    by_key: HashMap<String, RefClass>,
}

impl RefClasses {
    /// The empty table: every operand compares literally. Only plans with no
    /// reference leaf, or test plans built without a graph, read it.
    pub(crate) fn none() -> &'static RefClasses {
        static NONE: std::sync::OnceLock<RefClasses> = std::sync::OnceLock::new();
        NONE.get_or_init(RefClasses::default)
    }

    pub(crate) fn build(graph: &ReadSnapshot, filter: &Filter) -> RefClasses {
        let mut operands = Vec::new();
        collect(filter, false, &mut operands);
        let mut by_key = HashMap::new();
        for operand in operands {
            let key = refs::page_key(&operand);
            if by_key.contains_key(&key) {
                continue;
            }
            let (keys, days) = crate::query::reference_class(graph, &operand);
            let mut keys: HashSet<String> = keys.into_iter().collect();
            keys.insert(key.clone());
            by_key.insert(key, RefClass { keys, days });
        }
        RefClasses { by_key }
    }

    /// The class of operand `name`; `None` when it was not collected.
    pub(crate) fn class(&self, name: &str) -> Option<&RefClass> {
        if self.by_key.is_empty() {
            return None;
        }
        self.by_key.get(&refs::page_key(name))
    }

    pub(crate) fn estimated_bytes(&self) -> usize {
        self.by_key.iter().fold(0usize, |sum, (key, class)| {
            class.keys.iter().fold(
                sum.saturating_add(key.capacity() + 64 + class.days.capacity() * 8),
                |sum, name| sum.saturating_add(name.capacity() + 32),
            )
        })
    }
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
