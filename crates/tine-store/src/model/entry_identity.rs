//! Which registered page a path names (B1, REVIEW-AH2-AW1-plan): the one
//! identity rule shared by the held map, the page host's `identify` (Q4)
//! and the watcher's forwarding. Identity is the directory entry, never a
//! spelling. The spelling a key does its I/O through is kept apart, in one
//! table the page host owns ([`Spellings`]).
//!
//! The rule:
//! 1. Ancestors: the path's existing parent directory, canonicalized
//!    (ancestor symlinks, Windows 8.3 names, ancestor case and `\\?\`
//!    prefixes all resolve). A parent outside the canonical root is
//!    [`Identity::Outside`]. The canonicalizations are memoized for one
//!    call only, never kept.
//! 2. Leaf: in that directory, the exact leaf of a registered key's
//!    spelling is that key. A leaf that collides with a registered leaf
//!    (NFC plus a Unicode case fold: [`fold_leaf`], a candidate filter
//!    only, never identity) is a distinct entry only when both names exist,
//!    are listed separately in the directory and `same_file` says they are
//!    different files. Every other collision (the same file, either name
//!    absent, any error) is [`Identity::Unknown`].
//! 3. Unknown is conservative: nothing is installed from disk for it (it
//!    counts as held, A-H2 (G)); for the host it is an alias candidate that
//!    the Q4 transaction checks decide. A leaf with no collision is its own.
//!
//! Native coverage (macOS NFD names, Windows short names, per-directory
//! case flags) runs only on those platforms; elsewhere it is reported as
//! skipped, never counted as proof.

use super::*;
use std::collections::{BTreeSet, HashSet};
use std::ffi::{OsStr, OsString};
use unicode_normalization::UnicodeNormalization;

/// What a path names among the registered keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Identity {
    /// The registered key whose spelling's leaf is this path's, in the
    /// same canonical directory.
    Key(String),
    /// Collides with these registered keys and is not proved distinct.
    /// `alias` is the one Q4's own test accepts as the same entry: the
    /// same file, not listed separately.
    Unknown {
        candidates: Vec<String>,
        alias: Option<String>,
    },
    /// No registered key collides: a page of its own.
    New,
    /// Not under the graph root: never a graph page.
    Outside,
}

/// A file name folded for the collision filter: NFC and a Unicode case
/// fold (`page_key`'s lowercase, then upper and lower again, so `ß` and
/// `SS` meet). Coarser than any filesystem's equivalence is harmless (the
/// filter only selects which collisions are checked); finer would not be.
pub(crate) fn fold_leaf(leaf: &OsStr) -> String {
    let leaf = leaf.to_string_lossy();
    tine_core::refs::page_key(&leaf)
        .to_uppercase()
        .to_lowercase()
        .nfc()
        .collect()
}

/// The page host's spelling table (STEP3 §2, B1): each registered key's
/// current graph-relative spelling, the key moved to each spelling, and
/// the keys by folded leaf. The host's I/O adapter is its only writer
/// (registration and the alias spelling move); the held map reads it.
#[derive(Default)]
pub(crate) struct Spellings(RwLock<Table>);

#[derive(Default)]
struct Table {
    /// A key's spelling when it differs from the key.
    spelled: HashMap<String, String>,
    /// `spelled` inverted (A-R5, D-10: a key is found by lookup).
    respelled: HashMap<String, String>,
    /// Registered keys by their current spelling's folded leaf.
    by_leaf: HashMap<String, BTreeSet<String>>,
}

fn leaf_of(spelling: &str) -> &OsStr {
    Path::new(spelling)
        .file_name()
        .unwrap_or_else(|| OsStr::new(spelling))
}

impl Spellings {
    /// The key's current spelling; a key is its own until `spell` moves it.
    pub(crate) fn spelling(&self, key: &str) -> String {
        let table = self.0.read().unwrap();
        table
            .spelled
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.into())
    }

    /// Register `key` at `spelling`, or move it there (the alias spelling
    /// move, Q4).
    pub(crate) fn spell(&self, key: &str, spelling: &str) {
        let mut table = self.0.write().unwrap();
        let old = table.spelled.remove(key).unwrap_or_else(|| key.into());
        // The reverse entry goes only if it is this key's: a later spelling
        // of another key to `old` keeps its claim.
        if table.respelled.get(&old).is_some_and(|owner| owner == key) {
            table.respelled.remove(&old);
        }
        let fold = fold_leaf(leaf_of(&old));
        if let Some(keys) = table.by_leaf.get_mut(&fold) {
            keys.remove(key);
            if keys.is_empty() {
                table.by_leaf.remove(&fold);
            }
        }
        if key != spelling {
            table.spelled.insert(key.into(), spelling.into());
            table.respelled.insert(spelling.into(), key.into());
        }
        table
            .by_leaf
            .entry(fold_leaf(leaf_of(spelling)))
            .or_default()
            .insert(key.into());
    }

    /// The key moved to `spelling`, if any.
    pub(crate) fn respelled(&self, spelling: &str) -> Option<String> {
        self.0.read().unwrap().respelled.get(spelling).cloned()
    }

    /// The registered keys whose spelling's leaf folds to `fold`, with
    /// their spellings, for which `keep` holds.
    pub(crate) fn candidates(
        &self,
        fold: &str,
        keep: impl Fn(&str) -> bool,
    ) -> Vec<(String, String)> {
        let table = self.0.read().unwrap();
        let Some(keys) = table.by_leaf.get(fold) else {
            return Vec::new();
        };
        keys.iter()
            .filter(|key| keep(key))
            .map(|key| {
                let spelling = table
                    .spelled
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| key.clone());
                (key.clone(), spelling)
            })
            .collect()
    }
}

/// One identification's memo (never kept past the call): canonical
/// directories and directory listings.
#[derive(Default)]
struct Memo {
    root: Option<Option<PathBuf>>,
    dirs: HashMap<PathBuf, Option<PathBuf>>,
    listings: HashMap<PathBuf, Option<HashSet<OsString>>>,
}

impl Memo {
    fn root(&mut self, root: &Path) -> Option<PathBuf> {
        self.root
            .get_or_insert_with(|| canonical_existing_path(root).ok())
            .clone()
    }

    /// `path`'s parent directory, canonical where it exists and lexically
    /// normalised below that; None when it is outside the canonical root.
    fn parent(&mut self, root: &Path, path: &Path) -> Option<PathBuf> {
        let parent = path.parent()?.to_path_buf();
        if let Some(dir) = self.dirs.get(&parent) {
            return dir.clone();
        }
        let dir = self.resolve(root, &parent);
        self.dirs.insert(parent, dir.clone());
        dir
    }

    fn resolve(&mut self, root: &Path, dir: &Path) -> Option<PathBuf> {
        let root = self.root(root)?;
        let (existing, mut resolved) = canonical_existing_ancestor(dir).ok()?;
        for component in dir.strip_prefix(existing).ok()?.components() {
            match component {
                std::path::Component::ParentDir => {
                    if !resolved.pop() {
                        return None;
                    }
                }
                std::path::Component::Normal(name) => resolved.push(name),
                _ => {}
            }
        }
        resolved.starts_with(&root).then_some(resolved)
    }

    fn listing(&mut self, dir: &Path) -> Option<&HashSet<OsString>> {
        self.listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                fs::read_dir(dir)
                    .and_then(|entries| {
                        entries
                            .map(|entry| entry.map(|entry| entry.file_name()))
                            .collect()
                    })
                    .ok()
            })
            .as_ref()
    }
}

/// Whether `path` is spelled lexically under `root` with no `.`/`..`
/// component: the spelling every listing, watcher event and key builds.
fn lexically_inside(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    })
}

impl Graph {
    /// Which of the `registered` keys `path` names (B1; module doc).
    /// `registered(fold)` returns the registered keys whose spelling's leaf
    /// folds to `fold`, with their spellings. With none (no host, nothing
    /// held, or no collision) the answer takes no file system call for a
    /// lexical path under the root.
    pub(crate) fn identify(
        &self,
        path: &Path,
        registered: &dyn Fn(&str) -> Vec<(String, String)>,
    ) -> Identity {
        let Some(leaf) = path.file_name() else {
            return Identity::Outside;
        };
        let found = registered(&fold_leaf(leaf));
        let mut memo = Memo::default();
        if found.is_empty() {
            if lexically_inside(&self.root, path) {
                return Identity::New;
            }
            return match memo.parent(&self.root, path) {
                Some(_) => Identity::New,
                None => Identity::Outside,
            };
        }
        let Some(dir) = memo.parent(&self.root, path) else {
            return Identity::Outside;
        };
        let mut colliders = Vec::new();
        for (key, spelling) in found {
            let candidate = self.root.join(&spelling);
            if memo.parent(&self.root, &candidate).as_ref() != Some(&dir) {
                continue;
            }
            let Some(name) = candidate.file_name() else {
                continue;
            };
            if name == leaf {
                return Identity::Key(key);
            }
            colliders.push((key, name.to_os_string()));
        }
        if colliders.is_empty() {
            return Identity::New;
        }
        let query = dir.join(leaf);
        let mut candidates = Vec::new();
        let mut alias = None;
        for (key, name) in colliders {
            let listed = memo
                .listing(&dir)
                .is_some_and(|names| names.contains(leaf) && names.contains(&name));
            match same_file::is_same_file(&query, dir.join(&name)) {
                // Two entries the directory lists, of different files.
                Ok(false) if listed => continue,
                // Q4's alias: one entry, reached through another spelling.
                Ok(true) if !listed => {
                    alias.get_or_insert_with(|| key.clone());
                }
                _ => {}
            }
            candidates.push(key);
        }
        if candidates.is_empty() {
            Identity::New
        } else {
            Identity::Unknown { candidates, alias }
        }
    }
}

#[cfg(test)]
#[path = "entry_identity_tests.rs"]
mod tests;
