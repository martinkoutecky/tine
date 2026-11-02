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
        self.transaction_publish_page_inner(path, bytes, saved, file_set_changed);
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
    ) -> Option<PageEntry> {
        let before_gen = self.cache_generation();
        let before_name = self
            .with_cached(path, |row| {
                row.map(|(entry, _)| (entry.kind, entry.name.clone()))
            })
            .flatten();
        let mut named = None;
        match bytes {
            Some(bytes) => {
                let content = validate_parse_bytes_for_path(bytes, path).and_then(|()| {
                    std::str::from_utf8(bytes)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
                });
                match content {
                    Err(error) => self.record_unindexable_bytes(path, error),
                    Ok(content) => {
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            if let Some(saved) = saved {
                                #[cfg(test)]
                                if !self.cache_built() {
                                    crate::store::pause_at_hook(&self.cold_cache_reconcile_pause);
                                }
                                if let Some(entry) =
                                    self.cacheable_page_entry_in(path, Some(content))
                                {
                                    named = Some(entry.clone());
                                    self.cache_upsert(entry, saved.clone(), DiskObs::of(content));
                                }
                            } else {
                                self.reconcile_page_content(path, content, false);
                            }
                        }))
                        .is_err()
                        {
                            named = None;
                            self.record_index_failure(path);
                        }
                    }
                }
            }
            None => {
                let _ = self.forget_file_internal(path);
            }
        }
        let after_name = self
            .with_cached(path, |row| {
                row.map(|(entry, _)| (entry.kind, entry.name.clone()))
            })
            .flatten();
        // As for publish_owned, retag only when an installed row proves
        // that this publication kept the old name, including its spelling.
        // A moved generation already retires the memos; they stay for the
        // publication to adopt its patched index over (`adopt_name_index`).
        let renamed = before_name.is_none() || after_name != before_name;
        let after_gen = self.cache_generation();
        if !(file_set_changed || renamed) {
            if after_gen == before_gen || after_gen == before_gen + 1 {
                self.retag_name_memos(before_gen, after_gen);
            }
        } else if after_gen == before_gen {
            self.forget_name_memos();
        }
        self.recent_writes.lock().unwrap().remove(path);
        named
    }
}
