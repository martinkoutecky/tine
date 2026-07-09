// One-shot migration of the app-data directory after the app-identifier rename
// `dev.tine.app` -> `page.tine.app` (v0.4.x). The OS app-data dir is keyed by the
// bundle identifier, so without this the rename would silently orphan the user's
// settings, persisted tab/zoom session, and automatic backups. We move the old
// dir into place on first launch, guarded so we never clobber real new-id data and
// never touch a directory that isn't demonstrably ours.
//
// Scope: app-data dir only (settings / session / backups — the state that matters).
// Window geometry (tauri-plugin-window-state, in the config dir) may reset once;
// that's covered by the one-time toast the frontend shows after a migration.
//
// Android is intentionally NOT handled here: an applicationId change is a new app
// at the OS level and app-private storage cannot be migrated across it. Tine's
// Android data is minimal (the graph lives in external files), so the only residual
// is re-picking the graph folder once. This shim is desktop-only in effect.

use std::path::Path;
use tauri::Manager;

/// The identifier Tine shipped under before the rename.
const LEGACY_IDENTIFIER: &str = "dev.tine.app";

/// One-shot flag: true iff this launch actually moved a legacy dir into place, so
/// the frontend can explain the (possible) reset. Read-and-cleared exactly once by
/// `take_identifier_migration_notice`.
pub(crate) struct MigrationNotice(pub std::sync::atomic::AtomicBool);

/// Belt-and-suspenders check that `dir` is a Tine app-data dir we created — beyond
/// the already globally-unique `dev.tine.app` identifier. Any one of our own
/// artifacts is proof enough.
fn looks_like_tine_data(dir: &Path) -> bool {
    dir.join("tine-settings.json").exists()
        || dir.join("tine-session.json").exists()
        || dir.join("backups").is_dir()
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

/// Core migration, factored out of the Tauri handle so it is unit-testable: given
/// the CURRENT (new-identifier) app-data dir, move the sibling legacy dir into it.
/// Returns true iff something was moved.
fn migrate_app_data_dir(new_dir: &Path) -> bool {
    // The legacy dir is a sibling: same parent, legacy identifier as the leaf.
    // (app-data dirs differ only in the trailing identifier component across OSes,
    // so this avoids hand-rolling per-platform base-dir logic.)
    let Some(parent) = new_dir.parent() else {
        return false;
    };
    let old_dir = parent.join(LEGACY_IDENTIFIER);
    // Nothing to do if the ids resolve the same, or there's no legacy dir.
    if old_dir == new_dir || !old_dir.is_dir() {
        return false;
    }
    // Never overwrite a real new-identifier install.
    if dir_has_entries(new_dir) {
        return false;
    }
    // Only ever move a directory that is demonstrably ours.
    if !looks_like_tine_data(&old_dir) {
        return false;
    }
    // An empty new dir may already exist (the OS/Tauri can pre-create it); remove
    // it so the atomic rename can take the name.
    if new_dir.exists() {
        let _ = std::fs::remove_dir(new_dir);
    }
    if let Some(gp) = new_dir.parent() {
        let _ = std::fs::create_dir_all(gp);
    }
    // old & new share a parent => same filesystem => rename is atomic and cheap.
    match std::fs::rename(&old_dir, new_dir) {
        Ok(()) => true,
        Err(_) => {
            // Defensive fallback (e.g. exotic mounts): copy then best-effort remove.
            if copy_dir_all(&old_dir, new_dir).is_ok() {
                let _ = std::fs::remove_dir_all(&old_dir);
                true
            } else {
                false
            }
        }
    }
}

/// Run the migration for the app's real app-data dir. Call as the FIRST thing in
/// `setup()`, before anything reads settings/session/backups. Returns true iff a
/// migration happened this launch.
pub(crate) fn run(app: &tauri::AppHandle) -> bool {
    let Ok(new_dir) = app.path().app_data_dir() else {
        return false;
    };
    migrate_app_data_dir(&new_dir)
}

/// Command: return true ONCE if this launch migrated the legacy app-data dir, then
/// clear the flag so a later reload doesn't re-toast. The frontend calls this on
/// boot and shows an explanatory toast when it returns true.
#[tauri::command]
pub(crate) fn take_identifier_migration_notice(app: tauri::AppHandle) -> bool {
    app.try_state::<MigrationNotice>()
        .map(|n| n.0.swap(false, std::sync::atomic::Ordering::SeqCst))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tine-migrate-{}-{}", std::process::id(), tag))
    }

    fn write(p: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(p).unwrap();
        std::fs::write(p.join(name), body).unwrap();
    }

    #[test]
    fn migrates_a_genuine_legacy_dir() {
        let base = tmp("ok");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join("page.tine.app");
        write(&old, "tine-settings.json", "{\"backupKeep\":5}");
        std::fs::create_dir_all(old.join("backups")).unwrap();

        assert!(migrate_app_data_dir(&new));
        assert!(new.join("tine-settings.json").exists());
        assert!(new.join("backups").is_dir());
        assert!(!old.exists(), "legacy dir consumed by the move");
        // Idempotent: nothing left to migrate.
        assert!(!migrate_app_data_dir(&new));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn never_clobbers_existing_new_data() {
        let base = tmp("noclobber");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join("page.tine.app");
        write(&old, "tine-settings.json", "{\"old\":true}");
        write(&new, "tine-settings.json", "{\"new\":true}");

        assert!(!migrate_app_data_dir(&new), "must not migrate over real new data");
        // New data untouched; old left alone.
        let got = std::fs::read_to_string(new.join("tine-settings.json")).unwrap();
        assert!(got.contains("\"new\""));
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ignores_a_dir_that_is_not_ours() {
        let base = tmp("notours");
        let _ = std::fs::remove_dir_all(&base);
        let old = base.join("dev.tine.app");
        let new = base.join("page.tine.app");
        // A same-named dir with none of Tine's artifacts (paranoia guard).
        write(&old, "someone-elses.json", "{}");

        assert!(!migrate_app_data_dir(&new), "no Tine artifacts => do not touch");
        assert!(!new.exists());
        assert!(old.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_legacy_dir_is_a_noop() {
        let base = tmp("none");
        let _ = std::fs::remove_dir_all(&base);
        let new = base.join("page.tine.app");
        std::fs::create_dir_all(&new).unwrap();
        assert!(!migrate_app_data_dir(&new));
        let _ = std::fs::remove_dir_all(&base);
    }
}
