//! The readiness boundary for launch-time SQL answers.
use super::*;
use crate::direct_projection::{derived_reads::DerivedSelection, DirectProjection};
use std::collections::HashMap;

/// Whether this thread is serving a display read, and whether retirement cut
/// it short. See [`Graph::display_read`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayRead {
    Off,
    On,
    Skipped,
}

thread_local! {
    static DISPLAY_READ: std::cell::Cell<DisplayRead> = const { std::cell::Cell::new(DisplayRead::Off) };
}

impl Graph {
    /// The app no longer serves this graph: it was switched away from or
    /// replaced by a refresh. Display reads still running on it stop waiting
    /// and start no whole-graph parse; see [`Graph::display_read`] (GH #543).
    pub fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Run a read whose answer is only displayed. `None` means the graph was
    /// retired and the answer would have needed a whole-graph parse of it;
    /// the caller asks the graph that replaced it instead. Reads that act on
    /// their answer (export, asset listing, creation checks) never go through
    /// here and keep their full answer on a retired graph.
    pub fn display_read<T>(&self, read: impl FnOnce() -> T) -> Option<T> {
        if self.is_retired() {
            return None;
        }
        let previous = DISPLAY_READ.with(|state| state.replace(DisplayRead::On));
        let answer = read();
        let skipped = DISPLAY_READ.with(|state| state.replace(previous)) == DisplayRead::Skipped;
        if skipped && previous != DisplayRead::Off {
            DISPLAY_READ.with(|state| state.set(DisplayRead::Skipped));
        }
        (!skipped).then_some(answer)
    }

    /// False once this thread's display read skipped a parse: its answer is
    /// incomplete and must not be memoized, or a later read of this graph that
    /// acts on its answer (export, creation) would be served the gap.
    pub(super) fn answer_is_complete(&self) -> bool {
        DISPLAY_READ.with(|state| state.get() != DisplayRead::Skipped)
    }

    /// True when a display read on a retired graph must not parse the graph;
    /// records that its answer is incomplete.
    pub(super) fn skip_display_parse(&self) -> bool {
        if !self.is_retired() {
            return false;
        }
        DISPLAY_READ.with(|state| {
            if state.get() == DisplayRead::Off {
                return false;
            }
            state.set(DisplayRead::Skipped);
            true
        })
    }

    #[cfg(test)]
    pub(crate) fn open_page_during_next_derived_read_test(&self, path: Option<PathBuf>) {
        *self.page_build_test.derived_read_open_once.lock().unwrap() = path;
    }

    fn derived_reader(&self) -> Option<(Arc<DirectProjection>, u64)> {
        let projection = self
            .direct_projection
            .lock()
            .unwrap()
            .as_ref()
            .map(Arc::clone)?;
        let generation = self.wait_for_derived_read(&projection, self.cache_generation())?;
        Some((projection, generation))
    }

    /// Run `read` against the index at a ready generation. A generation move
    /// during the read (a page opened or saved meanwhile) retries at the new
    /// generation once the index has applied it; only an index that cannot
    /// answer returns `None`, which sends the caller to the parser. Falling
    /// back on a move would parse the whole graph because a page opened at
    /// launch (GH #543).
    pub(super) fn indexed_read<T>(
        &self,
        mut read: impl FnMut(&Arc<DirectProjection>, u64) -> Option<T>,
    ) -> Option<T> {
        loop {
            let (projection, generation) = self.derived_reader()?;
            let answer = read(&projection, generation);
            #[cfg(test)]
            if let Some(path) = self
                .page_build_test
                .derived_read_open_once
                .lock()
                .unwrap()
                .take()
            {
                let entry = self.entry_for_path(&path).expect("test page exists");
                self.load_page(&entry).expect("test page opens");
            }
            if self.cache_generation() == generation {
                return answer;
            }
        }
    }

    pub(super) fn indexed_derived_pages(
        &self,
        selection: DerivedSelection<'_>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        self.indexed_read(|projection, generation| {
            self.indexed_derived_pages_at(projection, generation, &selection)
        })
    }

    fn indexed_derived_pages_at(
        &self,
        projection: &Arc<DirectProjection>,
        generation: u64,
        selection: &DerivedSelection<'_>,
    ) -> Option<Vec<(PageEntry, Arc<Document>)>> {
        let rows = projection.derived_pages(generation, selection)?;
        let mut pages = rows
            .into_iter()
            .map(|row| {
                if let Some((revision, preorder)) = row.session_ids {
                    if let Some(ids) = SessionPageIds::from_projection(
                        &revision,
                        self.config.parse_config().digest(),
                        preorder,
                    ) {
                        self.session_page_ids
                            .write()
                            .unwrap()
                            .entry(self.root.join(&row.path))
                            .or_insert(ids);
                    }
                }
                let kind = crate::direct_projection::page_kind_from_sql(row.kind)?;
                let date_key = (kind == PageKind::Journal)
                    .then(|| {
                        self.journal_format
                            .parse(&row.name)
                            .map(|date| date.ordinal_key())
                    })
                    .flatten();
                Some((
                    PageEntry {
                        name: row.name,
                        kind,
                        date_key,
                        path: self.root.join(&row.path),
                        rel_path: row.path,
                    },
                    Arc::new(row.document),
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        if let DerivedSelection::Resolve(ids) | DerivedSelection::Preview(ids) = *selection {
            let mut available = HashSet::new();
            for (_, doc) in &pages {
                let mut pending = doc.roots.iter().collect::<Vec<_>>();
                while let Some(block) = pending.pop() {
                    available.insert(block.uuid.clone());
                    if let Some(id) = block.property("id") {
                        available.insert(id);
                    }
                    pending.extend(&block.children);
                }
            }
            let missing = ids
                .iter()
                .filter(|id| !available.contains(*id))
                .cloned()
                .collect::<HashSet<_>>();
            if !missing.is_empty() {
                let cached = self.cache.read().unwrap().clone();
                if let Some(cached) = cached {
                    pages.extend(cached.iter().cloned());
                } else {
                    let config = self.config.parse_config().digest();
                    let sources = self
                        .session_page_ids
                        .read()
                        .unwrap()
                        .iter()
                        .filter(|(_, ids)| ids.config == config && ids.contains(&missing))
                        .filter_map(|(path, ids)| {
                            Some((
                                path.strip_prefix(&self.root).ok()?.to_path_buf(),
                                crate::direct_projection::projection_source_revision(
                                    &ids.revision,
                                    config,
                                ),
                            ))
                        })
                        .collect::<Vec<_>>();
                    // A stale locator is a miss. Never turn an exact-revision
                    // failure into a whole-graph parse after the index answered.
                    for source in sources {
                        if let Some(hydrated) =
                            self.parse_pages_on_demand_with_revisions(generation, vec![source])
                        {
                            pages.extend(hydrated);
                        }
                    }
                }
            }
        }
        Some(pages)
    }

    pub(super) fn indexed_page_icons(&self, names: &[String]) -> Option<HashMap<String, String>> {
        self.indexed_read(|projection, generation| {
            self.indexed_page_icons_at(projection, generation, names)
        })
    }

    fn indexed_page_icons_at(
        &self,
        projection: &Arc<DirectProjection>,
        generation: u64,
        names: &[String],
    ) -> Option<HashMap<String, String>> {
        let aliases = projection.page_aliases_with_owners(generation)?;
        let mut keys = names
            .iter()
            .map(|name| crate::refs::page_key(name))
            .collect::<HashSet<_>>();
        for (alias, canonical, _) in &aliases {
            if keys.contains(&crate::refs::page_key(alias)) {
                keys.insert(crate::refs::page_key(canonical));
            }
        }
        let rows = projection.page_icon_rows(generation, &keys.into_iter().collect::<Vec<_>>())?;
        let mut real = HashSet::new();
        let mut icons = HashMap::new();
        for (key, preamble) in rows {
            real.insert(key.clone());
            // Duplicate keys use the first icon in deterministic path order.
            if let Some(icon) = pre_block_icon(&preamble) {
                icons.entry(key).or_insert(icon);
            }
        }
        for (alias, canonical, _) in aliases {
            let key = crate::refs::page_key(&alias);
            if !real.contains(&key) {
                if let Some(icon) = icons.get(&crate::refs::page_key(&canonical)).cloned() {
                    icons.entry(key).or_insert(icon);
                }
            }
        }
        Some(
            names
                .iter()
                .filter_map(|name| {
                    icons
                        .get(&crate::refs::page_key(name))
                        .map(|icon| (name.clone(), icon.clone()))
                })
                .collect(),
        )
    }

    pub(super) fn indexed_journal_content_days(&self) -> Option<Vec<i64>> {
        self.indexed_read(|projection, generation| {
            let names = projection.journal_content_names(generation)?;
            let days = names
                .iter()
                .filter_map(|name| {
                    self.journal_format
                        .parse(name)
                        .map(|date| date.ordinal_key())
                })
                .collect();
            Some(days)
        })
    }
}
