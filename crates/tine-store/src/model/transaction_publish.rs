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
    pub(crate) fn transaction_publish_page_inner(
        &self,
        path: &Path,
        bytes: Option<&[u8]>,
        saved: Option<&Document>,
        file_set_changed: bool,
        own_rename: bool,
    ) {
        let before_gen = self.cache_generation();
        match bytes {
            Some(bytes) => {
                if validate_parse_bytes_for_path(bytes, path).is_err() {
                    self.invalidate_cache();
                    return;
                }
                let Ok(content) = std::str::from_utf8(bytes) else {
                    self.invalidate_cache();
                    return;
                };
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(saved) = saved {
                        #[cfg(test)]
                        if self.cache.read().unwrap().is_none() {
                            crate::store::pause_at_hook(&self.cold_cache_reconcile_pause);
                        }
                        if let Some(entry) = self.cacheable_page_entry(path) {
                            self.cache_upsert(entry, saved.clone(), DiskObs::of(content));
                        }
                    } else if own_rename {
                        // Publication already observed changed bytes and matched them
                        // to this guarded rewrite. Comparing with the old normalized
                        // document cannot change that observation, and re-parses it.
                        if let Some(entry) = self.cacheable_page_entry(path) {
                            let (doc, disk) = parse_page_content(&entry, content);
                            self.cache_upsert(entry, doc, disk);
                        }
                    } else {
                        self.reconcile_page_content(path, content, false);
                    }
                }))
                .is_err()
                {
                    self.invalidate_cache();
                    self.page_index_failures
                        .write()
                        .unwrap()
                        .push(self.rel_path(path));
                }
            }
            None => {
                let _ = self.forget_file_internal(path);
            }
        }
        if file_set_changed {
            *self.page_list_cache.write().unwrap() = None;
            *self.find_entry_cache.write().unwrap() = None;
        } else {
            let after_gen = self.cache_generation();
            if after_gen == before_gen || after_gen == before_gen + 1 {
                if let Some((gen, _)) = self.page_list_cache.write().unwrap().as_mut() {
                    if *gen == before_gen {
                        *gen = after_gen;
                    }
                }
                if let Some((gen, _)) = self.find_entry_cache.write().unwrap().as_mut() {
                    if *gen == before_gen {
                        *gen = after_gen;
                    }
                }
            }
        }
        self.recent_writes.lock().unwrap().remove(path);
    }
}
