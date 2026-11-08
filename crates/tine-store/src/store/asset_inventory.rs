//! Orphan-asset safety inventory: parsed pages plus set-aside duplicate-day files (audit L8-03).
use super::*;
use std::io;

impl Snapshot {
    /// Carry the previous generation's set-aside files by the paths this
    /// publication changed: each changed path is set aside when it is named
    /// but holds no parsed page. Unset when the previous generation had none
    /// built, or its change list cannot describe the move (a whole-table
    /// replacement); the next orphan check builds it. Cost O(changed x log P).
    pub(super) fn carry_unparsed(&self, graph: &Graph, old: Option<&Snapshot>, changed: &[String]) {
        let Some((old, previous)) = old.and_then(|old| Some((old, old.unparsed.get()?))) else {
            return;
        };
        if changed.is_empty() && !self.graph.same_pages(&old.graph) {
            return;
        }
        let mut unparsed = Arc::clone(previous);
        for rel in changed {
            let path = graph.root.join(rel);
            let set_aside =
                self.name_by_path.contains_key(&path) && !self.graph.asset_cache_covers(rel, &path);
            if set_aside != unparsed.contains_key(&path) {
                let map = Arc::make_mut(&mut unparsed);
                if set_aside {
                    map.insert(path, ());
                } else {
                    map.remove(&path);
                }
            }
        }
        let _ = self.unparsed.set(unparsed);
    }

    /// Named page files without a parsed page; one walk of the name table on
    /// a generation's first use. Cost O(P log P) then, O(1) after.
    fn unparsed(&self, graph: &Graph) -> &Arc<SharedMap<PathBuf, ()>> {
        let unparsed = self.unparsed.get_or_init(|| {
            Arc::new(
                self.name_by_path
                    .iter()
                    .filter(|(path, _)| !self.graph.asset_cache_covers(&graph.rel_path(path), path))
                    .map(|(path, _)| (path.clone(), ()))
                    .collect(),
            )
        });
        #[cfg(test)]
        {
            // Oracle: the carried set is the full comparison's answer.
            let parsed: HashSet<_> = self
                .graph
                .physical_pages
                .iter()
                .filter(|(entry, _)| {
                    entry.kind != PageKind::Journal
                        || entry.date_key.is_none()
                        || self
                            .graph
                            .pages
                            .positions
                            .contains_key(entry.rel_path_str())
                })
                .map(|(entry, _)| &entry.path)
                .collect();
            let scanned: Vec<_> = (self.name_by_path.iter())
                .filter(|(path, _)| !parsed.contains(path))
                .map(|(path, _)| path.clone())
                .collect();
            let carried: Vec<_> = unparsed.iter().map(|(path, _)| path.clone()).collect();
            assert_eq!(
                carried, scanned,
                "set-aside file set drifted from a full comparison"
            );
        }
        unparsed
    }
}

impl Store {
    /// Asset references from every published physical page file, including
    /// duplicate-day journals omitted from queries. Waits for launch reconciliation.
    /// An unreadable or unanalyzable file fails the inventory, naming that file.
    /// No writes and nothing persisted; only set-aside files are read and
    /// parsed. Cost O(referenced names) plus those set-aside files' bytes,
    /// after a one-time O(P + B + text bytes) walk per session.
    pub fn referenced_assets(&self) -> io::Result<Arc<HashSet<String>>> {
        let view = self.whole_graph_reconciled().map_err(|error| match error {
            LoadError::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
            LoadError::Failed { reason } => io::Error::other(reason),
        })?;
        let mut names = self.set_aside_asset_refs(&view)?;
        names.extend(view.graph.asset_refs().names().cloned());
        Ok(Arc::new(names))
    }

    /// Build the latest view's asset index and set-aside file set if it
    /// inherited none, so the orphan check that follows under the writer
    /// finds them built. A publication captured while a build runs does not
    /// inherit it, so recheck the latest view, a bounded number of times
    /// (a save during every build would leave the check to build under the
    /// writer, as before this index). No file reads.
    pub(crate) fn warm_asset_refs(&self) {
        for _ in 0..3 {
            let Ok(view) = self.whole_graph_reconciled() else {
                return;
            };
            if view.graph.asset_refs_built() && view._snapshot.unparsed.get().is_some() {
                return;
            }
            view.graph.asset_refs();
            view._snapshot.unparsed(&self.graph);
        }
    }

    // The orphan-trash guard holds the writer through this check and the move.
    // A listing uses its captured view; the guard rechecks the latest view.
    /// Whether any published page file references asset `name`. Cost
    /// O(log A) on the view's asset index plus the set-aside files' bytes;
    /// the index is built here only if no earlier reader warmed it.
    pub(crate) fn asset_referenced_in(&self, view: &WholeGraph, name: &str) -> io::Result<bool> {
        let set_aside = self.set_aside_asset_refs(view)?;
        Ok(set_aside.contains(name) || view.graph.asset_refs().references(name))
    }

    /// Asset references of the files set aside from `view`'s parsed pages,
    /// after refusing any unreadable file in the view or the live graph.
    fn set_aside_asset_refs(&self, view: &WholeGraph) -> io::Result<HashSet<String>> {
        // A quiet in-flux read is still an incomplete reference inventory.
        // Never use delayed automatic notices as proof that trash is safe.
        let current_failures = self.graph.unreadable_pages();
        if let Some((file, reason)) = current_failures
            .first()
            .or_else(|| view.unreadable_files().first())
        {
            return Err(io::Error::other(format!("{}: {reason}", file.as_str())));
        }
        let mut names = HashSet::new();
        for (path, _) in view._snapshot.unparsed(&self.graph).iter() {
            let id = PageId::from(self.graph.rel_path(path));
            let result = self
                .page_spot(&id)
                .map_err(|error| io::Error::other(format!("{error:?}")))
                .and_then(|(_, path)| crate::model::collect_file_asset_refs(&path, &mut names));
            result.map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", id.as_str()))
            })?;
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_stray_cannot_authorize_listing_or_trash() {
        let mut observations = Vec::new();
        for (trash, delivery) in [false, true].into_iter().flat_map(|trash| {
            ["replace", "truncate", "grow", "rewrite"].map(|delivery| (trash, delivery))
        }) {
            let root = tempfile::tempdir().unwrap();
            for dir in ["journals", "assets", "logseq"] {
                fs::create_dir_all(root.path().join(dir)).unwrap();
            }
            fs::write(root.path().join("journals/2026-10-02.md"), "- day\n").unwrap();
            let stray = root.path().join("journals/2026_10_02.md");
            let referenced = "- ![photo](../assets/photo.png)\n";
            fs::write(&stray, " ".repeat(referenced.len())).unwrap();
            let asset = root.path().join("assets/photo.png");
            fs::write(&asset, b"png").unwrap();
            let store = Store::open(root.path(), Default::default()).unwrap().0;
            store.whole_graph_reconciled().unwrap();
            store.watch.stop_background_for_test();
            assert!(store.referenced_assets().unwrap().is_empty());
            let id = store.file_id(Area::Assets, "photo.png").unwrap();
            let (_, rev) = store.read(&id, None).unwrap();

            // Sync changes the file after the inventory opened it, before it
            // reads that descriptor. Replacements/rewrites keep length + mtime.
            let modified = fs::metadata(&stray).unwrap().modified().unwrap();
            let replacement = root.path().join("replacement.md");
            fs::write(&replacement, referenced).unwrap();
            // Windows sets file times only through a handle opened for writing.
            fs::File::options()
                .write(true)
                .open(&replacement)
                .unwrap()
                .set_modified(modified)
                .unwrap();
            crate::model::READ_PARSE_BYTES_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || match delivery {
                    "replace" => fs::rename(&replacement, &stray).unwrap(),
                    "truncate" => fs::write(&stray, b"").unwrap(),
                    "grow" => fs::write(&stray, format!("{referenced}- another block\n")).unwrap(),
                    "rewrite" => {
                        fs::write(&stray, referenced).unwrap();
                        fs::File::options()
                            .write(true)
                            .open(&stray)
                            .unwrap()
                            .set_modified(modified)
                            .unwrap();
                    }
                    _ => unreachable!(),
                }));
            });
            let result = if trash {
                let mut tx = store.transaction(None);
                tx.trash(&id, rev, crate::TrashIf::UnreferencedAsset);
                match tx.commit() {
                    crate::TxOutcome::NotCommitted {
                        why: crate::Why::Failed(error),
                        ..
                    } => Err(format!("{error:?}")),
                    other => Ok(format!("{other:?}")),
                }
            } else {
                store
                    .referenced_assets()
                    .map(|refs| format!("{refs:?}"))
                    .map_err(|error| error.to_string())
            };
            observations.push((trash, delivery, result, asset.exists()));
            if delivery != "truncate" {
                assert!(store.referenced_assets().unwrap().contains("photo.png"));
            }
        }
        for (trash, delivery, result, kept) in &observations {
            // Some platforms cannot observe an in-place same-stamp rewrite.
            // This hook completes it before the read, so those platforms may
            // safely count the new reference instead of refusing uncertainty.
            let counted_rewrite = *delivery == "rewrite"
                && result.as_ref().is_ok_and(|value| {
                    if *trash {
                        value.contains("AssetReferenced")
                    } else {
                        value.contains("photo.png")
                    }
                });
            assert!(
                (result.as_ref().is_err_and(|error| error.contains("journals/2026_10_02.md")) || counted_rewrite) && *kept,
                "trash={trash}, {delivery}: a changing stray read authorized an orphan decision: {observations:?}"
            );
        }
    }
}
