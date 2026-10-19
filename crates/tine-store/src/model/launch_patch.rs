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

/// What a launch pass built and observed, up to its install.
pub(super) struct LaunchPass {
    pub(super) built: PageCacheBuild,
    pub(super) stamps: HashMap<PathBuf, crate::watch::Stamp>,
    pub(super) racy: HashSet<PathBuf>,
    pub(super) named: Vec<PageEntry>,
    pub(super) discovery: Vec<(crate::FileId, crate::IoError)>,
    /// The pages a hold withheld from the build when it listed the files.
    pub(super) withheld: HashSet<PathBuf>,
}

/// An installed pass's named listing and file observations.
pub(super) struct Installed {
    pub(super) named: Vec<PageEntry>,
    pub(super) stamps: HashMap<PathBuf, crate::watch::Stamp>,
    pub(super) racy: HashSet<PathBuf>,
}

impl Graph {
    /// Install `pass`, built from the files as of `start` (generation, held
    /// epoch). The forced rebuild (`replace`) installs only at `start`.
    pub(super) fn install_pass(
        &self,
        mut pass: LaunchPass,
        start: (u64, u64),
        replace: bool,
    ) -> Option<Installed> {
        let mut at = start;
        for _ in 0..PATCH_ROUNDS {
            let now = (self.cache_gen.load(Ordering::Acquire), self.held.epoch());
            if replace || now == at {
                let LaunchPass {
                    built,
                    stamps,
                    racy,
                    named,
                    discovery,
                    ..
                } = pass;
                let installed =
                    self.install_built(built, at, replace, Some(&stamps), Some(discovery));
                return installed.then_some(Installed {
                    named,
                    stamps,
                    racy,
                });
            }
            // Identities are decided at this epoch; a later hold, release or
            // respelling moves it again and the next round re-derives.
            let epoch = self.held.epoch();
            let (gen, mut paths) = self.cold_moves_since(start.0)?;
            if epoch != start.1 {
                paths.extend(pass.withheld.iter().cloned());
            }
            self.rederive(&mut pass, &paths)?;
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
            // The listing collapses duplicate journal days before parsing.
            let day = entry.date_key.filter(|_| entry.kind == PageKind::Journal);
            if day.is_some()
                && (pass.named.iter())
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
                    if entry.kind == PageKind::Page {
                        let failure = io::Error::new(error.kind(), error.to_string());
                        pass.discovery
                            .push((crate::FileId::from(rel.clone()), failure.into()));
                    }
                    let failure = PageParseFailure::Unreadable(rel.clone(), error.to_string());
                    pass.built.collect(Err(failure));
                    pass.stamps.insert(path.clone(), stamp);
                    pass.named.push(entry);
                    continue;
                }
            };
            match self.name_from(&entry, &read) {
                Ok(name) => entry.name = name,
                Err(error) => pass
                    .discovery
                    .push((crate::FileId::from(rel.clone()), error.into())),
            }
            pass.named.push(entry.clone());
            let parsed = isolate_page_parse(entry, |entry| Some(parse_page_content(entry, &read)));
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
