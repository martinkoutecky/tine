//! TEMPORARY experiment-build convenience (docs/app-identity.md, "Experiment
//! config seed"). DELETE THIS FILE, its `mod` line and its one call in
//! `lib::run()` when `src-tauri/app-identity.json` ships the released identity:
//! `src/appIdentity.guard.test.ts` fails until you do.
//!
//! While an experiment build ships its own identifier, its app-data dir starts
//! empty and a tester would meet the Welcome screen instead of their graphs.
//! On a launch whose own app-data dir holds no configured graph, this copies
//! the released Tine's CONFIG files into it once, before the webview exists
//! (WebKitGTK creates its store while the Tauri Builder is assembled).
//!
//! - The released dir is only read (`read_dir`, `fs::copy` source); nothing in
//!   it is created, rewritten, renamed or deleted.
//! - Only the allowlist [`CONFIG_ENTRIES`] is copied. Backups, the index and
//!   projection, diagnostics, caches and anything unknown are never copied.
//! - The copy is staged in a sibling dir, fsynced, and renamed into place
//!   whole. Crash or power loss mid-copy (in-scope: crash / torn write) leaves
//!   only the staging dir, which the next launch discards and rebuilds; a disk
//!   error abandons the seed and the build starts fresh (Welcome) — recovery,
//!   never a refusal. A pre-existing experiment dir without a configured graph
//!   (e.g. a launch that only showed Welcome) is renamed aside, not deleted.
//! - A released Tine running concurrently (honest concurrent instance) may be
//!   mid-write to its WebKit store; the copy may then be stale or unreadable to
//!   WebKit, which discards it. The graph still opens from the copied
//!   `tine-settings.json` (`last_graph_path`).

use crate::device_io::{copy_tree, publish_directory_entry as publish};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// App-data entries that make up "my config", relative to the app-data dir:
/// device settings (known graphs, last graph, preferences), per-graph sessions
/// and workspaces, installed plugins, and — Linux WebKitGTK — the webview's
/// localStorage (theme, shortcuts, sidebar, recents): `localstorage/` in the
/// legacy layout, `storage/` (salted per-origin dirs) in the current one.
const CONFIG_ENTRIES: &[&str] = &[
    "tine-settings.json",
    "sessions",
    "plugins",
    "localstorage",
    "storage",
];
/// Window geometry (tauri-plugin-window-state), in the app-CONFIG dir.
const WINDOW_STATE: &str = ".window-state.json";

/// Seed the running experiment build's app-data dir from the released one.
/// No-op in a release build or when there is nothing to do (desktop only:
/// mobile app data is app-private per application id).
pub(crate) fn seed_from_release_once() {
    use crate::app_identity::{app_data_dir_of, APP_IDENTIFIER, RELEASE_IDENTIFIER};
    if APP_IDENTIFIER == RELEASE_IDENTIFIER {
        return;
    }
    let (Some(own), Some(release)) = (
        app_data_dir_of(APP_IDENTIFIER),
        app_data_dir_of(RELEASE_IDENTIFIER),
    ) else {
        return;
    };
    let config_dirs =
        dirs::config_dir().map(|base| (base.join(APP_IDENTIFIER), base.join(RELEASE_IDENTIFIER)));
    match seed(&own, &release, config_dirs) {
        Ok(Seeded::Copied(entries)) => {
            crate::debug::diag_private(
                "experiment-config-seeded",
                format!("copied {entries:?} from the released Tine's app-data dir"),
            );
        }
        Ok(Seeded::Skipped(_)) => {}
        Err(error) => crate::debug::diag_private(
            "experiment-config-seed-failed",
            format!("starting without the released Tine's config: {error}"),
        ),
    }
    // Config and external browser stores publish independently. Retry a missing
    // browser store on reopen even if config was already published before a crash.
    if let Err(error) = seed_missing_webview(&own, &release, desktop_webview_dirs()) {
        crate::debug::diag_private("experiment-webview-seed-failed", error.to_string());
    }
}

fn seed_missing_webview(
    own_data: &Path,
    release_data: &Path,
    stores: Option<(PathBuf, PathBuf)>,
) -> io::Result<()> {
    if let Some((own, release)) = stores {
        if has_configured_graph(own_data) && has_configured_graph(release_data) {
            seed_webview_store(&own, &release)?;
        }
    }
    Ok(())
}

/// Tauri's Windows default uses LocalData/<identifier>/EBWebView, while
/// settings use RoamingAppData. Wry uses WKWebsiteDataStore::defaultDataStore
/// on macOS; WebKit keeps its origin data in Library/WebKit/<bundle>/WebsiteData.
fn desktop_webview_dirs() -> Option<(PathBuf, PathBuf)> {
    use crate::app_identity::{APP_IDENTIFIER, RELEASE_IDENTIFIER};
    webview_dirs(
        std::env::consts::OS,
        dirs::data_local_dir().as_deref(),
        dirs::home_dir().as_deref(),
        APP_IDENTIFIER,
        RELEASE_IDENTIFIER,
    )
}

fn webview_dirs(
    os: &str,
    local: Option<&Path>,
    home: Option<&Path>,
    own: &str,
    release: &str,
) -> Option<(PathBuf, PathBuf)> {
    let (base, leaf) = match os {
        "windows" => (local?.to_path_buf(), "EBWebView"),
        "macos" => (home?.join("Library/WebKit"), "WebsiteData"),
        // Linux is covered by CONFIG_ENTRIES. Mobile data is app-private.
        "linux" | "android" | "ios" => return None,
        _ => return None,
    };
    Some((base.join(own).join(leaf), base.join(release).join(leaf)))
}

fn seed_webview_store(own: &Path, release: &Path) -> io::Result<()> {
    seed_webview_after_stage(own, release, || {})
}

fn seed_webview_after_stage(own: &Path, release: &Path, after_stage: impl Fn()) -> io::Result<()> {
    if own.exists() || !release.is_dir() {
        return Ok(());
    }
    let staged = own.with_extension("seeding");
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    copy_tree(release, &staged)?;
    after_stage();
    publish(&staged, own)
}

#[derive(Debug, PartialEq, Eq)]
enum Seeded {
    Copied(Vec<&'static str>),
    Skipped(&'static str),
}

/// A dir has a configured graph when its device settings name one.
fn has_configured_graph(dir: &Path) -> bool {
    fs::read_to_string(dir.join("tine-settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|json| {
            json.get("last_graph_path")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|path| !path.is_empty())
                || json
                    .get("known_graphs")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|graphs| !graphs.is_empty())
        })
}

fn seed(own: &Path, release: &Path, config_dirs: Option<(PathBuf, PathBuf)>) -> io::Result<Seeded> {
    if has_configured_graph(own) {
        return Ok(Seeded::Skipped("this build already has a configured graph"));
    }
    if !has_configured_graph(release) {
        return Ok(Seeded::Skipped("the released Tine has no configured graph"));
    }
    let parent = own
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "app-data dir has no parent"))?;
    let leaf = own
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "app-data dir has no name"))?
        .to_string_lossy()
        .into_owned();
    let staging = parent.join(format!("{leaf}.seeding"));
    if staging.exists() {
        // Our own staging from a crashed earlier launch: never published.
        fs::remove_dir_all(&staging)?;
    }
    let result = stage(release, &staging).and_then(|copied| {
        if own.exists() {
            publish(own, &free_aside_path(parent, &leaf)?)?;
        }
        publish(&staging, own)?;
        Ok(copied)
    });
    if result.is_ok() {
        if let Some((own_config, release_config)) = config_dirs {
            // Geometry is a convenience: a failure here keeps the seeded config.
            let _ = seed_window_state(&own_config, &release_config);
        }
    }
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result.map(Seeded::Copied)
}

fn stage(release: &Path, staging: &Path) -> io::Result<Vec<&'static str>> {
    fs::create_dir_all(staging)?;
    let mut copied = Vec::new();
    for entry in CONFIG_ENTRIES {
        let from = release.join(entry);
        if fs::symlink_metadata(&from).is_ok() {
            copy_tree(&from, &staging.join(entry))?;
            copied.push(*entry);
        }
    }
    tine_store::directory_durability::sync_directory_entry(staging)?;
    Ok(copied)
}

fn seed_window_state(own_config: &Path, release_config: &Path) -> io::Result<()> {
    let from = release_config.join(WINDOW_STATE);
    let to = own_config.join(WINDOW_STATE);
    if !from.is_file() || fs::symlink_metadata(&to).is_ok() {
        return Ok(());
    }
    fs::create_dir_all(own_config)?;
    let staged = own_config.join(format!("{WINDOW_STATE}.seeding"));
    copy_tree(&from, &staged)?;
    publish(&staged, &to)
}

fn free_aside_path(parent: &Path, leaf: &str) -> io::Result<PathBuf> {
    (0..64)
        .map(|n| parent.join(format!("{leaf}.pre-seed.{n}")))
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free aside name"))
}

#[cfg(test)]
#[path = "experiment_config_seed_tests.rs"]
mod tests;
