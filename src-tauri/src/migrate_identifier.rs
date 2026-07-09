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
// once; that's covered by the one-time toast the frontend shows after a migration.
//
// Android is intentionally NOT handled and keeps applicationId `page.tine.app`.
// An applicationId change is a new app at the OS level and app-private storage
// cannot be migrated across it. Tine's Android data is minimal (the graph lives in
// external files), so the only residual is re-picking the graph folder once. This
// shim is desktop-only in effect.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// The identifier Tine currently ships under (must match tauri.conf.json).
const CURRENT_IDENTIFIER: &str = "page.tine.Tine";

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
    dir_has_entries(&dir.join("backups"))
}

fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut rd| rd.next().is_some())
        .unwrap_or(false)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Core migration, factored out of the platform dir lookup so it is unit-testable:
/// given the CURRENT (new-identifier) app-data dir, move the newest sibling legacy
/// dir into it. Returns true iff something was moved.
///
/// Legacy dirs are siblings: same parent, legacy identifier as the leaf. (App-data
/// dirs differ only in the trailing identifier component across OSes, so this avoids
/// hand-rolling per-platform base-dir logic.)
fn migrate_app_data_dir(new_dir: &Path) -> bool {
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
        // over as one consistent unit. The discarded scaffolding is regenerated.
        if new_dir.exists() && std::fs::remove_dir_all(new_dir).is_err() {
            return false;
        }
        let _ = std::fs::create_dir_all(parent);
        // old & new share a parent => same filesystem => rename is atomic and cheap.
        match std::fs::rename(&old_dir, new_dir) {
            Ok(()) => return true,
            Err(_) => {
                // Defensive fallback (e.g. exotic mounts): copy then best-effort remove.
                if copy_dir_all(&old_dir, new_dir).is_ok() {
                    let _ = std::fs::remove_dir_all(&old_dir);
                    return true;
                }
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
fn current_app_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|base| base.join(CURRENT_IDENTIFIER))
}

/// Run the migration for the app's real app-data dir. Call this at the TOP of
/// `lib::run()`, BEFORE `tauri::Builder` (see the module note on timing). Records a
/// one-shot flag so the frontend can toast about the migration.
#[cfg(not(target_os = "android"))]
pub(crate) fn run_early() {
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
/// boot and shows an explanatory toast when it returns true.
#[tauri::command]
pub(crate) fn take_identifier_migration_notice() -> bool {
    MIGRATED.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
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
        let new = base.join("page.tine.Tine");
        write(&old, "someone-elses.json", "{}"); // none of Tine's artifacts

        assert!(!migrate_app_data_dir(&new));
        assert!(!new.exists());
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_legacy_dir_is_a_noop() {
        let base = tmp("none");
        let _ = std::fs::remove_dir_all(&base);
        let new = base.join("page.tine.Tine");
        std::fs::create_dir_all(&new).unwrap();
        assert!(!migrate_app_data_dir(&new));
        let _ = std::fs::remove_dir_all(&base);
    }
}
