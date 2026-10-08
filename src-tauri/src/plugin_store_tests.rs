//! Plugin package-store crash safety (master e6f131c09 + 3f9dcdbdb, ported onto
//! og's plain `std::fs` + `device_io::move_file_noreplace` store; og lane 18B).
//!
//! In-scope threat scenarios: crash or power loss mid-install or mid-uninstall
//! (staging/retirement residue, a version directory left half-removed by an
//! interrupted pre-18B `remove_dir_all` uninstall) and honest concurrent
//! instances / commands installing the same immutable version.

use super::*;

const WASM: &[u8] = b"\0asm\x01\0\0\0";

fn manifest(id: &str, version: &str) -> String {
    format!(r#"{{"id":"{id}","version":"{version}"}}"#)
}

fn install_plugin_package_at(
    root: &Path,
    id: &str,
    version: &str,
    manifest_json: &str,
    wasm: &[u8],
) -> Result<PackagePublishOutcome, String> {
    publish_package(root, id, version, manifest_json.as_bytes(), wasm)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn fresh_root(label: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join(format!("plugins-{label}"));
    std::fs::create_dir_all(&root).unwrap();
    (temp, root)
}

#[test]
fn store_recovery_runs_once_per_process_so_a_second_command_cannot_reclaim_live_staging() {
    let (_temp, root) = fresh_root("once");
    let stale = root.join(".install-dev.tine.example-0.1.0-1-1");
    std::fs::create_dir_all(&stale).unwrap();
    plugin_store_root(root.clone()).unwrap();
    assert!(!stale.exists(), "first use reclaims crash-cut residue");

    // A concurrent honest instance's in-flight staging directory appears after
    // this process has already recovered the store.
    let live = root.join(".install-dev.tine.example-0.1.0-2-1");
    std::fs::create_dir_all(&live).unwrap();
    plugin_store_root(root.clone()).unwrap();
    assert!(
        live.exists(),
        "later commands must not reclaim another process's staging"
    );
}

#[test]
fn reopen_reclaims_transient_and_wedged_packages_before_retry() {
    let (_temp, root) = fresh_root("wedged");
    std::fs::create_dir_all(root.join(".install-dev.tine.example-1.0.0-7-1")).unwrap();
    std::fs::create_dir_all(root.join(".retired-dev.tine.example-1.0.0-7-2/x")).unwrap();
    // An uninstall interrupted by a crash (the pre-18B `remove_dir_all`) left
    // the version directory with only one of its two required files.
    let wedged = root.join("dev.tine.example/1.0.0");
    std::fs::create_dir_all(&wedged).unwrap();
    std::fs::write(wedged.join("plugin.wasm"), WASM).unwrap();
    // A complete sibling package must survive recovery byte-for-byte.
    let kept = root.join("dev.tine.other/2.0.0");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(
        kept.join("manifest.json"),
        manifest("dev.tine.other", "2.0.0"),
    )
    .unwrap();
    std::fs::write(kept.join("plugin.wasm"), WASM).unwrap();

    plugin_store_root(root.clone()).unwrap();

    assert!(!wedged.exists());
    assert_eq!(names(&root), vec!["dev.tine.other"]);
    assert_eq!(names(&kept), vec!["manifest.json", "plugin.wasm"]);
    assert_eq!(
        install_plugin_package_at(
            &root,
            "dev.tine.example",
            "1.0.0",
            &manifest("dev.tine.example", "1.0.0"),
            WASM,
        )
        .unwrap(),
        PackagePublishOutcome::Published
    );
}

#[cfg(unix)]
#[test]
fn recovery_never_follows_a_symlink_out_of_plugin_storage() {
    use std::os::unix::fs::symlink;
    let (temp, root) = fresh_root("symlink");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(outside.join("1.0.0")).unwrap();
    std::fs::write(outside.join("1.0.0/plugin.wasm"), WASM).unwrap();
    symlink(&outside, root.join("dev.tine.example")).unwrap();
    symlink(&outside, root.join(".install-dev.tine.example-1.0.0-9-9")).unwrap();

    recover_plugin_store_at(&root).unwrap();

    assert!(outside.join("1.0.0/plugin.wasm").exists());
    assert!(!root.join(".install-dev.tine.example-1.0.0-9-9").exists());
}

#[test]
fn reinstalling_identical_bytes_is_exact_and_different_bytes_collide() {
    let (_temp, root) = fresh_root("exact");
    let text = manifest("dev.tine.example", "1.0.0");
    let install = |manifest: &str, wasm: &[u8]| {
        install_plugin_package_at(&root, "dev.tine.example", "1.0.0", manifest, wasm)
    };
    assert_eq!(
        install(&text, WASM).unwrap(),
        PackagePublishOutcome::Published
    );
    assert_eq!(
        install(&text, WASM).unwrap(),
        PackagePublishOutcome::AlreadyPresentExact
    );
    assert_eq!(
        install(&text, b"\0asm\x01\0\0\0\x01").unwrap_err(),
        "that immutable plugin version is already installed with different bytes"
    );
    assert_eq!(names(&root), vec!["dev.tine.example"], "no staging left");
    assert_eq!(
        std::fs::read(root.join("dev.tine.example/1.0.0/plugin.wasm")).unwrap(),
        WASM
    );
}

#[test]
fn concurrent_different_installs_leave_one_complete_immutable_winner() {
    use std::sync::{Arc, Barrier};

    for _ in 0..8 {
        let (_temp, root) = fresh_root("race");
        let root = Arc::new(root);
        let barrier = Arc::new(Barrier::new(3));
        let manifests = [
            r#"{"id":"dev.tine.example","version":"1.0.0"}"#,
            r#"{ "id": "dev.tine.example", "version": "1.0.0" }"#,
        ];
        let workers = manifests
            .iter()
            .map(|manifest| {
                let root = Arc::clone(&root);
                let barrier = Arc::clone(&barrier);
                let manifest = manifest.to_string();
                std::thread::spawn(move || {
                    barrier.wait();
                    install_plugin_package_at(&root, "dev.tine.example", "1.0.0", &manifest, WASM)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        assert!(outcomes
            .iter()
            .any(|outcome| outcome.as_ref().is_err_and(|error| {
                error == "that immutable plugin version is already installed with different bytes"
            })));
        let stored =
            std::fs::read_to_string(root.join("dev.tine.example/1.0.0/manifest.json")).unwrap();
        assert!(manifests.contains(&stored.as_str()));
        assert_eq!(names(&root), vec!["dev.tine.example"], "no staging left");
    }
}

#[test]
fn a_crash_after_retirement_leaves_no_half_package_and_recovery_reclaims_it() {
    let (_temp, root) = fresh_root("retire-cut");
    install_plugin_package_at(
        &root,
        "dev.tine.example",
        "1.0.0",
        &manifest("dev.tine.example", "1.0.0"),
        WASM,
    )
    .unwrap();
    let crash = std::io::Error::other("simulated crash after the retire move");
    let error =
        uninstall_package_with(&root, "dev.tine.example", "1.0.0", || Err(crash)).unwrap_err();
    assert!(error.contains("simulated crash"), "{error}");

    // The version vanished in one no-replace move; only reclaimable residue remains.
    assert!(!root.join("dev.tine.example/1.0.0").exists());
    assert!(list_installed_plugins_at(&root, &Default::default())
        .unwrap()
        .is_empty());
    assert!(names(&root)
        .iter()
        .any(|name| name.starts_with(".retired-")));

    recover_plugin_store_at(&root).unwrap();
    assert!(names(&root).is_empty(), "{:?}", names(&root));
    assert_eq!(
        install_plugin_package_at(
            &root,
            "dev.tine.example",
            "1.0.0",
            &manifest("dev.tine.example", "1.0.0"),
            WASM,
        )
        .unwrap(),
        PackagePublishOutcome::Published
    );
}

#[test]
fn settings_clear_cut_leaves_a_recoverable_package_then_retry_finishes() {
    let (temp, root) = fresh_root("settings-cut");
    install_plugin_package_at(
        &root,
        "dev.tine.example",
        "1.0.0",
        &manifest("dev.tine.example", "1.0.0"),
        WASM,
    )
    .unwrap();
    let settings = temp.path().join("tine-settings.json");
    std::fs::write(
        &settings,
        r#"{"plugin_states":{"dev.tine.example":{"version":"1.0.0","enabled":true}},"plugin-settings:dev.tine.example":{"keep":true}}"#,
    )
    .unwrap();

    crate::settings::update_settings_strict_at(&settings, |json| {
        clear_uninstalled_plugin_settings(json, "dev.tine.example", "1.0.0", true);
        Ok(())
    })
    .unwrap();
    // Crash cut: settings are durable, retirement has not started.
    recover_plugin_store_at(&root).unwrap();

    validate_uninstall_target(&root, "dev.tine.example", "1.0.0").unwrap();
    let persisted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert!(persisted["plugin_states"].get("dev.tine.example").is_none());
    assert!(persisted.get("plugin-settings:dev.tine.example").is_none());
    assert!(uninstall_package(&root, "dev.tine.example", "1.0.0").unwrap());
    assert!(!root.join("dev.tine.example").exists());
}

#[test]
fn production_plugin_writes_stay_on_the_no_replace_store_path() {
    let source = include_str!("plugins.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    let body = |signature: &str| {
        let start = &production[production.find(signature).unwrap()..];
        start[..start.find("\n}\n").unwrap()].to_string()
    };
    // R3 (og-flow3): the command runs its work off the main thread in
    // install_plugin_blocking; the store-path property is that function's.
    assert!(body("pub(crate) async fn install_plugin(").contains("install_plugin_blocking("));
    let install = body("fn install_plugin_blocking(");
    assert!(install.contains("publish_package(") && !install.contains("std::fs::"));
    let uninstall = body("pub(crate) async fn uninstall_plugin(");
    assert!(uninstall.contains("uninstall_package(") && !uninstall.contains("remove_dir_all"));
    assert!(body("fn plugins_dir(").contains("plugin_store_root("));
    assert!(body("fn plugin_store_root(").contains("recover_plugin_store_once("));
    for publisher in ["fn publish_package(", "fn uninstall_package_with("] {
        let publisher = body(publisher);
        assert!(
            publisher.contains("crate::device_io::move_file_noreplace("),
            "I-16: package publication/retirement is a no-replace move"
        );
    }
    assert!(
        !production.contains("std::fs::rename("),
        "I-16: no replacing rename in plugin storage; use device_io::move_file_noreplace"
    );
}

#[test]
fn plugin_states_distinguish_absent_from_unreadable_and_keep_unknown_entries() {
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("tine-settings.json");
    assert!(plugin_states_at(&settings).unwrap().is_empty());

    std::fs::write(
        &settings,
        r#"{"plugin_states":{"a":{"version":"1.0.0","enabled":true"#,
    )
    .unwrap();
    let error = plugin_states_at(&settings).unwrap_err();
    assert!(error.contains("tine-settings.json"), "{error}");

    std::fs::write(
        &settings,
        r#"{"plugin_states":{"a":{"version":"1.0.0","enabled":true},"future":{"channel":"x"}}}"#,
    )
    .unwrap();
    let states = plugin_states_at(&settings).unwrap();
    assert_eq!(states.len(), 1);
    assert!(states["a"].enabled);
}
