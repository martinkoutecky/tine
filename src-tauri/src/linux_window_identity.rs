//! Linux shell identity for raw binaries and native windows.
//!
//! Wayland compositors use `xdg_toplevel.app_id` to resolve the desktop entry
//! (and therefore the window-list/titlebar icon). Tauri's `enableGTKAppId`
//! cannot be used here: GTK registers a unique application before Tauri's
//! single-instance plugin runs, consuming second launches such as
//! `tine --capture`. Assign the app ID per window instead.

use std::{
    env,
    ffi::CString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use gtk::{
    gdk::prelude::DisplayExtManual,
    glib::{prelude::ObjectExt, translate::ToGlibPtr},
    prelude::WidgetExt,
};

const APP_ID: &str = "page.tine.Tine";
const DESKTOP_FILE: &str = "page.tine.Tine.desktop";
const MANAGED_MARKER: &str = "X-Tine-Managed=true";

const ICONS: &[(&str, &[u8])] = &[
    ("512x512", include_bytes!("../icons/icon.png")),
    ("256x256", include_bytes!("../icons/128x128@2x.png")),
    ("128x128", include_bytes!("../icons/128x128.png")),
    ("64x64", include_bytes!("../icons/64x64.png")),
    ("32x32", include_bytes!("../icons/32x32.png")),
];

#[derive(Debug, Eq, PartialEq)]
enum InstallOutcome {
    Installed,
    PreservedUserEntry,
    PackagedEntry,
}

fn desktop_exec_path(executable: &Path) -> String {
    let raw = executable.to_string_lossy();
    let mut escaped = String::with_capacity(raw.len() + 2);
    escaped.push('"');
    for character in raw.chars() {
        match character {
            '\\' | '"' | '`' | '$' => {
                escaped.push('\\');
                escaped.push(character);
            }
            '%' => escaped.push_str("%%"),
            '\n' | '\r' => escaped.push('_'),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn desktop_entry(executable: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=Tine\n\
GenericName=Outliner\n\
Comment=Fast, local-first Logseq-compatible outliner\n\
Exec={} %U\n\
Icon={APP_ID}\n\
Terminal=false\n\
Categories=Office;Utility;\n\
Keywords=outliner;logseq;markdown;notes;org;journal;\n\
StartupNotify=true\n\
StartupWMClass={APP_ID}\n\
{MANAGED_MARKER}\n",
        desktop_exec_path(executable)
    )
}

fn atomic_write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tine-identity");
    let temporary = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn install_into(
    data_home: &Path,
    system_data_dirs: &[PathBuf],
    executable: &Path,
) -> io::Result<InstallOutcome> {
    let desktop_path = data_home.join("applications").join(DESKTOP_FILE);
    if desktop_path.exists() {
        if fs::symlink_metadata(&desktop_path)?
            .file_type()
            .is_symlink()
        {
            return Ok(InstallOutcome::PreservedUserEntry);
        }
        let Ok(existing) = fs::read_to_string(&desktop_path) else {
            return Ok(InstallOutcome::PreservedUserEntry);
        };
        if !existing.lines().any(|line| line == MANAGED_MARKER) {
            return Ok(InstallOutcome::PreservedUserEntry);
        }
    } else if system_data_dirs
        .iter()
        .any(|base| base.join("applications").join(DESKTOP_FILE).is_file())
    {
        return Ok(InstallOutcome::PackagedEntry);
    }

    // Write the icon payloads first, then publish the desktop entry that refers
    // to them. A compositor can never observe a new entry with a missing icon.
    for (size, bytes) in ICONS {
        atomic_write_if_changed(
            &data_home
                .join("icons/hicolor")
                .join(size)
                .join("apps")
                .join(format!("{APP_ID}.png")),
            bytes,
        )?;
    }
    atomic_write_if_changed(&desktop_path, desktop_entry(executable).as_bytes())?;
    Ok(InstallOutcome::Installed)
}

/// Make the desktop-entry/icon lookup available for unpackaged release binaries
/// before the first Wayland window is mapped. Packages already provide these
/// files and user-managed entries are never overwritten.
pub(crate) fn install_desktop_identity() {
    if cfg!(debug_assertions)
        || env::var_os("FLATPAK_ID").is_some()
        || env::var_os("SNAP").is_some()
    {
        return;
    }
    let Some(data_home) = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(dirs::data_dir)
    else {
        return;
    };
    let system_data_dirs = env::var_os("XDG_DATA_DIRS")
        .map(|value| env::split_paths(&value).collect())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    let executable = env::var_os("APPIMAGE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::current_exe().ok());
    let Some(executable) = executable else {
        return;
    };
    if let Err(error) = install_into(&data_home, &system_data_dirs, &executable) {
        crate::debug::diag(format!("could not install Linux desktop identity: {error}"));
    }
}

fn set_wayland_app_id(gdk_window: &gtk::gdk::Window) {
    let app_id = CString::new(APP_ID).expect("static app ID has no NUL");
    // SAFETY: this runs on GTK's main thread, only for a GDK Wayland display,
    // and `gdk_window` remains alive for the duration of the call.
    unsafe {
        let raw: *mut gtk::gdk::ffi::GdkWindow = gdk_window.to_glib_none().0;
        let pointer = raw.cast::<gdk_wayland_sys::GdkWaylandWindow>();
        let _ = gdk_wayland_sys::gdk_wayland_window_set_application_id(pointer, app_id.as_ptr());
    }
}

fn attach_to_gdk_window(gdk_window: &gtk::gdk::Window) {
    // If the xdg_toplevel already exists (for example, a dynamically-created
    // graph window built visible), update it immediately.
    set_wayland_app_id(gdk_window);

    // GtkWidget::realize only creates the GdkWindow. GTK creates the Wayland
    // xdg_toplevel later, while mapping it, and silently ignores
    // gdk_wayland_window_set_application_id before that point. This GDK signal
    // is emitted after the xdg_toplevel exists and before its first surface
    // commit, immediately after GTK has assigned its executable-name fallback.
    // It was added late in GTK 3.24, so look it up rather than panicking on
    // older supported WebKitGTK stacks.
    if let Some(signal) =
        gtk::glib::subclass::SignalId::lookup("xdg-toplevel-realized", gdk_window.type_())
    {
        let gdk_window = gdk_window.clone();
        gdk_window
            .clone()
            .connect_local_id(signal, None, false, move |_| {
                set_wayland_app_id(&gdk_window);
                None
            });
    }
}

fn attach_to_gtk_window(window: &gtk::ApplicationWindow) {
    if let Some(gdk_window) = window.window() {
        attach_to_gdk_window(&gdk_window);
    }
}

fn set_mapped_wayland_app_id(window: &gtk::ApplicationWindow) {
    if let Some(gdk_window) = window.window() {
        // xdg-shell explicitly permits changing app_id after mapping. This is
        // the compatibility path for GTK versions without
        // xdg-toplevel-realized, and a harmless verification update otherwise.
        set_wayland_app_id(&gdk_window);
    }
}

/// Give a Tauri top-level window the stable Wayland identity whose desktop entry
/// supplies Tine's icon. The configured windows are usually realized by setup;
/// the signal paths cover dynamically-created, lazily-realized, and older GTK
/// windows whose xdg_toplevel appears only while mapping.
pub(crate) fn apply_to_window(window: &tauri::WebviewWindow) {
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    if !gtk_window.display().backend().is_wayland() {
        return;
    }
    if gtk_window.is_realized() {
        attach_to_gtk_window(&gtk_window);
    } else {
        gtk_window.connect_realize(attach_to_gtk_window);
    }
    gtk_window.connect_map(set_mapped_wayland_app_id);
    if gtk_window.is_mapped() {
        set_mapped_wayland_app_id(&gtk_window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "tine-linux-window-identity-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn installs_matching_desktop_entry_and_all_icon_sizes() {
        let root = temp_root("install");
        let data_home = root.join("user");
        let executable = Path::new("/opt/Tine Builds/tine%preview");

        assert_eq!(
            install_into(&data_home, &[root.join("system")], executable).unwrap(),
            InstallOutcome::Installed
        );
        let desktop = fs::read_to_string(
            data_home
                .join("applications")
                .join("page.tine.Tine.desktop"),
        )
        .unwrap();
        assert!(desktop.contains("Exec=\"/opt/Tine Builds/tine%%preview\" %U"));
        assert!(desktop.contains("Icon=page.tine.Tine"));
        assert!(desktop.contains("StartupWMClass=page.tine.Tine"));
        assert!(desktop.contains(MANAGED_MARKER));
        for (size, bytes) in ICONS {
            assert_eq!(
                fs::read(
                    data_home
                        .join("icons/hicolor")
                        .join(size)
                        .join("apps/page.tine.Tine.png")
                )
                .unwrap(),
                *bytes
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserves_user_owned_desktop_entry() {
        let root = temp_root("preserve");
        let data_home = root.join("user");
        let desktop_path = data_home.join("applications").join(DESKTOP_FILE);
        fs::create_dir_all(desktop_path.parent().unwrap()).unwrap();
        fs::write(&desktop_path, "[Desktop Entry]\nName=My Tine launcher\n").unwrap();

        assert_eq!(
            install_into(&data_home, &[], Path::new("/new/tine")).unwrap(),
            InstallOutcome::PreservedUserEntry
        );
        assert_eq!(
            fs::read_to_string(desktop_path).unwrap(),
            "[Desktop Entry]\nName=My Tine launcher\n"
        );
        assert!(!data_home.join("icons").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn preserves_non_utf8_user_owned_desktop_entry() {
        let root = temp_root("preserve-non-utf8");
        let data_home = root.join("user");
        let desktop_path = data_home.join("applications").join(DESKTOP_FILE);
        fs::create_dir_all(desktop_path.parent().unwrap()).unwrap();
        fs::write(&desktop_path, [0xff, 0xfe]).unwrap();

        assert_eq!(
            install_into(&data_home, &[], Path::new("/new/tine")).unwrap(),
            InstallOutcome::PreservedUserEntry
        );
        assert_eq!(fs::read(desktop_path).unwrap(), [0xff, 0xfe]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn package_owned_entry_wins_when_no_user_entry_exists() {
        let root = temp_root("package");
        let data_home = root.join("user");
        let system_home = root.join("system");
        let system_desktop = system_home.join("applications").join(DESKTOP_FILE);
        fs::create_dir_all(system_desktop.parent().unwrap()).unwrap();
        fs::write(system_desktop, "[Desktop Entry]\nName=Tine\n").unwrap();

        assert_eq!(
            install_into(&data_home, &[system_home], Path::new("/usr/bin/tine")).unwrap(),
            InstallOutcome::PackagedEntry
        );
        assert!(!data_home.exists());
        let _ = fs::remove_dir_all(root);
    }
}
