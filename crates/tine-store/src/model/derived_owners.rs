//! The ownership transitions of [`super`]: attaching the host's spellings,
//! holds, releases, respellings and an owner's publication. Each moves the
//! held map and settles the rows whose authority it moves, in one cache
//! critical section ([`Graph::transition`], REVIEW-3a4). A child of
//! `derived.rs`, so the derived state's fields stay private to the two.

use super::*;

impl Graph {
    /// The running host's spelling table becomes the held keys' (one table,
    /// owned by the host, B1). The caller holds the writer.
    pub(crate) fn attach_spellings(&self, spellings: Arc<Spellings>) {
        let _cache = self.derived.cache.write().unwrap();
        *self.held.spellings.write().unwrap() = spellings;
        self.held.bump();
    }

    /// The one ownership transition (module doc, REVIEW-3a4): in one cache
    /// critical section `change` moves the held map (false: it changed
    /// nothing), then every path whose authority that can move is settled:
    /// `spelled`, the keys' spellings before and after, and every installed
    /// row or error whose leaf collides with one of theirs. The caller holds
    /// the writer and publishes the settled paths
    /// ([`crate::Store::publish_retired`]).
    fn transition(&self, spelled: &[PathBuf], change: impl FnOnce() -> bool) {
        let mut guard = self.derived.cache.write().unwrap();
        if !change() {
            return;
        }
        self.held.bump();
        let affected = self.affected(guard.as_deref(), spelled);
        let mut memo = Memo::default();
        let mut settled = false;
        for path in affected {
            settled |= self.settle(guard.as_mut(), &mut memo, &path);
        }
        // The generation-keyed listings named settled pages as before.
        if settled {
            self.cache_gen.fetch_add(1, Ordering::Release);
        }
        drop(guard);
    }

    /// `spelled` and every cached row or recorded error whose leaf folds as
    /// one of theirs: the identity candidates a transition can move.
    fn affected(&self, pages: Option<&Pages>, spelled: &[PathBuf]) -> BTreeSet<PathBuf> {
        let folds: HashSet<String> = spelled
            .iter()
            .filter_map(|path| path.file_name())
            .map(fold_leaf)
            .collect();
        let mut paths: BTreeSet<PathBuf> = spelled.iter().cloned().collect();
        if let Some(pages) = pages {
            let mut index = self.derived.cache_index.write().unwrap();
            let index = index.get_or_insert_with(|| build_page_cache_index(pages));
            for fold in &folds {
                paths.extend(index.folded(fold).iter().cloned());
            }
        }
        let mut errors: Vec<String> = self.derived.page_index_failures.read().unwrap().clone();
        errors.extend(
            (self.derived.unreadable_pages.read().unwrap().iter())
                .map(|(id, _)| id.as_str().to_owned()),
        );
        errors.extend(
            (self.derived.discovery_errors.read().unwrap().iter())
                .map(|(id, _)| id.as_str().to_owned()),
        );
        paths.extend(
            errors
                .into_iter()
                .map(|rel| self.root.join(rel))
                .filter(|path| {
                    path.file_name()
                        .is_some_and(|leaf| folds.contains(&fold_leaf(leaf)))
                }),
        );
        paths
    }

    /// Bring `path`'s installed state to its authority now ((G)): a disk
    /// row stays; an owner's row comes from its indexed bytes at the key's
    /// spelling (another spelling of it has none); a withheld path has no
    /// row and no errors. True when the installed state moved: the next
    /// capture renames the path from what it names now. An owner's row
    /// already derived from its bytes moves nothing (an open's hold).
    ///
    /// `cache` is copied on write only where a row moves: a settle that
    /// moves nothing leaves it the published generation's, which a launch
    /// checkpoint capture requires (else every capture is "unpublished"
    /// until the next publication).
    fn settle(&self, cache: Option<&mut Arc<Pages>>, memo: &mut Memo, path: &Path) -> bool {
        match self.authority_in(memo, path) {
            Authority::Disk => return false,
            Authority::Owner(key, bytes) => {
                let at = self.root.join(self.held.spellings().spelling(&key));
                if at != path {
                    self.retire_row(cache.map(Arc::make_mut), path, true);
                } else if let Some(cache) = cache {
                    let rev = std::str::from_utf8(&bytes).ok().map(content_rev);
                    if self.derived.disk_revs.read().unwrap().get(path) == rev.as_ref() {
                        return false;
                    } else {
                        let pages = Arc::make_mut(cache);
                        match self.owned_row(&key, &bytes, None) {
                            Some(row) => self.put_row(pages, row),
                            None => {
                                self.retire_row(Some(pages), path, true);
                            }
                        }
                    }
                }
            }
            Authority::Withheld => {
                self.retire_row(cache.map(Arc::make_mut), path, true);
            }
        }
        self.held.retired.lock().unwrap().insert(path.to_path_buf());
        true
    }

    /// Hand `key`'s index to its owner. A new hold settles the page and its
    /// colliding entries (R1, REVIEW-3a4 #3): until its owner publishes,
    /// nothing about them is installed. Holding it again keeps what that
    /// owner already indexed. The caller holds the writer.
    pub(crate) fn hold(&self, key: String) {
        self.hold_indexed(key, None);
    }

    /// [`Self::hold`] for a page open (STEP3 §5, (G) as amended 2026-10-11):
    /// when the file holds the bytes the installed row was derived from, the
    /// hold starts with them indexed as the owner's. The row stays, nothing
    /// is published, and an Open read of the same bytes changes nothing. The
    /// Open read runs later, outside the writer: a file changed in between is
    /// published by that read, which replaces the kept row; until then the
    /// row is the file's previous state, as for any change not yet
    /// reconciled. A file that changed before the hold, or a page not
    /// installed, is held as by [`Self::hold`]. One read of the page file,
    /// under the writer, and none for a key already held.
    pub(crate) fn hold_current(&self, key: String) {
        if self.held.indexed_of(&key).is_some() {
            return; // held already: holding again keeps what it has
        }
        let path = self.root.join(self.held.spellings().spelling(&key));
        let current = self.cached_rev(&path).and_then(|rev| {
            let bytes = std::fs::read(&path).ok()?;
            let text = std::str::from_utf8(&bytes).ok()?;
            (content_rev(text) == rev).then(|| Arc::<[u8]>::from(bytes))
        });
        self.hold_indexed(key, current);
    }

    /// [`Self::hold`] for a claim, the page an operation or a recovered draft
    /// takes on ((G) as amended 2026-10-11, extended from opens to claims):
    /// `observed` is the bytes the host read for the page, which its first
    /// publication carries. When they are the bytes the installed row was
    /// derived from, the hold starts with them indexed as the owner's, so the
    /// row stays and that publication changes nothing. Bytes that differ, no
    /// observation, or a page not installed hold as by [`Self::hold`]. No file
    /// read: the owner's own observation is the comparison.
    pub(crate) fn hold_observed(&self, key: String, observed: Option<Arc<[u8]>>) {
        let path = self.root.join(self.held.spellings().spelling(&key));
        let current = observed.filter(|bytes| {
            let rev = self.cached_rev(&path);
            rev.is_some() && std::str::from_utf8(bytes).ok().map(content_rev) == rev
        });
        self.hold_indexed(key, current);
    }

    fn hold_indexed(&self, key: String, current: Option<Arc<[u8]>>) {
        let path = self.root.join(self.held.spellings().spelling(&key));
        let row = self.with_cached(&path, |row| row.cloned()).flatten();
        let disk = self.cached_rev(&path).map(|rev| DiskObs {
            rev,
            anchored: self.derived.vcs_anchored.read().unwrap().contains(&path),
        });
        self.transition(&[path], || {
            let mut keys = self.held.keys.write().unwrap();
            if keys.contains_key(&key) {
                return false;
            }
            if let (Some((entry, doc)), Some(disk)) = (row, disk) {
                let claim = (entry, doc, disk);
                self.held.claims.lock().unwrap().insert(key.clone(), claim);
            }
            keys.insert(key, current.map(Some)).is_none()
        });
    }

    /// Hold `key` at its own spelling with no host running (test stores).
    #[cfg(any(test, feature = "test-faults"))]
    pub(crate) fn hold_unhosted(&self, key: &str) {
        self.held.spellings().spell(key, key);
        self.hold(key.into());
    }

    /// Return `key` to the watcher (a transition): the paths its watcher
    /// reconcile rereads, its page file and the entries withheld as
    /// colliding with it, when it was held. Its rows stay the owner's until
    /// then. The caller holds the writer.
    pub(crate) fn release(&self, key: &str) -> Vec<PathBuf> {
        let path = self.root.join(self.held.spellings().spelling(key));
        let mut reread = Vec::new();
        self.transition(std::slice::from_ref(&path), || {
            if self.held.keys.write().unwrap().remove(key).is_none() {
                return false;
            }
            self.held.claims.lock().unwrap().remove(key);
            let withheld = self.held.withheld.lock().unwrap().remove(key);
            reread.push(path.clone());
            reread.extend(withheld.into_iter().flatten());
            true
        });
        reread
    }

    /// Return every held key to the watcher and detach the host's spelling
    /// table (a transition): the paths the watcher rereads. The caller
    /// holds the writer.
    pub(crate) fn release_all(&self) -> Vec<PathBuf> {
        let mut reread = Vec::new();
        self.transition(&[], || {
            let keys: Vec<String> = (self.held.keys.write().unwrap().drain())
                .map(|(key, _)| key)
                .collect();
            self.held.claims.lock().unwrap().clear();
            let spellings = std::mem::take(&mut *self.held.spellings.write().unwrap());
            reread.extend(
                keys.iter()
                    .map(|key| self.root.join(spellings.spelling(key))),
            );
            reread.extend(
                self.held
                    .withheld
                    .lock()
                    .unwrap()
                    .drain()
                    .flat_map(|(_, paths)| paths),
            );
            true
        });
        reread
    }

    /// The alias spelling move gave held `key` a new spelling (Q4), a
    /// transition: the rows at `from` go and the owner's bytes are
    /// installed at the new spelling. The caller holds the writer.
    pub(crate) fn respelled(&self, key: &str, from: &Path) {
        let to = self.root.join(self.held.spellings().spelling(key));
        self.transition(&[from.to_path_buf(), to], || {
            self.held.indexed_of(key).is_some()
        });
    }

    /// The owner's publication of held `key` (R1): its bytes (None: no
    /// file) become the page's index, and its row is derived from them, in
    /// one critical section. `doc` is the Document the bytes were
    /// serialized from, when the owner has it (R8, E104). Returns the entry
    /// named from the bytes, None when the page has no row. `Err` when no
    /// host holds `key`: the caller publishes as for disk.
    #[allow(clippy::result_unit_err)]
    pub(crate) fn publish_owned(
        &self,
        key: &str,
        bytes: Option<Arc<[u8]>>,
        doc: Option<&Document>,
    ) -> Result<Option<PageEntry>, ()> {
        let path = self.root.join(self.held.spellings().spelling(key));
        let rev = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(content_rev);
        // Bytes the installed row was already derived from change nothing
        // but the index (an owner's observation of its own save).
        let mut row = None;
        let mut claim = self.held.claims.lock().unwrap().remove(key);
        if self.cache_built() && self.cached_rev(&path) != rev {
            row = bytes
                .as_deref()
                .and_then(|bytes| self.claimed_row(key, bytes, doc, claim.take()));
        }
        let mut guard = self.derived.cache.write().unwrap();
        {
            let mut keys = self.held.keys.write().unwrap();
            let Some(indexed) = keys.get_mut(key) else {
                return Err(());
            };
            *indexed = Some(bytes.clone());
        }
        let unchanged = self.derived.disk_revs.read().unwrap().get(&path).cloned() == rev;
        if !unchanged || guard.is_none() {
            let before = self.cache_gen.load(Ordering::Acquire);
            let mut renamed = true;
            if let Some(cache) = guard.as_mut() {
                // Derived before the lock unless the installed row changed
                // since (the writer excludes that; this keeps it exact).
                if row.is_none() {
                    row = bytes
                        .as_deref()
                        .and_then(|bytes| self.claimed_row(key, bytes, doc, claim.take()));
                }
                match row.take() {
                    Some(row) => {
                        let pages = Arc::make_mut(cache);
                        let slot = self.cached_page_index_for_path(pages, &path);
                        renamed = !slot
                            .and_then(|slot| pages.get(slot))
                            .is_some_and(|(old, _)| {
                                old.kind == row.0.kind && old.name == row.0.name
                            });
                        self.put_row(pages, row);
                    }
                    // The owner's absence: settled like a transition.
                    None => {
                        self.settle(Some(cache), &mut Memo::default(), &path);
                    }
                }
            }
            self.cache_gen.fetch_add(1, Ordering::Release);
            // The memos list the page's earlier name: still valid only when
            // this publication named it as before.
            if !renamed {
                self.retag_name_memos(before, before + 1);
            }
        }
        drop(guard);
        self.recent_writes.lock().unwrap().remove(&path);
        Ok(bytes.and_then(|bytes| self.owned_entry(key, &bytes)))
    }
}
