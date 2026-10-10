//! Constrained page operations; rewrites reuse tine-core's pure boundary.
use super::*;
use tine_core::config::FileNameFormat;
use tine_core::model::Format;

impl<F: HostIo> Host<F> {
    fn operation_page(&mut self, key: &str) -> Result<Page, ()> {
        if let Some(page) = self.pages.get(key) {
            Ok(page.clone())
        } else {
            // s3.1: the read is a visible load before operation capture.
            if self.load_locked(key) == Disposition::Applied {
                Ok(self.pages[key].clone())
            } else {
                Err(())
            }
        }
    }

    pub(super) fn delete(&mut self, key: &str) -> Disposition {
        self.delete_with_load(key, true)
    }

    #[cfg(test)]
    pub(super) fn delete_loaded(&mut self, key: &str) -> Disposition {
        self.delete_with_load(key, false)
    }

    fn delete_with_load(&mut self, key: &str, load: bool) -> Disposition {
        if !self.alive || !self.keys.contains(key) {
            return Disposition::Disabled;
        }
        if self.worker.is_some() || self.busy(key) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        let keys = BTreeSet::from([key.into()]);
        // Plan before any side effect (STEP3 §1): the caller takes the lock.
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            if !load && !host.pages.contains_key(key) {
                return Disposition::Refused;
            }
            let Ok(mut page) = host.operation_page(key) else {
                return Disposition::Refused;
            };
            if !page.clean() || page.buf.is_none() {
                return Disposition::Refused;
            }
            let reads = BTreeMap::from([(key.into(), page.base.clone())]);
            page.buf = None;
            page.typed = true;
            page.risk = true;
            page.version = host.next_version();
            let record = host.record(key, &page);
            let version = page.version;
            host.install_operation(
                BTreeMap::from([(key.into(), page)]),
                vec![record],
                None,
                version,
                reads,
            );
            Disposition::Pending
        })
    }

    fn rewrite(
        bytes: &Text,
        key: &str,
        from: &str,
        to: &str,
        format: FileNameFormat,
    ) -> Result<Text, ()> {
        let Some(bytes) = bytes else { return Ok(None) };
        let text = std::str::from_utf8(bytes).map_err(|_| ())?;
        Ok(Some(Arc::from(
            tine_core::refs::rename_rewrite(
                text,
                key.ends_with(".org"),
                &[(from.into(), to.into())],
                format,
            )
            .into_bytes(),
        )))
    }

    /// Referrer discovery's index portion belongs to the caller; held buffers
    /// are checked here so an unindexed held reference is never missed.
    pub(super) fn rename(
        &mut self,
        source: &str,
        target: &str,
        referrers: &BTreeSet<PageKey>,
        from: &str,
        to: &str,
        format: FileNameFormat,
    ) -> Disposition {
        self.rename_with(source, target, referrers, |bytes, key, moving| {
            if !moving {
                return Self::rewrite(bytes, key, from, to, format);
            }
            let Some(raw) = bytes else { return Ok(None) };
            let text = std::str::from_utf8(raw).map_err(|_| ())?;
            let syntax = Format::from_path(std::path::Path::new(source));
            Ok(tine_core::model::rebind_page_title(text, syntax, from, to)
                .map(|text| Some(Arc::from(text.into_bytes())))
                .unwrap_or_else(|| bytes.clone()))
        })
    }

    // The model deliberately abstracts the pure rewrite. Tests substitute
    // opaque labels here; all locks, guards, allocation and I/O remain shared.
    pub(super) fn rename_with(
        &mut self,
        source: &str,
        target: &str,
        referrers: &BTreeSet<PageKey>,
        rewrite: impl Fn(&Text, &str, bool) -> Result<Text, ()>,
    ) -> Disposition {
        if !self.alive
            || source == target
            || !self.keys.contains(source)
            || !self.keys.contains(target)
            || !self.keys.includes(referrers)
            || referrers.contains(source)
            || referrers.contains(target)
        {
            return Disposition::Refused;
        }
        if self.worker.is_some() || self.allocator_busy() {
            return Disposition::Waiting;
        }
        let Ok(refs) = self.rename_refs(source, target, referrers, &rewrite) else {
            return Disposition::Refused;
        };
        // A known dirty referrer already disproves the operation guard;
        // refuse before loading any other path.
        if refs
            .iter()
            .any(|key| self.pages.get(key).is_some_and(|p| !p.clean()))
        {
            return Disposition::Refused;
        }
        // Acquire the possible operation paths before the first load. An
        // absent source may make source/target unchanged; no path read for the
        // unused target is required in that branch.
        if self.retained.contains(source) {
            return Disposition::Waiting;
        }
        let mut locks = refs.clone();
        locks.extend([source.into(), target.into()]);
        if self.lacks_locks(&locks) {
            return Disposition::Waiting;
        }
        self.with_locks(&locks, |host| {
            let Ok(src) = host.operation_page(source) else {
                return Disposition::Refused;
            };
            let full = src.buf.is_some();
            let mut keys = refs.clone();
            if full {
                keys.extend([source.into(), target.into()]);
            }
            if keys.is_empty() {
                return Disposition::Refused;
            }
            if keys.iter().any(|key| host.busy(key)) {
                return Disposition::Waiting;
            }
            let mut pages = BTreeMap::new();
            let mut reads = BTreeMap::new();
            for key in &keys {
                let Ok(page) = host.operation_page(key) else {
                    return Disposition::Refused;
                };
                if !page.clean() {
                    return Disposition::Refused;
                }
                reads.insert(key.clone(), page.base.clone());
                pages.insert(key.clone(), page);
            }
            if full && pages[target].buf.is_some() {
                return Disposition::Refused;
            }
            if full {
                let Ok(bytes) = rewrite(&src.buf, source, true) else {
                    return Disposition::Refused;
                };
                pages.get_mut(target).unwrap().buf = bytes;
                pages.get_mut(source).unwrap().buf = None;
            }
            for key in &refs {
                let page = pages.get_mut(key).unwrap();
                let Ok(bytes) = rewrite(&page.buf, key, false) else {
                    return Disposition::Refused;
                };
                page.buf = bytes;
            }
            // Quint L505 reserves PAGES.size() versions, each key at its rank
            // offset. The host allocates densely over the operation's pages in
            // key order: the conformance map compares order, not numbering
            // (STEP3-REVIEW-1 F8), and a rename visits no untouched registered
            // key (A-R5, D-10).
            let mut last_version = host.version;
            let mut records = vec![];
            for (key, page) in &mut pages {
                last_version = last_version.checked_add(1).expect("host version exhausted");
                page.version = last_version;
                page.typed = true;
                page.risk = true;
                records.push(host.record(key, page));
            }
            host.install_operation(pages, records, None, last_version, reads);
            Disposition::Pending
        })
    }
}
