//! The launch pass's install when Tine itself moved the cache during the
//! parse (BqbeQ, D-20261019-B6). An own save, a page hold or the host's
//! start used to make the pass's install decline, so the Ready publication
//! built the whole graph a second time (O(P + B), seconds on a large
//! graph). Every move made while no cache is installed is in the cold-move
//! ledger with the paths it moved (`derived_cold.rs`); the pass re-derives
//! just those paths, and the pages a hold had withheld from it, from their
//! files now, and installs. A move the ledger cannot account for path by
//! path (a discarded cache, a config change, a duplicate journal day) still
//! declines, and the Ready publication builds on demand as before.
use super::*;
use std::collections::{BTreeSet, HashSet};
use std::sync::atomic::Ordering;

/// Re-derivation rounds before the pass gives up (a save landing during
/// every round): each round reads only the moved paths.
const PATCH_ROUNDS: usize = 8;

/// Why [`Graph::install_built`] installed nothing.
pub(super) enum Declined {
    /// A cache is installed already (and the build does not replace it).
    CacheBuilt,
    /// A cache move, publication or hold raced the build. Return its rows
    /// unchanged so the caller can patch them.
    Raced(PageCacheBuild),
}

/// What a launch pass built and observed, up to its install.
pub(super) struct LaunchPass {
    pub(super) built: PageCacheBuild,
    pub(super) stamps: HashMap<PathBuf, crate::watch::Stamp>,
    pub(super) racy: HashSet<PathBuf>,
    pub(super) named: Vec<PageEntry>,
    pub(super) discovery: Vec<(crate::FileId, crate::IoError)>,
    /// The pages a hold withheld from the build when it listed the files.
    pub(super) withheld: HashSet<PathBuf>,
    pub(super) identities: entry_identity::Memo,
}

/// An installed pass's file observations.
pub(super) struct Installed {
    pub(super) stamps: HashMap<PathBuf, crate::watch::Stamp>,
    pub(super) racy: HashSet<PathBuf>,
}

impl Graph {
    pub(super) fn warm_page_cache_cancellable(
        &self,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> bool {
        self.warm_page_cache_inner(cancelled, false)
    }

    /// The Settings "Rescan graph" build: the cold-launch build over every
    /// file, replacing a cache that already exists. Returns true only when
    /// the fresh snapshot was installed; false means cancelled, a file moved
    /// during the parse, or a save raced it, and the caller retries.
    pub(crate) fn rebuild_cache_cancellable(&self, cancelled: impl Fn() -> bool + Sync) -> bool {
        self.warm_page_cache_inner(&cancelled, true)
    }

    /// One whole-graph build pass. `replace` is the forced rebuild: the page
    /// listing is re-read from disk, an existing cache does not end the pass,
    /// and the install replaces it. Without it this is the launch warm-up.
    fn warm_page_cache_inner(&self, cancelled: &(impl Fn() -> bool + Sync), replace: bool) -> bool {
        use std::sync::atomic::Ordering;
        if cancelled() {
            return false;
        }
        if !replace && self.cache_built() {
            return true; // already built (e.g. by a query) — nothing to warm
        }
        // Build WITHOUT holding build_lock during the parse, so an on-demand
        // `with_pages` (a user query) can still take build_lock and build
        // itself; if it wins, we discard our work. The parse is the on-demand
        // build's parallel parse (`parse_pages_parallel`). GH #623: it used to be paced
        // (one thread, 2 ms sleep every 24 files), which on a 13k-page graph
        // spent ~1.2 s of a 2.65 s parse asleep, on every launch.
        let gen0 = (self.cache_gen.load(Ordering::Acquire), self.held.epoch());
        #[cfg(test)]
        self.warm_passes.fetch_add(1, Ordering::Relaxed);
        // Phase timings for `Store::diagnostics`: local accumulators, one
        // recorder call per pass (see `launch_diag`).
        use crate::launch_diag as diag;
        let began = std::time::Instant::now();
        let mut pass = diag::PassStats::default();
        // GH #623 / storage spec §5.1 step 1: one read per file. The listing
        // opens no file (an ordinary page's name comes from the bytes read
        // below), each file's stamp is taken before its read, and the
        // revision comes from the same bytes that are parsed. The watcher
        // baseline is then installed from these observations
        // (`LaunchObservations`) instead of a second hash pass.
        let (listed, tracked_only, walk_errors, listing_stamps) =
            page_identity::launch_listing_walk(self);
        // A held page's row is its owner's ((G), `install_built`): the build
        // never reads it, and the watcher rereads its file.
        let mut identities = entry_identity::Memo::default();
        let withheld = self.withheld(&listed, &mut identities);
        let mut stamps = HashMap::with_capacity(listed.len() + tracked_only.len());
        let (mut named, entries): (Vec<_>, Vec<_>) = listed
            .into_iter()
            .partition(|entry| withheld.contains(&entry.path));
        pass.listing_us = diag::micros(began.elapsed());
        pass.entries = entries.len() as u64;
        let mut built = PageCacheBuild::with_capacity(entries.len());
        let mut racy = std::collections::HashSet::new();
        // Successful reads name their own parsed rows. Retain just the
        // physical names for unreadable, unparsed or withheld pages.
        let journals = self.journals_path();
        let mut discovery: Vec<(crate::FileId, crate::IoError)> = Vec::new();
        // Per-file phase time accumulates across the worker threads (atomics, no
        // allocation): read/parse/stat are summed THREAD time, so with N workers
        // they can exceed the parallel wall time reported next to them.
        let clock = diag::PassClock::default();
        let announce_after = self
            .opened_at
            .checked_sub(crate::watch::RACY_WINDOW)
            .unwrap_or(self.opened_at);
        let mut announce = Vec::new();
        // Each file's stamp comes from its directory entry in the listing
        // above (no per-file open on Windows), taken before its read.
        let listing_stamp = |path: &Path| listing_stamps.get(path).cloned();
        let parse_one = |mut e: PageEntry| {
            let phase = std::time::Instant::now();
            let (stamp, observed) = match listing_stamp(&e.path) {
                Some((stamp, observed)) => (Some(stamp), observed),
                None => (None, std::time::SystemTime::now()),
            };
            clock.stat(phase.elapsed());
            let path = e.path.clone();
            let phase = std::time::Instant::now();
            let read = read_parse_input(&e.path);
            let read_len = read.as_ref().ok().map(String::len);
            clock.read(phase.elapsed(), read_len);
            let mut name_failure = None;
            let mut name_only = None;
            let parsed = match read {
                Ok(content) => {
                    if let Err(error) = self.name_from(&mut e, &content) {
                        name_failure = Some(error);
                    }
                    clock.crlf(line_endings::convention(Some(&content)) == "\r\n");
                    let phase = std::time::Instant::now();
                    let parsed =
                        isolate_page_parse(e, |entry| Some(parse_page_content(entry, &content)));
                    clock.parse(phase.elapsed());
                    parsed
                }
                // Removed between the listing and the read (a sync delivery
                // or external editor): not part of this graph state. The
                // launch diff reconciles whatever is there now.
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => {
                    if !e.path.starts_with(&journals) {
                        name_failure = Some(io::Error::new(error.kind(), error.to_string()));
                    }
                    let result = PageParseFailure::unreadable(
                        e.rel_path_str().to_owned(),
                        error,
                        "Tine unreadable file producer=launch-read",
                    );
                    name_only = Some(e);
                    result
                }
            };
            let racy = stamp.as_ref().is_some_and(|stamp| stamp.racy_at(observed));
            // Written since open (with the racy window's slack for coarse
            // timestamps): a client may have read the earlier bytes.
            let since_open = stamp
                .as_ref()
                .and_then(|stamp| stamp.modified())
                .is_some_and(|modified| modified >= announce_after)
                .then(|| {
                    fs::symlink_metadata(&path)
                        .and_then(|meta| meta.created())
                        .is_ok_and(|created| created >= self.opened_at)
                });
            (
                path,
                stamp,
                racy,
                (name_only, name_failure, since_open),
                parsed,
            )
        };
        let mut take = |built: &mut PageCacheBuild,
                        (path, stamp, is_racy, (name_only, name_failure, since_open), parsed): (
            PathBuf,
            Option<crate::watch::Stamp>,
            bool,
            (Option<PageEntry>, Option<io::Error>, Option<bool>),
            PageParseResult,
        )| {
            if let Ok(Some((entry, _, rev))) = &parsed {
                if let Some(created) = since_open {
                    announce.push((path.clone(), created, entry.kind, entry.name.clone()));
                }
                if let Some(stamp) = stamp {
                    let rev = Some(crate::store::FileRev::from(rev.rev.clone()));
                    stamps.insert(path.clone(), stamp.with_rev(rev));
                    if is_racy {
                        racy.insert(path.clone());
                    }
                }
            } else if let Some(stamp) = stamp {
                // Read failed or panicked: the watcher tracks the stamp
                // without a revision, so its next look rereads the file.
                stamps.insert(path.clone(), stamp);
                racy.insert(path.clone());
            }
            if let Some(error) = name_failure {
                discovery.push((crate::FileId::from(self.rel_path(&path)), error.into()));
            }
            named.extend(name_only);
            if let Err(PageParseFailure::Panic(entry, _)) = &parsed {
                named.push(entry.clone());
            }
            built.collect(parsed);
        };
        let mut entries = entries.into_iter();
        if let Some(first) = entries.next() {
            if cancelled() {
                return pass.finish(&self.diag, began, diag::OUTCOME_CANCELLED, false);
            }
            take(&mut built, parse_one(first));
            #[cfg(test)]
            crate::store::pause_at_hook(&self.warm_after_first_page_pause);
        }
        let still_wanted = || !cancelled() && (replace || !self.cache_built());
        let parallel_began = std::time::Instant::now();
        let Some(parsed_chunks) =
            parse_pages_parallel(entries.collect(), &still_wanted, &parse_one)
        else {
            // Either a query built the cache while we parsed, or we were cancelled.
            clock.fold(&mut pass, parallel_began.elapsed(), 0);
            let built_meanwhile = !replace && self.cache_built();
            let outcome = if built_meanwhile {
                diag::OUTCOME_CACHE_ALREADY_BUILT
            } else {
                diag::OUTCOME_CANCELLED
            };
            return pass.finish(&self.diag, began, outcome, built_meanwhile && !cancelled());
        };
        clock.fold(&mut pass, parallel_began.elapsed(), parsed_chunks.len());
        for parsed in parsed_chunks.into_iter().flatten() {
            take(&mut built, parsed);
        }
        drop(take);
        // The watcher tracks sync conflict copies and held pages by stamp
        // without a revision (§5.1); nothing parses them here. A held
        // page's stamp is racy, so the watcher rereads the file (A-V4).
        let phase = std::time::Instant::now();
        for path in named
            .iter()
            .filter(|entry| withheld.contains(&entry.path))
            .map(|entry| &entry.path)
            .chain(tracked_only.iter())
        {
            if let Some((stamp, observed)) = listing_stamp(path) {
                if stamp.racy_at(observed) || withheld.contains(path) {
                    racy.insert(path.clone());
                }
                stamps.insert(path.clone(), stamp);
            }
        }
        // The forced rebuild has no launch diff behind it: a file that changed
        // during its parse discards the pass (a false positive just rebuilds).
        // The launch pass instead hands its pre-read stamps to the watcher,
        // whose launch diff reconciles any such file before Ready.
        if replace {
            // A second listing, not a stamp per file: no file open on Windows.
            let (_, _, _, now) = page_identity::launch_listing_walk(self);
            let changed = built.pages.iter().any(|(entry, _, _)| {
                now.get(&entry.path)
                    .map(|(now, _)| (now.modified(), now.len()))
                    != stamps
                        .get(&entry.path)
                        .map(|then| (then.modified(), then.len()))
            });
            if changed {
                pass.recheck_us = diag::micros(phase.elapsed());
                return pass.finish(&self.diag, began, diag::OUTCOME_FILE_CHANGED, false);
            }
        }
        pass.recheck_us = diag::micros(phase.elapsed());
        if cancelled() {
            return pass.finish(&self.diag, began, diag::OUTCOME_CANCELLED, false);
        }
        // A previously held path may be released without superseding the
        // build. Carry its installed observation as the disk-side fallback;
        // the installer still uses the owner whenever the path is held.
        if replace {
            for path in &withheld {
                if let Some(row) = self.cached_observation(path) {
                    built.pages.push(row);
                }
            }
        }
        discovery.extend(
            walk_errors
                .into_iter()
                .map(|(path, error)| (crate::FileId::from(self.rel_path(&path)), error.into())),
        );
        // Install only if nobody else built it and no Tine save/remove raced our
        // reads (its cache mutation would have no-op'd against the None cache, so
        // its disk write must be folded in by a rebuild — defer to the next
        // on-demand build rather than install a stale snapshot).
        let phase = std::time::Instant::now();
        let _bl = self.build_lock.lock().unwrap();
        let built = LaunchPass {
            built,
            stamps,
            racy,
            named,
            discovery,
            withheld,
            identities,
        };
        let installed = self.install_pass(built, gen0, replace);
        let installed_any = installed.is_some();
        if let Some(Installed { stamps, racy }) = installed.filter(|_| !replace) {
            *self.launch_observations.lock().unwrap() = Some(LaunchObservations {
                stamps,
                racy,
                announce,
            });
        }
        let installed = installed_any;
        pass.install_us = diag::micros(phase.elapsed());
        #[cfg(test)]
        crate::store::pause_at_hook(&self.warm_after_install_pause);
        let outcome = if installed {
            diag::OUTCOME_INSTALLED
        } else {
            diag::OUTCOME_INSTALL_DECLINED
        };
        pass.finish(
            &self.diag,
            began,
            outcome,
            if replace { installed } else { !cancelled() },
        )
    }

    /// Install `pass`, built from the files as of `start` (generation, held
    /// epoch). A forced rebuild may reuse its reads after a hold or release
    /// of unchanged bytes. Other moves supersede those observations.
    pub(super) fn install_pass(
        &self,
        mut pass: LaunchPass,
        start: (u64, u64),
        replace: bool,
    ) -> Option<Installed> {
        let mut at = start;
        for _ in 0..PATCH_ROUNDS {
            let now = (self.cache_gen.load(Ordering::Acquire), self.held.epoch());
            if replace && now.0 != start.0 {
                return None;
            }
            if replace || now == at {
                let built = std::mem::take(&mut pass.built);
                match self.install_built(
                    built,
                    at,
                    replace,
                    Some(&pass.stamps),
                    Some((&pass.named, &mut pass.identities)),
                    Some(&pass.discovery),
                ) {
                    Ok(()) => {
                        return Some(Installed {
                            stamps: pass.stamps,
                            racy: pass.racy,
                        });
                    }
                    Err(Declined::Raced(built))
                        if replace && self.cache_gen.load(Ordering::Acquire) == start.0 =>
                    {
                        pass.built = built;
                        at = (start.0, self.held.epoch());
                        continue;
                    }
                    // A hold, release or save landed while the install ran
                    // unlocked (a page host releasing an own create's hold):
                    // the next round patches it in, as for one during the
                    // parse, instead of a second whole-graph build.
                    Err(Declined::Raced(built)) if !replace => {
                        pass.built = built;
                        continue;
                    }
                    Err(_) => return None,
                }
            }
            // Identities are decided at this epoch; a later hold, release or
            // respelling moves it again and the next round re-derives.
            let epoch = self.held.epoch();
            let (gen, mut paths) = self.cold_moves_since(start.0)?;
            if epoch != start.1 {
                paths.extend(pass.withheld.iter().cloned());
            }
            self.rederive(&mut pass, &paths)?;
            pass.identities = entry_identity::Memo::default();
            at = (gen, epoch);
        }
        None
    }

    /// Replace what `pass` holds for each of `paths` with what its file
    /// gives now, as the pass's own read would (stamp, then one read whose
    /// bytes give the revision, the name and the document). None when a
    /// path cannot be patched in alone.
    fn rederive(&self, pass: &mut LaunchPass, paths: &BTreeSet<PathBuf>) -> Option<()> {
        let config = self.current_config();
        let format = self.current_journal_format();
        let journals = self.journals_path();
        for path in paths {
            // A config change rereads every page (eligibility, formats).
            if *path == self.root.join("logseq/config.edn") {
                return None;
            }
            let rel = self.rel_path(path);
            pass.built.pages.retain(|(entry, _, _)| entry.path != *path);
            pass.built.failures.retain(|failed| *failed != rel);
            pass.built.unreadable.retain(|(failed, _)| *failed != rel);
            pass.discovery.retain(|(id, _)| id.as_str() != rel);
            pass.named.retain(|entry| entry.path != *path);
            pass.stamps.remove(path);
            pass.racy.remove(path);
            let stamp = match fs::symlink_metadata(path) {
                Ok(meta) => crate::watch::stamp_from_metadata(&meta),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return None,
            };
            let Some(stamp) = stamp else {
                continue; // not a regular file: never listed
            };
            if !page_identity::graph_text_watch_relevant(&self.root, path, &config) {
                continue;
            }
            // The watcher rereads every re-derived file once (racy).
            pass.racy.insert(path.clone());
            let listed = page_identity::graph_text_eligible(&self.root, path, &config)
                .then(|| {
                    let formats = (&*format, journals.as_path(), config.file_name_format);
                    page_identity::listed_entry(self, formats, path.clone())
                })
                .flatten();
            let Some(mut entry) = listed else {
                pass.stamps.insert(path.clone(), stamp);
                continue; // tracked by its stamp only (a sync conflict copy)
            };
            // A changed duplicate journal day needs a fresh inventory.
            let day = entry.date_key.filter(|_| entry.kind == PageKind::Journal);
            if day.is_some()
                && pass
                    .named
                    .iter()
                    .chain(pass.built.pages.iter().map(|(entry, _, _)| entry))
                    .any(|other| other.kind == entry.kind && other.date_key == day)
            {
                return None;
            }
            // A held page's row is its owner's: listed, never read.
            if !self.disk_sourced(path) {
                pass.stamps.insert(path.clone(), stamp);
                pass.named.push(entry);
                continue;
            }
            let read = match read_parse_input(path) {
                Ok(content) => content,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    if !path.starts_with(&journals) {
                        let failure = io::Error::new(error.kind(), error.to_string());
                        pass.discovery
                            .push((crate::FileId::from(rel.clone()), failure.into()));
                    }
                    pass.built.collect(PageParseFailure::unreadable(
                        rel.clone(),
                        error,
                        "Tine unreadable file producer=launch-patch",
                    ));
                    pass.stamps.insert(path.clone(), stamp);
                    pass.named.push(entry);
                    continue;
                }
            };
            if let Err(error) = self.name_from(&mut entry, &read) {
                pass.discovery
                    .push((crate::FileId::from(rel.clone()), error.into()));
            }
            let parsed = isolate_page_parse(entry, |entry| Some(parse_page_content(entry, &read)));
            if let Err(PageParseFailure::Panic(entry, _)) = &parsed {
                pass.named.push(entry.clone());
            }
            let rev = match &parsed {
                Ok(Some((_, _, disk))) => Some(crate::store::FileRev::from(disk.rev.clone())),
                _ => None,
            };
            pass.stamps.insert(path.clone(), stamp.with_rev(rev));
            pass.built.collect(parsed);
        }
        Some(())
    }
}
