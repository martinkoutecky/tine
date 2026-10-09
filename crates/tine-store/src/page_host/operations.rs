//! Constrained page operations; rewrites reuse tine-core's pure boundary.
use super::*;
use tine_core::config::FileNameFormat;
use tine_core::model::Format;

impl<F: HostIo> Host<F> {
    fn operation_page(&mut self, key: &str) -> Result<Page, ()> {
        if let Some(page) = self.pages.get(key) {
            Ok(page.clone())
        } else {
            self.fs
                .read_page(key)
                .map(|bytes| Self::initial_page(bytes, 0))
                .map_err(|_| ())
        }
    }

    pub(super) fn delete(&mut self, key: &str) -> Disposition {
        if !self.alive || !self.keys.contains(key) {
            return Disposition::Disabled;
        }
        if self.worker.is_some() || self.busy(key) || self.allocator_busy() {
            return Disposition::Waiting;
        }
        self.with_locks(&BTreeSet::from([key.into()]), |host| {
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
        if !self.alive
            || source == target
            || !self.keys.contains(source)
            || !self.keys.contains(target)
            || !referrers.is_subset(&self.keys)
            || referrers.contains(source)
            || referrers.contains(target)
        {
            return Disposition::Refused;
        }
        if self.worker.is_some() || self.allocator_busy() {
            return Disposition::Waiting;
        }
        let mut refs = referrers.clone();
        for (key, page) in &self.pages {
            if key != source && key != target {
                let Ok(rewritten) = Self::rewrite(&page.buf, key, from, to, format) else {
                    return Disposition::Refused;
                };
                if rewritten != page.buf {
                    refs.insert(key.clone());
                }
            }
        }
        // Read source under its own lock first; the complete key set is then
        // acquired in canonical order and re-read before allocation/install.
        if self.busy(source) {
            return Disposition::Waiting;
        }
        let initial = self.with_locks(&BTreeSet::from([source.into()]), |host| {
            host.operation_page(source)
        });
        let Ok(source_page) = initial else {
            return Disposition::Refused;
        };
        let full = source_page.buf.is_some();
        let mut keys = refs.clone();
        if full {
            keys.extend([source.into(), target.into()]);
        }
        if keys.is_empty() {
            return Disposition::Refused;
        }
        if keys.iter().any(|key| self.busy(key)) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| {
            let Ok(src) = host.operation_page(source) else {
                return Disposition::Refused;
            };
            if src.buf.is_some() != full {
                return Disposition::Refused;
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
                let mut bytes = src.buf.clone();
                // Reuse the title treatment already shared by transaction
                // validation; no local title parser or rewrite twin.
                if let Some(raw) = bytes.as_ref() {
                    let Ok(text) = std::str::from_utf8(raw) else {
                        return Disposition::Refused;
                    };
                    let syntax = Format::from_path(std::path::Path::new(source));
                    if let Some(rebound) =
                        tine_core::model::rebind_page_title(text, syntax, from, to)
                    {
                        bytes = Some(Arc::from(rebound.into_bytes()));
                    }
                }
                pages.get_mut(target).unwrap().buf = bytes;
                pages.get_mut(source).unwrap().buf = None;
            }
            for key in &refs {
                let page = pages.get_mut(key).unwrap();
                let Ok(bytes) = Self::rewrite(&page.buf, key, from, to, format) else {
                    return Disposition::Refused;
                };
                page.buf = bytes;
            }
            // Quint L505 reserves PAGES.size(), with each key's rank offset.
            let versions: BTreeMap<_, _> = host
                .keys
                .iter()
                .enumerate()
                .map(|(rank, key)| {
                    (
                        key.clone(),
                        host.version
                            .checked_add(rank as u64 + 1)
                            .expect("host version exhausted"),
                    )
                })
                .collect();
            let last_version = *versions.values().max().unwrap();
            let mut records = vec![];
            for (key, page) in &mut pages {
                page.version = versions[key];
                page.typed = true;
                page.risk = true;
                records.push(host.record(key, page));
            }
            host.install_operation(pages, records, None, last_version, reads);
            Disposition::Pending
        })
    }
}
