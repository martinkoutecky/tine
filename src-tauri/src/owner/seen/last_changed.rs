//! Bullet dates reuse the seen owner's walk, snapshots and attribution.
use super::*;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", content = "time", rename_all = "snake_case")]
pub(crate) enum Changed {
    Committed(i64),
    Uncommitted,
    HistoryLimit,
    Unknown,
}

#[derive(Clone, Serialize)]
pub(crate) struct BlockChanged {
    block_path: BlockPath,
    changed: Changed,
}

/// Binding-owned, one-page/HEAD cache. Historical parses are temporary; only
/// HEAD's normalized tree and its dates survive, plus the last working map.
#[derive(Default)]
pub(crate) struct Cache(Option<Entry>);

struct Entry {
    path: String,
    tip: Option<Oid>,
    doc: Option<Document>,
    dates: HashMap<BlockPath, Changed>,
    working: String,
    result: Arc<Vec<BlockChanged>>,
}

impl Cache {
    /// Read a page on hover. One HEAD ref read on hits; at most WALK_LIMIT
    /// first-parent links on misses. No file writes or index/status reads.
    /// Re-mapping changed working text costs one page parse/alignment.
    pub(crate) fn read_page_dates(
        &mut self,
        root: &Path,
        path: &str,
        current: &str,
    ) -> Result<Arc<Vec<BlockChanged>>, String> {
        relative(path)?;
        let Some((repo, prefix)) = open_if_repository(root)? else {
            return Ok(Arc::new(Vec::new()));
        };
        let tip = head(&repo)?;
        if !self
            .0
            .as_ref()
            .is_some_and(|e| e.path == path && e.tip == tip)
        {
            let (doc, dates, at_head) = match derive_page_dates(
                &repo,
                &prefix.join(path),
                path.ends_with(".org"),
                tip,
                current,
            ) {
                Ok((doc, dates, at_head)) => (Some(doc), dates, at_head),
                Err(reason) if reason == NOT_UTF8 => (None, HashMap::new(), false),
                Err(reason) => return Err(reason),
            };
            let mut result = Vec::new();
            if at_head {
                each(&doc.as_ref().unwrap().roots, &[], &mut |p| {
                    result.push(BlockChanged {
                        changed: dates[&p].clone(),
                        block_path: p,
                    });
                });
            }
            self.0 = Some(Entry {
                path: path.into(),
                tip,
                doc,
                dates,
                working: if at_head {
                    current.into()
                } else {
                    String::new()
                },
                result: Arc::new(result),
            });
        }
        let Some(entry) = &mut self.0 else {
            return Ok(Arc::new(Vec::new()));
        };
        let Some(head) = &entry.doc else {
            return Ok(entry.result.clone());
        };
        if entry.working != current {
            let org = path.ends_with(".org");
            let mut doc = parse(current, org);
            normalize(&mut doc.roots, org);
            let working = align(&head.roots, &doc.roots, &mut PersistedIds::new());
            let mut result = Vec::new();
            each(&doc.roots, &[], &mut |p| {
                let changed = if working.changed.contains(&p) {
                    Changed::Uncommitted
                } else {
                    working
                        .back(&p)
                        .and_then(|old| entry.dates.get(&old))
                        .cloned()
                        .unwrap_or(Changed::Unknown)
                };
                result.push(BlockChanged {
                    block_path: p,
                    changed,
                });
            });
            entry.working = current.into();
            entry.result = Arc::new(result);
        }
        Ok(entry.result.clone())
    }
}

fn derive_page_dates(
    repo: &Repository,
    path: &Path,
    org: bool,
    tip: Option<Oid>,
    current: &str,
) -> Result<(Document, HashMap<BlockPath, Changed>, bool), String> {
    let tip_blob = tip
        .map(|oid| blob_id(repo, oid, path))
        .transpose()?
        .flatten();
    if tip_blob.is_none() {
        return Ok((Document::default(), HashMap::new(), current.is_empty()));
    }
    let walk = Walk::new(repo, tip, tip_blob, None, path, None)?;
    let touching: Vec<_> = walk
        .links
        .iter()
        .filter(|l| l.before != l.after)
        .rev()
        .collect();
    let mut blobs = vec![walk.blob_at(walk.links.len())];
    let mut times = vec![0];
    for link in touching {
        blobs.push(link.after);
        times.push(link.time);
    }
    let mut history = History::new(repo, org, blobs, vec![false; times.len()]);
    let last = history.last();
    let doc = history.snapshot_doc(last)?.clone();
    let at_head = history.snapshot_text(last)? == current;
    let mut paths = Vec::new();
    each(&doc.roots, &[], &mut |p| paths.push(p));
    // Dates ask about every HEAD block in one pass. Release each newer
    // snapshot after using its alignment; seen retains steps for removals.
    let changed = history.last_changed_steps(paths.clone(), false)?;
    let mut dates = HashMap::new();
    for p in paths {
        let date = changed
            .get(&p)
            .map(|i| Changed::Committed(times[*i]))
            .unwrap_or(if walk.root {
                Changed::Unknown
            } else {
                Changed::HistoryLimit
            });
        dates.insert(p, date);
    }
    Ok((doc, dates, at_head))
}

#[cfg(test)]
thread_local! { static COUNTS: std::cell::RefCell<[usize; 6]> = const { std::cell::RefCell::new([0; 6]) }; }
#[cfg(test)]
thread_local! { static PEAK_DOCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub(super) fn record_docs(n: usize) {
    PEAK_DOCS.with(|peak| peak.set(peak.get().max(n)));
}
#[cfg(test)]
pub(super) fn count(i: usize) {
    COUNTS.with(|c| c.borrow_mut()[i] += 1);
}

#[cfg(test)]
#[path = "last_changed_tests.rs"]
mod tests;
