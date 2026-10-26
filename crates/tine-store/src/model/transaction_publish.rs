//! Transaction publication of bytes observed after the guarded writes.
use super::*;
impl Graph {
    // Test/manual publication keeps the external-reconciliation comparison.
    #[cfg(test)]
    pub(crate) fn transaction_publish_page(
        &self,
        path: &Path,
        bytes: Option<&[u8]>,
        saved: Option<&Document>,
        file_set_changed: bool,
    ) {
        self.transaction_publish_page_inner(path, bytes, saved, file_set_changed, false);
    }
    /// Publish `path`'s observed `bytes`. For an own save or reference
    /// rewrite, returns the cacheable entry named from those bytes (no
    /// preamble reopen), which the snapshot's name index reuses.
    pub(crate) fn transaction_publish_page_inner(
        &self,
        path: &Path,
        bytes: Option<&[u8]>,
        saved: Option<&Document>,
        file_set_changed: bool,
        own_rename: bool,
    ) -> Option<PageEntry> {
        let before_gen = self.cache_generation();
        let mut named = None;
        match bytes {
            Some(bytes) => {
                if validate_parse_bytes_for_path(bytes, path).is_err() {
                    self.invalidate_cache();
                    return None;
                }
                let Ok(content) = std::str::from_utf8(bytes) else {
                    self.invalidate_cache();
                    return None;
                };
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(saved) = saved {
                        #[cfg(test)]
                        if !self.cache_built() {
                            crate::store::pause_at_hook(&self.cold_cache_reconcile_pause);
                        }
                        if let Some(entry) = self.cacheable_page_entry_in(path, Some(content)) {
                            named = Some(entry.clone());
                            self.cache_upsert(entry, saved.clone(), DiskObs::of(content));
                        }
                    } else if own_rename {
                        // Publication already observed changed bytes and matched them
                        // to this guarded rewrite. Comparing with the old normalized
                        // document cannot change that observation, and re-parses it.
                        if let Some(entry) = self.cacheable_page_entry_in(path, Some(content)) {
                            let (doc, disk) = parse_page_content(&entry, content);
                            named = Some(entry.clone());
                            self.cache_upsert(entry, doc, disk);
                        }
                    } else {
                        self.reconcile_page_content(path, content, false);
                    }
                }))
                .is_err()
                {
                    named = None;
                    self.invalidate_cache();
                    self.record_index_failure(path);
                }
            }
            None => {
                let _ = self.forget_file_internal(path);
            }
        }
        if file_set_changed {
            self.forget_name_memos();
        } else {
            let after_gen = self.cache_generation();
            if after_gen == before_gen || after_gen == before_gen + 1 {
                self.retag_name_memos(before_gen, after_gen);
            }
        }
        self.recent_writes.lock().unwrap().remove(path);
        named
    }
}
