//! Orphan-asset safety inventory: parsed pages plus set-aside duplicate-day files (audit L8-03).
use super::*;
use std::io;

impl Store {
    /// Asset references from every published physical page file, including
    /// duplicate-day journals omitted from queries. Waits for launch reconciliation.
    /// An unreadable or unanalyzable file fails the inventory, naming that file.
    /// No writes or new
    /// index: only files absent from the parsed snapshot are read and parsed.
    /// Cost O(P + B + scanned text bytes), plus those set-aside files' bytes.
    pub fn referenced_assets(&self) -> io::Result<Arc<HashSet<String>>> {
        let view = self.whole_graph_reconciled().map_err(|error| match error {
            LoadError::Closed => io::Error::new(io::ErrorKind::BrokenPipe, "store closed"),
            LoadError::Failed { reason } => io::Error::other(reason),
        })?;
        self.referenced_assets_in(&view)
    }

    // The orphan-trash guard holds the writer through this check and the move.
    // A listing uses its captured view; the guard rechecks the latest view.
    pub(crate) fn referenced_assets_in(
        &self,
        view: &WholeGraph,
    ) -> io::Result<Arc<HashSet<String>>> {
        // A quiet in-flux read is still an incomplete reference inventory.
        // Never use delayed automatic notices as proof that trash is safe.
        let current_failures = self.graph.unreadable_pages();
        if let Some((file, reason)) = current_failures
            .first()
            .or_else(|| view.unreadable_files().first())
        {
            return Err(io::Error::other(format!("{}: {reason}", file.as_str())));
        }
        let mut names = Arc::unwrap_or_clone(view.referenced_assets());
        let parsed: HashSet<_> = view
            .graph
            .pages
            .iter()
            .map(|(entry, _)| &entry.path)
            .collect();
        for (path, _) in view._snapshot.name_by_path.iter() {
            if parsed.contains(path) {
                continue;
            }
            let id = PageId::from(self.graph.rel_path(path));
            let result = self
                .page_spot(&id)
                .map_err(|error| io::Error::other(format!("{error:?}")))
                .and_then(|(_, path)| crate::model::collect_file_asset_refs(&path, &mut names));
            result.map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", id.as_str()))
            })?;
        }
        Ok(Arc::new(names))
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
            fs::write(root.path().join("journals/2026_10_02.md"), "- day\n").unwrap();
            let stray = root.path().join("journals/2026-10-02.md");
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
                (result.as_ref().is_err_and(|error| error.contains("journals/2026-10-02.md")) || counted_rewrite) && *kept,
                "trash={trash}, {delivery}: a changing stray read authorized an orphan decision: {observations:?}"
            );
        }
    }
}
