//! Which rows a whole-graph index build panics on. Such a build names no page,
//! so when one panics the publication asks here, then lists those rows
//! unreadable and builds without them (D-20261019-QC). Failure path only; a
//! build that does not panic never comes here.
use super::persistent::Page;
use super::*;

impl Graph {
    /// The rows the index stages panic on when built without the rest of the
    /// graph; none when no row explains the panic or naming them would take
    /// more than [`PROBE_BUILDS`] builds. Under the writer.
    pub(crate) fn rows_that_panic_an_index_build(&self) -> Vec<PathBuf> {
        #[cfg(test)]
        let _cost = ProbeCost::charge(&self.root);
        let Some(pages) = self.peek_pages() else {
            return Vec::new();
        };
        let rows: Vec<Page> = (pages.iter())
            .map(|(entry, doc)| (entry.clone(), Arc::clone(doc)))
            .collect();
        (blame(&rows, &|rows: &[Page]| self.index_build_panics(rows)).into_iter())
            .map(|(entry, _)| entry.path.clone())
            .collect()
    }

    /// Whether every index a first capture builds panics over just `rows`.
    /// Touches no installed state.
    pub(super) fn index_build_panics(&self, rows: &[Page]) -> bool {
        #[cfg(test)]
        let _probing = ProbeScope::enter();
        #[cfg(test)]
        ProbeCost::count_build(&self.root);
        // The panic this provokes is caught right here: no crash to record.
        let _contained = tine_core::diag_line::ContainedPanics::enter();
        let entries: Vec<PageEntry> = rows.iter().map(|(entry, _)| entry.clone()).collect();
        let config = (*self.current_config()).clone();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ReadSnapshot::capture_rows(
                Arc::new(Pages::from(rows.to_vec())),
                0,
                Default::default(),
                Default::default(),
                config,
                Arc::new(EntryList::from(entries.as_slice())),
                None,
                &[],
            )
        }))
        .is_err()
    }
}

/// The most probe builds one failed whole-graph build may cost. One bad row in
/// n takes 2 + 2·⌈log2 n⌉ (28 measured at 10k pages, 42 at a million) and two
/// take at most 4·⌈log2 n⌉, so this names one bad file in any graph and two in
/// any graph up to 2^15 rows. More bad files than that are no bad file but a
/// regression on something common: probing on would cost thousands of
/// panicking builds under the writer, at every launch (13,886 builds with 30%
/// of a 10k graph bad), so the graph fails as it did before probing existed.
/// Builds run depth first, so a probe that hits this has spent about two whole
/// builds' work.
pub(crate) const PROBE_BUILDS: usize = 64;

/// The rows `panics` reproduces on: narrowed by halves from a set that panics,
/// after a control over no rows at all, within [`PROBE_BUILDS`] probes. A
/// panic only some combination of rows gives blames none of them, and neither
/// does one every row of several gives: that is no bad file, and the graph
/// fails instead (`refresh()` retries).
fn blame<'a, R>(rows: &'a [R], panics: &impl Fn(&[R]) -> bool) -> Vec<&'a R> {
    let mut left = PROBE_BUILDS;
    let mut probe = |rows: &[R]| {
        left = left.checked_sub(1)?;
        Some(panics(rows))
    };
    let mut guilty = Vec::new();
    if rows.is_empty()
        || probe(&[]) != Some(false)
        || probe(rows) != Some(true)
        || narrow(rows, &mut probe, &mut guilty).is_none()
        || (rows.len() > 1 && guilty.len() == rows.len())
    {
        return Vec::new();
    }
    guilty
}

/// None when the budget ran out.
fn narrow<'a, R>(
    rows: &'a [R],
    probe: &mut impl FnMut(&[R]) -> Option<bool>,
    guilty: &mut Vec<&'a R>,
) -> Option<()> {
    if let [row] = rows {
        guilty.push(row);
        return Some(());
    }
    let (left, right) = rows.split_at(rows.len() / 2);
    for half in [left, right] {
        if probe(half)? {
            narrow(half, probe, guilty)?;
        }
    }
    Some(())
}

/// Test-only: while listed, this page's row panics a reference-index stage,
/// whatever its bytes: a transient panic its bytes do not explain.
#[cfg(test)]
pub(crate) static TEST_ROW_PANIC_PATHS: std::sync::Mutex<Vec<PathBuf>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(super) fn test_row_fault(path: &Path) {
    if TEST_ROW_PANIC_PATHS
        .lock()
        .unwrap()
        .iter()
        .any(|p| p == path)
    {
        panic!("deterministic test fault: this row panics an index stage");
    }
}

/// Test-only: per graph root, the probe builds and the wall time the probe
/// spent under the writer.
#[cfg(test)]
pub(crate) static PROBE_COSTS: std::sync::Mutex<Vec<(PathBuf, usize, std::time::Duration)>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn probe_cost(root: &Path) -> (usize, std::time::Duration) {
    (PROBE_COSTS.lock().unwrap().iter())
        .find(|(known, ..)| known == root)
        .map_or((0, Default::default()), |(_, builds, time)| {
            (*builds, *time)
        })
}

#[cfg(test)]
struct ProbeCost(PathBuf, std::time::Instant);

#[cfg(test)]
impl ProbeCost {
    fn charge(root: &Path) -> Self {
        Self(root.to_path_buf(), std::time::Instant::now())
    }

    fn count_build(root: &Path) {
        Self::add(root, 1, Default::default());
    }

    fn add(root: &Path, builds: usize, time: std::time::Duration) {
        let mut costs = PROBE_COSTS.lock().unwrap();
        match costs.iter_mut().find(|(known, ..)| known == root) {
            Some(cost) => (cost.1, cost.2) = (cost.1 + builds, cost.2 + time),
            None => costs.push((root.to_path_buf(), builds, time)),
        }
    }
}

#[cfg(test)]
impl Drop for ProbeCost {
    fn drop(&mut self) {
        Self::add(&self.0, 0, self.1.elapsed());
    }
}

#[cfg(test)]
thread_local! {
    static PROBING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
struct ProbeScope;

#[cfg(test)]
impl ProbeScope {
    fn enter() -> Self {
        PROBING.with(|probing| probing.set(true));
        Self
    }
}

#[cfg(test)]
impl Drop for ProbeScope {
    fn drop(&mut self) {
        PROBING.with(|probing| probing.set(false));
    }
}

/// `TEST_FULL_INDEX_PANIC_ROOTS` means "a panic no row explains": it fires on
/// a publication's own build and is silent in the probes above.
#[cfg(test)]
pub(super) fn test_full_build_fault<'a>(mut rows: impl Iterator<Item = &'a Path>) {
    if PROBING.with(std::cell::Cell::get) {
        return;
    }
    let roots = TEST_FULL_INDEX_PANIC_ROOTS.lock().unwrap();
    if rows.any(|path| roots.iter().any(|root| path.starts_with(root))) {
        drop(roots);
        panic!("deterministic test sentinel for a full graph index build panic");
    }
}

#[cfg(test)]
mod tests {
    use super::blame;
    use std::cell::Cell;

    fn blamed(rows: &[u32], panics: impl Fn(&[u32]) -> bool) -> (Vec<u32>, usize) {
        let probes = Cell::new(0);
        let probe = |rows: &[u32]| {
            probes.set(probes.get() + 1);
            panics(rows)
        };
        let guilty = blame(rows, &probe).into_iter().copied().collect();
        (guilty, probes.get())
    }

    #[test]
    fn blame_narrows_to_the_rows_that_panic_alone() {
        let rows: Vec<u32> = (0..1024).collect();
        let (guilty, probes) = blamed(&rows, |rows| rows.contains(&700));
        assert_eq!(guilty, [700]);
        assert!(probes <= 2 + 2 * 10, "{probes} probes for one bad row");
        let rows: Vec<u32> = (0..32_768).collect();
        let (guilty, probes) = blamed(&rows, |rows| rows.contains(&0) || rows.contains(&32_767));
        assert_eq!(guilty, [0, 32_767], "two bad rows at 2^15 fit the budget");
        assert!(probes <= super::PROBE_BUILDS);
        let (guilty, _) = blamed(&rows, |rows| rows.contains(&3) || rows.contains(&900));
        assert_eq!(guilty, [3, 900]);
        assert_eq!(
            blamed(&[7], |rows| !rows.is_empty()).0,
            [7],
            "a one-page graph"
        );
    }

    #[test]
    fn blame_names_no_row_for_a_panic_no_row_explains() {
        let rows: Vec<u32> = (0..64).collect();
        assert!(blamed(&rows, |_| true).0.is_empty(), "panics with no rows");
        assert!(blamed(&rows, |_| false).0.is_empty(), "does not reproduce");
        assert!(
            blamed(&rows, |rows| !rows.is_empty()).0.is_empty(),
            "every row at once"
        );
        let pair = |rows: &[u32]| rows.contains(&1) && rows.contains(&60);
        assert!(blamed(&rows, pair).0.is_empty(), "only a combination");
    }

    /// Many bad rows cost thousands of panicking builds under the
    /// writer before. They cost the budget, then the graph fails unblamed.
    #[test]
    fn blame_gives_up_within_its_budget() {
        let rows: Vec<u32> = (0..10_000).collect();
        for (case, bad) in [
            ("every row", (|_| true) as fn(&u32) -> bool),
            ("30% of rows", |row| row % 10 < 3),
            ("three rows", |row| [5, 5_000, 9_999].contains(row)),
        ] {
            let (guilty, probes) = blamed(&rows, |rows| rows.iter().any(bad));
            assert!(guilty.is_empty(), "{case}: {} blamed", guilty.len());
            assert!(probes <= super::PROBE_BUILDS, "{case}: {probes} probes");
        }
    }
}
