// One-shot migration of the app-data directory across the app-identifier renames
// `dev.logseqclaude.app` -> `dev.tine.app` -> `page.tine.app` -> `page.tine.Tine`.
// The OS app-data dir is keyed by the bundle identifier, and on Linux WebKitGTK
// ALSO stores the webview's localStorage / IndexedDB there. localStorage is where
// the frontend keeps the last-opened graph path, the open-tab session, recent
// pages and the right sidebar — so without this, a rename silently orphans not
// just settings and backups but the *entire working state*, and the app boots to
// the Welcome screen as if freshly installed.
//
// TIMING IS LOAD-BEARING: this MUST run before Tauri builds the webview. WebKitGTK's
// WebsiteDataManager eagerly creates the new-identifier data dir (WebKitCache/,
// storage/, …) while the Tauri `Builder` is assembled — BEFORE the `setup()` hook
// runs. If we migrated from inside `setup()` we'd always find the new dir already
// populated with fresh (empty) scaffolding and back off. So `run_early()` is called
// at the very top of `lib::run()`, before `tauri::Builder::default()`, when the new
// dir does not yet exist. (This was the original shipped bug: it migrated too late.)
//
// Scope: the whole app-data dir — settings, session, backups AND the WebKit
// localStorage store — moved as one unit so the graph reopens and the session is
// intact. Window geometry (tauri-plugin-window-state, in the *config* dir) may reset
// once; the one-shot command exposes this to the frontend for a migration notice.
//
// Android is intentionally NOT handled and keeps applicationId `page.tine.app`.
// An applicationId change is a new app at the OS level and app-private storage
// cannot be migrated across it. Tine's Android data is minimal (the graph lives in
// external files), so the only residual is re-picking the graph folder once. This
// shim is desktop-only in effect.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::device_io::{copy_tree as copy_dir_all, publish_directory_entry};

/// Identifiers Tine shipped under before, NEWEST FIRST. We migrate from the most
/// recent one that still holds our data, so a user who upgraded through the whole
/// chain gets their latest working state (not a stale prototype dir).
const LEGACY_IDENTIFIERS: &[&str] = &["page.tine.app", "dev.tine.app", "dev.logseqclaude.app"];

/// Set true iff `run_early()` actually moved a legacy dir into place this launch, so
/// the frontend can explain the (possible) window-geometry / prefs reset. Read and
/// cleared exactly once by `take_identifier_migration_notice`. This is a process
/// global because the migration runs before the Tauri app (and its managed state)
/// exists.
static MIGRATED: AtomicBool = AtomicBool::new(false);

/// Belt-and-suspenders check that `dir` is a Tine app-data dir we created — beyond
/// the already globally-unique identifier. Any one of our own artifacts is proof
/// enough. Used to decide a legacy dir is genuinely ours before moving it.
fn looks_like_tine_data(dir: &Path) -> bool {
    dir.join("tine-settings.json").exists()
        || dir.join("tine-session.json").exists()
        || dir.join("backups").is_dir()
}

/// Does `dir` hold REAL user working state (not just fresh WebKit/Tauri
/// scaffolding)? A non-empty `backups/` is the one bulletproof signal: Tine snapshots
/// a backup whenever a graph is OPENED (backup.rs, called from load_graph) and the
/// keep-count is clamped to >= 1, so a non-empty `backups/` proves the user actually
/// opened a graph in THIS identifier's dir. We deliberately do NOT count:
///   - `tine-settings.json` — written with defaults even on the Welcome screen;
///   - `tine-session.json`  — the window-close handler flushes a session (with the
///     default "journals" tab) on EVERY quit, including a Welcome-only launch, so
///     its mere presence does not mean a graph was ever opened;
///   - a bare WebKit `storage/` — localStorage may hold only Welcome-tour UI state.
/// Counting any of those would wrongly block a backfill for exactly the user we need
/// to rescue: someone who launched the renamed build, saw Welcome, and quit.
fn has_real_user_data(dir: &Path) -> bool {
    match std::fs::read_dir(dir.join("backups")) {
        Ok(mut rd) => rd.next().is_some(),
        // A missing backups/ is the ordinary "no graph was ever opened under
        // this identifier" case. Any OTHER error — EACCES, EIO, a stale
        // mount, ENOTDIR — means we cannot PROVE the directory is disposable,
        // and the caller's next step is to replace its active location. Fail closed:
        // claim user data and skip the migration (audit 4, D3).
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Core migration, factored out of the platform dir lookup so it is unit-testable:
/// given the CURRENT (new-identifier) app-data dir, move the newest sibling legacy
/// dir into it. Returns true iff something was moved.
///
/// Legacy dirs are siblings: same parent, legacy identifier as the leaf. (App-data
/// dirs differ only in the trailing identifier component across OSes, so this avoids
/// hand-rolling per-platform base-dir logic.)
fn migrate_app_data_dir(new_dir: &Path) -> bool {
    migrate_after_park(new_dir, || {})
}

fn migrate_after_park(new_dir: &Path, after_park: impl Fn()) -> bool {
    let Some(parent) = new_dir.parent() else {
        return false;
    };
    // Never touch a dir that already holds real working state — the user has been
    // using this identifier; migrating a legacy dir over it would destroy data.
    if has_real_user_data(new_dir) {
        return false;
    }
    // Pick the newest legacy dir that is demonstrably ours.
    for legacy in LEGACY_IDENTIFIERS {
        let old_dir = parent.join(legacy);
        if old_dir == new_dir || !old_dir.is_dir() || !looks_like_tine_data(&old_dir) {
            continue;
        }
        // The new dir is absent, or holds only disposable WebKit/Tauri scaffolding
        // (caches + an empty localStorage) that WebKitGTK pre-created — guaranteed
        // by has_real_user_data() above. Replace it WHOLESALE with the legacy dir so
        // localStorage (the graph path + session), settings and backups all carry
        // over as one consistent unit. The scaffolding is renamed ASIDE, not
        // deleted, including after the replacement is in place — destruction
        // before placement turned any later failure into silent loss of
        // whatever the guard mis-assessed (audit 4, D3).
        let mut aside: Option<std::path::PathBuf> = None;
        if new_dir.exists() {
            let Some(leaf) = new_dir.file_name().map(|leaf| leaf.to_owned()) else {
                return false;
            };
            let mut parked = None;
            for attempt in 0..8u32 {
                let candidate = parent.join(format!(
                    "{}.pre-migration.{attempt}",
                    leaf.to_string_lossy()
                ));
                if candidate.exists() {
                    continue;
                }
                if publish_directory_entry(new_dir, &candidate).is_ok() {
                    parked = Some(candidate);
                }
                break;
            }
            let Some(parked) = parked else {
                return false;
            };
            aside = Some(parked);
        }
        let restore_aside = |aside: &Option<std::path::PathBuf>| {
            if let Some(parked) = aside {
                let _ = publish_directory_entry(parked, new_dir);
            }
        };
        let _ = std::fs::create_dir_all(parent);
        after_park();
        // old & new share a parent => same filesystem => rename is atomic and cheap.
        match publish_directory_entry(&old_dir, new_dir) {
            Ok(()) => {
                return true;
            }
            Err(_) => {
                // Defensive fallback (e.g. exotic mounts): stage the copy in a
                // sibling temp dir and move it into place only when complete,
                // so a partial copy (ENOSPC) can never become the destination
                // and permanently block the migration.
                let staging = parent.join(format!(
                    "{}.migrating",
                    new_dir
                        .file_name()
                        .map(|leaf| leaf.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "tine-app-data".into())
                ));
                let _ = std::fs::remove_dir_all(&staging);
                if copy_dir_all(&old_dir, &staging).is_ok()
                    && publish_directory_entry(&staging, new_dir).is_ok()
                {
                    return true;
                }
                let _ = std::fs::remove_dir_all(&staging);
                restore_aside(&aside);
                return false;
            }
        }
    }
    false
}

/// Resolve the current app-data dir WITHOUT a Tauri handle (which doesn't exist yet
/// this early). `dirs::data_dir()` is exactly the base Tauri v2's `app_data_dir()`
/// joins the identifier onto (verified on Linux: `~/.local/share/<id>`, where
/// WebKitGTK also puts the webview store).
pub(crate) fn current_app_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|base| base.join(crate::app_identity::RELEASE_IDENTIFIER))
}

/// Run the migration for the app's real app-data dir. Call this at the TOP of
/// `lib::run()`, BEFORE `tauri::Builder` (see the module note on timing). Records a
/// one-shot flag so the frontend can toast about the migration.
#[cfg(not(target_os = "android"))]
pub(crate) fn run_early() {
    if crate::app_identity::APP_IDENTIFIER != crate::app_identity::RELEASE_IDENTIFIER {
        return;
    }
    if let Some(new_dir) = current_app_data_dir() {
        if migrate_app_data_dir(&new_dir) {
            MIGRATED.store(true, Ordering::SeqCst);
        }
    }
}

/// Android keeps applicationId `page.tine.app`; never migrate it to the desktop id.
#[cfg(target_os = "android")]
pub(crate) fn run_early() {}

/// Command: return true ONCE if this launch migrated a legacy app-data dir, then
/// clear the flag so a later reload doesn't re-toast. The frontend calls this on
/// boot can show an explanatory toast when it returns true.
#[tauri::command]
pub(crate) fn take_identifier_migration_notice() -> bool {
    MIGRATED.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn crash_child_after_parking() {
        let Some(root) = std::env::var_os("OG_R6_CRASH_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        migrate_after_park(&root.join(crate::app_identity::RELEASE_IDENTIFIER), || {
            std::fs::write(root.join("parked"), b"ready").unwrap();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
    }

    #[test]
    fn killed_migration_reopens_without_losing_either_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let old = root.join(LEGACY_IDENTIFIERS[0]);
        let new = root.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(
            &old,
            "tine-settings.json",
            r#"{"last_graph_path":"/graphs/notes"}"#,
        );
        write(
            &old.join("backups"),
            "master-snapshot",
            "legacy backup bytes",
        );
        write(
            &new.join("direct-files-projections"),
            "master-only",
            "current bytes",
        );
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "migrate_identifier::tests::crash_child_after_parking",
                "--nocapture",
            ])
            .env("OG_R6_CRASH_FIXTURE", root)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !root.join("parked").exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            root.join("parked").exists(),
            "child never reached the migration boundary"
        );
        assert!(!new.exists());
        assert!(
            migrate_app_data_dir(&new),
            "reopen must finish the interrupted migration"
        );
        assert_eq!(
            std::fs::read(new.join("backups/master-snapshot")).unwrap(),
            b"legacy backup bytes"
        );
        let aside = root.join(format!(
            "{}.pre-migration.0",
            crate::app_identity::RELEASE_IDENTIFIER
        ));
        assert_eq!(
            std::fs::read(aside.join("direct-files-projections/master-only")).unwrap(),
            b"current bytes"
        );
        let settings: serde_json::Value =
            serde_json::from_slice(&std::fs::read(new.join("tine-settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["last_graph_path"], "/graphs/notes");
    }

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-migrate-{}-{}", std::process::id(), tag))
    }

    fn write(p: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(p).unwrap();
        std::fs::write(p.join(name), body).unwrap();
    }

    /// Fresh WebKit/Tauri scaffolding: the dirs the webview pre-creates before we run.
    fn scaffold(new: &Path) {
        std::fs::create_dir_all(new.join("WebKitCache")).unwrap();
        std::fs::create_dir_all(new.join("storage")).unwrap();
        std::fs::write(new.join("storage/salt"), b"salt").unwrap();
        // A default settings file gets written even on the Welcome screen.
        std::fs::write(new.join("tine-settings.json"), b"{}").unwrap();
    }

    #[test]
    fn migrates_a_genuine_legacy_dir_when_new_is_absent() {
        let base = tmp("absent");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "tine-settings.json", "{\"backupKeep\":5}");
        write(&old, "SENTINEL", "graph-path-lives-in-localstorage");
        std::fs::create_dir_all(old.join("storage")).unwrap();

        assert!(migrate_app_data_dir(&new));
        assert!(
            new.join("SENTINEL").exists(),
            "localStorage-carrying dir moved"
        );
        assert!(new.join("storage").is_dir());
        assert!(!old.exists(), "legacy dir consumed by the move");
        assert!(!migrate_app_data_dir(&new), "idempotent");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn backfills_over_fresh_webkit_scaffolding() {
        // The regression that shipped: WebKit pre-created the new dir, so the old
        // guard bailed. Now we replace scaffolding-only dirs.
        let base = tmp("scaffold");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "SENTINEL", "real-data");
        std::fs::create_dir_all(old.join("backups")).unwrap();
        write(&old.join("backups"), "snap1", "x");
        scaffold(&new);

        assert!(migrate_app_data_dir(&new), "must backfill over scaffolding");
        assert!(new.join("SENTINEL").exists());
        assert!(new.join("backups/snap1").exists());
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_welcome_only_session_file_does_not_block_backfill() {
        // THE regression to guard: the user launched the renamed build, saw Welcome
        // and quit — the close handler flushed a `tine-session.json` (default
        // "journals" tab) into the new dir. That must NOT be mistaken for real use,
        // or the stranded legacy data never gets recovered.
        let base = tmp("welcome-session");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "tine-settings.json", "{}");
        write(&old, "SENTINEL", "real");
        std::fs::create_dir_all(old.join("backups")).unwrap();
        write(&old.join("backups"), "snap1", "x");
        // New dir = fresh scaffolding + a Welcome-close session (no backups!).
        scaffold(&new);
        write(
            &new,
            "tine-session.json",
            "{\"tabs\":[{\"history\":[{\"kind\":\"journals\"}]}]}",
        );

        assert!(
            migrate_app_data_dir(&new),
            "must still backfill over a Welcome session"
        );
        assert!(new.join("SENTINEL").exists());
        assert!(!old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn backups_in_new_also_block_clobber() {
        let base = tmp("noclobber-backups");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "tine-settings.json", "{\"old\":true}");
        std::fs::create_dir_all(new.join("backups")).unwrap();
        write(&new.join("backups"), "snap", "x");

        assert!(!migrate_app_data_dir(&new), "non-empty backups = real use");
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn migrates_from_page_tine_app_identifier() {
        let base = tmp("page-tine-app");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("page.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "tine-settings.json", "{\"old_id\":\"page.tine.app\"}");
        write(&old, "SENTINEL", "final-rename");
        std::fs::create_dir_all(old.join("storage")).unwrap();

        assert!(migrate_app_data_dir(&new));
        assert!(new.join("SENTINEL").exists());
        assert!(!old.exists(), "page.tine.app dir consumed by the move");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn prefers_newest_legacy_identifier() {
        let base = tmp("prefer");
        let _ = std::fs::remove_dir_all(&base);
        let older = base.join("dev.logseqclaude.app");
        let newer = base.join("dev.tine.app");
        let newest = base.join("page.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&older, "tine-settings.json", "{}");
        write(&older, "WHICH", "logseqclaude");
        write(&newer, "tine-settings.json", "{}");
        write(&newer, "WHICH", "tine");
        write(&newest, "tine-settings.json", "{}");
        write(&newest, "WHICH", "page");

        assert!(migrate_app_data_dir(&new));
        let which = std::fs::read_to_string(new.join("WHICH")).unwrap();
        assert_eq!(which, "page", "newest legacy wins");
        assert!(older.exists(), "older legacy left untouched");
        assert!(newer.exists(), "middle legacy left untouched");
        assert!(!newest.exists(), "newest legacy consumed");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn falls_back_to_oldest_when_only_it_exists() {
        let base = tmp("fallback");
        let _ = std::fs::remove_dir_all(&base);
        let older = base.join("dev.logseqclaude.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&older, "tine-settings.json", "{}");
        write(&older, "WHICH", "logseqclaude");

        assert!(migrate_app_data_dir(&new));
        assert_eq!(
            std::fs::read_to_string(new.join("WHICH")).unwrap(),
            "logseqclaude"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ignores_a_dir_that_is_not_ours() {
        let base = tmp("notours");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        write(&old, "someone-elses.json", "{}"); // none of Tine's artifacts

        assert!(!migrate_app_data_dir(&new));
        assert!(!new.exists());
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rollback_master_reads_the_moved_config_and_foreign_artifacts_byte_for_byte() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join(LEGACY_IDENTIFIERS[0]);
        let new = temp.path().join(crate::app_identity::RELEASE_IDENTIFIER);
        write(
            &old,
            "tine-settings.json",
            r#"{"last_graph_path":"/graphs/notes","future":true}"#,
        );
        for artifact in [
            "backups",
            "direct-files-projections",
            "direct-move-recovery",
            "concord-ledger",
            "conflict-capsules",
        ] {
            write(&old.join(artifact), "master-only", "unchanged bytes");
        }
        assert!(migrate_app_data_dir(&new));
        assert!(!migrate_app_data_dir(&new));
        let rollback: serde_json::Value =
            serde_json::from_slice(&std::fs::read(new.join("tine-settings.json")).unwrap())
                .unwrap();
        assert_eq!(rollback["last_graph_path"], "/graphs/notes");
        assert_eq!(rollback["future"], true);
        for artifact in [
            "backups",
            "direct-files-projections",
            "direct-move-recovery",
            "concord-ledger",
            "conflict-capsules",
        ] {
            assert_eq!(
                std::fs::read(new.join(artifact).join("master-only")).unwrap(),
                b"unchanged bytes"
            );
        }
    }

    #[test]
    fn no_legacy_dir_is_a_noop() {
        let base = tmp("none");
        let _ = std::fs::remove_dir_all(&base);
        let new = base.join(crate::app_identity::RELEASE_IDENTIFIER);
        std::fs::create_dir_all(&new).unwrap();
        assert!(!migrate_app_data_dir(&new));
        let _ = std::fs::remove_dir_all(&base);
    }
}
