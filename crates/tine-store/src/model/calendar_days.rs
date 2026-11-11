//! Calendar content days in active page-slot order. Built from parsed pages,
//! patched only at changed paths, and rebuilt on checkpoint load; never persisted.
//! Enumeration is O(D) for D content days, with no document probes. Publication
//! evaluates one changed journal's content and patches O(log D) shared nodes.
use super::*;

#[derive(Clone, Default)]
pub(super) struct CalendarDays(SharedMap<usize, i64>);

impl CalendarDays {
    pub(super) fn capture(
        old: Option<&ReadSnapshot>,
        pages: &Arc<Pages>,
        paths: &[String],
    ) -> Self {
        if let Some(old) = old.filter(|old| !paths.is_empty() || Arc::ptr_eq(&old.pages, pages)) {
            let mut next = old.calendar_days.clone();
            for path in paths {
                let before = old.pages.positions.get(path).copied();
                let after = pages.positions.get(path).copied();
                if let Some(slot) = before.filter(|slot| Some(*slot) != after) {
                    next.0.remove(&slot);
                }
                if let Some(slot) = after {
                    let day = Self::content_day(pages.get(slot).expect("active slot"));
                    if next.0.get(&slot).copied() != day {
                        if let Some(day) = day {
                            next.0.insert(slot, day);
                        } else {
                            next.0.remove(&slot);
                        }
                    }
                }
            }
            return next;
        }
        Self(SharedMap::from_sorted_unique(
            pages
                .slots()
                .filter_map(|(slot, page)| Self::content_day(page).map(|day| (slot, day)))
                .collect(),
        ))
    }

    fn content_day((entry, doc): &persistent::Page) -> Option<i64> {
        #[cfg(test)]
        page_probe();
        (entry.kind == PageKind::Journal)
            .then_some(entry.date_key)
            .flatten()
            .filter(|_| doc_has_content(&doc.roots))
    }
}

impl ReadSnapshot {
    pub(crate) fn journal_content_days(&self) -> Vec<i64> {
        self.calendar_days.0.values().copied().collect()
    }
}

#[cfg(test)]
thread_local! {
    static WORK: std::cell::Cell<(usize, usize, usize)> = const { std::cell::Cell::new((0, 0, 0)) };
}

#[cfg(test)]
pub(super) fn take_work() -> (usize, usize, usize) {
    WORK.with(|work| work.replace((0, 0, 0)))
}

#[cfg(test)]
pub(super) fn page_probe() {
    WORK.with(|work| {
        let (p, d, b) = work.get();
        work.set((p + 1, d, b));
    });
}

/// Journal text beyond properties (calendar days, carry); the parser's visible text
/// decides, so Org `memo:: x` prose counts (OG-C5 L12-S1). Not the template guard.
pub(super) fn doc_has_content(blocks: &[DocBlock]) -> bool {
    #[cfg(test)]
    WORK.with(|work| {
        let (p, d, b) = work.get();
        work.set((p, d + 1, b));
    });
    fn visit(blocks: &[DocBlock]) -> bool {
        blocks.iter().any(|block| {
            #[cfg(test)]
            WORK.with(|work| {
                let (p, d, b) = work.get();
                work.set((p, d, b + 1));
            });
            !block.visible_text().trim().is_empty() || visit(&block.children)
        })
    }
    visit(blocks)
}

#[cfg(test)]
#[path = "calendar_days_tests.rs"]
mod tests;
