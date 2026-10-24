//! Device-local namespaces follow a validated relocation, never graph content.
use std::path::{Path, PathBuf};
use std::{fs, io};

#[cfg(test)]
thread_local! { static CENSUS_READ_DIRS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
fn census_entries(dir: &Path) -> io::Result<fs::ReadDir> {
    #[cfg(test)]
    CENSUS_READ_DIRS.with(|count| count.set(count.get() + 1));
    fs::read_dir(dir)
}

/// Pure validator is tested on the host; only iOS may authorize it in production.
pub(crate) fn is_container_rebase(old: &Path, new: &Path) -> bool {
    fn split(path: &Path) -> Option<(Vec<String>, uuid::Uuid, Vec<String>)> {
        if !path.is_absolute() {
            return None;
        }
        // Reject traversal before Path::components can normalize it.
        if path
            .to_str()?
            .split('/')
            .any(|part| part == "." || part == "..")
        {
            return None;
        }
        let parts = path
            .components()
            .map(|part| part.as_os_str().to_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()?;
        let at = parts
            .windows(3)
            .position(|p| p == ["Containers", "Data", "Application"])?;
        let id = parts.get(at + 3)?;
        if id.len() != 36 || parts.get(at + 4)? != "Documents" {
            return None;
        }
        let uuid = uuid::Uuid::parse_str(id).ok()?;
        Some((parts[..at].to_vec(), uuid, parts[at + 5..].to_vec()))
    }
    match (split(old), split(new)) {
        (Some((a, id, rel)), Some((b, next, other))) => a == b && id != next && rel == other,
        _ => false,
    }
}

pub(crate) fn backup_root_matches(old: &str, new: &Path) -> bool {
    if old == new.to_string_lossy() {
        return true;
    }
    #[cfg(target_os = "ios")]
    {
        is_container_rebase(Path::new(old), new)
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows",
        target_os = "android"
    ))]
    {
        false
    }
}

/// An unreadable census is never empty. Only the two known empty directories
/// are skeletons; temporary files and unknown names also retain custody.
pub(crate) fn census_empty(dir: &Path) -> io::Result<bool> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() || meta.is_symlink() {
        return Ok(false);
    }
    for entry in census_entries(dir)? {
        let entry = entry?;
        if !matches!(
            entry.file_name().to_str(),
            Some("unreadable" | "trash-custody")
        ) || !entry.file_type()?.is_dir()
        {
            return Ok(false);
        }
        if census_entries(&entry.path())?.next().transpose()?.is_some() {
            return Ok(false);
        }
    }
    Ok(true)
}

fn remove_skeleton(dir: &Path) -> io::Result<()> {
    if !dir.try_exists()? {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        fs::remove_dir(entry?.path())?;
    }
    fs::remove_dir(dir)
}

fn rename_empty_target(old: &Path, new: &Path, drafts: bool) -> io::Result<()> {
    if !old.try_exists()? {
        return Ok(());
    }
    if new.try_exists()? {
        if drafts {
            if !census_empty(new)? {
                return Ok(());
            }
            remove_skeleton(new)?;
        } else {
            if fs::read_dir(new)?.next().transpose()?.is_some() {
                return Ok(());
            }
            fs::remove_dir(new)?;
        }
    }
    // Existing audited move: no replacement, with WRITE_THROUGH on Windows,
    // where private-directory sync has no OS flush primitive.
    crate::device_io::move_file_noreplace(old, new)?;
    crate::device_io::sync_private_directory(new.parent().expect("namespace parent"))
}

/// Each independent rename can be retried after a partial failure. No vehicle,
/// v1 draft or sealed archive is decoded or rewritten here.
pub(crate) fn relocate_namespaces(data: &Path, old: &Path, new: &Path) {
    let a = crate::settings::host_graph_id(old);
    let b = crate::settings::host_graph_id(new);
    let ba = crate::backup::root_backup_id(old);
    let bb = crate::backup::root_backup_id(new);
    let mut steps = vec![
        (
            data.join("drafts-v2").join(&a),
            data.join("drafts-v2").join(&b),
            true,
        ),
        (
            data.join("backups").join(&ba),
            data.join("backups").join(&bb),
            false,
        ),
        (
            data.join("backups").join(format!("{ba}.zips")),
            data.join("backups").join(format!("{bb}.zips")),
            false,
        ),
        (
            data.join("concord-ledger-og").join(ba),
            data.join("concord-ledger-og").join(bb),
            false,
        ),
        (
            data.join("drafts").join(format!("{a}.v1.json")),
            data.join("drafts").join(format!("{b}.v1.json")),
            false,
        ),
    ];
    for suffix in [".json", "-workspaces.json", "-notices.json"] {
        steps.push((
            data.join("sessions").join(format!("{a}{suffix}")),
            data.join("sessions").join(format!("{b}{suffix}")),
            false,
        ));
    }
    for (old, new, drafts) in steps {
        if let Err(e) = rename_empty_target(&old, &new, drafts) {
            crate::debug::diag_private("graph-relocation-step-failed", e.to_string());
        }
    }
    let checkpoint = data.join("launch-checkpoints").join(format!("{a}.bin"));
    if let Err(e) = fs::remove_file(checkpoint) {
        if e.kind() != io::ErrorKind::NotFound {
            crate::debug::diag_private("graph-relocation-step-failed", e.to_string());
        }
    }
    if let Err(e) =
        crate::settings::update_settings_at(&crate::settings::settings_path_in(data), |json| {
            let old = old.to_string_lossy();
            let new = new.to_string_lossy();
            if let Some(graphs) = json.get_mut("known_graphs").and_then(|v| v.as_array_mut()) {
                for graph in graphs {
                    if graph["path"].as_str() == Some(old.as_ref()) {
                        graph["path"] = serde_json::json!(new);
                    }
                }
            }
            if json["last_graph_path"].as_str() == Some(old.as_ref()) {
                json["last_graph_path"] = serde_json::json!(new);
            }
            if let Some(approvals) = json
                .get_mut("external_assets_approvals")
                .and_then(|v| v.as_object_mut())
            {
                if let Some(approval) = approvals.remove(old.as_ref()) {
                    approvals.entry(new.into_owned()).or_insert(approval);
                }
            }
        })
    {
        crate::debug::diag_private("graph-relocation-step-failed", e);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Orphan {
    pub(crate) id: String,
    pub(crate) old_root: Option<String>,
}

pub(crate) fn find_orphans(data: &Path, root: &Path, known: &[String]) -> Vec<Orphan> {
    let current = crate::settings::host_graph_id(root);
    let roots: std::collections::HashMap<_, _> = known
        .iter()
        .map(|root| (crate::settings::host_graph_id(Path::new(root)), root))
        .collect();
    let Ok(entries) = census_entries(&data.join("drafts-v2")) else {
        return vec![];
    };
    let mut found = vec![];
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if id == current
            || !valid_graph_id(&id)
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        if census_empty(&entry.path()).unwrap_or(false) {
            continue;
        }
        let old_root = roots.get(&id).copied();
        let missing =
            old_root.is_none_or(
                |old| match tine_store::Store::canonical_root(Path::new(old)) {
                    Ok(canonical) => canonical != Path::new(old),
                    Err(_) => true,
                },
            );
        if missing {
            found.push(Orphan {
                id,
                old_root: old_root.cloned(),
            });
        }
    }
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

fn valid_graph_id(id: &str) -> bool {
    id.rsplit_once('-').is_some_and(|(label, hash)| {
        !label.is_empty()
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && hash.len() == 16
            && hash
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

/// Unknown old roots still have their original FNV owner key in the existing
/// graph id. Validation prevents a caller supplying a path or arbitrary lock.
pub(crate) fn orphan_owner_lock(id: &str) -> Result<Option<crate::owner::lock::OwnerLock>, String> {
    if !valid_graph_id(id) {
        return Err("invalid draft namespace".into());
    }
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let paths = crate::owner::namespace::Namespace::resolve()
            .and_then(|ns| ns.graph_id(id))
            .map_err(|e| e.to_string())?;
        crate::owner::lock::OwnerLock::try_take(&paths.lock, "draft-adoption")
            .map_err(|e| e.to_string())?
            .map(Some)
            .ok_or_else(|| "draft-recovery-owner-held".into())
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        Ok(None)
    } // one process per app; per-app sandboxed drafts
}

pub(crate) fn adopt_namespace(data: &Path, id: &str, root: &Path) -> Result<(), String> {
    if !valid_graph_id(id) {
        return Err("invalid draft namespace".into());
    }
    let old = data.join("drafts-v2").join(id);
    let new = data
        .join("drafts-v2")
        .join(crate::settings::host_graph_id(root));
    if old == new || !old.is_dir() {
        return Err("draft-recovery-no-longer-available".into());
    }
    if !census_empty(&new).map_err(|e| e.to_string())? {
        return Err("draft-recovery-current-unsaved".into());
    }
    rename_empty_target(&old, &new, true).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn orphan_drafts(
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<Vec<Orphan>, String> {
    use tauri::Manager;
    let slot = crate::state::slot_for_context(&state)?;
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::state::off_ui(move || {
        let known = crate::settings::read_settings_value(&app)
            .and_then(|json| json.get("known_graphs").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|g| g["path"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        Ok(find_orphans(&data, &slot.root_key, &known))
    })
    .await
}

#[tauri::command]
pub(crate) async fn recover_orphan_drafts(
    id: String,
    consumed_last_id: u64,
    app: tauri::AppHandle,
    state: crate::state::GraphContext<'_>,
) -> Result<crate::backup::RestoreReply, String> {
    use tauri::Manager;
    let slot = crate::state::slot_for_context(&state)?;
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::state::off_ui(move || {
        // Serialize graph opens, host retirement/adoption and namespace rename.
        let state = app.state::<crate::state::AppState>();
        let _load = state.graph_load.lock().unwrap();
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        if slot
            .owner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            return Err("draft-recovery-lock-unavailable".into());
        }
        let _old = orphan_owner_lock(&id)?;
        let known = crate::settings::read_settings_value(&app)
            .and_then(|json| json.get("known_graphs").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|g| g["path"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        if !find_orphans(&data, &slot.root_key, &known)
            .iter()
            .any(|orphan| orphan.id == id)
        {
            return Err("draft-recovery-no-longer-available".into());
        }
        crate::backup::restore_hosted(&slot.host, consumed_last_id, || {
            adopt_namespace(&data, &id, &slot.root_key)
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn roots() -> (PathBuf, PathBuf) {
        (PathBuf::from("/private/var/mobile/Containers/Data/Application/11111111-1111-1111-1111-111111111111/Documents/g"),
         PathBuf::from("/private/var/mobile/Containers/Data/Application/22222222-2222-2222-2222-222222222222/Documents/g"))
    }
    #[test]
    fn orphan_census_is_bounded_by_device_namespaces_including_empty_subdirectories() {
        let data = tempfile::tempdir().unwrap();
        for n in 0..4 {
            let dir = data
                .path()
                .join("drafts-v2")
                .join(crate::settings::host_graph_id(Path::new(&format!(
                    "/missing/{n}"
                ))));
            fs::create_dir_all(dir.join("trash-custody")).unwrap();
            fs::create_dir_all(dir.join("unreadable")).unwrap();
        }
        CENSUS_READ_DIRS.with(|count| count.set(0));
        assert!(find_orphans(data.path(), Path::new("/current"), &[]).is_empty());
        assert_eq!(CENSUS_READ_DIRS.with(|count| count.get()), 1 + 3 * 4);
    }
    #[test]
    fn a_failed_relocation_step_leaves_the_source_and_retry_finishes() {
        let data = tempfile::tempdir().unwrap();
        let (a, b) = roots();
        let ba = crate::backup::root_backup_id(&a);
        let bb = crate::backup::root_backup_id(&b);
        let old = data.path().join("backups").join(ba);
        let blocked = data.path().join("backups").join(bb);
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("snapshot"), b"sealed").unwrap();
        fs::write(&blocked, b"partial setup").unwrap();
        relocate_namespaces(data.path(), &a, &b);
        assert_eq!(fs::read(old.join("snapshot")).unwrap(), b"sealed");
        fs::remove_file(&blocked).unwrap();
        relocate_namespaces(data.path(), &a, &b);
        assert_eq!(fs::read(blocked.join("snapshot")).unwrap(), b"sealed");
    }
    #[test]
    fn adoption_requires_an_empty_target_and_moves_the_whole_census() {
        let data = tempfile::tempdir().unwrap();
        let old = Path::new("/old/notes");
        let new = Path::new("/new/notes");
        let id = crate::settings::host_graph_id(old);
        let source = data.path().join("drafts-v2").join(&id);
        let target = data
            .path()
            .join("drafts-v2")
            .join(crate::settings::host_graph_id(new));
        fs::create_dir_all(source.join("trash-custody")).unwrap();
        fs::write(source.join("vehicle"), b"draft bytes").unwrap();
        fs::write(source.join("trash-custody/marker"), b"custody bytes").unwrap();
        fs::create_dir_all(target.join("unreadable")).unwrap();
        fs::write(target.join("unreadable/kept"), b"unreadable input").unwrap();
        assert_eq!(
            adopt_namespace(data.path(), &id, new).unwrap_err(),
            "draft-recovery-current-unsaved"
        );
        assert!(source.join("vehicle").exists());
        fs::remove_file(target.join("unreadable/kept")).unwrap();
        adopt_namespace(data.path(), &id, new).unwrap();
        assert_eq!(fs::read(target.join("vehicle")).unwrap(), b"draft bytes");
        assert_eq!(
            fs::read(target.join("trash-custody/marker")).unwrap(),
            b"custody bytes"
        );
        assert!(!source.exists());
        assert!(adopt_namespace(data.path(), "../escape", new).is_err());
    }
    #[test]
    fn old_graph_id_uses_the_same_owner_lock_as_the_host() {
        let data = tempfile::tempdir().unwrap();
        let namespace = crate::owner::namespace::Namespace::at(data.path().join("owners")).unwrap();
        let old = Path::new("/old/notes");
        let host = namespace.graph(old).unwrap();
        let adoption = namespace
            .graph_id(&crate::settings::host_graph_id(old))
            .unwrap();
        assert_eq!(host.lock, adoption.lock);
        let first = crate::owner::lock::OwnerLock::try_take(&host.lock, "app")
            .unwrap()
            .unwrap();
        assert!(
            crate::owner::lock::OwnerLock::try_take(&adoption.lock, "draft-adoption")
                .unwrap()
                .is_none()
        );
        drop(first);
        assert!(
            crate::owner::lock::OwnerLock::try_take(&adoption.lock, "draft-adoption")
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn adoption_exclusion_pins_desktop_locks_and_mobile_sandboxes() {
        let source = include_str!("root_identity.rs");
        let production = &source[..source.find("#[cfg(test)]\nmod tests").unwrap()];
        let arm = &production[production.find("fn orphan_owner_lock").unwrap()
            ..production.find("pub(crate) fn adopt_namespace").unwrap()];
        assert!(arm.contains(
            "#[cfg(any(target_os = \"linux\", target_os = \"macos\", target_os = \"windows\"))]"
        ));
        assert!(arm.contains("OwnerLock::try_take"));
        assert!(arm.contains("#[cfg(any(target_os = \"ios\", target_os = \"android\"))]"));
        assert!(arm.contains("Ok(None)"));
    }
    #[test]
    fn only_the_container_uuid_can_change() {
        let (a, b) = roots();
        assert!(is_container_rebase(&a, &b));
        assert!(!is_container_rebase(&a, &a));
        assert!(!is_container_rebase(&a, &b.join("other")));
        assert!(!is_container_rebase(&a,Path::new("/elsewhere/Containers/Data/Application/22222222-2222-2222-2222-222222222222/Documents/g")));
        assert!(!is_container_rebase(
            &a,
            Path::new("/private/var/mobile/Containers/Data/Application/not-a-uuid/Documents/g")
        ));
        assert!(!is_container_rebase(
            Path::new("/graphs/g"),
            Path::new("/elsewhere/g")
        ));
        assert!(!is_container_rebase(&a, &b.join("../different")));
    }
    #[test]
    fn relocation_is_idempotent_and_preserves_sealed_bytes() {
        let data = tempfile::tempdir().unwrap();
        let (a, b) = roots();
        let old = crate::settings::host_graph_id(&a);
        let new = crate::settings::host_graph_id(&b);
        let draft = data.path().join("drafts-v2").join(&old);
        fs::create_dir_all(&draft).unwrap();
        fs::write(draft.join("vehicle"), b"typed").unwrap();
        let checkpoint = data.path().join("launch-checkpoints");
        fs::create_dir_all(&checkpoint).unwrap();
        fs::write(checkpoint.join(format!("{old}.bin")), b"cache").unwrap();
        let backups = data.path().join("backups");
        fs::create_dir_all(backups.join(format!("{}.zips", crate::backup::root_backup_id(&a))))
            .unwrap();
        fs::write(
            backups.join(format!(
                "{}.zips/sealed.zip",
                crate::backup::root_backup_id(&a)
            )),
            b"sealed",
        )
        .unwrap();
        relocate_namespaces(data.path(), &a, &b);
        relocate_namespaces(data.path(), &a, &b);
        assert_eq!(
            fs::read(data.path().join("drafts-v2").join(new).join("vehicle")).unwrap(),
            b"typed"
        );
        assert_eq!(
            fs::read(backups.join(format!(
                "{}.zips/sealed.zip",
                crate::backup::root_backup_id(&b)
            )))
            .unwrap(),
            b"sealed"
        );
        assert!(!checkpoint.join(format!("{old}.bin")).exists());
    }
    #[test]
    fn relocation_never_merges_nonempty_namespaces() {
        let data = tempfile::tempdir().unwrap();
        let (a, b) = roots();
        for (root, text) in [(&a, "old"), (&b, "new")] {
            let dir = data
                .path()
                .join("drafts-v2")
                .join(crate::settings::host_graph_id(root));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("vehicle"), text).unwrap();
        }
        relocate_namespaces(data.path(), &a, &b);
        for (root, text) in [(&a, "old"), (&b, "new")] {
            assert_eq!(
                fs::read_to_string(
                    data.path()
                        .join("drafts-v2")
                        .join(crate::settings::host_graph_id(root))
                        .join("vehicle")
                )
                .unwrap(),
                text
            );
        }
    }
    #[test]
    fn orphan_detection_ignores_a_present_old_graph() {
        let data = tempfile::tempdir().unwrap();
        let graph = tempfile::tempdir().unwrap();
        let old = graph.path().join("old");
        fs::create_dir(&old).unwrap();
        let id = crate::settings::host_graph_id(&old);
        let dir = data.path().join("drafts-v2").join(&id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("vehicle"), b"typed").unwrap();
        let known = vec![old.display().to_string()];
        assert!(find_orphans(data.path(), Path::new("/new"), &known).is_empty());
        fs::rename(&old, graph.path().join("moved")).unwrap();
        assert_eq!(
            find_orphans(data.path(), Path::new("/new"), &known),
            vec![Orphan {
                id,
                old_root: Some(known[0].clone())
            }]
        );
    }
}
