//! Store page identity and validated OS path handoff.

use super::*;

// File ids are slash-separated, already validated lexical paths. Test the
// component boundary without allocating a directory prefix on every read.
fn in_directory(path: &str, directory: &str) -> bool {
    path.strip_prefix(directory)
        .is_some_and(|tail| tail.starts_with('/'))
}

impl Store {
    /// The file-name listing answer: read a page by effective name, or a day
    /// by any accepted spelling (its files in any directory, CM SPEC §2.1),
    /// without waiting for the initial parse. A page some file is named for
    /// is found from a listing of file names and the matching files;
    /// otherwise the first lookup or invalidation builds a live O(P)
    /// file/preamble index, seeing files created after `open`, and warm
    /// lookups reuse it. This remains available after a failed initial
    /// parse; aliases require a successful graph view and are not resolved
    /// here. When files claim the same name or day, it uses the same
    /// claimant order as `WholeGraph::resolve` (journals directory first,
    /// then unsigned UTF-8 relative paths, SPEC §2.2). A missing name
    /// returns `None`, also while a file's name is undecodable: that file is
    /// listed in `unreadable_files()` and one bad file never blocks the graph.
    /// The selected file is read through `page()`, with the same safety, revision, and publication
    /// rules, including lock wait and read errors. It writes no page bytes.
    pub(super) fn page_named_from_files(
        &self,
        name: &str,
        kind: PageKind,
    ) -> Result<Option<PageRead>, StoreError> {
        // The ready view's lookup rule, without reclassifying the file.
        let lookup = crate::model::claim_lookup(&self.graph.current_journal_format(), name, kind);
        let held = |kind| {
            (self.graph.find_claimants(&lookup.name, kind).into_iter())
                .find(|entry| lookup.holds(entry))
        };
        // A day a pages file's `title::` names (CM-DAYFILE) is read only by
        // the pages listing; a journals listing keeps the rows it found.
        if lookup.day.is_some() {
            self.graph.find_claimants(&lookup.name, PageKind::Page);
        }
        let journals_dir = &self.graph.current_config().journals_dir;
        // A name that is no day is one identity across both kinds (a
        // journals-directory file whose name is no date, SPEC §2.1).
        let entry = held(lookup.kind).and_then(|_| {
            [PageKind::Page, PageKind::Journal]
                .into_iter()
                .filter(|kind| lookup.day.is_none() || *kind == PageKind::Journal)
                .filter_map(held)
                .min_by(|a, b| crate::model::compare_page_claimants(journals_dir, a, b))
        });
        let Some(entry) = entry else {
            return Ok(None);
        };
        let id = PageId::from(self.graph.rel_path(&entry.path));
        match self.page(&id) {
            Ok(read) => Ok(Some(read)),
            Err(StoreError::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Whether the page file `file` may carry a VCS anchor line (`<<<<<<< ` or
    /// `>>>>>>> ` at column 0), from state the store already holds (the launch
    /// pass reads every file once and the launch checkpoint carries the
    /// answer; saves and external changes refresh it with the revision they
    /// record). `false` means reading the file cannot find an anchor line in
    /// the bytes last observed; `true` means it may, or that the store cannot
    /// tell (the page is still loading or never cached: shadow journals, sync
    /// copies, unreadable or oversized files; or the store is closed), so the
    /// caller reads and scans. Touches no file. Cost O(1).
    pub fn may_carry_vcs_markers(&self, file: &FileId) -> bool {
        self.vcs_anchor_flag(file) != Some(false)
    }

    /// Test oracle: the tri-state behind [`Store::may_carry_vcs_markers`].
    /// `None` = not in the page cache (or closed), so only a read can tell.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn vcs_anchor_state(&self, file: &FileId) -> Option<bool> {
        self.vcs_anchor_flag(file)
    }

    fn vcs_anchor_flag(&self, file: &FileId) -> Option<bool> {
        if self.is_closed() {
            return None;
        }
        self.graph
            .vcs_anchor_state(&self.graph.root.join(file.as_str()))
    }

    /// On a case-insensitive volume, an old route may reach a file through
    /// another case or normalization spelling. Canonicalization reveals the
    /// disk spelling; accept it only under the shared Unicode fold and as an
    /// eligible graph text file. The normal page path guard runs afterward.
    /// A spelling resolver only (for a read and a new host key): which page a
    /// path names is [`crate::model::Graph::identify`]'s (B1).
    /// Cost: two canonicalizations, no read.
    pub(crate) fn disk_spelling(&self, id: &PageId) -> Option<PageId> {
        let root = canonical_existing_path(&self.graph.root).ok()?;
        let actual = canonical_existing_path(&root.join(id.as_str())).ok()?;
        let rel = actual.strip_prefix(&root).ok()?;
        let spelling = rel.to_str()?.replace('\\', "/");
        if spelling == id.as_str()
            || crate::model::entry_identity::fold_leaf(std::ffi::OsStr::new(&spelling))
                != crate::model::entry_identity::fold_leaf(std::ffi::OsStr::new(id.as_str()))
            || !crate::model::graph_text_eligible(&root, &actual, &self.graph.current_config())
        {
            return None;
        }
        Some(PageId::from(spelling))
    }

    /// The one answer to "which entry occupies this move destination", for
    /// the rename planner and the transaction's no-replace move guard alike:
    /// `None` when no directory entry answers to `to`; otherwise the listed
    /// spelling of that entry. `to` is first checked the way the move guard
    /// checks it ([`Self::path_for_os_handoff`] and the write-target check),
    /// so an absolute, parent-relative or out-of-graph id, or one under an
    /// ancestor linked outside the graph, is refused before anything is
    /// probed. Then [`Self::occupant_at`] answers.
    /// Cost O(path components); see [`Self::occupant_at`] for the probe.
    pub fn move_occupant(&self, to: &FileId) -> Result<Option<FileId>, StoreError> {
        let path = self.path_for_os_handoff(to, false)?;
        self.ensure_write_target(to, &path)
            .map_err(StoreError::from_io)?;
        let spelled = self.spelled_destination(to).map_err(StoreError::from_io)?;
        self.occupant_at(to, &spelled).map_err(StoreError::from_io)
    }

    /// [`Self::move_occupant`]'s probe of an already checked destination,
    /// `spelled` being [`Self::spelled_destination`] of `to`. A final symlink
    /// is its own entry and is never followed (discovery skips it; the guard
    /// treats it as occupied at the requested spelling). Only a regular entry
    /// may resolve to another listed spelling: on a case-folding volume
    /// `pages/B.md` reaches the listed `pages/b.md` ([`Self::disk_spelling`]).
    /// Cost: one `lstat`; two canonicalizations when a regular entry answers.
    pub(crate) fn occupant_at(
        &self,
        to: &FileId,
        spelled: &Path,
    ) -> std::io::Result<Option<FileId>> {
        match fs::symlink_metadata(spelled) {
            Err(error) if crate::atomic_file::names_nothing(&error) => Ok(None),
            Err(error) => Err(error),
            Ok(meta) if meta.file_type().is_symlink() => Ok(Some(to.clone())),
            Ok(_) => Ok(Some(
                self.as_page(to)
                    .and_then(|page| self.disk_spelling(&page))
                    .map_or_else(|| to.clone(), |page| page.file()),
            )),
        }
    }

    /// `file`'s OS path with its parent resolved and its requested leaf kept,
    /// never a final symlink's target: a separately listed destination must
    /// remain occupied at its requested location. The caller has checked
    /// `file` ([`Self::path_for_os_handoff`]).
    pub(crate) fn spelled_destination(&self, file: &FileId) -> std::io::Result<PathBuf> {
        let candidate = self.graph.root.join(file.as_str());
        let parent = candidate.parent().unwrap();
        let (existing, resolved) = crate::model::canonical_existing_ancestor(parent)?;
        Ok(resolved
            .join(parent.strip_prefix(existing).unwrap())
            .join(candidate.file_name().unwrap()))
    }

    /// The write-target boundary for `path`, `file`'s checked OS path: the
    /// approved assets root for `assets/`, the graph root otherwise.
    pub(crate) fn ensure_write_target(&self, file: &FileId, path: &Path) -> std::io::Result<()> {
        if file.as_str().starts_with("assets/") {
            self.graph.ensure_asset_write_target(path)
        } else {
            self.graph.ensure_write_target(path)
        }
    }

    /// Type an eligible graph `.md`, `.markdown`, or `.org` file as a page id
    /// (case-insensitive extensions), including files
    /// outside the configured page and journal directories.
    /// Syncthing `.sync-conflict-` and Dropbox `(conflicted copy)` names,
    /// and invalid ids, return `None`; `page()` also refuses such ids. Use
    /// `read()` with their `FileId` to inspect raw conflict-copy bytes.
    /// No disk read or wait; cost O(path components).
    pub fn as_page(&self, file: &FileId) -> Option<PageId> {
        let config = self.graph.current_config();
        self.validate_file_with_config(file, &config).ok()?;
        let path = file.as_str();
        if !crate::model::graph_text_relative_eligible(path, &config) {
            return None;
        }
        Some(PageId::from(path))
    }

    /// `file_id(Area::Graph, rel)`: an eligible graph-text file anywhere in
    /// the graph, named by its graph-relative path.
    pub(super) fn graph_text_file_id(&self, rel: &str) -> Result<FileId, StoreError> {
        if !crate::model::graph_text_relative_eligible(rel, &self.graph.current_config()) {
            return Err(StoreError::InvalidTarget(rel.into()));
        }
        let id = FileId::from(rel.to_owned());
        self.validate_file(&id)?;
        Ok(id)
    }

    /// Whether an `Area::Graph` scan lists (or descends into) `rel`: the same
    /// scope graph discovery reads, so a backup copies exactly the graph text.
    pub(super) fn graph_text_listed(&self, rel: &str, is_dir: bool) -> bool {
        let config = self.graph.current_config();
        if is_dir {
            crate::model::graph_text_directory_scannable(Path::new(""), Path::new(rel), &config)
        } else {
            crate::model::graph_text_relative_eligible(rel, &config)
        }
    }

    pub(crate) fn validate_file(&self, file: &FileId) -> Result<(), StoreError> {
        self.validate_file_with_config(file, &self.graph.current_config())
    }

    fn validate_file_with_config(
        &self,
        file: &FileId,
        config: &tine_core::config::Config,
    ) -> Result<(), StoreError> {
        let path = file.as_str();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        // Raw file ids under pages/journals must not bypass the same configured
        // graph-text exclusion used by discovery and direct page reads.
        if crate::file_kind::is_graph_text_path(std::path::Path::new(path))
            && !path.starts_with("logseq/.tine-trash/")
            && crate::model::configured_hidden(path, config)
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        // D-20261026-GI: only this fixed graph-root non-page file joins the
        // existing areas, for autocommit's guarded ignore-rule append.
        if !in_directory(path, &config.pages_dir)
            && !in_directory(path, &config.journals_dir)
            && !path.starts_with("assets/")
            && !path.starts_with("logseq/")
            && !crate::model::graph_text_relative_eligible(path, config)
            && path != ".gitignore"
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        Ok(())
    }

    pub(super) fn area_root(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        self.validate_file(file)?;
        let path = file.as_str();
        let config = self.graph.current_config();
        let area = if in_directory(path, &config.pages_dir) {
            config.pages_dir.as_str()
        } else if in_directory(path, &config.journals_dir) {
            config.journals_dir.as_str()
        } else if path == ".gitignore" || crate::model::graph_text_relative_eligible(path, &config)
        {
            return Ok(self.graph.root.clone());
        } else {
            path.split('/').next().unwrap_or_default()
        };
        Ok(self.graph.root.join(area))
    }

    /// A validated OS path. `existing_regular_file` requires a live file in
    /// the graph-text scope or assets for an opener; it refuses meta, trash, and
    /// conflict-copy paths even if their files exist. A page source may follow
    /// an older graph layout's in-graph link between pages and journals.
    /// Otherwise a missing final file is allowed, with ancestors inside its area.
    /// Cost O(path components), independent of graph size. Refuses an escaped
    /// target; missing or unreadable existing files return their I/O error.
    pub fn path_for_os_handoff(
        &self,
        file: &FileId,
        existing_regular_file: bool,
    ) -> Result<PathBuf, StoreError> {
        if self.is_closed() {
            return Err(StoreError::Closed);
        }
        let area = self.area_root(file)?;
        let (area, candidate) = if let Some(rel) = file.as_str().strip_prefix("assets/") {
            let approved = self.graph.assets_path();
            let lexical = self.graph.root.join("assets");
            let live = match canonical_existing_path(&lexical) {
                Ok(path) => path,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound && approved == lexical =>
                {
                    lexical
                }
                Err(error) => return Err(StoreError::from_io(error)),
            };
            if live != approved {
                return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
            }
            (approved.clone(), approved.join(rel))
        } else {
            (area, self.graph.root.join(file.as_str()))
        };
        if existing_regular_file {
            let target = canonical_existing_path(&candidate).map_err(StoreError::from_io)?;
            if !target.is_file() {
                return Err(if file.as_str().starts_with("assets/") {
                    StoreError::InvalidTarget(file.as_str().to_owned())
                } else {
                    StoreError::PageSource("page source is not a file".into())
                });
            }
            if file.as_str().starts_with("assets/") {
                let assets = canonical_existing_path(&self.graph.assets_path())
                    .map_err(StoreError::from_io)?;
                if !target.starts_with(&assets) {
                    return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
                }
            } else {
                if self.as_page(file).is_none() {
                    return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
                }
                let root =
                    canonical_existing_path(&self.graph.root).map_err(StoreError::from_io)?;
                if !target.starts_with(&root)
                    || !crate::model::graph_text_eligible(
                        &root,
                        &target,
                        &self.graph.current_config(),
                    )
                {
                    return Err(StoreError::PageSource(
                        "page source escapes graph text scope".into(),
                    ));
                }
            }
            return Ok(target);
        }
        let (area_canonical, area_missing) = match canonical_existing_path(&area) {
            Ok(path) => (path, false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let root =
                    canonical_existing_path(&self.graph.root).map_err(StoreError::from_io)?;
                (
                    root.join(
                        area.strip_prefix(&self.graph.root)
                            .map_err(|_| StoreError::InvalidTarget(file.as_str().to_owned()))?,
                    ),
                    true,
                )
            }
            Err(error) => return Err(StoreError::from_io(error)),
        };
        let (existing, resolved) =
            crate::model::canonical_existing_ancestor(&candidate).map_err(StoreError::from_io)?;
        if !candidate.starts_with(&area)
            || (!resolved.starts_with(&area_canonical)
                && !(area_missing && area_canonical.starts_with(&resolved)))
            || (self.as_page(file).is_some()
                && resolved.is_file()
                && !crate::model::graph_text_eligible(
                    &self.graph.root,
                    &resolved,
                    &self.graph.current_config(),
                ))
        {
            return Err(StoreError::InvalidTarget(file.as_str().to_owned()));
        }
        let suffix = candidate
            .strip_prefix(existing)
            .expect("candidate ancestor");
        if suffix.as_os_str().is_empty() {
            Ok(resolved)
        } else {
            Ok(resolved.join(suffix))
        }
    }
}
