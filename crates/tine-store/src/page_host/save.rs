//! The host's save pipeline (STEP2 §3): starting a page save and
//! advancing its guarded phases.
use super::*;
use crate::bench_events::Ev;

/// The one page save in progress and the phase it reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SaveJob {
    pub(super) page: PageKey,
    pub(super) phase: SavePhase,
    pub(super) bytes: Text,
    pub(super) base: Base,
    pub(super) version: u64,
    /// The model's ghost external-write epoch at the rename (tests only).
    #[cfg(test)]
    pub(super) epoch: u64,
    pub(super) removed: Text,
    /// A deletion's marker name and payload basename.
    pub(super) marker: Option<(String, String)>,
    pub(super) trash_durable: bool,
    /// Consecutive trash-move collisions on this job (K6).
    pub(super) collisions: u8,
    /// The bench events' key of this save (0: events off).
    pub(super) bench: u64,
}

/// Consecutive trash-move collisions one save job retries uncounted (K6).
const COLLISIONS_BEFORE_FAILURE: u8 = 3;

impl<F: HostIo> Host<F> {
    pub(super) fn start_save(&mut self, key: &str) -> Disposition {
        if !self.alive || self.job.is_some() {
            return Disposition::Disabled;
        }
        if self.busy(key) {
            return Disposition::Waiting;
        }
        let Some(page) = self.pages.get(key).cloned() else {
            return Disposition::Disabled;
        };
        if page.clean() || page.conflict {
            return Disposition::Disabled;
        }
        // A running rename's order (option 2): its witness wakes the driver.
        if self.order.gated(key) {
            return Disposition::Waiting;
        }
        // A durable pair's held, blocked or cancelled source removal; and
        // any paired removal while draft I/O is down (an unowned host, or
        // one whose drafts are unavailable): the destructive step needs a
        // durable vehicle (model `flushDel`'s `canDraft`). `draft_status`
        // says why; the removal waits for draft I/O to come up.
        if page.buf.is_none() {
            match self.order.removal(key) {
                Removal::Refused => return Disposition::Disabled,
                Removal::Checked { .. } if self.fs.draft_status().unavailable.is_some() => {
                    return Disposition::Disabled
                }
                _ => {}
            }
        } else if self.order.supersedes(key).is_some() {
            // Newer text on a source its pair may still remove: the
            // superseding choice is durable before that text can publish
            // over the removal, or the save waits for draft I/O.
            if self.fs.draft_status().unavailable.is_some() || !self.write_superseding(key, &page) {
                return Disposition::Disabled;
            }
        }
        let bench = crate::bench_events::save_begin(page.version, || self.fs.spelling(key));
        self.job = Some(SaveJob {
            page: key.into(),
            phase: if self.custody.contains_key(key) {
                SavePhase::Custody
            } else {
                Self::first_phase(&page.buf)
            },
            bytes: page.buf.clone(),
            base: page.base.clone(),
            version: page.version,
            #[cfg(test)]
            epoch: 0,
            removed: None,
            marker: None,
            trash_durable: true,
            collisions: 0,
            bench,
        });
        Disposition::Pending
    }

    pub(super) fn first_phase(bytes: &Text) -> SavePhase {
        if bytes.is_none() {
            SavePhase::Check
        } else {
            SavePhase::Temp
        }
    }

    /// Advance the save job one phase.
    pub(super) fn advance_save(&mut self) -> Disposition {
        let Some(job) = &self.job else {
            return Disposition::Disabled;
        };
        if self
            .worker
            .as_ref()
            .is_some_and(|w| w.pages.contains(&job.page))
        {
            return Disposition::Waiting;
        }
        let mut keys = BTreeSet::from([job.page.clone()]);
        // A paired source's removal also reads its receiver: both path locks,
        // in the usual order.
        if let Some(receiver) = self.checked_receiver(job) {
            keys.insert(receiver);
        }
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.advance_save_locked())
    }

    /// The receiver a deletion job checks around its trash step.
    fn checked_receiver(&self, job: &SaveJob) -> Option<PageKey> {
        if job.bytes.is_some() {
            return None;
        }
        match self.order.removal(&job.page) {
            Removal::Checked { receiver, .. } => Some(receiver),
            _ => None,
        }
    }

    /// The pair's receiver check: `None` when `key` is no checked source or
    /// its receiver is certified (`receiver_certified`); else the receiver
    /// and the bytes it has.
    fn receiver_diverged(&mut self, key: &str) -> Result<Option<(PageKey, Text)>, IoFailure> {
        let Removal::Checked { receiver, image } = self.order.removal(key) else {
            return Ok(None);
        };
        let bytes = self.fs.read_page(&receiver)?;
        let certified = Self::receiver_certified(&mut self.fs, &receiver, &bytes, &image);
        Ok((!certified).then_some((receiver, bytes)))
    }

    /// The one answer to "does the receiver hold its certified image?" (the
    /// model's `certified`): read as `bytes`, it has exactly `image`, with a
    /// durability witness for them (`HostIo::durable_page`). Matching bytes
    /// another program wrote, or that a power cut may still revert, certify
    /// nothing. The witness is memory only: at launch none survives, so a
    /// launch-time match certifies nothing (Restore or Retained, never
    /// Cancelled) until this host publishes the receiver again. Every pair
    /// check asks this: the removal checks, Use Disk's choice and phase,
    /// launch's decided phases and a held pair's resumption.
    pub(super) fn receiver_certified(
        fs: &mut F,
        receiver: &str,
        bytes: &Text,
        image: &Text,
    ) -> bool {
        bytes == image && fs.durable_page(receiver)
    }

    /// Surface a diverged receiver as its ordinary read (a held page with
    /// unsaved text conflicts, as for any outside change).
    fn receiver_differs(&mut self, receiver: &str, bytes: Text) {
        if self.pages.contains_key(receiver) {
            self.adopt_read(receiver, bytes);
        }
    }

    /// A creation's twin check before the rename (STEP3 §3.2): a twin fails
    /// the save, as any failure before the rename. Scenario:
    /// Syncthing/Dropbox delivers `c.org` while the user creates `c.md`; two
    /// files would claim one page (contract rows `disk_twin::Twin` and
    /// `check_page_claim::Twin`).
    fn twin_or_marker(
        &mut self,
        job: &mut SaveJob,
        cause: &mut Option<IoFailure>,
    ) -> Option<Outcome> {
        let key = job.page.clone();
        match self.creation_twin(job) {
            Ok(None) => {
                job.phase = if job.bytes.is_none() {
                    SavePhase::Marker
                } else {
                    SavePhase::Rename
                };
                None
            }
            Ok(Some(existing)) => {
                self.events.push(Event::Twin {
                    page: key,
                    existing,
                    version: job.version,
                    saved: false,
                });
                Some(Outcome::Failed)
            }
            Err(e) => {
                *cause = Some(e);
                Some(Outcome::Failed)
            }
        }
    }

    /// The second receiver check, after the source's trash, trash sync and
    /// source-directory sync: the exact receiver settles the pair; anything
    /// else (a receiver replaced meanwhile, or an unreadable one) holds it,
    /// and the operation draft keeps the source's text as custody. Unlike
    /// the pre-check it does not surface the receiver's read (the model's
    /// `publishedPair`): its next observation does, as for any outside change.
    fn post_check(&mut self, key: &str) {
        match self.receiver_diverged(key) {
            Ok(None) => {
                if let Some(name) = self.order.settle(key) {
                    self.repair_op(&name);
                }
            }
            Ok(Some(_)) | Err(_) => self.hold_pair(key, true),
        }
    }

    /// PC-9: a page's durability witness serves only the receiver checks of
    /// an unresolved pair. Released after each publication and whenever an
    /// operation's pairs change (`index_op`), unless such a pair still needs
    /// it; so a page that closes holds one only while its pair is unresolved.
    pub(super) fn release_witness(&mut self, key: &str) {
        if !self.order.needs_witness(key) {
            self.fs.release_durable(key);
        }
    }

    /// `receiver`'s disk bytes are known to be `bytes`: a held pair whose
    /// receiver is exact again resumes.
    pub(super) fn receiver_seen(&mut self, receiver: &str, bytes: &Text) {
        let fs = &mut self.fs;
        let changed = (self.order).receiver_read(receiver, |image| {
            Self::receiver_certified(fs, receiver, bytes, image)
        });
        for name in changed {
            self.repair_op(&name);
            let sources: Vec<PageKey> = (self.order.paired(&name))
                .map(|op| op.pairs.iter().map(|p| p.source.clone()).collect())
                .unwrap_or_default();
            for source in sources {
                self.mark(&source);
            }
        }
    }

    /// A failed receiver check holds the pair (its draft stays custody).
    fn hold_pair(&mut self, key: &str, removed: bool) {
        if let Some(name) = self.order.hold(key, removed) {
            self.repair_op(&name);
            self.mark(key);
        }
    }

    /// `advance_save` with the model's ghost external-write epoch, which a
    /// rename records for the conformance promise (`Published.epoch`).
    #[cfg(test)]
    pub(super) fn advance_save_at(&mut self, epoch: u64) -> Disposition {
        self.test.epoch = epoch;
        let result = self.advance_save();
        self.test.epoch = 0;
        result
    }

    pub(super) fn advance_save_locked(&mut self) -> Disposition {
        let Some(mut job) = self.job.take() else {
            return Disposition::Disabled;
        };
        let key = job.page.clone();
        // The failed step's diagnosis for the page's notice (Q-P2b-3).
        let mut cause = None;
        let result = match job.phase {
            // A failure is saveFail (L442-445): nothing was renamed. After three
            // consecutive filesystem errors the save goes ahead, the error stays
            // visible and the marker is retried at the next launch (R-STORAGE-ERROR).
            // While the custody listing is unknown (REVIEW-2b-r2 V1) a save
            // with no known debt never enters this phase and proceeds: the
            // listing error is the filesystem's report, the sticky
            // custody-unknown notice keeps it visible, and A4 rule 4's barrier
            // covers known markers only. That is R-STORAGE-ERROR.
            SavePhase::Custody => {
                if self.settle(&key) {
                    job.phase = Self::first_phase(&job.bytes);
                    None
                } else {
                    let debt = self.custody.get_mut(&key).unwrap();
                    debt.failures += 1;
                    if debt.failures >= 3 {
                        self.custody_errors.extend(debt.markers.keys().cloned());
                        #[cfg(test)]
                        for payload in debt.markers.values() {
                            self.events.push(Event::CustodyError {
                                page: key.clone(),
                                payload: payload.clone(),
                            });
                        }
                        job.phase = Self::first_phase(&job.bytes);
                        None
                    } else {
                        Some(Outcome::Failed)
                    }
                }
            }
            SavePhase::Temp => match self.fs.page_temp(&key, &job.bytes) {
                Ok(()) => {
                    job.phase = SavePhase::Check;
                    None
                }
                Err(e) => {
                    cause = Some(e);
                    Some(Outcome::Failed)
                }
            },
            SavePhase::Check => match self.fs.read_page(&key) {
                Ok(bytes) if job.base == Base::Known(bytes.clone()) => {
                    // The first receiver check, immediately before the trash
                    // step: the exact certified receiver, or no removal.
                    let checked = if job.bytes.is_none() {
                        self.receiver_diverged(&key)
                    } else {
                        Ok(None)
                    };
                    match checked {
                        Ok(Some((receiver, bytes))) => {
                            if self.allocator_busy() {
                                self.job = Some(job);
                                return Disposition::Waiting;
                            }
                            self.receiver_differs(&receiver, bytes);
                            self.hold_pair(&key, false);
                            self.fs.page_finish(&key);
                            return Disposition::Applied;
                        }
                        Err(e) => {
                            cause = Some(e);
                            Some(Outcome::Failed)
                        }
                        Ok(None) => self.twin_or_marker(&mut job, &mut cause),
                    }
                }
                // A page an order waits for whose path already holds this
                // save's exact state (a record reinstalled at relaunch after
                // its rename or trash step, its directory sync unknown): the
                // ordinary save of the same state re-witnesses durability
                // before the order moves on. A deletion moves nothing then;
                // its post-check still guards the pair.
                Ok(bytes) if bytes == job.bytes && self.order.awaits(&key) => {
                    job.base = Base::Known(bytes);
                    self.twin_or_marker(&mut job, &mut cause)
                }
                Ok(bytes) => {
                    if self.allocator_busy() {
                        self.job = Some(job);
                        return Disposition::Waiting;
                    }
                    self.adopt_read(&key, bytes);
                    // The base-revision guard ended this save without a
                    // publication. Count the refusal, not each idle retry.
                    self.storage_diag
                        .failed
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.fs.page_finish(&key);
                    return Disposition::Applied;
                }
                Err(e) => {
                    cause = Some(e);
                    Some(Outcome::Failed)
                }
            },
            SavePhase::Rename if job.bytes.is_some() => match self.fs.page_rename(&key) {
                Ok(()) => {
                    self.order.renamed(&key, job.version, job.bytes.as_deref());
                    #[cfg(test)]
                    self.events.push(Event::Renamed {
                        page: key.clone(),
                        bytes: job.bytes.clone(),
                        version: job.version,
                    });
                    // Again after the rename: a twin delivered meanwhile is a
                    // notice, never this save's outcome or an undo (Q9). A
                    // failed probe raises nothing; the watcher still sees it.
                    if let Ok(Some(existing)) = self.creation_twin(&job) {
                        self.events.push(Event::Twin {
                            page: key.clone(),
                            existing,
                            version: job.version,
                            saved: true,
                        });
                    }
                    job.phase = SavePhase::DirectorySync;
                    #[cfg(test)]
                    {
                        job.epoch = self.test.epoch;
                    }
                    None
                }
                Err(e) if e.completed => {
                    self.order.renamed(&key, job.version, job.bytes.as_deref());
                    cause = Some(e);
                    #[cfg(test)]
                    self.events.push(Event::Renamed {
                        page: key.clone(),
                        bytes: job.bytes.clone(),
                        version: job.version,
                    });
                    Some(Outcome::Uncertain)
                }
                Err(e) => {
                    cause = Some(e);
                    Some(Outcome::Failed)
                }
            },
            SavePhase::Marker => {
                // One fresh identity names both the marker and its payload.
                let id = uuid::Uuid::new_v4().simple().to_string();
                let filename = std::path::Path::new(&key).file_name().unwrap();
                let marker = drafts::Marker {
                    page: key.clone(),
                    payload: crate::atomic_file::prefixed_name(
                        &format!("{id}__"),
                        &filename.to_string_lossy(),
                    ),
                };
                let name = format!("{id}.tcm");
                match self
                    .fs
                    .custody_write(&name, &drafts::encode_marker(&marker))
                {
                    Ok(()) => {
                        let debt = self.custody.entry(key.clone()).or_default();
                        debt.markers.insert(name.clone(), marker.payload.clone());
                        job.marker = Some((name, marker.payload));
                        job.phase = SavePhase::Rename;
                        None
                    }
                    Err(e) => {
                        cause = Some(e);
                        Some(Outcome::Failed)
                    }
                }
            }
            SavePhase::Rename => {
                let (marker, payload) = job.marker.clone().expect("deletion marker");
                let movement = self.fs.trash_move(&key, &payload);
                job.removed = movement.removed;
                #[cfg(test)]
                if job.removed.is_some() {
                    self.events.push(Event::Removed {
                        page: key.clone(),
                        bytes: job.removed.clone(),
                    });
                }
                match movement.result {
                    Ok(()) => {
                        self.order.renamed(&key, job.version, job.bytes.as_deref());
                        #[cfg(test)]
                        self.events.push(Event::Renamed {
                            page: key.clone(),
                            bytes: job.bytes.clone(),
                            version: job.version,
                        });
                        // A restored deletion moved nothing; its marker owes nothing.
                        job.phase = if job.removed.is_some() {
                            SavePhase::TrashSync
                        } else {
                            SavePhase::DirectorySync
                        };
                        #[cfg(test)]
                        {
                            job.epoch = self.test.epoch;
                        }
                        None
                    }
                    // The occupied target is never adopted: retire the unused
                    // marker (it owes no custody; a failed unlink is
                    // retire-only debt), then retry under a fresh one. The
                    // third consecutive collision fails the save, so the
                    // failure bound counts a persistent one (K6). Scenario:
                    // sync delivery or an external tool keeps the name taken.
                    Err(e) if e.kind == ErrorKind::Collision => {
                        self.retire_marker(&key, &marker, payload);
                        job.marker = None;
                        job.collisions += 1;
                        if job.collisions < COLLISIONS_BEFORE_FAILURE {
                            job.phase = SavePhase::Marker;
                            None
                        } else {
                            cause = Some(e);
                            Some(Outcome::Failed)
                        }
                    }
                    Err(e) if e.completed => {
                        self.order.renamed(&key, job.version, job.bytes.as_deref());
                        cause = Some(e);
                        #[cfg(test)]
                        self.events.push(Event::Renamed {
                            page: key.clone(),
                            bytes: job.bytes.clone(),
                            version: job.version,
                        });
                        Some(Outcome::Uncertain)
                    }
                    Err(e) => {
                        cause = Some(e);
                        Some(Outcome::Failed)
                    }
                }
            }
            SavePhase::TrashSync => {
                let (_, payload) = job.marker.as_ref().expect("deletion marker");
                match self.fs.trash_sync(&key, payload) {
                    Ok(witness) => {
                        job.trash_durable = witness == Witness::Durable;
                        job.phase = SavePhase::DirectorySync;
                        None
                    }
                    Err(e) => {
                        cause = Some(e);
                        Some(Outcome::Uncertain)
                    }
                }
            }
            SavePhase::DirectorySync => match self.fs.page_sync(&key) {
                Ok(witness) => {
                    if witness == Witness::Durable && job.trash_durable && job.bytes.is_none() {
                        crate::bench_events::emit(|| Ev::CustodyComplete {
                            path: self.fs.spelling(&key),
                        });
                        #[cfg(test)]
                        self.events.push(Event::DeleteDurable {
                            page: key.clone(),
                            bytes: job.removed.clone(),
                        });
                    }
                    Some(Outcome::Published)
                }
                Err(e) => {
                    cause = Some(e);
                    Some(Outcome::Uncertain)
                }
            },
        };
        if result.is_some() && job.phase == SavePhase::DirectorySync {
            // Rule 2.5: custody completed before this phase.
            if let Some((marker, payload)) = job.marker.clone() {
                self.retire_marker(&key, &marker, payload);
            }
        }
        if let Some(outcome) = result {
            let counter = match outcome {
                Outcome::Published => &self.storage_diag.published,
                Outcome::Uncertain => &self.storage_diag.uncertain,
                Outcome::Failed => &self.storage_diag.failed,
            };
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.fs.page_finish(&key);
            let mut page = self.pages[&key].clone();
            if outcome == Outcome::Published {
                page.base = Base::Known(job.bytes.clone());
                page.typed = false;
                page.risk = false;
                if job.bytes.is_none() {
                    self.post_check(&key);
                } else {
                    self.receiver_seen(&key, &job.bytes);
                    // A paired source that published newer text: its removal
                    // is superseded, and its operation stops blocking; with
                    // the original text anchored nowhere else (not this
                    // text, no certified receiver) the pair retains it. A
                    // restored preimage cancels its pair.
                    if let Some((_, receiver, image, preimage)) = self.order.disarming(&key) {
                        let certified = match self.fs.read_page(&receiver) {
                            Ok(bytes) => {
                                Self::receiver_certified(&mut self.fs, &receiver, &bytes, &image)
                            }
                            Err(_) => false,
                        };
                        let anchored = job.bytes == preimage || certified;
                        if let Some(name) = self.order.disarm(&key, anchored) {
                            self.repair_op(&name);
                        }
                    }
                    for name in self.order.restored(&key, &job.bytes) {
                        self.repair_op(&name);
                    }
                }
                self.order.published(&key, job.version);
                self.release_witness(&key);
                crate::bench_events::emit(|| Ev::Published {
                    key: job.bench,
                    version: job.version,
                    path: self.fs.spelling(&key),
                    text: job.bytes.clone(),
                });
                self.events.push(Event::Published {
                    page: key.clone(),
                    bytes: job.bytes,
                    version: job.version,
                    #[cfg(test)]
                    epoch: job.epoch,
                });
            } else {
                page.risk = true;
            }
            self.set_page(&key, Some(page));
            self.events.push(Event::SaveOutcome {
                page: key,
                outcome,
                cause,
            });
            Disposition::Applied
        } else {
            self.job = Some(job);
            Disposition::Pending
        }
    }

    /// A creating save's physical twin or logical/unreadable owner (audit
    /// L8-02). Replacements and deletions have no creation slot to check.
    pub(super) fn creation_twin(&mut self, job: &SaveJob) -> Result<Option<String>, IoFailure> {
        if job.base == Base::Known(None) && job.bytes.is_some() {
            if let Some(twin) = self.fs.page_twin(&job.page)? {
                return Ok(Some(twin));
            }
            let source = self
                .order
                .partner(&job.page)
                .map(|key| self.fs.spelling(key));
            self.fs.page_owner(&job.page, &job.bytes, source.as_deref())
        } else {
            Ok(None)
        }
    }
}
