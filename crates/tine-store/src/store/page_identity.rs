//! Store page identity and validated OS path handoff.

use super::*;

// File ids are slash-separated, already validated lexical paths. Test the
// component boundary without allocating a directory prefix on every read.
fn in_directory(path: &str, directory: &str) -> bool {
    path.strip_prefix(directory)
        .is_some_and(|tail| tail.starts_with('/'))
}

impl Store {
    /// The file-name listing answer: read an ordinary page by effective name, or a journal by its file
    /// stem or parseable display title (falling back to an ordinary file of
    /// the requested date name when no journal exists), without waiting for the initial
    /// parse. A page some file is named for is found from a listing of file
    /// names and the matching files; otherwise the first lookup or
    /// invalidation builds a live O(P) file/preamble index, seeing files
    /// created after `open`, and warm lookups reuse it. This remains available after a
    /// failed initial parse; aliases require a successful graph view and are
    /// not resolved here. When files claim the same name or journal day, it
    /// uses the same claimant ranking as `WholeGraph::resolve` (canonical
    /// date-stem journal first, then Markdown before Org). A missing name
    /// returns `None`, also while a file's name is undecodable: that file is
    /// listed in `unreadable_files()` and one bad file never blocks the graph.
    /// The selected file is read through `page()`, with the same safety, revision, and publication
    /// rules, including lock wait and read errors. It writes no page bytes.
    pub(super) fn page_named_from_files(
        &self,
        name: &str,
        kind: PageKind,
    ) -> Result<Option<PageRead>, StoreError> {
        let lookup = if kind == PageKind::Journal {
            self.graph
                .current_journal_format()
                .parse(name)
                .map(|date| self.graph.current_journal_format().title(date))
                .unwrap_or_else(|| name.to_owned())
        } else {
            name.to_owned()
        };
        let entry = self
            .graph
            .find_entry_named_file_first(&lookup, kind)
            .or_else(|| {
                // Match ready-view link resolution without reclassifying the file.
                (kind == PageKind::Journal
                    && self.graph.current_journal_format().parse(name).is_some())
                .then(|| self.graph.find_entry_named_file_first(name, PageKind::Page))
                .flatten()
            });
        let Some(entry) = entry else {
            return Ok(None);
        };
        let id = PageId::from(self.graph.rel_path(&entry.path));
        match self.page(&id) {
            Ok(read) => Ok(Some(read)),
            Err(StoreError::NotFound) => Ok(None),
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
    /// another case spelling. Canonicalization reveals the disk spelling;
    /// accept it only when case is the entire difference and it remains an
    /// eligible graph text file. The normal page path guard runs afterward.
    /// A spelling resolver only (for a read and a new host key): which page
    /// a path names is [`crate::model::Graph::identify`]'s (B1).
    pub(crate) fn disk_spelling(&self, id: &PageId) -> Option<PageId> {
        let root = canonical_existing_path(&self.graph.root).ok()?;
        let actual = canonical_existing_path(&root.join(id.as_str())).ok()?;
        let rel = actual.strip_prefix(&root).ok()?;
        let spelling = rel.to_str()?.replace('\\', "/");
        if spelling == id.as_str()
            || spelling.to_lowercase() != id.as_str().to_lowercase()
            || !crate::model::graph_text_eligible(&root, &actual, &self.graph.current_config())
        {
            return None;
        }
        Some(PageId::from(spelling))
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
