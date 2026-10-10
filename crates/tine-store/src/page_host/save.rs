//! The host's save pipeline (STEP2 §3): starting a page save and
//! advancing its guarded phases.
use super::*;

impl<F: HostIo> Host<F> {
    pub(super) fn start_save(&mut self, key: &str) -> Disposition {
        if !self.alive || self.job.is_some() {
            return Disposition::Disabled;
        }
        if self.busy(key) {
            return Disposition::Waiting;
        }
        let Some(page) = self.pages.get(key) else {
            return Disposition::Disabled;
        };
        if page.clean() || page.conflict {
            return Disposition::Disabled;
        }
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
            epoch: 0,
            removed: None,
            marker: None,
            trash_durable: true,
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

    /// `epoch` is the path's external-write epoch supplied by the event driver;
    /// the 2b adapter will obtain it from the watch boundary, not a file hash.
    pub(super) fn advance_save(&mut self, epoch: u64) -> Disposition {
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
        let keys = BTreeSet::from([job.page.clone()]);
        if self.lacks_locks(&keys) {
            return Disposition::Waiting;
        }
        self.with_locks(&keys, |host| host.advance_save_locked(epoch))
    }

    pub(super) fn advance_save_locked(&mut self, epoch: u64) -> Disposition {
        let Some(mut job) = self.job.take() else {
            return Disposition::Disabled;
        };
        let key = job.page.clone();
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
                        for (marker, payload) in &debt.markers {
                            self.custody_errors.insert(marker.clone());
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
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Check => match self.fs.read_page(&key) {
                Ok(bytes) if job.base == Base::Known(bytes.clone()) => {
                    // A creation's twin check before the rename (STEP3 §3.2):
                    // a twin fails the save, as any failure before the rename.
                    // Scenario: Syncthing/Dropbox delivers `c.org` while the
                    // user creates `c.md`; two files would claim one page
                    // (contract row `page_create_checks::Twin`).
                    match self.creation_twin(&job) {
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
                                page: key.clone(),
                                existing,
                                version: job.version,
                                saved: false,
                            });
                            Some(Outcome::Failed)
                        }
                        Err(_) => Some(Outcome::Failed),
                    }
                }
                Ok(bytes) => {
                    if self.allocator_busy() {
                        self.job = Some(job);
                        return Disposition::Waiting;
                    }
                    self.adopt_read(&key, bytes);
                    self.fs.page_finish(&key);
                    return Disposition::Applied;
                }
                Err(_) => Some(Outcome::Failed),
            },
            SavePhase::Rename if job.bytes.is_some() => match self.fs.page_rename(&key) {
                Ok(()) => {
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
                    job.epoch = epoch;
                    None
                }
                Err(e) if e.completed => {
                    self.events.push(Event::Renamed {
                        page: key.clone(),
                        bytes: job.bytes.clone(),
                        version: job.version,
                    });
                    Some(Outcome::Uncertain)
                }
                Err(_) => Some(Outcome::Failed),
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
                    Err(_) => Some(Outcome::Failed),
                }
            }
            SavePhase::Rename => {
                let (marker, payload) = job.marker.clone().expect("deletion marker");
                let movement = self.fs.trash_move(&key, &payload);
                job.removed = movement.removed;
                if job.removed.is_some() {
                    self.events.push(Event::Removed {
                        page: key.clone(),
                        bytes: job.removed.clone(),
                    });
                }
                match movement.result {
                    Ok(()) => {
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
                        job.epoch = epoch;
                        None
                    }
                    // The occupied target is never adopted: retire the unused
                    // marker (it owes no custody; a failed unlink is
                    // retire-only debt), then retry under a fresh one.
                    Err(e) if e.kind == ErrorKind::Collision => {
                        self.retire_marker(&key, &marker, payload);
                        job.marker = None;
                        job.phase = SavePhase::Marker;
                        None
                    }
                    Err(e) if e.completed => {
                        self.events.push(Event::Renamed {
                            page: key.clone(),
                            bytes: job.bytes.clone(),
                            version: job.version,
                        });
                        Some(Outcome::Uncertain)
                    }
                    Err(_) => Some(Outcome::Failed),
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
                    Err(_) => Some(Outcome::Uncertain),
                }
            }
            SavePhase::DirectorySync => match self.fs.page_sync(&key) {
                Ok(witness) => {
                    if witness == Witness::Durable && job.trash_durable && job.bytes.is_none() {
                        self.events.push(Event::DeleteDurable {
                            page: key.clone(),
                            bytes: job.removed.clone(),
                        });
                    }
                    Some(Outcome::Published)
                }
                Err(_) => Some(Outcome::Uncertain),
            },
        };
        if result.is_some() && job.phase == SavePhase::DirectorySync {
            // Rule 2.5: custody (a)+(b) completed before this phase.
            if let Some((marker, payload)) = job.marker.clone() {
                self.retire_marker(&key, &marker, payload);
            }
        }
        if let Some(outcome) = result {
            self.fs.page_finish(&key);
            let mut page = self.pages[&key].clone();
            if outcome == Outcome::Published {
                page.base = Base::Known(job.bytes.clone());
                page.typed = false;
                page.risk = false;
                self.events.push(Event::Published {
                    page: key.clone(),
                    bytes: job.bytes,
                    version: job.version,
                    epoch: job.epoch,
                });
            } else {
                page.risk = true;
            }
            self.set_page(&key, Some(page));
            self.events.push(Event::SaveOutcome { page: key, outcome });
            Disposition::Applied
        } else {
            self.job = Some(job);
            Disposition::Pending
        }
    }

    /// A creating save's alternate-extension twin (STEP3 §3.2); a save that
    /// replaces a file or deletes one has none to check.
    pub(super) fn creation_twin(&mut self, job: &SaveJob) -> Result<Option<String>, IoFailure> {
        if job.base == Base::Known(None) && job.bytes.is_some() {
            self.fs.page_twin(&job.page)
        } else {
            Ok(None)
        }
    }
}
