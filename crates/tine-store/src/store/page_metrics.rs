//! Publication metrics, using held stamps and the shared membership owner.
use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct MetricDelta {
    pub(super) modified: Vec<(String, Option<i64>)>,
    pub(super) counts: Option<Vec<(String, Option<u32>)>>,
}

impl Snapshot {
    /// O(changed paths + affected count identities); cold name initialization
    /// and reference-policy rebuilds compare complete row answers. No disk I/O.
    pub(super) fn metric_changes(
        &self,
        old: Option<&Self>,
        paths: &[String],
        moved: &[String],
    ) -> MetricDelta {
        let modified = paths
            .iter()
            .filter_map(|path| {
                let time = self.graph.metric_mtime(path);
                (time.is_some() || old.and_then(|old| old.graph.metric_mtime(path)).is_some())
                    .then(|| (path.clone(), time))
            })
            .collect();
        let counts = old
            .filter(|old| old.graph.day_support_built())
            .and_then(|old| {
                old.graph.resident_backlink_memberships().map(|before| {
                        let after = self.graph.scan_all_backlink_memberships();
                        let changes = if Arc::ptr_eq(&self.graph, &old.graph) { Some(&[][..]) }
                            else { self.graph.backlink_count_changes_since(&old.graph) };
                        let full = changes.is_none() || !self.graph.day_support_built();
                        let mut keys: BTreeSet<_> = if full {
                            before
                                .scan_all_counts()
                                .into_keys()
                                .chain(after.scan_all_counts().into_keys())
                                .collect()
                        } else {
                            changes.unwrap().iter().cloned().collect()
                        };
                        if full {
                            for snapshot in [old, self] {
                                for row in snapshot.graph.reference_only_pages() {
                                    keys.insert((
                                        row.day.is_some(),
                                        tine_core::refs::page_key(&row.name),
                                    ));
                                }
                            }
                        }
                        for key in moved {
                            for journal in [false, true] {
                                keys.insert((journal, key.clone()));
                            }
                        }
                        for path in paths {
                            for snapshot in [old, self] {
                                if let Some(&slot) = snapshot.graph.pages.positions.get(path) {
                                    if let Some((entry, _)) = snapshot.graph.pages.get(slot) {
                                        keys.insert((
                                            entry.kind == PageKind::Journal,
                                            tine_core::refs::page_key(&entry.name),
                                        ));
                                        for (_, key) in
                                            crate::model::namespace_parents::parent_prefixes(
                                                &entry.name,
                                            )
                                        {
                                            for journal in [false, true] {
                                                keys.insert((journal, key.clone()));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        let rows = |snapshot: &Self, journal, key: &str, count| {
                            let mut rows = BTreeMap::new();
                            if let Some(entry) = snapshot
                                .claimants
                                .get(&(journal, key.to_owned()))
                                .and_then(|entries| entries.first())
                            {
                                rows.insert(entry.rel_path_str().to_owned(), count);
                            }
                            if let Some(Some(row)) =
                                snapshot.graph.resident_reference_only_page(key)
                            {
                                if row.day.is_some() == journal {
                                    rows.insert(format!("\0{}", row.name), count);
                                }
                            }
                            rows
                        };
                        let mut delta = BTreeMap::new();
                        for (journal, key) in keys {
                            let was = rows(old, journal, &key, before.count(journal, &key));
                            let now = rows(self, journal, &key, after.count(journal, &key));
                            for key in was.keys().chain(now.keys()) {
                                if was.get(key) != now.get(key) {
                                    delta.insert(key.clone(), now.get(key).copied());
                                }
                            }
                        }
                        delta.into_iter().collect()
                    })
            });
        MetricDelta { modified, counts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publications_reusing_the_graph_do_not_copy_metric_snapshots() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("pages")).unwrap();
        std::fs::write(root.path().join("pages/Source.md"), "- [[Target]]\n").unwrap();
        let store = Store::open(root.path(), Default::default()).unwrap().0;
        store.stop_watcher_for_tests();
        store.whole_graph().unwrap().page_backlink_counts();
        let held = store.changes.snapshot.read().unwrap().clone().unwrap();
        let answer = held.graph.resident_backlink_memberships().unwrap();
        let before = answer
            .snapshot_reads
            .load(std::sync::atomic::Ordering::Relaxed);
        let delta = held.metric_changes(Some(&held), &["assets/image.png".to_owned()], &[]);
        assert!(delta.modified.is_empty());
        assert_eq!(delta.counts, Some(Vec::new()));
        assert_eq!(
            answer
                .snapshot_reads
                .load(std::sync::atomic::Ordering::Relaxed),
            before
        );
    }
}
