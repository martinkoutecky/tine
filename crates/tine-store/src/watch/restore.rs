//! The restore baseline: what a graph restore compares before and after
//! (`Store` restore), hashing every graph file, asset sidecar and the config.

use std::fs;
use std::time::SystemTime;

use super::{collect_with_revs, stamp, Core, RestoreBaseline, WatchHandle};
use crate::store::{journal_ids_from_entries, ChangeKind, LoadStatus, Origin};

fn collect_restore(core: &Core) -> RestoreBaseline {
    let mut files = collect_with_revs(&core.dirs.read().unwrap(), &core.graph.current_config());
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
        let baseline = collect_with_revs(
            &self.core.dirs.read().unwrap(),
            &self.core.graph.current_config(),
        );
        *self.core.racy.lock().unwrap() = baseline
            .iter()
            .filter(|(_, value)| value.racy_at(observed))
            .map(|(path, _)| path.clone())
            .collect();
        *self.core.snapshot.lock().unwrap() = baseline;
        if files.is_empty() && !config_changed {
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
                .publish(Origin::Own, files, config_changed, Vec::new())
        }
    }
}
