//! Graph's page inventory: listing pages, the published page-inventory
//! snapshot, and the referenced-page-names set.

use super::*;

impl Graph {
    /// List all pages and journals in the graph.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((g, entries)) = self.page_list_cache.read().unwrap().as_ref() {
            if *g == gen {
                return entries.clone();
            }
        }
        // R6: a ready projection already holds the effective inventory; the
        // whole-graph parse below is the not-ready fallback.
        if let Some((gen, entries)) = self.direct_projection_page_inventory() {
            *self.page_list_cache.write().unwrap() = Some((gen, entries.clone()));
            return entries;
        }
        // Cold inventory used to run its own whole-graph parse, independently
        // of the page-build flight used by templates, queries and background
        // warm-up. On first open those passes competed for every core and for
        // storage, starving the three-page journal feed (GH #550). Join the
        // existing generation-scoped flight instead.
        //
        // Except after a known parse failure. `publish_warm_page_inventory`
        // deliberately leaves the memo absent then, so that the next listing
        // revalidates the failed path from disk; the flight answers from the
        // parsed cache, which still holds the page as it was before it became
        // unreadable, so joining it here would republish exactly the stale
        // entry that mechanism exists to drop. Cold open has no failures, so
        // it keeps the shared flight.
        let entries = if self.page_index_failures.read().unwrap().is_empty() {
            self.with_pages(|pages| {
                pages
                    .iter()
                    .map(|(entry, _)| entry.clone())
                    .collect::<Vec<_>>()
            })
        } else if let Some(entries) = self.cached_inventory_with_failures_revalidated(gen) {
            entries
        } else {
            match self.exact_page_inventory_from_disk() {
                Some(entries) => entries,
                None => return Vec::new(),
            }
        };
        if self.answer_is_complete() {
            *self.page_list_cache.write().unwrap() = Some((gen, entries.clone()));
        }
        entries
    }

    /// The listing after a known parse failure, from the parsed cache: its
    /// healthy entries, minus every failed path, whose bytes are re-read to
    /// confirm they still fail. The cache is current for every page the
    /// watcher reconciled; only a failed path can hold a stale entry, and
    /// that is exactly what the listing must drop.
    ///
    /// `None` -- take the exact whole-graph listing -- when there is no parsed
    /// cache, a failure is not a readable-failing file (it now reads, is gone,
    /// or names the scope), or the generation moved. Re-reading every page
    /// on each watcher delivery of one still-unreadable file was a
    /// whole-graph parse per event at 10k pages (GH #543, indexing audit
    /// IT-07).
    fn cached_inventory_with_failures_revalidated(
        &self,
        generation: u64,
    ) -> Option<Vec<PageEntry>> {
        let pages = self.cache.read().unwrap().clone()?;
        let failures = self.page_index_failures.read().unwrap().clone();
        let permit = self.admit_retained_graph_text_writer().ok()?;
        for failure in &failures {
            // A failure is a graph-relative path only when it names a file;
            // scope and skip reasons ("<path>: <why>") do not.
            let path = self.root.join(failure);
            if !std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
                return None;
            }
            if self
                .graph_text_read_optional_text_with_identity(&permit, &path)
                .is_ok()
            {
                return None;
            }
        }
        let failed = failures.iter().map(String::as_str).collect::<HashSet<_>>();
        let entries = pages
            .iter()
            .filter(|(entry, _)| !failed.contains(entry.rel_path.as_str()))
            .map(|(entry, _)| entry.clone())
            .collect();
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation).then_some(entries)
    }

    /// Read and parse every graph-text file, exactly as it is on disk right
    /// now, and republish the parse failures found on the way.
    ///
    /// This is the expensive path. It exists for one case: a file whose parse
    /// failed under the watcher must be revalidated from its bytes rather than
    /// answered from a cache that predates the failure. `None` means the graph
    /// text scope itself could not be read; the caller then lists nothing, and
    /// the reason is left in `page_index_failures`.
    fn exact_page_inventory_from_disk(&self) -> Option<Vec<PageEntry>> {
        if self.skip_display_parse() {
            return None;
        }
        let built = self.admit_retained_graph_text_writer().and_then(|permit| {
            let (entries, skipped) = self.graph_text_entries_and_skipped(&permit)?;
            let limits = graph_text_inventory_limits();
            let mut raw_bytes = 0_u64;
            let mut effective = Vec::with_capacity(entries.len());
            let mut failures = skipped;
            for entry in entries {
                let loaded = self.graph_text_read_optional_text_with_identity(&permit, &entry.path);
                let parsed = match loaded {
                    Ok(Some((content, _))) => {
                        raw_bytes = raw_bytes
                            .checked_add(usize_to_u64(content.len())?)
                            .ok_or_else(|| {
                                graph_text_inventory_limit_error("aggregate text bytes")
                            })?;
                        if raw_bytes > limits.retained_content_bytes {
                            return Err(graph_text_inventory_limit_error("aggregate text bytes"));
                        }
                        parse_exact_page(self, &entry, &content)
                    }
                    Ok(None) => {
                        failures.push(format!(
                            "{}: disappeared during graph text listing",
                            entry.rel_path
                        ));
                        continue;
                    }
                    Err(error) => Err(error),
                };
                match parsed {
                    Ok((entry, _, _)) => effective.push(entry),
                    Err(_) => failures.push(entry.rel_path),
                }
            }
            *self.page_index_failures.write().unwrap() = failures;
            Ok(effective)
        });
        match built {
            Ok(entries) => Some(entries),
            Err(error) => {
                *self.page_index_failures.write().unwrap() =
                    vec![format!("graph-text-scope: {error}")];
                None
            }
        }
    }

    /// Publish the exact physical/effective page inventory already represented by
    /// the warm parsed cache. This is used only after a successful scoped cache
    /// mutation; watcher parse failures deliberately leave the memo absent so a
    /// later listing revalidates the failed path from disk.
    pub(super) fn publish_warm_page_inventory(&self, generation: u64) {
        let entries = {
            let guard = self.cache.read().unwrap();
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) != generation {
                return;
            }
            let Some(pages) = guard.as_ref() else {
                return;
            };
            pages.iter().map(|(entry, _)| entry.clone()).collect()
        };
        if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation {
            *self.page_list_cache.write().unwrap() = Some((generation, entries));
        }
    }

    /// Capture a current list memo before a transaction that must discard the
    /// parsed cache. The transaction may update this in memory from bytes it
    /// already owns, avoiding a second whole-graph read/parse after commit.
    pub(super) fn current_page_inventory_snapshot(&self) -> Option<(Vec<PageEntry>, Vec<String>)> {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        let entries = self
            .page_list_cache
            .read()
            .unwrap()
            .as_ref()
            .filter(|(memo_generation, _)| *memo_generation == generation)
            .map(|(_, entries)| entries.clone())?;
        let failures = self.page_index_failures.read().unwrap().clone();
        (self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation)
            .then_some((entries, failures))
    }

    pub(super) fn publish_page_inventory_snapshot(
        &self,
        mut entries: Vec<PageEntry>,
        mut failures: Vec<String>,
    ) {
        entries.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
        failures.sort();
        failures.dedup();
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        *self.page_index_failures.write().unwrap() = failures;
        *self.page_list_cache.write().unwrap() = Some((generation, entries));
    }

    /// Page names referenced anywhere in the graph — inline `[[link]]`/`#tag`/
    /// `#[[..]]` plus `tags::`/`alias::` property values (block- and page-level) —
    /// display case preserved, deduped case-insensitively. These are the pages
    /// that "exist" by reference even without a file of their own (OG semantics),
    /// so autocomplete/quick-switch can offer them instead of "Create …".
    ///
    /// Read from the disposable SQLite fact projection at the exact current
    /// cache generation. If that projection is unavailable or stale, the
    /// already-parsed whole-graph cache is the correctness fallback; this never
    /// forces a full-graph parse (it runs on autocomplete keystrokes).
    pub fn referenced_page_names(&self) -> Vec<String> {
        self.referenced_page_names_versioned(None)
            .names
            .unwrap_or_default()
    }

    /// The same inventory, plus an order-independent digest of it, so a caller
    /// that already holds an identical set can skip transporting it.
    ///
    /// The frontend asks for this after every typing lull, because its resource
    /// is keyed on `dataRev` and a save bumps that. The answer almost never
    /// changes — typing inside a block rarely adds or removes a `[[link]]` — but
    /// several thousand strings were serialised, sent over IPC and re-parsed on
    /// the UI thread every time. Passing back the digest the caller already has
    /// turns the unchanged case into an integer. (Direct Files performance audit
    /// 2026-08-09, finding F7; 15–23 ms and ~5,000 names at 5,225 files.)
    ///
    /// The digest is a commutative fold, deliberately: the memo's own order comes
    /// from a `HashMap`, so a sequence-dependent hash would report spurious
    /// changes. It carries the count as well, so adding a name that collides with
    /// a removed one still shows up unless the count also matches.
    pub fn referenced_page_names_versioned(&self, known: Option<u64>) -> ReferencedPageNames {
        let generation = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
        if let Some((cached, digest, names)) = self.referenced_names_cache.read().unwrap().as_ref()
        {
            if *cached == generation {
                return ReferencedPageNames::answer(*digest, names, known);
            }
        }
        if let Some(names) = self.direct_projection_referenced_page_names() {
            let digest = referenced_names_digest(&names);
            let answer = ReferencedPageNames::answer(digest, &names, known);
            // Only a read that still matches the generation we keyed on may be
            // memoized. The projection revalidates internally, but the
            // generation can move between our load and its answer, and a set
            // recorded under a stale key would outlive the edit that
            // invalidated it — an autocomplete offering pages that no longer
            // exist, or missing one just linked.
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == generation {
                *self.referenced_names_cache.write().unwrap() = Some((generation, digest, names));
            }
            return answer;
        }
        // The parser fallback is deliberately NOT memoized: it answers with an
        // empty set when the page cache is not yet warm (it must not force a
        // parse here), and caching that would make "this graph references no
        // pages" stick for the whole generation — autocomplete would silently
        // offer nothing until the next edit happened to bump it.
        let names = self.rebuild_referenced_page_names();
        let digest = referenced_names_digest(&names);
        ReferencedPageNames::answer(digest, &names, known)
    }

    fn rebuild_referenced_page_names(&self) -> Vec<String> {
        let guard = self.cache.read().unwrap();
        let Some(pages) = guard.as_ref() else {
            return Vec::new(); // cache not warm — don't force a parse
        };
        referenced_page_names_from_snapshot(pages)
    }
}

pub(super) fn referenced_page_names_from_snapshot(
    pages: &[(PageEntry, Arc<Document>)],
) -> Vec<String> {
    crate::query::referenced_page_names_from_snapshot_cancellable(pages, &|| false)
        .unwrap_or_default()
}
