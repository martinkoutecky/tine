//! Startup rendering policy, before GTK/WebKit creates any webviews (GH #777).

use std::ffi::OsStr;

const USER_FALLBACK: &str =
    "TINE_GPU=0 → set WEBKIT_DISABLE_DMABUF_RENDERER=1 (software rendering fallback)";
const NVIDIA_FALLBACK: &str = "older or unidentified NVIDIA driver (GH #777) → set WEBKIT_DISABLE_DMABUF_RENDERER=1 (software rendering fallback)";

/// The oldest NVIDIA driver branch confirmed to render with DMABUF (610.57.04,
/// GH #777). Older branches, the legacy ones among them, showed a blank window.
const NVIDIA_DMABUF_MIN_MAJOR: u32 = 610;

/// Driver branch from `/proc/driver/nvidia/version`, e.g. "NVRM version: NVIDIA
/// UNIX x86_64 Kernel Module  610.57.04  Release Build ..." → 610.
fn nvidia_major(version_file: &str) -> Option<u32> {
    let line = version_file
        .lines()
        .find(|l| l.starts_with("NVRM version:"))?;
    line.split_whitespace().find_map(|token| {
        let (major, rest) = token.split_once('.')?;
        let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        (numeric(major) && rest.split('.').all(numeric))
            .then(|| major.parse().ok())
            .flatten()
    })
}

fn fallback_reason(
    tine_gpu: Option<&OsStr>,
    dmabuf_override: bool,
    nvidia_loaded: bool,
    nvidia_major: Option<u32>,
) -> Option<&'static str> {
    if dmabuf_override {
        return None;
    }
    if tine_gpu == Some(OsStr::new("0")) {
        return Some(USER_FALLBACK);
    }
    // Any explicit TINE_GPU value opts out of automatic driver policy. An
    // unreadable version counts as old: a false positive costs smoothness, a
    // false negative a blank window.
    if tine_gpu.is_none()
        && nvidia_loaded
        && nvidia_major.map_or(true, |major| major < NVIDIA_DMABUF_MIN_MAJOR)
    {
        return Some(NVIDIA_FALLBACK);
    }
    None
}

pub(crate) fn configure() {
    // These signals identify the NVIDIA stack, unlike PCI vendor IDs which
    // would also match nouveau. Do not load libraries or start helper processes.
    // See https://github.com/tauri-apps/tauri/issues/9394 and WebKit bug 254901.
    let version = std::fs::read_to_string("/proc/driver/nvidia/version").ok();
    let reason = fallback_reason(
        std::env::var_os("TINE_GPU").as_deref(),
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some(),
        version.is_some() || std::path::Path::new("/sys/module/nvidia_drm").exists(),
        version.as_deref().and_then(nvidia_major),
    );
    if let Some(reason) = reason {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        crate::diag(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_or_unknown_nvidia_gets_a_working_default() {
        for major in [None, Some(340), Some(390), Some(470), Some(580), Some(609)] {
            assert_eq!(
                fallback_reason(None, false, true, major),
                Some(NVIDIA_FALLBACK)
            );
        }
    }

    #[test]
    fn confirmed_nvidia_and_other_drivers_keep_gpu_rendering() {
        for major in [Some(610), Some(615)] {
            assert_eq!(fallback_reason(None, false, true, major), None);
        }
        assert_eq!(fallback_reason(None, false, false, None), None);
    }

    #[test]
    fn reads_the_driver_branch() {
        let proprietary = "NVRM version: NVIDIA UNIX x86_64 Kernel Module  470.256.02  Thu May  2 14:37:44 UTC 2024\nGCC version:  gcc version 12.2.0 (Debian 12.2.0-14)\n";
        assert_eq!(nvidia_major(proprietary), Some(470));
        let open = "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  610.57.04  Release Build  (dvs-builder@U16-I1-N08-12-3)  Mon Oct  5 2026\n";
        assert_eq!(nvidia_major(open), Some(610));
        assert_eq!(nvidia_major("garbage"), None);
        assert_eq!(
            nvidia_major("NVRM version: NVIDIA UNIX x86_64 Kernel Module\n"),
            None
        );
    }

    #[test]
    fn explicit_overrides_win() {
        for loaded in [false, true] {
            for major in [None, Some(470), Some(610)] {
                for gpu in [
                    None,
                    Some(OsStr::new("0")),
                    Some(OsStr::new("1")),
                    Some(OsStr::new("")),
                ] {
                    assert_eq!(fallback_reason(gpu, true, loaded, major), None);
                }
                for gpu in ["1", "", "custom"] {
                    assert_eq!(
                        fallback_reason(Some(OsStr::new(gpu)), false, loaded, major),
                        None
                    );
                }
                assert_eq!(
                    fallback_reason(Some(OsStr::new("0")), false, loaded, major),
                    Some(USER_FALLBACK)
                );
            }
        }
    }
}
