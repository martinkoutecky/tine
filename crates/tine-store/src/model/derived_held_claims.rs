//! The claims of held pages with no owner publication yet (R-CREATE-TWIN),
//! looked up by name for the create guard. A child of `derived.rs`; the
//! ownership transitions (`derived_owners.rs`) keep it current.
use super::*;

/// A held unindexed page's claim, filed under its day when it has one and
/// under its name (marked dated or not) always, so a lookup reproduces the
/// guard's rule: two days compare as days, anything else by name.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum ClaimSlot {
    Day(i64),
    Name(bool, (bool, String)),
}

impl ClaimSlot {
    fn of(entry: &PageEntry) -> Vec<Self> {
        let name = claimant_key(&(entry.kind, tine_core::refs::page_key(&entry.name)));
        let dated = Self::Name(entry.date_key.is_some(), name);
        entry
            .date_key
            .map(Self::Day)
            .into_iter()
            .chain([dated])
            .collect()
    }

    /// The slots a page of `entry`'s claim collides in.
    fn colliding(entry: &PageEntry) -> [Self; 2] {
        let name = claimant_key(&(entry.kind, tine_core::refs::page_key(&entry.name)));
        match entry.date_key {
            Some(day) => [Self::Day(day), Self::Name(false, name)],
            None => [Self::Name(false, name.clone()), Self::Name(true, name)],
        }
    }
}

/// The held unindexed pages' claims ([`super::HeldPages`]). A page's file is
/// read at most once per hold, respelling or watcher-forwarded change, and
/// not at all when its hold retired an installed row.
#[derive(Default)]
pub(super) struct HeldNames {
    /// Keys still to be named from their files, each with the ticket of its
    /// latest invalidation: a name read under an older ticket is dropped.
    pub(super) unread: HashMap<String, u64>,
    pub(super) ticket: u64,
    pub(super) named: HashMap<String, Vec<ClaimSlot>>,
    pub(super) by_slot: HashMap<ClaimSlot, BTreeSet<String>>,
}

impl HeldNames {
    pub(super) fn forget(&mut self, key: &str) {
        self.unread.remove(key);
        for slot in self.named.remove(key).into_iter().flatten() {
            if let Some(keys) = self.by_slot.get_mut(&slot) {
                keys.remove(key);
                if keys.is_empty() {
                    self.by_slot.remove(&slot);
                }
            }
        }
    }

    pub(super) fn unread(&mut self, key: &str) {
        self.forget(key);
        self.ticket += 1;
        self.unread.insert(key.to_owned(), self.ticket);
    }

    pub(super) fn name(&mut self, key: &str, entry: Option<&PageEntry>) {
        self.forget(key);
        let slots = entry.map(ClaimSlot::of).unwrap_or_default();
        for slot in &slots {
            self.by_slot
                .entry(slot.clone())
                .or_default()
                .insert(key.to_owned());
        }
        self.named.insert(key.to_owned(), slots);
    }
}

impl Graph {
    /// The file of a held page with no owner publication yet (the launch
    /// install) that claims `entry`'s name or day, other than `except`. Such
    /// a page is in no name index, yet its file claims its name: a creation
    /// must see it (R-CREATE-TWIN). A lookup by `entry`'s claim; each such
    /// page's preamble is read once per hold (none when its hold retired an
    /// installed row), and again only after a respelling or a change the
    /// watcher forwarded: the claims are as fresh as the watcher's view,
    /// like every other claimant index the guard reads.
    pub(crate) fn unindexed_held_claimant(
        &self,
        entry: &PageEntry,
        except: Option<&Path>,
    ) -> Option<PathBuf> {
        let unindexed = |key: &str| matches!(self.held.indexed_of(key), Some(None));
        let unread: Vec<(String, u64)> = (self.held.names.lock().unwrap().unread.iter())
            .map(|(key, ticket)| (key.clone(), *ticket))
            .collect();
        for (key, ticket) in unread {
            let held = unindexed(&key);
            let path = self.root.join(self.held.spellings().spelling(&key));
            let other = held.then(|| self.page_entry_in(&path, None)).flatten();
            let mut names = self.held.names.lock().unwrap();
            if names.unread.get(&key) == Some(&ticket) {
                match held {
                    true => names.name(&key, other.as_ref()),
                    false => names.forget(&key),
                }
            }
        }
        let names = self.held.names.lock().unwrap();
        let candidates: Vec<String> = ClaimSlot::colliding(entry)
            .iter()
            .filter_map(|slot| names.by_slot.get(slot))
            .flatten()
            .cloned()
            .collect();
        drop(names);
        candidates.into_iter().find_map(|key| {
            let path = self.root.join(self.held.spellings().spelling(&key));
            let other = path != entry.path && except != Some(path.as_path());
            (other && unindexed(&key)).then_some(path)
        })
    }

    /// The watcher forwarded a change of these held keys' files to their
    /// owners: a claim read from the file before it is stale.
    pub(crate) fn note_held_changed(&self, keys: &[String]) {
        let mut names = self.held.names.lock().unwrap();
        for key in keys {
            if names.named.contains_key(key) || names.unread.contains_key(key) {
                names.unread(key);
            }
        }
    }
}
