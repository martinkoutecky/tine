//! Rows kept out of the index for one page's sake, each listed unreadable so
//! no page goes missing silently, and how they get back: other bytes, a
//! rebuild, or for a retired row a bounded retry of the same bytes.
use super::*;

/// A page's row panicked an index stage on the bytes of revision `rev`. Those
/// bytes stay out until a retry passes (`retrying`), of which each revision
/// gets [`RETRIES_PER_REVISION`], so a page that really panics stops costing.
pub(super) struct Retired {
    rev: String,
    retries: u8,
    retrying: bool,
}

const RETRIES_PER_REVISION: u8 = 3;

pub(super) fn is_parser_panic(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<ParserPanicked>())
}

impl Graph {
    /// A publication's index stage panicked on `path`'s row: the same
    /// isolation, for a held page's row too. A held row cannot be left in
    /// place, since every later capture would patch it and panic again; its
    /// owner's next publication of other bytes installs it afresh.
    pub(crate) fn retire_index_stage_failure(&self, path: &Path) {
        let cache = self.derived.cache.write().unwrap();
        if let Some(rev) = self.derived.disk_revs.read().unwrap().get(path).cloned() {
            let mut retired = self.derived.index_retired.write().unwrap();
            let retries = (retired.get(path).filter(|known| known.rev == rev))
                .map_or(RETRIES_PER_REVISION, |known| known.retries);
            let entry = Retired {
                rev,
                retries,
                retrying: false,
            };
            retired.insert(path.to_path_buf(), entry);
        }
        self.retire_failed_row(cache, path);
    }

    /// Whether `path`'s row needs nothing for the bytes of revision `rev`:
    /// it was derived from them, or they panicked an index stage.
    pub(in crate::model) fn row_settled(&self, path: &Path, rev: &str) -> bool {
        self.cached_rev(path).is_some_and(|r| r == rev)
            || (self.derived.index_retired.read().unwrap().get(path))
                .is_some_and(|known| known.rev == rev && !known.retrying)
    }

    /// Whether a row from the bytes of revision `rev` may be installed. Other
    /// bytes than the retired ones end the retirement.
    pub(super) fn admits_row(&self, path: &Path, rev: &str) -> bool {
        let mut retired = self.derived.index_retired.write().unwrap();
        match retired.get(path) {
            Some(known) if known.rev == rev => known.retrying,
            Some(_) => {
                retired.remove(path);
                true
            }
            None => true,
        }
    }

    pub(super) fn retire_failed_row(
        &self,
        mut cache: std::sync::RwLockWriteGuard<'_, Option<Arc<Pages>>>,
        path: &Path,
    ) {
        // A quiet retry must not erase an already reported launch error.
        self.retire_row(cache.as_mut().map(Arc::make_mut), path, false);
        let rel = self.rel_path(path);
        let mut failures = self.derived.page_index_failures.write().unwrap();
        if !failures.contains(&rel) {
            failures.push(rel.clone());
        }
        drop(failures);
        // A checkpoint launch also reconciles through this boundary.
        // Replacing a whole-cache rebuild must retain its per-page error
        // reporting, including when the watcher defers its own notice.
        self.list_unreadable(
            rel,
            ParserPanicked.to_string(),
            "Tine unreadable file producer=index-panic",
        );
        self.move_generation(cache.is_none(), &[path]);
    }

    fn list_unreadable(&self, rel: String, reason: String, producer: &'static str) {
        let id = crate::FileId::from(rel);
        let mut unreadable = self.derived.unreadable_pages.write().unwrap();
        if !unreadable.iter().any(|(known, _)| *known == id) {
            tine_core::diag_line::diagnostic_line(producer);
            Arc::make_mut(&mut unreadable).push((id, reason));
        }
    }

    /// A held page's owner published bytes that give no row. The caller holds
    /// the cache lock and retired the row; its owner's next bytes clear this.
    pub(super) fn list_held_row_failure(&self, path: &Path, error: &io::Error) {
        let rel = self.rel_path(path);
        if is_parser_panic(error) {
            let mut failures = self.derived.page_index_failures.write().unwrap();
            if !failures.contains(&rel) {
                failures.push(rel.clone());
            }
        }
        self.list_unreadable(
            rel,
            error.to_string(),
            "Tine unreadable file producer=held-row",
        );
    }

    /// A transaction observed bytes of `path` that no parse may take
    /// (undecodable, over the size or depth cap). That page's row goes and it
    /// is listed; every other page stays cached.
    pub(crate) fn record_unindexable_bytes(&self, path: &Path, error: impl std::fmt::Display) {
        let mut cache = self.derived.cache.write().unwrap();
        if !self.disk_sourced(path) {
            return;
        }
        self.retire_row(cache.as_mut().map(Arc::make_mut), path, true);
        if graph_text_eligible(&self.root, path, &self.current_config()) {
            self.list_unreadable(
                self.rel_path(path),
                error.to_string(),
                "Tine unreadable file producer=transaction-publish",
            );
        }
        self.move_generation(cache.is_none(), &[path]);
    }

    /// Whether a retired page has a retry left and its bytes are the disk's.
    /// A held page's bytes are its owner's: its release rereads them, so it
    /// costs a focus nothing.
    pub(crate) fn contained_retry_due(&self) -> bool {
        !self.due_retries().is_empty()
    }

    fn due_retries(&self) -> Vec<(PathBuf, String)> {
        let due: Vec<(PathBuf, String)> = (self.derived.index_retired.read().unwrap().iter())
            .filter(|(_, known)| known.retries > 0 && !known.retrying)
            .map(|(path, known)| (path.clone(), known.rev.clone()))
            .collect();
        due.into_iter()
            .filter(|(path, _)| self.disk_sourced(path))
            .collect()
    }

    /// A probe blamed `path`, and the build without it panicked all the
    /// same: the probe's panic was no proof it is the bad file.
    /// Its row is read back and its listing goes; a read that fails keeps it
    /// listed, so it is never missing unreported.
    pub(crate) fn unblame(&self, path: &Path) {
        self.derived.index_retired.write().unwrap().remove(path);
        if let SyncFileResult::Reconciled { .. } = self.sync_file_internal(path, None) {
            let rel = self.rel_path(path);
            (self.derived.page_index_failures.write().unwrap()).retain(|failed| *failed != rel);
            Arc::make_mut(&mut self.derived.unreadable_pages.write().unwrap())
                .retain(|(id, _)| id.as_str() != rel);
        }
    }

    /// Spend a retry on each page a panic retired whose bytes are unchanged:
    /// a read, a parse and a one-row index build, under the writer. Bytes that
    /// still panic change nothing; bytes that pass are installed again, for
    /// the caller to publish. A panic the publication then meets retires the
    /// row again with what is left of its retries.
    pub(crate) fn retry_contained_rows(
        &self,
    ) -> Vec<(PathBuf, Option<PageEntry>, crate::store::FileRev)> {
        let mut back = Vec::new();
        for (path, rev) in self.due_retries() {
            if let Some(known) = self.derived.index_retired.write().unwrap().get_mut(&path) {
                known.retries -= 1;
            }
            if !self.row_passes_alone(&path, &rev) {
                continue;
            }
            self.set_retrying(&path, true);
            let expected = crate::store::FileRev::from(rev);
            match self.sync_file_internal(&path, Some(&expected)) {
                SyncFileResult::Reconciled { entry, rev } => back.push((path, entry, rev)),
                _ => self.set_retrying(&path, false),
            }
        }
        back
    }

    fn set_retrying(&self, path: &Path, retrying: bool) {
        if let Some(known) = self.derived.index_retired.write().unwrap().get_mut(path) {
            known.retrying = retrying;
        }
    }

    /// Whether `path` still holds the bytes of revision `rev` and they index,
    /// on their own, without a panic. Touches no installed state.
    fn row_passes_alone(&self, path: &Path, rev: &str) -> bool {
        let Ok(content) = read_parse_input(path) else {
            return false;
        };
        if content_rev(&content) != rev {
            return false;
        }
        let Some(entry) = self.cacheable_page_entry(path) else {
            return false;
        };
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (doc, _) = parse_page_content(&entry, &content);
            !self.index_build_panics(&[(entry.clone(), Arc::new(doc))])
        }))
        .unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn contained_retries_left(&self, path: &Path) -> Option<u8> {
        let retired = self.derived.index_retired.read().unwrap();
        retired.get(path).map(|known| known.retries)
    }
}
