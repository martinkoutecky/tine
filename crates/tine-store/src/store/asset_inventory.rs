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
        if let Some((file, reason)) = view.unreadable_files().first() {
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
