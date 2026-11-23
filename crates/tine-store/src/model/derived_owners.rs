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
    /// critical section `change` moves the held map and returns the paths
    /// it released or respelled (None: no change). Every affected path
    /// is then settled:
    /// `spelled`, the keys' spellings before and after, and every installed
    /// row or error whose leaf collides with one of theirs. The caller holds
    /// the writer and publishes the settled paths
    /// ([`crate::Store::publish_retired`]).
    fn transition(&self, change: impl FnOnce() -> Option<Vec<PathBuf>>) {
        let mut guard = self.derived.cache.write().unwrap();
        let Some(spelled) = change() else { return };
        self.held.bump();
        let generation = self.cache_generation();
        let affected = self.affected(&guard, &spelled, generation);
        let mut memo = Memo::default();
        let mut settled = false;
        let mut renamed = false;
        for path in &affected {
            let authority = self.authority_in(&mut memo, path);
            let (rows, names) = self.settle(&mut guard, authority, path, generation);
            settled |= rows;
            renamed |= names;
        }
        // The generation-keyed listings named settled pages as before.
        if settled {
            let paths: Vec<&Path> = affected.iter().map(PathBuf::as_path).collect();
            self.move_generation(guard.pages.is_none(), &paths);
            if let Some((gen, _)) = guard
                .inventory
                .as_mut()
                .filter(|(gen, _)| *gen == generation)
            {
                *gen = generation + 1;
            }
        }
        if renamed {
            self.forget_name_memos();
        }
    }

    /// `spelled` and every cached row or recorded error whose leaf folds as
    /// one of theirs: the identity candidates a transition can move.
    fn affected(
        &self,
        cache: &InstalledCache,
        spelled: &[PathBuf],
        generation: u64,
    ) -> BTreeSet<PathBuf> {
        let folds: HashSet<String> = spelled
            .iter()
            .filter_map(|path| path.file_name())
            .map(fold_leaf)
            .collect();
        let mut paths: BTreeSet<PathBuf> = spelled.iter().cloned().collect();
        if let Some((_, inventory)) = cache
            .inventory
            .as_ref()
            .filter(|(gen, _)| *gen == generation)
        {
            for fold in &folds {
                paths.extend(inventory.affected(fold).cloned());
            }
        }
        if let Some(pages) = cache.pages.as_deref() {
            let mut index = self.derived.cache_index.write().unwrap();
            let index = index.get_or_insert_with(|| build_page_cache_index(pages));
            for fold in &folds {
                paths.extend(
                    index
                        .folded(fold)
                        .iter()
                        .filter_map(|slot| pages.get(*slot).map(|(entry, _)| entry.path.clone())),
                );
            }
        }
        let mut errors: Vec<String> = self.derived.page_index_failures.read().unwrap().clone();
        errors.extend(
            (self.derived.unreadable_pages.read().unwrap().iter())
                .map(|(id, _)| id.as_str().to_owned()),
        );
        errors.extend((cache.discovery.iter()).map(|(id, _)| id.as_str().to_owned()));
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
    fn settle(
        &self,
        cache: &mut InstalledCache,
        authority: Authority,
        path: &Path,
        generation: u64,
    ) -> (bool, bool) {
        let (renamed, absent) = cache
            .inventory
            .as_mut()
            .filter(|(gen, _)| *gen == generation)
            .map_or((false, false), |(_, inventory)| {
                inventory.settle(path, &authority)
            });
        match authority {
            Authority::Disk if !absent => return (false, renamed),
            // Confirmed absent: the path's S(P) ends.
            Authority::Disk => {
                self.retire_row(cache, path, true);
                self.end_path(path);
            }
            // Withheld (uncertain identity, a twin a hold withholds, an
            // unindexed hold): the row's S(P) is kept as the path's claim,
            // which its next publication or reconcile compares against (D5).
            Authority::Withheld => {
                self.claim_row(cache, path);
                self.retire_row(cache, path, true);
            }
            Authority::Owner(key, bytes) => {
                let at = self.root.join(self.held.spellings().spelling(&key));
                if at != path {
                    self.claim_row(cache, path);
                    self.retire_row(cache, path, true);
                } else if let Some(pages) = cache.pages.as_mut() {
                    let rev = std::str::from_utf8(&bytes).ok().map(content_rev);
                    // Undecodable bytes have no revision and no row: still settled.
                    if rev.is_some()
                        && self.derived.disk_revs.read().unwrap().get(path) == rev.as_ref()
                    {
                        return (false, renamed);
                    } else {
                        let prior = self.prior_in(pages, path);
                        match self.owned_row(&key, &bytes, None, prior.as_ref()) {
                            Ok(Some(row)) => self.put_row(Arc::make_mut(pages), row),
                            Ok(None) => {
                                self.retire_row(cache, path, true);
                            }
                            Err(error) => {
                                self.retire_row(cache, path, true);
                                self.list_held_row_failure(path, &error);
                            }
                        }
                    }
                }
            }
        }
        self.held.retired.lock().unwrap().insert(path.to_path_buf());
        (true, renamed)
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
        let claim = self.cached_observation(&path);
        self.transition(|| {
            let mut keys = self.held.keys.write().unwrap();
            if keys.contains_key(&key) {
                return None;
            }
            if let Some(claim) = claim {
                self.held.claims.lock().unwrap().insert(path.clone(), claim);
            }
            keys.insert(key, current.map(Some));
            Some(vec![path])
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
        self.transition(|| {
            self.held.keys.write().unwrap().remove(key)?;
            // The path's claim stays for the reread's reconcile, which
            // compares against it (T6, M2) and frees it.
            let withheld = self.held.withheld.lock().unwrap().remove(key);
            reread.push(path.clone());
            reread.extend(withheld.into_iter().flatten());
            Some(vec![path])
        });
        reread
    }

    /// Return every held key to the watcher and detach the host's spelling
    /// table (a transition): the paths the watcher rereads. The caller
    /// holds the writer.
    pub(crate) fn release_all(&self) -> Vec<PathBuf> {
        let mut reread = Vec::new();
        self.transition(|| {
            let keys: Vec<String> = (self.held.keys.write().unwrap().drain())
                .map(|(key, _)| key)
                .collect();
            // Claims stay for the rereads' reconciles (T6), by path.
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
            Some(reread.clone())
        });
        reread
    }

    /// The alias spelling move gave held `key` a new spelling (Q4), a
    /// transition: the rows at `from` go and the owner's bytes are
    /// installed at the new spelling. The caller holds the writer and
    /// confirmed it is one file (the host's case-only spelling move and its
    /// launch recovery): its S(P) moves to the new spelling with its keys,
    /// W advances there, and nothing stays at `from` (S5).
    pub(crate) fn respelled(&self, key: &str, from: &Path) {
        let to = self.root.join(self.held.spellings().spelling(key));
        let moved = (to != from)
            .then(|| {
                self.cached_observation(from)
                    .or_else(|| self.held.claims.lock().unwrap().get(from).cloned())
            })
            .flatten();
        self.transition(|| {
            self.held.indexed_of(key)?;
            if let Some(row) = &moved {
                self.ids.respelled(from, &to, &row.1);
                let mut claims = self.held.claims.lock().unwrap();
                claims.insert(to.clone(), row.clone());
            }
            Some(vec![from.to_path_buf(), to.clone()])
        });
        if moved.is_some() {
            // Nothing stays at the old spelling: the file lives at `to`.
            self.held.claims.lock().unwrap().remove(from);
        }
    }

    /// The owner's publication of held `key` (R1): its bytes (None: no
    /// file) become the page's index, and its row is derived from them, in
    /// one critical section. `saved` is the serializer's keyed parse of a
    /// save with the bytes it parsed (D4, E104): when those are these bytes,
    /// it is installed whole, also
    /// at an unchanged revision when it moves a key (A1). Bytes it did not
    /// save keep the path's prior keys when they hold its outline. Before the
    /// cache exists, the answer is kept as the path's seed, which the build
    /// adopts (S6, A7). Returns the entry named from the bytes (the derived
    /// row's, when one was derived; None when the page has no row) and
    /// whether the page's keyed outline changed at an unchanged revision.
    /// `Err` when no host holds `key`: the caller publishes as for disk.
    #[allow(clippy::result_unit_err)]
    pub(crate) fn publish_owned(
        &self,
        key: &str,
        bytes: Option<Arc<[u8]>>,
        saved: Option<page_ids::SaveHandoff>,
    ) -> Result<OwnerPublished, ()> {
        let doc = saved.and_then(|saved| saved.of(bytes.as_ref()));
        let path = self.root.join(self.held.spellings().spelling(key));
        let rev = bytes
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(content_rev);
        // Bytes the installed row was already derived from change nothing
        // but the index (an owner's observation of its own save), unless the
        // serializer's document moves a key among them.
        let mut row = None;
        let mut named = None;
        let mut claim = self.held.claims.lock().unwrap().remove(&path);
        // The row the bytes give against `prior` (the claim is taken only
        // when there are bytes).
        let derive = |claim: &mut Option<Row>, prior: Option<&Arc<Document>>| {
            let bytes = bytes.as_deref()?;
            (self.claimed_row(key, bytes, doc.clone(), claim.take(), prior)).ok()?
        };
        let built = self.cache_built();
        if built && (self.cached_rev(&path) != rev || doc.is_some()) {
            let current =
                (self.with_cached(&path, |row| row.map(|(_, doc)| Arc::clone(doc)))).flatten();
            row = derive(&mut claim, current.as_ref());
        } else if !built && (doc.is_some() || claim.is_some()) {
            // The seed: this publication's S(P), for the build (S6).
            if let Some(seed) = derive(&mut claim, None) {
                self.held.claims.lock().unwrap().insert(path.clone(), seed);
            }
        }
        let mut guard = self.derived.cache.write().unwrap();
        #[cfg(test)]
        crate::store::pause_at_hook(&self.cache_publish_pause);
        {
            let mut keys = self.held.keys.write().unwrap();
            let Some(indexed) = keys.get_mut(key) else {
                if let Some(claim) = claim {
                    self.held.claims.lock().unwrap().insert(path.clone(), claim);
                }
                return Err(());
            };
            let same = match (indexed.as_ref(), &bytes) {
                (Some(Some(before)), Some(after)) => Arc::ptr_eq(before, after),
                (Some(None), None) => true,
                _ => false,
            };
            if !same {
                self.held.bump();
            }
            *indexed = Some(bytes.clone());
        }
        // Bytes with no revision (undecodable) have no row to be unchanged from.
        let unchanged = (rev.is_some() || bytes.is_none())
            && self.derived.disk_revs.read().unwrap().get(&path).cloned() == rev;
        let mut identity = false;
        if let (true, Some(pages)) = (unchanged, guard.pages.as_ref()) {
            // An unchanged revision: only a key move installs (A1).
            let slot = self.cached_page_index_for_path(pages, &path);
            let current = slot.and_then(|slot| pages.get(slot)).map(|(_, doc)| doc);
            identity = row.as_ref().is_some_and(|(_, doc, _)| {
                current.is_some_and(|current| !Arc::ptr_eq(current, doc))
            });
            if !identity {
                row = None;
            }
        }
        if !unchanged || identity || guard.pages.is_none() {
            let before = self.cache_gen.load(Ordering::Acquire);
            let mut renamed = true;
            if let Some(cache) = guard.pages.as_mut() {
                // Derived before the lock unless the installed row changed
                // since (the writer excludes that; this keeps it exact).
                if row.is_none() {
                    row = derive(&mut claim, self.prior_in(cache, &path).as_ref());
                }
                match row.take() {
                    Some(row) => {
                        named = Some(row.0.clone());
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
                        self.settle(&mut guard, self.authority(&path), &path, before);
                    }
                }
            }
            self.move_generation(guard.pages.is_none(), &[&path]);
            // The memos list the page's earlier name: still valid only when
            // this publication named it as before.
            if !renamed {
                self.retag_name_memos(before, before + 1);
            }
        }
        drop(guard);
        self.recent_writes.lock().unwrap().remove(&path);
        // A derived row's entry is these bytes' naming: not named again.
        let named = named.or_else(|| row.map(|(entry, _, _)| entry));
        Ok(OwnerPublished {
            entry: bytes.and_then(|bytes| named.or_else(|| self.owned_entry(key, &bytes))),
            identity,
        })
    }
}

/// What an owner's publication installed ([`Graph::publish_owned`]).
pub(crate) struct OwnerPublished {
    /// The entry named from the bytes; None when the page has no row.
    pub(crate) entry: Option<PageEntry>,
    /// The page's keyed outline changed at an unchanged revision (A1): a
    /// page change, though no byte changed.
    pub(crate) identity: bool,
}
