//! Reading one page file for `Store::page`: the target's validation, the
//! read's revision and read-only state, and the publication of a change the
//! read observed (GH #623 BR3 split these out of `Store::page`).
use super::*;

/// The journal files `Store::reconcile_unindexed_day` found for a day the
/// index resolved absent: the first held one's read, and the unheld ones it
/// asked the watcher to take in.
pub(super) struct UnindexedDay {
    day: tine_core::date::JournalDate,
    read: Option<PageRead>,
    unheld: Vec<PageId>,
}

/// Provenance of a page answer, carried from the content-selection seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageReadSource {
    /// An unheld file read.
    Disk,
    /// The held owner's indexed bytes.
    HeldBytes,
    /// The held owner's indexed absence, without reading disk.
    HeldAbsent,
    /// An unindexed or uncertain held identity, read from disk unpublished.
    HeldUnindexed,
}

/// Parsed page and the raw-byte revision used for a guarded save.
pub struct PageRead {
    /// Exact disk path identity. A case-alias request on a case-insensitive
    /// volume returns the file's disk spelling.
    pub id: PageId,
    /// Parsed document.
    pub doc: PageDto,
    /// Revision of the bytes that produced `doc`.
    pub rev: FileRev,
    /// Reason a file cannot round-trip safely, if any. Reflects the parsed
    /// `doc.read_only` flag; a twin claim alone does not set it. Save checks
    /// disk safety again rather than trusting either caller-supplied flag.
    pub read_only: Option<String>,
    /// Where this answer obtained its bytes.
    pub source: PageReadSource,
}

impl Store {
    /// The validated page file `id` names: its disk spelling, path and
    /// listing entry. Refuses a symlinked page or one outside the areas.
    pub(super) fn page_target(
        &self,
        id: &PageId,
    ) -> Result<(PageId, PathBuf, PageEntry), StoreError> {
        let (id, path) = self.page_spot(id)?;
        let entry = self.page_entry(&id, &path)?;
        Ok((id, path, entry))
    }

    /// [`Self::page_target`]'s checks that read no page file: its disk
    /// spelling and path, refusing a symlinked page or one outside the areas.
    pub(crate) fn page_spot(&self, id: &PageId) -> Result<(PageId, PathBuf), StoreError> {
        self.page_spot_observed(id).map(|(id, path, _)| (id, path))
    }

    /// [`Self::page_spot`] with the file's metadata its symlink check read:
    /// what a held page's read stamps (A-H1), at no added system call.
    pub(super) fn page_spot_observed(
        &self,
        id: &PageId,
    ) -> Result<(PageId, PathBuf, std::io::Result<fs::Metadata>), StoreError> {
        if self.as_page(&id.file()).is_none() {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        let id = self.disk_spelling(id).unwrap_or_else(|| id.clone());
        // v0.6.5's page walker never indexes a symlinked page file (it could
        // expose a file outside the graph), so no listing hands out such an
        // id; refuse one here too. Ancestors must stay inside the area. The
        // read itself uses the lexical path, which is the page's identity.
        let path = self.graph.root.join(id.as_str());
        self.path_for_os_handoff(&id.file(), false)?;
        let metadata = fs::symlink_metadata(&path);
        if metadata
            .as_ref()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(StoreError::InvalidTarget(id.as_str().to_owned()));
        }
        Ok((id, path, metadata))
    }

    /// `Store::page`'s writer from Ready, the cache generation it was taken
    /// at and the page's target. A held page whose file moved since its
    /// owner's last indexed observation first waits, without the writer, at
    /// most `OPERATION_WAIT` for the owner to observe it (A-H1); the target
    /// is then read again without asking twice. A failed observation is its
    /// read error.
    pub(super) fn ready_page_writer(
        &self,
        id: &PageId,
    ) -> Result<(std::sync::MutexGuard<'_, ()>, u64, PageId, PathBuf), StoreError> {
        debug_assert!(
            !crate::watch::on_observer_thread(),
            "Store::page on the page host driver thread would wait for itself"
        );
        let mut asked: Option<(PageId, PathBuf)> = None;
        loop {
            let waiting = Instant::now();
            let writer = self.writer.lock().unwrap();
            self.graph.diag.page_writer_wait(waiting.elapsed());
            let before_generation = self.graph.cache_generation();
            if self.is_closed() {
                return Err(StoreError::Closed);
            }
            let (id, path) = match asked.take() {
                Some(spot) => spot,
                None => {
                    let (id, path, metadata) = self.page_spot_observed(id)?;
                    if let Some(told) = self.held_observation_owed(&path, &metadata) {
                        // The driver takes the writer first: never wait holding it.
                        drop(writer);
                        // Its read error, never `NotFound`: whatever its kind,
                        // the file was not observed absent.
                        if let Ok(Err(error)) = told.recv_timeout(self.held_read_wait()) {
                            return Err(StoreError::Io(error.into()));
                        }
                        asked = Some((id, path));
                        continue;
                    }
                    (id, path)
                }
            };
            return Ok((writer, before_generation, id, path));
        }
    }

    /// How long a held page's read waits for its owner's observation.
    fn held_read_wait(&self) -> std::time::Duration {
        #[cfg(test)]
        if let Some(wait) = *self.held_read_wait.lock().unwrap() {
            return wait;
        }
        crate::watch::OBSERVED_WAIT
    }

    /// The page at `path`, if a host holds it and its file moved since its
    /// owner's last indexed observation (A-H1): the owner is asked to observe
    /// it, as a watcher notification asks, and the returned receiver is told
    /// once that observation settled. `metadata` is the read's own
    /// `symlink_metadata` of the file; a failed one (other than a missing
    /// file) never matches. None, at the cost of one map lookup, when no host
    /// holds the page, its owner has not indexed it (the read parses the
    /// file), the stamp matches, or no owner is attached. The caller holds
    /// the writer and must release it before waiting.
    pub(super) fn held_observation_owed(
        &self,
        path: &Path,
        metadata: &std::io::Result<fs::Metadata>,
    ) -> Option<std::sync::mpsc::Receiver<std::io::Result<()>>> {
        if !matches!(
            self.graph.source(path),
            crate::model::Source::Held(_) | crate::model::Source::HeldAbsent
        ) {
            return None;
        }
        let now = match metadata {
            Ok(metadata) => crate::watch::stamp_from_metadata(metadata).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(None),
            Err(_) => None,
        };
        if now.is_some_and(|now| self.watch.unchanged_since_observed(path, now.as_ref())) {
            return None;
        }
        let key = self.graph.held_key(path)?;
        let (done, told) = std::sync::mpsc::channel();
        let asked = self.watch.forward_observed(key, done);
        #[cfg(feature = "test-faults")]
        if asked {
            crate::cost_counters::held_read_forward();
        }
        asked.then_some(told)
    }

    /// A journal day `name` the index resolves absent may have arrived
    /// unobserved under another accepted file name (GH #863, D3): stat each,
    /// the day's accepted stems (`JournalFormat::accepted_stems`) times the
    /// graph-text extensions. A hit the watcher has not observed is read and
    /// published before this returns: an unheld one by the watcher's
    /// exact-path reconcile under the writer, a held one through its owner
    /// as a held page's read is (`Store::page`, writer released), whose read
    /// error this returns. `None` when none exists; a stat error other than
    /// NotFound is returned. A bounded number of stats under the writer;
    /// nothing is read unless found. CM's real-name owner must leave the
    /// canonical day title unclaimed; a masked journal is no day's file.
    pub(super) fn reconcile_unindexed_day(
        &self,
        name: &str,
        view: &WholeGraph,
    ) -> Result<Option<UnindexedDay>, StoreError> {
        let format = &view.journal_format;
        let Some(day) = format.parse(name) else {
            return Ok(None);
        };
        let key = tine_core::refs::page_key(&format.title(day));
        if view.graph.real_page_owners().get(&key).is_some() {
            return Ok(None);
        }
        // From Ready only: a served launch checkpoint's diff holds the writer
        // and reconciles every file itself, and a read never waits for it.
        if !matches!(*self.load.status.lock().unwrap(), LoadStatus::Ready) {
            return Ok(None);
        }
        let directory = self.graph.current_config().journals_dir.clone();
        let writer = self.writer.lock().unwrap();
        if self.is_closed() {
            return Ok(None);
        }
        // Every existing candidate is kept for the fallback read (review r4):
        // a file the watcher already refused stays unchanged since observed,
        // yet it is still no absence. Only unobserved ones are reconciled. A
        // stat error other than NotFound is no absence either.
        let mut held = Vec::new();
        let mut unheld = Vec::new();
        let mut fresh = HashSet::new();
        for stem in format.accepted_stems(day) {
            if stem.contains(['/', '\\']) {
                continue;
            }
            for extension in crate::file_kind::GRAPH_TEXT_EXTENSIONS {
                let id = PageId::from(format!("{directory}/{stem}.{extension}").as_str());
                let path = self.graph.root.join(id.as_str());
                let now = match fs::symlink_metadata(&path) {
                    Ok(metadata) => crate::watch::stamp_from_metadata(&metadata),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(StoreError::Io(error.into())),
                };
                let disk = matches!(self.graph.source(&path), crate::model::Source::Disk);
                if disk && !self.watch.unchanged_since_observed(&path, now.as_ref()) {
                    fresh.insert(path);
                }
                if disk {
                    unheld.push(id)
                } else {
                    held.push(id)
                }
            }
        }
        if held.is_empty() && unheld.is_empty() {
            return Ok(None);
        }
        self.watch.reconcile_raced(&fresh);
        drop(writer);
        let mut read = None;
        for id in held {
            match self.page(&id) {
                Ok(page) => read = read.or(Some(page)),
                Err(StoreError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(Some(UnindexedDay { day, read, unheld }))
    }

    /// The day `reconcile_unindexed_day` found when the name still resolves
    /// absent after it (GH #863 review r3): a held file's read (an owner's
    /// first publication may not have happened yet; the read stays
    /// unpublished, owner-only indexing), else the first unheld file the
    /// watcher could not take in, read through `page` so its read error
    /// (invalid UTF-8, a sharing violation) reaches the caller instead of an
    /// empty day. CM's latest real-name owner must still leave the title
    /// unclaimed after reconciliation. One page read at most per candidate.
    pub(super) fn unindexed_day_read(
        &self,
        day: UnindexedDay,
        view: &WholeGraph,
    ) -> Result<Option<PageRead>, StoreError> {
        let key = tine_core::refs::page_key(&view.journal_format.title(day.day));
        if view.graph.real_page_owners().get(&key).is_some() {
            return Ok(None);
        }
        if day.read.is_some() {
            return Ok(day.read);
        }
        for id in day.unheld {
            match self.page(&id) {
                Ok(read) => return Ok(Some(read)),
                Err(StoreError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    /// The listing entry of the page file at `path` (its preamble names it).
    pub(super) fn page_entry(&self, id: &PageId, path: &Path) -> Result<PageEntry, StoreError> {
        self.graph
            .entry_for_path(path)
            .ok_or_else(|| StoreError::InvalidTarget(id.as_str().to_owned()))
    }

    /// A read of a page a host holds (A-V4, A-H1): its owner's indexed
    /// bytes, named and parsed from them, with no read of the file; one
    /// its owner has not indexed yet is parsed from the file, unpublished.
    /// None for a page no host holds.
    pub(super) fn held_page(
        &self,
        path: &Path,
    ) -> Result<Option<(PageDto, PageReadSource)>, StoreError> {
        let (read, source) = match self.graph.source(path) {
            crate::model::Source::Disk => return Ok(None),
            crate::model::Source::Held(bytes) => (
                page_dto(|| self.graph.page_dto_for_bytes(path, &bytes)),
                PageReadSource::HeldBytes,
            ),
            crate::model::Source::HeldAbsent => {
                return Err(StoreError::NotFound(PageReadSource::HeldAbsent))
            }
            crate::model::Source::HeldUnindexed => (
                page_dto(|| self.graph.load_by_validated_path(path)),
                PageReadSource::HeldUnindexed,
            ),
        };
        read.map(|doc| Some((doc, source)))
            .map_err(|error| match error {
                StoreError::NotFound(_) => StoreError::NotFound(source),
                other => other,
            })
    }

    /// A parsed page as a read: its revision and read-only state.
    pub(super) fn page_read(
        &self,
        id: PageId,
        mut doc: PageDto,
        source: PageReadSource,
    ) -> Result<PageRead, StoreError> {
        let rev = FileRev(doc.rev.clone().ok_or(StoreError::NotFound(source))?);
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
            source,
        })
    }

    /// Parse one page file no host holds: the canonical claimant through
    /// the cache (which it reconciles), any other file directly.
    pub(super) fn parse_page(
        &self,
        path: &Path,
        entry: &PageEntry,
        canonical: bool,
    ) -> Result<PageDto, StoreError> {
        page_dto(|| {
            #[cfg(test)]
            if fs::read_to_string(path)
                .is_ok_and(|text| text.contains("__TINE_TEST_PAGE_PARSE_PANIC__"))
            {
                panic!("deterministic test page parser panic");
            }
            if canonical {
                self.graph.load_page(entry).map(Some)
            } else {
                self.graph.load_by_validated_path(path)
            }
        })
    }

    /// The page `id` parsed from `bytes`, as a read of its file holding
    /// them would answer: for a caller that already read the file (with
    /// [`Store::read`]) and writes under that read's revision, so what it
    /// writes is computed from the bytes the revision guards (A-V4b).
    /// [`Store::page`] reads the file itself. Writes nothing. Cost O(bytes
    /// + blocks).
    pub fn page_of(&self, id: &PageId, bytes: &[u8]) -> Result<PageRead, StoreError> {
        let (id, path, _) = self.page_target(id)?;
        let doc = page_dto(|| self.graph.page_dto_for_bytes(&path, bytes))?;
        self.page_read(id, doc, PageReadSource::Disk)
    }

    /// Stage a byte-exact copy of `old`, the bytes of a conflicted page file
    /// `file`, under `logseq/.tine-trash/conflicts/<stamp>__markers__<name>`
    /// (no-replace, fsync'd), before a host operation resolves them (DESIGN
    /// §3.4 pre): a crash leaves the old file plus its copy, or the resolved
    /// file plus the copy, never a lost side. A refused operation leaves the
    /// copy in trash. One write + fsync of `old`.
    pub fn stage_marker_copy(&self, file: &FileId, old: &[u8]) -> Result<FileId, crate::Why> {
        #[cfg(test)]
        super::pause_at_hook(&self.marker_copy_pause);
        self.page_plan(None).stage_marker_file(file, old)
    }

    /// Hold page `id` as a page host does, with no host running: its index
    /// and reads answer the bytes on disk now until the store closes
    /// (A-V4). The caller's later disk edits stay unindexed.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn hold_page(&self, id: &PageId) -> Result<(), StoreError> {
        let _writer = self.writer.lock().unwrap();
        let (id, path, _) = self.page_target(id)?;
        let bytes = fs::read(&path).map_err(StoreError::from_io)?;
        self.graph.hold_unhosted(id.as_str());
        let _ = (self.graph).publish_owned(id.as_str(), Some(Arc::from(bytes)), None);
        self.publish_retired();
        Ok(())
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

/// One page's DTO from `parse`, with lsdoc's deliberate panics and the
/// read's errors mapped to the store's.
fn page_dto(
    parse: impl FnOnce() -> std::io::Result<Option<PageDto>>,
) -> Result<PageDto, StoreError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse))
        .map_err(|panic| {
            let reason = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "page parser panicked".to_owned());
            StoreError::Unparseable(reason)
        })?
        .map_err(|error| {
            if let Some(panicked) = (error.get_ref())
                .and_then(|inner| inner.downcast_ref::<crate::model::ParserPanicked>())
            {
                return StoreError::Unparseable(panicked.to_string());
            }
            if error.kind() == std::io::ErrorKind::InvalidData
                && error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<crate::model::ParseInputTooLarge>())
                    .is_none()
            {
                StoreError::Undecodable
            } else {
                StoreError::from_io(error)
            }
        })?
        .ok_or(StoreError::NotFound(PageReadSource::Disk))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_read_carries_the_selected_source_including_absence() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("pages")).unwrap();
        fs::write(temp.path().join("pages/a.md"), "- disk\n").unwrap();
        let (store, _, _) = Store::open(temp.path(), Default::default()).unwrap();
        store.whole_graph().unwrap();
        store
            .watch
            .core_for_load()
            .deaf
            .store(true, Ordering::Release);
        let id = PageId::from("pages/a.md");
        assert_eq!(store.page(&id).unwrap().source, PageReadSource::Disk);
        store.graph.hold_unhosted(id.as_str());
        assert_eq!(
            store.page(&id).unwrap().source,
            PageReadSource::HeldUnindexed
        );
        fs::remove_file(temp.path().join("pages/a.md")).unwrap();
        assert!(matches!(
            store.page(&id),
            Err(StoreError::NotFound(PageReadSource::HeldUnindexed))
        ));
        store
            .graph
            .publish_owned(id.as_str(), Some(Arc::from(&b"- owner\n"[..])), None)
            .unwrap();
        let read = store.page(&id).unwrap();
        assert_eq!(read.source, PageReadSource::HeldBytes);
        assert_eq!(read.doc.blocks[0].raw, "owner");
        store.graph.publish_owned(id.as_str(), None, None).unwrap();
        assert!(matches!(
            store.page(&id),
            Err(StoreError::NotFound(PageReadSource::HeldAbsent))
        ));
        store.close();
    }
}
