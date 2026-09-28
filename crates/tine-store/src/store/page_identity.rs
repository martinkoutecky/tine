//! Store page identity and validated OS path handoff.

use super::*;

impl Store {
    /// On a case-insensitive volume, an old route may reach a file through
    /// another case spelling. Canonicalization reveals the disk spelling;
    /// accept it only when case is the entire difference and it remains an
    /// eligible graph text file. The normal page path guard runs afterward.
    pub(super) fn disk_spelling_for_case_alias(&self, id: &PageId) -> Option<PageId> {
        let root = canonical_existing_path(&self.graph.root).ok()?;
        let actual = canonical_existing_path(&root.join(id.as_str())).ok()?;
        let rel = actual.strip_prefix(&root).ok()?;
        let spelling = rel.to_str()?.replace('\\', "/");
        if spelling == id.as_str()
            || spelling.to_lowercase() != id.as_str().to_lowercase()
            || !crate::model::graph_text_eligible(&root, &actual)
        {
            return None;
        }
        Some(PageId::from(spelling))
    }

    /// Type an eligible graph `.md` or `.org` file as a page id, including files
    /// outside the configured page and journal directories.
    /// Syncthing `.sync-conflict-` and Dropbox `(conflicted copy)` names,
    /// and invalid ids, return `None`; `page()` also refuses such ids. Use
    /// `read()` with their `FileId` to inspect raw conflict-copy bytes.
    /// No disk read or wait; cost O(path components).
    pub fn as_page(&self, file: &FileId) -> Option<PageId> {
        self.validate_file(file).ok()?;
        let path = file.as_str();
        if !crate::model::graph_text_eligible(&self.graph.root, &self.graph.root.join(path)) {
            return None;
        }
        Some(PageId::from(path))
    }

    pub(crate) fn validate_file(&self, file: &FileId) -> Result<(), StoreError> {
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
        if !path.starts_with(&format!("{}/", self.graph.current_config().pages_dir))
            && !path.starts_with(&format!("{}/", self.graph.current_config().journals_dir))
            && !path.starts_with("assets/")
            && !path.starts_with("logseq/")
            && !crate::model::graph_text_eligible(&self.graph.root, &self.graph.root.join(path))
        {
            return Err(StoreError::InvalidTarget(path.to_owned()));
        }
        Ok(())
    }

    pub(super) fn area_root(&self, file: &FileId) -> Result<PathBuf, StoreError> {
        self.validate_file(file)?;
        let path = file.as_str();
        let config = self.graph.current_config();
        let area = if path.starts_with(&format!("{}/", config.pages_dir)) {
            config.pages_dir.as_str()
        } else if path.starts_with(&format!("{}/", config.journals_dir)) {
            config.journals_dir.as_str()
        } else if crate::model::graph_text_eligible(&self.graph.root, &self.graph.root.join(path)) {
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
                if !target.starts_with(&root) || !crate::model::graph_text_eligible(&root, &target)
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
                && !crate::model::graph_text_eligible(&self.graph.root, &resolved))
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
