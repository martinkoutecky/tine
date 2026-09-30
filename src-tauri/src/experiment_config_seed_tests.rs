use super::*;
use std::collections::BTreeMap;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tine-seed-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Every file under `dir` with its bytes (a byte-level snapshot).
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

const RELEASE_SETTINGS: &str = r#"{
  "known_graphs": [{"name": "notes", "path": "/graphs/notes"}],
  "last_graph_path": "/graphs/notes",
  "link_autocomplete_policy": "adaptive",
  "theme.composition.v1": "{\"style\":\"\",\"colors\":\"\"}"
}
"#;

/// The layout the released Tine (master 6c380173) wrote on a real run:
/// config plus master-only artifacts that must never be copied.
fn released_dir(root: &Path) -> PathBuf {
    let dir = root.join("release-id");
    write(&dir.join("tine-settings.json"), RELEASE_SETTINGS.as_bytes());
    write(
        &dir.join("sessions/notes-c868ec19883e67ac-workspaces.json"),
        br#"{"activeId":"default","version":1,"workspaces":[{"blob":{"activeIndex":0,"tabs":[{"history":[{"kind":"journals"}],"pinned":false,"pos":0}]},"id":"default","name":""}]}"#,
    );
    write(
        &dir.join("sessions/notes-c868ec19883e67ac-notices.json"),
        br#"{"dismissed":["query-crossing"]}"#,
    );
    write(&dir.join("plugins/page.tine.x/0.1.0/manifest.json"), b"{}");
    write(&dir.join("plugins/page.tine.x/0.1.0/plugin.wasm"), b"\0asm");
    write(
        &dir.join("localstorage/tauri_localhost_0.localstorage"),
        b"SQLite format 3\0",
    );
    write(&dir.join("storage/salt"), b"salt");
    for master_only in [
        "backups/notes-b6e5cbb57a26572efaac2788630a5d15/2026-09-29_12-53-37/snapshot.json",
        "direct-files-projections/f8c0.sqlite",
        "direct-move-recovery/id/records/m.json",
        "conflict-capsules/notes-c868ec19883e67ac.v1.json",
        "concord-ledger/id/blobs/ab",
        "diagnostics/current.jsonl",
        "mediakeys/v1/salt",
        "hsts-storage.sqlite",
        "WebKitCache/Version 16/salt",
    ] {
        write(&dir.join(master_only), master_only.as_bytes());
    }
    dir
}

#[test]
fn seeds_only_the_config_allowlist_and_never_touches_the_released_dir() {
    let root = scratch("allowlist");
    let release = released_dir(&root);
    let own = root.join("own-id");
    let before = tree(&release);

    let outcome = seed(&own, &release, None).unwrap();

    assert_eq!(
        outcome,
        Seeded::Copied(vec![
            "tine-settings.json",
            "sessions",
            "plugins",
            "localstorage",
            "storage"
        ])
    );
    assert_eq!(tree(&release), before, "the released dir is read-only");
    let seeded = tree(&own);
    let expected: BTreeMap<_, _> = before
        .iter()
        .filter(|(rel, _)| CONFIG_ENTRIES.iter().any(|entry| rel.starts_with(entry)))
        .map(|(rel, bytes)| (rel.clone(), bytes.clone()))
        .collect();
    assert_eq!(
        seeded, expected,
        "exactly the config entries, byte for byte"
    );
    assert!(!root.join("own-id.seeding").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_seeded_settings_name_the_released_graph_so_startup_opens_it() {
    // E: Welcome shows only when the startup graph fails to load; startup reads
    // `last_graph_path` from the build's own tine-settings.json.
    let root = scratch("welcome");
    let release = released_dir(&root);
    let own = root.join("own-id");
    seed(&own, &release, None).unwrap();
    assert!(has_configured_graph(&own));
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(own.join("tine-settings.json")).unwrap()).unwrap();
    assert_eq!(settings["last_graph_path"], "/graphs/notes");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_build_with_its_own_configured_graph_is_left_alone() {
    let root = scratch("configured");
    let release = released_dir(&root);
    let own = root.join("own-id");
    write(
        &own.join("tine-settings.json"),
        br#"{"known_graphs":[{"name":"other","path":"/graphs/other"}],"last_graph_path":"/graphs/other"}"#,
    );
    let before = tree(&own);
    assert_eq!(
        seed(&own, &release, None).unwrap(),
        Seeded::Skipped("this build already has a configured graph")
    );
    assert_eq!(tree(&own), before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_welcome_only_launch_is_set_aside_intact_not_deleted() {
    let root = scratch("aside");
    let release = released_dir(&root);
    let own = root.join("own-id");
    write(
        &own.join("tine-settings.json"),
        b"{\"smooth_scroll\":true}\n",
    );
    write(
        &own.join("localstorage/tauri_localhost_0.localstorage"),
        b"welcome-only",
    );
    let before = tree(&own);

    assert!(matches!(
        seed(&own, &release, None).unwrap(),
        Seeded::Copied(_)
    ));
    assert_eq!(tree(&root.join("own-id.pre-seed.0")), before);
    assert!(has_configured_graph(&own));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_crashed_staging_dir_is_discarded_and_rebuilt() {
    let root = scratch("crash");
    let release = released_dir(&root);
    let own = root.join("own-id");
    // What a crash mid-copy leaves: a partial staging dir, no published dir.
    write(
        &root.join("own-id.seeding/tine-settings.json"),
        b"{\"known_gr",
    );
    assert!(matches!(
        seed(&own, &release, None).unwrap(),
        Seeded::Copied(_)
    ));
    assert_eq!(
        fs::read(own.join("tine-settings.json")).unwrap(),
        RELEASE_SETTINGS.as_bytes()
    );
    assert!(!root.join("own-id.seeding").exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_copy_error_publishes_nothing_and_keeps_the_existing_dir() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("copy-error");
    let release = released_dir(&root);
    let own = root.join("own-id");
    write(&own.join("tine-settings.json"), b"{}\n");
    let own_before = tree(&own);
    // A disk/permission error on one released file mid-copy.
    let unreadable = release.join("plugins/page.tine.x/0.1.0/plugin.wasm");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&unreadable).is_ok() {
        // Running as root: permissions cannot simulate the error.
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
        let _ = fs::remove_dir_all(root);
        return;
    }
    let outcome = seed(&own, &release, None);
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(outcome.is_err());
    assert_eq!(tree(&own), own_before, "the build's own dir is untouched");
    assert!(!root.join("own-id.seeding").exists());
    assert!(!root.join("own-id.pre-seed.0").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn nothing_is_seeded_from_a_released_dir_without_a_graph() {
    let root = scratch("empty-release");
    let release = root.join("release-id");
    write(
        &release.join("tine-settings.json"),
        b"{\"smooth_scroll\":false}\n",
    );
    let own = root.join("own-id");
    assert_eq!(
        seed(&own, &release, None).unwrap(),
        Seeded::Skipped("the released Tine has no configured graph")
    );
    assert!(!own.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn window_geometry_is_copied_once_and_never_over_the_builds_own() {
    let root = scratch("window-state");
    let release = released_dir(&root);
    let config = root.join("config");
    let (own_config, release_config) = (config.join("own-id"), config.join("release-id"));
    write(
        &release_config.join(WINDOW_STATE),
        br#"{"main":{"width":900}}"#,
    );
    let own = root.join("own-id");
    seed(
        &own,
        &release,
        Some((own_config.clone(), release_config.clone())),
    )
    .unwrap();
    assert_eq!(
        fs::read(own_config.join(WINDOW_STATE)).unwrap(),
        br#"{"main":{"width":900}}"#
    );

    // A second seed (own dir reset to Welcome-only) keeps the build's geometry.
    write(
        &own_config.join(WINDOW_STATE),
        br#"{"main":{"width":1200}}"#,
    );
    fs::remove_dir_all(&own).unwrap();
    seed(&own, &release, Some((own_config.clone(), release_config))).unwrap();
    assert_eq!(
        fs::read(own_config.join(WINDOW_STATE)).unwrap(),
        br#"{"main":{"width":1200}}"#
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn desktop_webview_seed_uses_native_layouts_and_keeps_rollback_data_intact() {
    let temp = tempfile::tempdir().unwrap();
    for os in ["windows", "macos"] {
        let (own, release) = webview_dirs(
            os,
            Some(temp.path()),
            Some(temp.path()),
            "experiment-id",
            "release-id",
        )
        .unwrap();
        write(
            &release.join("Default/Local Storage/leveldb/000003.log"),
            b"theme=dark;shortcuts=custom",
        );
        write(&release.join("salt"), b"origin-salt");
        let before = tree(&release);
        seed_webview_store(&own, &release).unwrap();
        assert_eq!(tree(&own), before);
        assert_eq!(
            tree(&release),
            before,
            "rollback reads the exact source store"
        );
        write(&own.join("salt"), b"experiment-state");
        seed_webview_store(&own, &release).unwrap();
        assert_eq!(fs::read(own.join("salt")).unwrap(), b"experiment-state");
        assert_eq!(tree(&release), before);
    }
    for os in ["linux", "android", "ios"] {
        assert!(webview_dirs(os, Some(temp.path()), Some(temp.path()), "own", "release").is_none());
    }
}

#[test]
fn crash_child_after_webview_stage() {
    let Some(root) = std::env::var_os("OG_R6_WEBVIEW_CRASH_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    seed_webview_after_stage(&root.join("own-store"), &root.join("release-store"), || {
        write(&root.join("staged"), b"ready");
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    })
    .unwrap();
}

#[test]
fn killed_webview_seed_retries_after_config_has_already_published() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let release = released_dir(root);
    let own = root.join("own-id");
    seed(&own, &release, None).unwrap();
    let own_store = root.join("own-store");
    let release_store = root.join("release-store");
    write(&release_store.join("preferences"), b"master browser state");
    let before = tree(&release_store);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "experiment_config_seed::tests::crash_child_after_webview_stage",
            "--nocapture",
        ])
        .env("OG_R6_WEBVIEW_CRASH_FIXTURE", root)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !root.join("staged").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        root.join("staged").exists(),
        "child never reached the staging boundary"
    );
    assert!(!own_store.exists());
    assert!(matches!(
        seed(&own, &release, None).unwrap(),
        Seeded::Skipped(_)
    ));
    seed_missing_webview(
        &own,
        &release,
        Some((own_store.clone(), release_store.clone())),
    )
    .unwrap();
    assert_eq!(tree(&own_store), before);
    assert_eq!(tree(&release_store), before, "rollback source changed");
    assert!(!own_store.with_extension("seeding").exists());
}
