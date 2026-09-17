//! Graph's lookup and read surface: find an entry by name, page names, aliases
//! and icons, reference candidates, block-ref counts, and loading a page or
//! document.

use super::*;

impl Graph {
    /// Find a page/journal entry by display name. When several files share the
    /// name — a duplicate journal day, a canonical `2026_06_26.org` plus a
    /// title-named stray `Friday, 26-06-2026.org` (#21) — prefer the canonical
    /// date-stem file, so opening the day by name (a `[[link]]`, quick-switch, or
    /// `get_page`) is deterministic AND lands on the same file a save resolves to
    /// (`path_for`), instead of whichever the directory listing happened to yield
    /// first (which could mismatch the save target and raise a phantom conflict).
    /// The stray is reached by path via `load_by_path`.
    pub fn find_entry(&self, name: &str, kind: PageKind) -> Option<PageEntry> {
        let key = (kind, crate::refs::page_key(name));
        loop {
            let gen = self.cache_gen.load(std::sync::atomic::Ordering::Acquire);
            if let Some((g, index)) = self.find_entry_cache.read().unwrap().as_ref() {
                if *g == gen && index.has_kind(kind) {
                    return index.entries.get(&key).cloned();
                }
            }

            let mut built = FindEntryIndex::new();
            for entry in self.list_pages() {
                let entry_key = (entry.kind, crate::refs::page_key(&entry.name));
                match built.entries.get_mut(&entry_key) {
                    Some(winner) => {
                        if !is_date_stem_entry(winner) && is_date_stem_entry(&entry) {
                            *winner = entry;
                        }
                    }
                    None => {
                        built.entries.insert(entry_key, entry);
                    }
                }
            }
            built.mark_kind_loaded(PageKind::Page);
            built.mark_kind_loaded(PageKind::Journal);

            let found = {
                let mut guard = self.find_entry_cache.write().unwrap();
                match guard.as_mut() {
                    Some((g, index)) if *g == gen => {
                        if !index.has_kind(kind) {
                            index.entries.extend(built.entries);
                            index.mark_kind_loaded(kind);
                        }
                        index.entries.get(&key).cloned()
                    }
                    _ => {
                        let found = built.entries.get(&key).cloned();
                        *guard = Some((gen, built));
                        found
                    }
                }
            };
            if self.cache_gen.load(std::sync::atomic::Ordering::Acquire) == gen {
                return found;
            }
        }
    }

    /// Load a page by name; returns `None` if it doesn't exist on disk. Falls
    /// back to alias resolution (`alias::`) for named pages.
    pub fn load_named(&self, name: &str, kind: PageKind) -> io::Result<Option<PageDto>> {
        // A file that vanished between listing and load (external delete) reports
        // NotFound from load_page — map it to "no page" rather than an error, so
        // the page is treated as absent (never resurrected) and the get_page
        // contract (Ok(None) = doesn't exist) holds.
        let load = |entry: &PageEntry| -> io::Result<Option<PageDto>> {
            match self.load_page(entry) {
                Ok(dto) => Ok(Some(dto)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        };
        if let Some(entry) = self.find_entry(name, kind) {
            return load(&entry);
        }
        if kind == PageKind::Page {
            let tnorm = crate::refs::page_key(name);
            if let Some((_, canon)) = self
                .page_aliases()
                .into_iter()
                .find(|(alias, _)| crate::refs::page_key(alias) == tnorm)
            {
                if let Some(entry) = self.find_entry(&canon, kind) {
                    return load(&entry);
                }
            }
        }
        Ok(None)
    }

    /// The `icon::` property value of each named page that has one (for rendering
    /// page icons next to titles / in the namespace tree, like OG). Scans the
    /// cached pages once, then answers the requested names; only pages WITH an
    /// icon appear in the result. On-demand (e.g. a `{{namespace}}` macro), not at
    /// index time.
    pub fn page_icons(&self, names: &[String]) -> std::collections::HashMap<String, String> {
        let (mut icons_by_name, real_page_names) = self.with_pages(|pages| {
            let mut icons = std::collections::HashMap::new();
            let mut real = std::collections::HashSet::new();
            for (entry, doc) in pages {
                if entry.kind != PageKind::Page {
                    continue;
                }
                let key = crate::refs::page_key(&entry.name);
                real.insert(key.clone());
                if let Some(icon) = doc.pre_block.as_deref().and_then(pre_block_icon) {
                    icons.entry(key).or_insert(icon);
                }
            }
            (icons, real)
        });
        for (alias, canon) in self.page_aliases() {
            if real_page_names.contains(&alias) {
                continue; // `load_named` prefers a real page over an alias fallback.
            }
            if let Some(icon) = icons_by_name.get(&crate::refs::page_key(&canon)).cloned() {
                icons_by_name.entry(alias).or_insert(icon);
            }
        }
        let mut out = std::collections::HashMap::new();
        for name in names {
            if let Some(icon) = icons_by_name.get(&crate::refs::page_key(name)) {
                out.insert(name.clone(), icon.clone());
            }
        }
        out
    }

    /// The subset of `names` that already name a document in this graph — a real
    /// page, a journal, or an alias of one. The UI dims the rest, so a link that
    /// will open a blank page is visible as such before it is clicked.
    ///
    /// A DELIBERATE divergence from Logseq, which renders a link to a
    /// non-existent page exactly like a live one (`components/block.cljs:548`
    /// dims only `untitled-page?`, meaning a page that exists under a uuid name).
    /// Approved by Martin 2026-08-09 after a `title::`/filename mismatch silently
    /// dead-ended 29 links with no visible signal. Obsidian's "unresolved link"
    /// styling is the same affordance.
    ///
    /// Resolution deliberately mirrors `load_named`'s: normalized page keys plus
    /// the alias table, and no `PageKind` filter — a link to an existing journal
    /// is alive. Costs one page-list read (memoized) per call, so callers must
    /// batch; the UI coalesces every reference in a tick into one request.
    pub fn existing_page_names(&self, names: &[String]) -> Vec<String> {
        let mut known: std::collections::HashSet<String> = self
            .list_pages()
            .iter()
            .map(|entry| crate::refs::page_key(&entry.name))
            .collect();
        for (alias, _canonical) in self.page_aliases() {
            known.insert(alias);
        }
        names
            .iter()
            .filter(|name| known.contains(&crate::refs::page_key(name)))
            .cloned()
            .collect()
    }

    /// Alias → canonical-page-name pairs (for the UI to resolve links/navigation).
    pub fn page_aliases(&self) -> Vec<(String, String)> {
        self.page_aliases_with_owners()
            .into_iter()
            .map(|(alias, canonical, _)| (alias, canonical))
            .collect()
    }

    pub(crate) fn page_aliases_with_owners(&self) -> Vec<(String, String, String)> {
        if let Some(aliases) = self.direct_projection_page_aliases_with_owners() {
            return aliases;
        }
        if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
            eprintln!("DIAG543 ALIASES parser_fallback");
        }
        crate::query::page_aliases_with_owners(self)
    }

    /// The page that owns a block UUID / UUID-valued `id::`, selected from the
    /// exact-generation disposable SQLite projection. A hint only: callers
    /// still verify parser evidence, and unsupported/stale projection state
    /// falls back to an exact parser scan.
    pub fn block_page_hint(&self, uuid: &str) -> Option<String> {
        if let Some(hint) = self.direct_projection_block_page_hint(uuid) {
            return hint;
        }
        fn walk_idx(
            blocks: &[DocBlock],
            name: &str,
            m: &mut std::collections::HashMap<String, String>,
        ) {
            for b in blocks {
                if !b.uuid.is_empty() {
                    m.entry(b.uuid.clone()).or_insert_with(|| name.to_string());
                }
                if let Some(id) = b.property("id") {
                    if !id.is_empty() {
                        m.entry(id).or_insert_with(|| name.to_string());
                    }
                }
                walk_idx(&b.children, name, m);
            }
        }
        let map = self.with_pages(|pages| {
            let mut m = std::collections::HashMap::new();
            for (entry, doc) in pages {
                walk_idx(&doc.roots, &entry.name, &mut m);
            }
            m
        });
        map.get(uuid).cloned()
    }

    /// Resolve a bounded set of physical cached pages that could contain one of
    /// `names_norm`. An unusable/stale/incomplete index returns the complete
    /// snapshot, so callers preserve exact full-scan correctness.
    ///
    /// This is the WALK policy: it always answers. Use it where no readiness
    /// retry exists to wait on — print/publish, diagnostics, tests. An
    /// interactive surface uses [`Graph::reference_candidate_pages_indexed`]
    /// instead, so a projection that is merely mid-turn does not cost a
    /// whole-graph parse.
    pub(crate) fn reference_candidate_pages(
        &self,
        names_norm: &[String],
        kind: ReferenceKind,
    ) -> ReferenceCandidatePages {
        if let Some((pages, blocks)) =
            self.direct_projection_reference_candidate_pages(names_norm, kind)
        {
            // R6: the inventory is the projection's (memoized), never a reason
            // to build the whole parsed graph.
            let full_page_count = self.list_pages().len();
            return ReferenceCandidatePages {
                pages,
                blocks,
                indexed: true,
                full_page_count,
            };
        }
        let pages = self.with_pages(|pages| pages.iter().cloned().collect::<Vec<_>>());
        ReferenceCandidatePages {
            full_page_count: pages.len(),
            pages,
            blocks: None,
            indexed: false,
        }
    }

    /// The same resolution for a surface that CAN wait, which is the whole
    /// difference: a reference panel has the readiness retry loop a query block
    /// has, so a projection that is indexing, recovering, applying pending
    /// edits or busy is reported rather than answered by parsing every page in
    /// the graph. That fallback is not free — during the cold-open window it is
    /// the entire graph, once per reference panel — and it is not more correct,
    /// because the same read a moment later is served from the index.
    ///
    /// It refuses ONLY where waiting has an end, because the caller's retry is
    /// unbounded and a refusal nothing will ever clear is a hang. `Working` is
    /// the one state a queued worker turn resolves, and it is the state the
    /// cold-open window actually produces. Everything else keeps the walk:
    ///
    /// * `Ready` while the narrowing read still declined — a save landed between
    ///   the two reads, or the read itself failed. The save case heals on the
    ///   caller's next request anyway, and the failure case would otherwise
    ///   refuse forever with no repair behind it, so one walk is both the
    ///   cheaper and the terminating answer. Repair belongs to the query
    ///   dispatcher, which can bound it; a panel read cannot.
    /// * `Stale`, `Stopped`, no projection at all, and a target the index cannot
    ///   narrow (`reference_narrowing_supported`) — nothing is coming for any of
    ///   these, and refusing would remove the only route to an answer rather
    ///   than delay it.
    pub(crate) fn reference_candidate_pages_indexed(
        &self,
        names_norm: &[String],
        kind: ReferenceKind,
    ) -> Result<ReferenceCandidatePages, crate::query::QueryExecutionError> {
        use crate::direct_projection::ProjectionProgress;
        if let Some((pages, blocks)) =
            self.direct_projection_reference_candidate_pages(names_norm, kind)
        {
            let full_page_count = self.list_pages().len();
            return Ok(ReferenceCandidatePages {
                pages,
                blocks,
                indexed: true,
                full_page_count,
            });
        }
        if crate::direct_projection::reference_narrowing_supported(names_norm, kind) {
            if let Some(ProjectionProgress::Working(reason)) = self.direct_projection_progress() {
                return Err(crate::query::QueryExecutionError::NotReady(reason));
            }
        }
        Ok(self.reference_candidate_pages(names_norm, kind))
    }

    pub(crate) fn reference_real_page_names(&self) -> Option<crate::query::RealPageNames> {
        let names = self.direct_projection_real_page_names()?;
        Some(names)
    }

    /// `block uuid → # of distinct referrer blocks`, over the whole graph. A referrer is a
    /// block whose text references the uuid (`((uuid))`, `[..](((uuid)))`, or
    /// `{{embed ((uuid))}}`); multiple refs from one block count once (OG semantics).
    /// The exact-generation SQLite projection is the ready route; its parser
    /// fallback is used only while that disposable projection is unavailable.
    pub fn block_ref_counts(&self) -> io::Result<Arc<std::collections::HashMap<String, usize>>> {
        if let Some(counts) = self.direct_projection_block_ref_counts() {
            return Ok(Arc::new(counts));
        }
        if std::env::var_os("TINE_DIAGNOSE_543").is_some() {
            eprintln!("DIAG543 REF_COUNTS parser_fallback");
        }
        let map = self.with_pages(|pages| -> io::Result<_> {
            let mut counts = std::collections::HashMap::new();
            for (_entry, doc) in pages {
                for (id, count) in document_block_ref_counts(doc)? {
                    let total = counts.entry(id).or_insert(0_usize);
                    *total = total.checked_add(count).ok_or_else(allocation_overflow)?;
                }
            }
            Ok(counts)
        })?;
        Ok(Arc::new(map))
    }

    /// Locate a page in the parsed-doc cache by its resolved physical path.
    /// Callers must already hold either `cache.read()` or `cache.write()`; this
    /// function only touches the companion index, preserving the lock order
    /// cache -> cache_index.
    pub(super) fn cached_page_index_for_path(
        &self,
        pages: &[(PageEntry, Arc<Document>)],
        path: &Path,
    ) -> Option<usize> {
        {
            let guard = self.cache_index.read().unwrap();
            if let Some(index) = guard.as_ref() {
                if let Some(&i) = index.by_path.get(path) {
                    if pages.get(i).is_some_and(|(e, _)| e.path == path) {
                        return Some(i);
                    }
                    // A mismatched slot means a previous mutation dropped/shifted
                    // entries without rebuilding. Rebuild below rather than
                    // serving whatever the stale slot now points at.
                } else {
                    return None;
                }
            }
        }

        let mut guard = self.cache_index.write().unwrap();
        let rebuild = match guard.as_ref() {
            Some(index) => index
                .by_path
                .get(path)
                .is_some_and(|&i| !pages.get(i).is_some_and(|(e, _)| e.path == path)),
            None => true,
        };
        if rebuild {
            #[cfg(test)]
            count_cache_linear_scan(pages.len());
            *guard = Some(build_page_cache_index(pages));
        }
        guard
            .as_ref()
            .and_then(|index| index.by_path.get(path).copied())
            .filter(|&i| pages.get(i).is_some_and(|(e, _)| e.path == path))
    }

    /// A page DTO from the cache ONLY if the cache is already built — never
    /// triggers a (synchronous, whole-graph) build. `None` on a cold cache or a
    /// page not yet cached, so latency-path callers can parse just one file.
    fn peek_cached_page(&self, entry: &PageEntry) -> Option<PageDto> {
        let guard = self.cache.read().unwrap();
        let pages = guard.as_ref()?;
        let i = self.cached_page_index_for_path(pages, &entry.path)?;
        pages.get(i).and_then(|(e, d)| page_dto_checked(e, d).ok())
    }

    /// Load a page by entry. Served from the in-memory cache so block uuids are
    /// stable and consistent with queries / refs / the sidebar. Falls back to a
    /// disk parse for a page not yet in the cache (e.g. just created externally).
    pub fn load_page(&self, entry: &PageEntry) -> io::Result<PageDto> {
        // A read is NOT an activation, and must not revoke one. Re-hydrating a
        // page that is already open — which this path does constantly — would
        // otherwise disarm a live banner the user can still see. The genuine
        // disk-move boundaries revoke explicitly elsewhere
        // (`observe_graph_text_external_paths`, `sync_file_checked`,
        // `forget_file`), so nothing is lost here. (GH #254 increment 3.)
        let permit = self.admit_retained_graph_text_writer()?;
        let Some(ExactGraphLoadedPage {
            entry: effective,
            document,
            content,
            revision,
            file_identity,
        }) = self.load_validated_graph_text_target(&permit, &entry.path)?
        else {
            self.forget_file(&entry.path);
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        self.sync_file_content(None, &entry.path, &content, false)?;
        self.loaded_file_identities
            .write()
            .unwrap()
            .insert(entry.path.clone(), (revision.clone(), file_identity));

        if let Some(mut dto) = self.peek_cached_page(&effective) {
            dto.read_only = read_only_org(&entry.path, &content);
            dto.rev = Some(revision);
            dto.path = self.rel_path(&entry.path);
            return Ok(dto);
        }
        let mut dto = page_dto_checked(&effective, &document)?;
        dto.read_only = read_only_org(&entry.path, &content);
        dto.rev = Some(revision);
        dto.path = self.rel_path(&entry.path);
        Ok(dto)
    }

    /// Load a page from a SPECIFIC file by its graph-root-relative path, parsing it
    /// directly and bypassing the `(kind,name)` page cache + `disk_revs`. This is
    /// how a duplicate-day stray (`journals/Friday, 26-06-2026.org`) — which shares
    /// a `(kind,name)` with the canonical `2026_06_26.org` and so is unreachable by
    /// name — gets opened and edited (#21). The direct parse is deliberate: the
    /// cache slot for that `(kind,name)` holds the CANONICAL file, so a cache lookup
    /// here would serve the wrong file's content. Returns `Ok(None)` if the path is
    /// invalid (see [`resolve_rel`]) or the file is gone.
    pub fn load_by_path(&self, rel: &str) -> io::Result<Option<PageDto>> {
        let Some(abs) = self.resolve_rel(rel) else {
            return Ok(None);
        };
        // A read is NOT an activation — see `load_page`. (GH #254 increment 3.)
        if self.entry_for_path(&abs).is_none() {
            return Ok(None);
        }
        let permit = self.admit_retained_graph_text_writer()?;
        let Some(ExactGraphLoadedPage {
            entry: effective,
            document,
            content,
            revision,
            file_identity,
        }) = self.load_validated_graph_text_target(&permit, &abs)?
        else {
            return Ok(None);
        };
        self.loaded_file_identities
            .write()
            .unwrap()
            .insert(abs.clone(), (revision.clone(), file_identity));
        let mut dto = page_dto_checked(&effective, &document)?;
        dto.read_only = read_only_org(&abs, &content);
        dto.rev = Some(revision);
        dto.path = self.rel_path(&abs);
        Ok(Some(dto))
    }

    /// Read and parse a page file into a [`Document`].
    pub fn read_document(&self, entry: &PageEntry) -> io::Result<Document> {
        let permit = self.admit_retained_graph_text_writer()?;
        self.load_validated_graph_text_target(&permit, &entry.path)?
            .map(|loaded| loaded.document)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}
