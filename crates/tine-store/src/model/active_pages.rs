//! The semantic view shares documents and stable slots with the physical cache.
//! Only affected claim buckets are revisited on an edit, including unchanged
//! siblings promoted or demoted by a claim transition. No filesystem I/O.
use super::*;
use std::collections::BTreeSet;

impl ReadSnapshot {
    pub(super) fn is_active_path(&self, path: &Path) -> bool {
        let Some((entry, _)) = self.physical_pages.iter().next() else {
            return false;
        };
        let Some(root) = entry
            .path
            .ancestors()
            .nth(Path::new(entry.rel_path_str()).components().count())
        else {
            return false;
        };
        let Ok(rel) = path.strip_prefix(root) else {
            return false;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        self.pages.positions.contains_key(rel.as_str())
    }
}

pub(super) fn capture(
    physical: &Arc<Pages>,
    names: &Arc<crate::query::RealPageNames>,
    old: Option<&ReadSnapshot>,
    paths: &[String],
) -> (Arc<Pages>, Vec<String>) {
    let owners = crate::query::RealPageOwners::new(Arc::clone(names));
    let old = old.filter(|old| !paths.is_empty() || Arc::ptr_eq(&old.physical_pages, physical));
    if let Some(old) = old {
        let mut affected: BTreeSet<String> = paths.iter().cloned().collect();
        let mut keys = BTreeSet::new();
        for path in paths {
            for pages in [&old.physical_pages, physical] {
                if let Some((entry, _)) = snapshot_page_by_rel(pages, None, path) {
                    keys.insert(tine_core::refs::page_key(&entry.name));
                }
            }
        }
        let previous = old.real_page_owners();
        for key in keys {
            for names in [&previous, &owners] {
                if let Some(entry) = names.opening_entry(&key) {
                    affected.insert(entry.rel_path_str().to_owned());
                }
            }
        }
        if affected.is_empty() {
            return (Arc::clone(&old.pages), Vec::new());
        }
        let mut active = (*old.pages).clone();
        for path in &affected {
            if let Some(&slot) = active.positions.get(path) {
                active.remove(slot);
            }
            if let Some(&slot) = physical.positions.get(path) {
                if let Some((entry, doc)) = physical.get(slot) {
                    if owners
                        .get(&tine_core::refs::page_key(&entry.name))
                        .is_some_and(|(winner, _)| winner == &entry.path)
                    {
                        active.set(slot, (entry.clone(), Arc::clone(doc)));
                    }
                }
            }
        }
        (Arc::new(active), affected.into_iter().collect())
    } else {
        let mut active = (**physical).clone();
        let mut previous_key = None;
        let mut winner = None;
        for (key, path, _) in names.claims_from("") {
            if previous_key != Some(key) {
                winner = owners.get(key).map(|(path, _)| path);
                previous_key = Some(key);
            }
            if winner != Some(path) {
                let root = physical.iter().next().and_then(|(entry, _)| {
                    entry
                        .path
                        .ancestors()
                        .nth(Path::new(entry.rel_path_str()).components().count())
                });
                if let Some(rel) = root.and_then(|root| path.strip_prefix(root).ok()) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    if let Some(&slot) = physical.positions.get(&rel) {
                        active.remove(slot);
                    }
                }
            }
        }
        (Arc::new(active), paths.to_vec())
    }
}
