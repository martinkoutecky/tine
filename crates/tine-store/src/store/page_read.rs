//! Reading one page file for `Store::page`: the target's validation, the
//! read's revision and read-only state, and the publication of a change the
//! read observed (GH #623 BR3 split these out of `Store::page`).
use super::*;

impl Store {
    /// The validated page file `id` names: its disk spelling, path and
    /// listing entry. Refuses a symlinked page or one outside the areas.
    pub(super) fn page_target(
        &self,
        id: &PageId,
    ) -> Result<(PageId, PathBuf, PageEntry), StoreError> {
        if self.as_page(&id.file()).is_none() {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        let id = self
            .disk_spelling_for_case_alias(id)
            .unwrap_or_else(|| id.clone());
        // v0.6.5's page walker never indexes a symlinked page file (it could
        // expose a file outside the graph), so no listing hands out such an
        // id; refuse one here too. Ancestors must stay inside the area. The
        // read itself uses the lexical path, which is the page's identity.
        let path = self.graph.root.join(id.as_str());
        self.path_for_os_handoff(&id.file(), false)?;
        if fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        let entry = self
            .graph
            .entry_for_path(&path)
            .ok_or_else(|| StoreError::InvalidTarget(id.as_str().to_owned()))?;
        Ok((id, path, entry))
    }

    /// A parsed page as a read: its revision and read-only state.
    pub(super) fn page_read(&self, id: PageId, mut doc: PageDto) -> Result<PageRead, StoreError> {
        let rev = FileRev(doc.rev.clone().ok_or(StoreError::NotFound)?);
        let read_only = if self.config().problem.is_some() {
            doc.read_only = true;
            Some("config.edn could not be read; graph is read-only".to_owned())
        } else {
            doc.read_only
                .then(|| "Org file does not round-trip".to_owned())
        };
        Ok(PageRead {
            id,
            doc,
            rev,
            read_only,
        })
    }

    /// Publish a change `read` observed on disk (its cache entry moved): a
    /// file the published claimants know is `Modified`, any other `Created`.
    pub(super) fn publish_observed(&self, read: &PageRead, path: &Path, entry: PageEntry) {
        let (id, rev) = (&read.id, &read.rev);
        let known = self
            .changes
            .snapshot
            .read()
            .unwrap()
            .as_ref()
            .is_some_and(|snapshot| {
                snapshot
                    .claimants
                    .get(&(
                        entry.kind == PageKind::Journal,
                        tine_core::refs::page_key(&entry.name),
                    ))
                    .is_some_and(|claimants| claimants.iter().any(|claimant| claimant.path == path))
            });
        let raced = self.watch.note_own(&[(id.file(), Some(rev.clone()))]);
        self.changes.publish(
            Origin::External,
            vec![(
                id.file(),
                if known {
                    ChangeKind::Modified
                } else {
                    ChangeKind::Created
                },
                Some(rev.clone()),
            )],
            false,
            vec![(id.file(), entry.kind, entry.name)],
        );
        self.watch.reconcile_raced(&raced);
    }
}
