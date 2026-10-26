//! Read-only rename planning, shared by the single-page and namespace paths.
use super::*;

/// What one rename attempt writes, planned from one refreshed view and the
/// config captured with it; commit and the host admit it only while both are
/// current (A-W1, B2).
pub(super) struct RenamePlan {
    pub(super) graph: tine_store::WholeGraph,
    pub(super) config: tine_store::ConfigState,
    pub(super) map: RenameMap,
    pub(super) lookup: HashMap<String, String>,
    pub(super) moves: HashMap<PageId, FileId>,
    pub(super) moved_titles_to_rebind: HashSet<PageId>,
    pub(super) merge: Option<(PageId, PageId)>,
    pub(super) ref_merge: bool,
    pub(super) candidates: Vec<PageId>,
    /// The candidates `pages` read to find the plan's write set, reused by
    /// the commit so it writes exactly what was reserved (A-R3).
    pub(super) reads: HashMap<PageId, (String, FileRev)>,
    /// Referrers `pages` leaves byte-identical for their VCS conflict markers.
    pub(super) skipped: Vec<PageId>,
    /// The new text of each referrer `pages` found changed, computed from
    /// its `reads` bytes; the host reuses it on equal bytes (GH #623).
    pub(super) rewritten: HashMap<PageId, String>,
}

impl RenamePlan {
    /// Every page the plan writes (A-R3): moved pages and their
    /// destinations, interrupted titles, a merge's survivor and source,
    /// and the referrers whose references it changes. A referrer the
    /// rename leaves byte-identical (unchanged, or carrying VCS conflict
    /// markers) is not written, so its unsaved input blocks nothing. The
    /// reads are kept for the commit, which decides on the same bytes.
    pub(super) fn pages(&mut self, store: &Store) -> io::Result<Vec<PageId>> {
        let format = self.config.file_name_format;
        let mut pages = Vec::new();
        for id in &self.candidates {
            if self.moves.contains_key(id) || self.moved_titles_to_rebind.contains(id) {
                pages.push(id.clone());
                continue;
            }
            let file = id.file();
            let (content, rev) = read_text(store, &file)
                .map_err(|e| error(e.kind(), &format!("{}: {e}", file.as_str())))?;
            // As `commit_rename` decides: a referrer is written when the
            // transaction's rewrite (using the page's source format)
            // changes it and it carries no VCS conflict markers.
            let syntax = Format::from_path(id.as_str().as_ref());
            let org = syntax == Format::Org;
            let rewritten = refs::rename_rewrite(&content, org, &self.map.0, format);
            let changed = rewritten != content;
            let marked =
                !tine_core::concord_queue::vcs_conflict_markers(&content, syntax).is_empty();
            match (changed, marked) {
                (true, false) => {
                    pages.push(id.clone());
                    self.rewritten.insert(id.clone(), rewritten);
                }
                (true, true) => self.skipped.push(id.clone()),
                _ => {}
            }
            self.reads.insert(id.clone(), (content, rev));
        }
        pages.extend(crate::retained::pages(store, self.moves.values()));
        pages.extend(
            self.merge
                .iter()
                .flat_map(|(src, dst)| [src.clone(), dst.clone()]),
        );
        Ok(pages)
    }

    /// The source and target of a single page's rename, which a host runs
    /// as one page operation (STEP3 §7, F10): one name, no namespace
    /// descendant, no merge, no interrupted title. A source with no file
    /// renames its references only, under the default Markdown spelling.
    pub(super) fn single(&self, store: &Store) -> io::Result<Option<(PageId, PageId)>> {
        if self.merge.is_some()
            || self.ref_merge
            || !self.moved_titles_to_rebind.is_empty()
            || self.map.0.len() != 1
        {
            return Ok(None);
        }
        let spelled = |name: &str| {
            let rel = format!(
                "{}.md",
                tine_core::model::encode_page_name(name, self.config.file_name_format)
            );
            store.file_id(Area::Pages, &rel).map_err(store_error)
        };
        let (source, target) = match self.moves.iter().next() {
            Some((source, to)) => (source.clone(), to.clone()),
            None => {
                let (old, new) = &self.map.0[0];
                (page(store, &spelled(old)?)?, spelled(new)?)
            }
        };
        Ok(Some((source, page(store, &target)?)))
    }
}

fn page(store: &Store, file: &FileId) -> io::Result<PageId> {
    store
        .as_page(file)
        .ok_or_else(|| error(io::ErrorKind::InvalidInput, "invalid file path"))
}

pub(super) fn plan_rename(
    store: &Store,
    old: &str,
    new: &str,
    expected_path: Option<&str>,
    merge_into: Option<&str>,
    #[cfg(test)] after_inventory: &dyn Fn(),
) -> io::Result<RenamePlan> {
    let graph = refreshed_view(store)?;
    // The planning context: every format decision below uses this config,
    // and commit checks it is still the view's (B2).
    let config = store.config();
    let old_key = refs::normalize(old);
    // Only file-claimed names move: the full inventory's alias and
    // reference-only name discovery is not needed here (GH #623).
    let owned = graph.inventory(tine_store::InventoryScope::FilesAtOrUnder(&old_key));
    #[cfg(test)]
    after_inventory();
    let source = existing(graph.resolve(old, false));
    validate_target(&source, expected_path)?;
    let mut pairs = Vec::new();
    let mut moves = HashMap::<PageId, FileId>::new();
    let mut moved_titles_to_rebind = HashSet::<PageId>::new();
    let mut destinations = HashSet::new();
    let mut identities = HashSet::new();
    let mut primary_is_file = false;
    let mut merge = None;
    for entry in owned.0.iter().filter(|entry| !entry.is_journal) {
        let ids = physical(&entry.target);
        let primary = refs::normalize(&entry.name) == old_key;
        let new_name = if primary {
            new.to_owned()
        } else {
            format!("{new}{}", namespace_suffix(&entry.name, &old_key))
        };
        if ids.len() > 1 {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "multiple files share this page identity; mutation is ambiguous",
            ));
        }
        let id = ids[0].clone();
        // An alias names its owner's page, so it is taken like a file name.
        let mut taken = claimants(graph.resolve(&new_name, false));
        taken.retain(|claimant| *claimant != id);
        if let (true, Some(into), [survivor]) = (primary, merge_into, taken.as_slice()) {
            if survivor.as_str() == into {
                identities.insert(refs::normalize(&new_name));
                merge = Some((id, survivor.clone()));
                primary_is_file = true;
                pairs.push((entry.name.clone(), new_name));
                continue;
            }
        }
        if !taken.is_empty() {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "target page identity already exists elsewhere in the graph",
            ));
        }
        let ext = Format::from_path(id.as_str().as_ref()).ext();
        let rel = format!(
            "{}.{}",
            tine_core::model::encode_page_name(&new_name, config.file_name_format),
            ext
        );
        let to = store.file_id(Area::Pages, &rel).map_err(store_error)?;
        if old == new {
            // Confirming the displayed title also finishes a case move left
            // after content publication. Preserve unrelated custom filenames
            // and leave already-correct entries alone.
            primary_is_file |= primary;
            if to == id.file() || !refs::same_page(to.as_str(), id.as_str()) {
                continue;
            }
        }
        if !destinations.insert(to.as_str().to_owned())
            || !identities.insert(refs::normalize(&new_name))
        {
            return Err(error(
                io::ErrorKind::AlreadyExists,
                "multiple pages map to the same rename target",
            ));
        }
        if to == id.file() {
            // A crash after the physical rename can leave Old's explicit
            // title in New's file. Finish that rewrite in place on retry.
            moved_titles_to_rebind.insert(id.clone());
            pairs.push((entry.name.clone(), new_name));
            primary_is_file |= primary;
            continue;
        }
        if primary {
            primary_is_file = true;
        }
        pairs.push((entry.name.clone(), new_name));
        moves.insert(id, to);
    }
    // v0.6.5 model.rs 3654: reference-only pages have no move, but refs
    // change. Onto an existing page that repoints them, which is OG
    // `merge-pages!` of a page with no blocks, so it needs the confirmation.
    let mut ref_merge = false;
    if !primary_is_file && old != new {
        let taken = claimants(graph.resolve(new, false));
        match (merge_into, taken.as_slice()) {
            (_, []) => {}
            (Some(into), [owner]) if owner.as_str() == into => ref_merge = true,
            (Some(_), _) => {}
            (None, _) => {
                return Err(error(
                    io::ErrorKind::AlreadyExists,
                    "target page identity already exists elsewhere in the graph",
                ))
            }
        }
        pairs.push((old.to_owned(), new.to_owned()));
    }
    if merge_into.is_some() && merge.is_none() && !ref_merge {
        return Err(error(
            io::ErrorKind::NotFound,
            "the page to merge into no longer owns that name",
        ));
    }
    let map = RenameMap(pairs);
    let lookup: HashMap<_, _> = map
        .0
        .iter()
        .map(|(from, to)| (refs::normalize(from), to.clone()))
        .collect();
    // v0.6.5 model.rs 3681-3690 (warm index) and OG `:block/refs`: only
    // pages that explicitly reference a renamed name are rewritten, plus
    // every moved page. A name mentioned only inside an ordinary query stays;
    // the pages holding export definitions are read too, whose page
    // selectors follow the rename (`tine_core::export_definition`).
    let olds: Vec<String> = map.0.iter().map(|(from, _)| from.clone()).collect();
    let mut candidates: Vec<PageId> = graph.explicit_referrers(&olds);
    candidates.extend(export_definition_pages(&graph, &olds)?);
    candidates.extend(moves.keys().cloned());
    candidates.extend(moved_titles_to_rebind.iter().cloned());
    candidates.retain(|id| {
        merge
            .as_ref()
            .is_none_or(|(src, dst)| id != src && id != dst)
    });
    candidates.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    candidates.dedup();
    Ok(RenamePlan {
        graph,
        config,
        map,
        lookup,
        moves,
        moved_titles_to_rebind,
        merge,
        ref_merge,
        candidates,
        reads: HashMap::new(),
        skipped: Vec::new(),
        rewritten: HashMap::new(),
    })
}
