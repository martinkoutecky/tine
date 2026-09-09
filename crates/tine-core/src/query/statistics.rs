//! Ordered, bounded folds over narrow values from the result statement.
//! This is the backend transcription of queryAggregate.ts's arithmetic; the
//! frontend owns formatting. Neither a group nor the overall fold retains rows.

use super::ir::{
    AggFn, QueryStatistics, QueryStatisticsCell as Cell, QueryStatisticsGroup,
    QueryStatisticsGroupingStatus as Status, QueryStatisticsMarker as Marker, ViewSettings,
};
use super::results::ResultReadError;
use std::collections::HashMap;

#[cfg(test)]
#[path = "statistics_tests.rs"]
mod tests;

/// One aggregate column's running total. It keeps three numbers and never a
/// row, which is the whole point: statistics describe the COMPLETE sample, and
/// the complete sample does not fit in memory for a large graph.
#[derive(Clone, Default)]
struct Accumulator {
    sum: f64,
    contributors: usize,
    skipped: usize,
}

impl Accumulator {
    fn add(&mut self, value: Option<&str>, op: AggFn) {
        // `count` counts ROWS, not values, so it never looks at one — including
        // `state=count`, where the named property is the column label rather
        // than a filter. That is what `queryAggregate.ts` did with `set.length`.
        if op == AggFn::Count {
            return;
        }
        match value
            .and_then(parse_float)
            .filter(|value| value.is_finite())
        {
            Some(value) => {
                self.sum += value;
                self.contributors += 1;
            }
            None => self.skipped += 1,
        }
    }

    /// The wire cell. Every branch that cannot produce an honest number
    /// produces a MARKER rather than a zero: the frontend used to render
    /// `avg` over no numeric rows as `0`, which reads as "the average is zero"
    /// when the truth is "there is no average". `count` is the row count of the
    /// scope this accumulator belongs to (the whole sample, or one group).
    fn cell(&self, op: AggFn, count: usize) -> Cell {
        if op == AggFn::Count {
            return Cell::Number {
                value: count as f64,
                skipped: 0,
            };
        }
        // Empty and non-numeric are DIFFERENT answers to the user: nothing was
        // selected, versus rows were selected and none of them held a number.
        // Testing `count` first is also what makes division by zero
        // unreachable below.
        let reason = if count == 0 {
            Some(Marker::EmptyGroup)
        } else if self.contributors == 0 {
            Some(Marker::NonNumeric)
        } else {
            None
        };
        if let Some(reason) = reason {
            return Cell::Marker {
                reason,
                skipped: self.skipped,
            };
        }
        // Average divides by the CONTRIBUTORS, not the row count, so a column
        // where half the rows are prose still reports the average of the half
        // that are numbers, with the rest counted in `skipped`.
        let value = match op {
            AggFn::Sum => self.sum,
            AggFn::Avg => self.sum / self.contributors as f64,
            AggFn::Count => unreachable!(),
        };
        if value.is_finite() {
            Cell::Number {
                value,
                skipped: self.skipped,
            }
        } else {
            Cell::Marker {
                reason: Marker::NonFinite,
                skipped: self.skipped,
            }
        }
    }
}

/// One grouping key's accumulators. `key: None` is the row that HAS no value
/// for the grouping field, which stays distinct from a row whose value is
/// literally the string `(none)`.
struct Group {
    key: Option<String>,
    count: usize,
    cells: Vec<Accumulator>,
}

/// The fold itself, driven one row at a time by the result reader as it streams
/// the ordered sample. Retains accumulators and group keys; never rows.
pub(crate) struct StatisticsFold {
    /// The EFFECTIVE view (Board grouping resolved, empty grouping cleared), so
    /// every later question about what was requested has one answer.
    view: ViewSettings,
    count: usize,
    overall: Vec<Accumulator>,
    /// First-seen order, which is the order the groups are displayed in.
    groups: Vec<Group>,
    by_key: HashMap<Option<String>, usize>,
    /// Bytes still available for NEW groups. A query grouped by a
    /// high-cardinality property is the one shape whose statistics can grow
    /// without bound, so it is the one shape that gets a budget.
    remaining: usize,
}

impl StatisticsFold {
    pub(crate) fn new(
        view: &ViewSettings,
        max_bytes: usize,
    ) -> Result<Option<Self>, ResultReadError> {
        let view = super::view::effective_statistics_view(view);
        // No aggregates requested and no grouping to imply one: the reader skips
        // the whole fold, and the result carries no statistics at all. Absent
        // statistics is not a zero, and the frontend renders nothing.
        if view.aggregates.is_empty() {
            return Ok(None);
        }
        // A rough retained-size model, deliberately generous rather than exact:
        // a fixed base, plus per-column and per-key allowances big enough to
        // cover the Vec/HashMap overhead the accumulators actually cost. It
        // exists to refuse an unbounded fold, not to predict an allocator.
        let overhead = view
            .aggregates
            .iter()
            .fold(256usize, |size, (field, _)| {
                size.saturating_add(field.as_str().len())
                    .saturating_add(128)
            })
            .saturating_add(
                view.group_by
                    .as_ref()
                    .map_or(0, |field| field.as_str().len()),
            );
        let remaining = max_bytes
            .checked_sub(overhead)
            .ok_or(ResultReadError::StatisticsResourceLimit)?;
        Ok(Some(Self {
            overall: vec![Accumulator::default(); view.aggregates.len()],
            view,
            count: 0,
            groups: Vec::new(),
            by_key: HashMap::new(),
            remaining,
        }))
    }

    pub(crate) fn view(&self) -> &ViewSettings {
        &self.view
    }

    pub(crate) fn add(
        &mut self,
        values: &[Option<String>],
        keys: Vec<Option<String>>,
    ) -> Result<(), ResultReadError> {
        self.count += 1;
        for ((cell, (_, op)), value) in self
            .overall
            .iter_mut()
            .zip(&self.view.aggregates)
            .zip(values)
        {
            cell.add(value.as_deref(), *op);
        }
        if self.view.group_by.is_none() || self.unsupported_formula() {
            return Ok(());
        }
        for key in keys {
            let at = match self.by_key.get(&key) {
                Some(at) => *at,
                None => {
                    // Charged once, when the key is first seen. Twice the key's
                    // bytes because it is retained by both the map and the
                    // group.
                    let bytes = key
                        .as_ref()
                        .map_or(0, String::len)
                        .saturating_mul(2)
                        .saturating_add(256)
                        .saturating_add(self.overall.len().saturating_mul(128));
                    self.remaining = self
                        .remaining
                        .checked_sub(bytes)
                        .ok_or(ResultReadError::StatisticsResourceLimit)?;
                    let at = self.groups.len();
                    self.by_key.insert(key.clone(), at);
                    self.groups.push(Group {
                        key,
                        count: 0,
                        cells: vec![Accumulator::default(); self.overall.len()],
                    });
                    at
                }
            };
            let group = &mut self.groups[at];
            group.count += 1;
            for ((cell, (_, op)), value) in group
                .cells
                .iter_mut()
                .zip(&self.view.aggregates)
                .zip(values)
            {
                cell.add(value.as_deref(), *op);
            }
        }
        Ok(())
    }

    /// Formula grouping is deferred (Martin, 2026-09-09), so a formula key
    /// suppresses the per-group fold entirely rather than inventing a
    /// breakdown. The overall cells stay exact and `finish` says why.
    fn unsupported_formula(&self) -> bool {
        self.view
            .group_by
            .as_ref()
            .is_some_and(|field| field.as_str().starts_with("formula:"))
    }

    pub(crate) fn finish(self) -> QueryStatistics {
        let grouping_status = if self.unsupported_formula() {
            Status::UnsupportedFormula
        } else if self.view.group_by.is_some() {
            Status::Exact
        } else {
            Status::None
        };
        let cells = |acc: &[Accumulator], count| {
            acc.iter()
                .zip(&self.view.aggregates)
                .map(|(cell, (_, op))| cell.cell(*op, count))
                .collect()
        };
        let overall = cells(&self.overall, self.count);
        let groups = (grouping_status == Status::Exact).then(|| {
            self.groups
                .into_iter()
                .map(|group| QueryStatisticsGroup {
                    cells: cells(&group.cells, group.count),
                    key: group.key,
                    count: group.count,
                })
                .collect()
        });
        QueryStatistics {
            count: self.count,
            aggregates: self.view.aggregates,
            group_by: self.view.group_by,
            overall,
            groups,
            grouping_status,
        }
    }
}

/// ECMAScript parseFloat: trim leading StringWhiteSpace, then consume the
/// longest decimal prefix. An incomplete exponent is outside that prefix.
fn parse_float(text: &str) -> Option<f64> {
    let text = text.trim_start_matches(|c: char| {
        matches!(c,
        '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' |
        '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
    });
    let bytes = text.as_bytes();
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    if text.get(at..)?.starts_with("Infinity") {
        return Some(if bytes.first() == Some(&b'-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    let mut digits = at - start;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        digits += at - start;
    }
    if digits == 0 {
        return None;
    }
    let mut end = at;
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at > start {
            end = at;
        }
    }
    text[..end].parse().ok()
}
