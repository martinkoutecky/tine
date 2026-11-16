//! The restore baseline: what a graph restore compares before and after
//! (`Store` restore), hashing every graph file, asset sidecar and the config.

use std::fs;
use std::time::SystemTime;

use super::{collect_with_revs, stamp, Core, RestoreBaseline, WatchHandle};
use crate::store::{journal_ids_from_entries, ChangeKind, LoadStatus, Origin};

fn collect_restore(core: &Core) -> RestoreBaseline {
    let mut files = collect_with_revs(&core.graph.root, &core.graph.current_config());
    let mut stack = vec![core.graph.assets_path()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some(".tine-restore-recovery")
                {
                    continue;
                }
                stack.push(path);
            } else if kind.is_file() && crate::file_kind::is_asset_sidecar_path(&path) {
                if let Some(value) = stamp(&path) {
                    files.insert(path, value);
                }
            }
        }
    }
    let config = core.graph.root.join("logseq/config.edn");
    if let Some(value) = stamp(&config) {
        files.insert(config, value);
    }
    files
}

impl WatchHandle {
    pub(crate) fn restore_baseline(&self) -> RestoreBaseline {
        collect_restore(&self.core)
    }

    pub(crate) fn publish_restore(&self, before: &RestoreBaseline) -> crate::store::GraphRev {
        let now = collect_restore(&self.core);
        let mut paths: Vec<_> = before.keys().chain(now.keys()).cloned().collect();
        paths.sort();
        paths.dedup();
        let mut files = Vec::new();
        let mut config_changed = false;
        for path in paths {
            let old = before.get(&path);
            let new = now.get(&path);
            if old.and_then(|value| value.rev.as_ref()) == new.and_then(|value| value.rev.as_ref())
            {
                continue;
            }
            let kind = match (old, new) {
                (None, Some(_)) => ChangeKind::Created,
                (Some(_), None) => ChangeKind::Removed,
                _ => ChangeKind::Modified,
            };
            if path == self.core.graph.root.join("logseq/config.edn") {
                config_changed = true;
                let _ = self.core.read_config(&path);
                *self.core.config_stamp.lock().unwrap() = stamp(&path);
            }
            if let Some(id) = self.core.file_id(&path) {
                if id.as_str().starts_with("assets/") {
                    self.core.assets.note_own(&path);
                }
                files.push((id, kind, new.and_then(|value| value.rev.clone())));
            }
        }
        let observed = SystemTime::now();
        let baseline = collect_with_revs(&self.core.graph.root, &self.core.graph.current_config());
        // The restore's complete observation supersedes pre-restore read
        // uncertainty. Keep failures only where this observation still could
        // not read the file; rebuilt page parsing reports invalid content.
        let read_again = |path: &std::path::Path| match baseline.get(path) {
            Some(value) => value.rev.is_some(),
            // An omitted path can also be behind an unreadable directory.
            // Only confirmed absence retires that uncertainty.
            None => fs::symlink_metadata(path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        };
        let mut read_state_changed = false;
        for path in self.core.graph.unsettled_read_paths() {
            if read_again(&path) {
                self.core.graph.note_unsettled_read(&path, None);
                read_state_changed = true;
            }
        }
        let mut refusals = self.core.read_refusals.lock().unwrap();
        let mut errors = self.core.unreadable_dirs.lock().unwrap();
        refusals.retain(|path, _| {
            if read_again(path) {
                errors.remove(path);
                read_state_changed = true;
                false
            } else {
                true
            }
        });
        drop(errors);
        drop(refusals);
        if read_state_changed {
            // Hashing proves byte identity, not parseability. Even a restore
            // of identical bytes must replace the old last-good/error view
            // before its previous uncertainty can stop guarding asset trash.
            self.core.graph.invalidate_cache();
        }
        *self.core.racy.lock().unwrap() = baseline
            .iter()
            .filter(|(_, value)| value.racy_at(observed))
            .map(|(path, _)| path.clone())
            .collect();
        *self.core.snapshot.lock().unwrap() = baseline;
        if files.is_empty() && !config_changed && !read_state_changed {
            self.core.changes.rev()
        } else if matches!(
            *self.core.load.status.lock().unwrap(),
            LoadStatus::Failed(_)
        ) {
            *self.core.journal_ids.lock().unwrap() = journal_ids_from_entries(
                &self.core.graph,
                self.core.graph.list_pages_shared().as_ref(),
            );
            self.core.changes.rev()
        } else {
            self.core
                .changes
                .publish_rebuilt(Origin::Own, files, config_changed)
        }
    }
}
