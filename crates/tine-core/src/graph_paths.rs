//! String-only graph path presentation: Windows verbatim prefixes and the iOS
//! sandbox container. Filesystem identity stays with the caller's canonical
//! PathBuf; this boundary never resolves or writes files.

use std::path::Path;

/// The one answer to "where is this graph, as the user is shown it" (GH #926,
/// IOS-PATH-926). On iOS a graph inside Tine's Documents folder is shown as
/// `Documents/<path inside it>`, whose last component is the folder name: the
/// sandbox prefix (`/private/var/mobile/Containers/Data/Application/<uuid>/`)
/// is noise that changes with every reinstall. Every other path, and every other
/// platform, gets [`normalize_graph_path`] only. Copying a path still copies the
/// canonical one; this spelling is never used for identity or file access.
pub fn display_graph_path(path: &str) -> String {
    if cfg!(target_os = "ios") {
        if let Some(shown) = ios_documents_display(path) {
            return shown;
        }
    }
    normalize_graph_path(path)
}

/// Pure part of [`display_graph_path`], testable on every host.
fn ios_documents_display(path: &str) -> Option<String> {
    let (_, _, inside) = ios_container_parts(Path::new(path))?;
    if inside.is_empty() {
        return None;
    }
    Some(format!("Documents/{}", inside.join("/")))
}

/// Splits an iOS app-container path into (what precedes `Containers`, the
/// container id, the components below `Documents`). Rejects relative paths,
/// `.`/`..` components and anything that is not a UUID container.
pub fn ios_container_parts(path: &Path) -> Option<(Vec<String>, uuid::Uuid, Vec<String>)> {
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

pub fn normalize_graph_path(path: &str) -> String {
    let Some(rest) = path.strip_prefix(r"\\?\") else {
        return path.to_owned();
    };
    let (candidate, components) = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        let parts: Vec<_> = unc.split('\\').collect();
        if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
            return path.to_owned();
        }
        (format!(r"\\{unc}"), parts)
    } else if rest.as_bytes().get(1) == Some(&b':')
        && rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && rest.as_bytes().get(2) == Some(&b'\\')
    {
        (rest.to_owned(), rest[3..].split('\\').collect())
    } else {
        return path.to_owned();
    };
    // dunce 1.0.5: UTF-16 length bounds, valid DOS components, no dot/space
    // suffix, reserved stem, or literal . / .. components. Unlike dunce's
    // disk-only simplifier, also admit UNC with these same conservative rules.
    if candidate.encode_utf16().count() > 260
        || components.iter().enumerate().any(|(index, name)| {
            if name.is_empty() {
                return index + 1 != components.len(); // a trailing separator is safe
            }
            let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
            let reserved = [
                "AUX", "NUL", "PRN", "CON", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9", "COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³",
            ];
            name.encode_utf16().count() > 255
                || name.ends_with(['.', ' '])
                || name
                    .chars()
                    .any(|c| c <= '\u{1f}' || "<>:\"/\\|?*".contains(c))
                || reserved
                    .iter()
                    .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        })
    {
        path.to_owned()
    } else {
        candidate
    }
}

/// Windows tools bypass WSL1's Linux metadata in LocalState/rootfs. Network
/// WSL routes are supported and must never be classified as the unsafe route.
pub fn is_windows_wsl_rootfs(path: &str) -> bool {
    let path = normalize_graph_path(path).to_ascii_lowercase();
    let disk = path.strip_prefix(r"\\?\").unwrap_or(&path);
    if disk.as_bytes().get(1) != Some(&b':') || disk.as_bytes().get(2) != Some(&b'\\') {
        return false;
    }
    let parts: Vec<_> = disk.split('\\').collect();
    parts.windows(4).any(|parts| {
        parts[0] == "packages"
            && !parts[1].is_empty()
            && parts[2] == "localstate"
            && parts[3] == "rootfs"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_metadata_preserves_identity_and_supplies_a_display_path() {
        let config = crate::config::Config::default();
        let format = crate::date::JournalFormat::new(None, None);
        let meta = crate::model::GraphMeta::from_config(r"\\?\C:\x".into(), &config, &format);
        assert_eq!(meta.root, r"\\?\C:\x");
        assert_eq!(meta.display_root, r"C:\x");
    }

    #[test]
    fn wsl_rootfs_is_detected_but_supported_network_routes_are_not() {
        for path in [
            r"C:\Users\m\AppData\Local\Packages\Ubuntu_123\LocalState\rootfs",
            r"\\?\C:\Users\m\APPDATA\LOCAL\PACKAGES\Ubuntu_123\LOCALSTATE\ROOTFS\home\m",
        ] {
            assert!(is_windows_wsl_rootfs(path), "{path}");
        }
        for path in [
            r"\\wsl$\Ubuntu\home\m",
            r"\\wsl.localhost\Ubuntu\home\m",
            r"C:\Users\m\Packages\Ubuntu\LocalState\rootfs-notes",
            r"C:\Users\m\LocalState\rootfs",
            "/home/m/rootfs",
        ] {
            assert!(!is_windows_wsl_rootfs(path), "{path}");
        }
    }

    #[test]
    fn representable_windows_paths_have_no_verbatim_prefix() {
        for (input, expected) in [
            (r"\\?\C:\x", r"C:\x"),
            (r"\\?\UNC\server\share\x", r"\\server\share\x"),
            (r"C:\x", r"C:\x"),
            ("/graphs/x", "/graphs/x"),
        ] {
            assert_eq!(normalize_graph_path(input), expected);
        }
    }

    #[test]
    fn paths_that_need_verbatim_semantics_are_preserved() {
        for input in [
            r"\\?\C:\CON",
            r"\\?\C:\con .txt",
            r"\\?\C:\NUL.tar.gz",
            r"\\?\C:\x.",
            r"\\?\C:\x ",
            r"\\?\C:\a\..\b",
            r"\\?\C:\a/b",
            r"\\?\C:\a:stream",
            r"\\?\GLOBALROOT\Device\x",
            r"\\?\UNC\server\share\CON",
        ] {
            assert_eq!(normalize_graph_path(input), input);
        }
        let long = format!(r"\\?\C:\{}\xy", "a".repeat(255));
        assert_eq!(normalize_graph_path(&long), long);
        let unicode = format!(r"\\?\C:\{}", "😀".repeat(128));
        assert_eq!(normalize_graph_path(&unicode), unicode);
    }

    const IOS_ID: &str = "0F8B2C1E-7A3D-4C55-9B21-1D2E3F4A5B6C";

    #[test]
    fn ios_graph_shows_documents_relative_path_and_keeps_the_folder_name() {
        for (full, shown) in [
            ("Tine graph", "Documents/Tine graph"),
            ("Graphs/Work notes", "Documents/Graphs/Work notes"),
        ] {
            for prefix in ["/private/var/mobile", "/var/mobile"] {
                let path =
                    format!("{prefix}/Containers/Data/Application/{IOS_ID}/Documents/{full}");
                assert_eq!(
                    ios_documents_display(&path).as_deref(),
                    Some(shown),
                    "{path}"
                );
            }
        }
    }

    #[test]
    fn ios_display_leaves_other_paths_alone() {
        for path in [
            format!("/private/var/mobile/Containers/Data/Application/{IOS_ID}/Documents"),
            format!("/private/var/mobile/Containers/Data/Application/{IOS_ID}/Library/g"),
            format!("/private/var/mobile/Containers/Data/Application/{IOS_ID}/Documents/../g"),
            "/private/var/mobile/Containers/Data/Application/not-a-uuid/Documents/g".to_owned(),
            "/private/var/mobile/Library/Mobile Documents/com~apple~CloudDocs/g".to_owned(),
            "/home/m/graph".to_owned(),
            "Documents/g".to_owned(),
        ] {
            assert_eq!(ios_documents_display(&path), None, "{path}");
        }
    }

    // The rewrite is iOS-only: the same container-shaped path on any other
    // platform is shown as the plain (Windows-normalized) path.
    #[test]
    #[cfg(not(target_os = "ios"))]
    fn other_platforms_show_the_plain_path() {
        let path = format!("/private/var/mobile/Containers/Data/Application/{IOS_ID}/Documents/g");
        assert_eq!(display_graph_path(&path), path);
        assert_eq!(display_graph_path(r"\\?\C:\x"), r"C:\x");
    }
}
