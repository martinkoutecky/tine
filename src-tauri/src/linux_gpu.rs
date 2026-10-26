//! Startup rendering policy, before GTK/WebKit creates any webviews (GH #777).

use std::ffi::OsStr;

const USER_FALLBACK: &str =
    "TINE_GPU=0 → set WEBKIT_DISABLE_DMABUF_RENDERER=1 (software rendering fallback)";
const NVIDIA_FALLBACK: &str = "NVIDIA driver detected (GH #777) → set WEBKIT_DISABLE_DMABUF_RENDERER=1 (software rendering fallback)";

fn fallback_reason(
    tine_gpu: Option<&OsStr>,
    dmabuf_override: bool,
    nvidia_drm: bool,
    nvidia_version: bool,
) -> Option<&'static str> {
    if dmabuf_override {
        return None;
    }
    if tine_gpu == Some(OsStr::new("0")) {
        return Some(USER_FALLBACK);
    }
    // Any explicit TINE_GPU value opts out of automatic driver policy.
    if tine_gpu.is_none() && (nvidia_drm || nvidia_version) {
        return Some(NVIDIA_FALLBACK);
    }
    None
}

pub(crate) fn configure() {
    // These signals identify the NVIDIA stack, unlike PCI vendor IDs which
    // would also match nouveau. Do not load libraries or start helper processes.
    // See https://github.com/tauri-apps/tauri/issues/9394 and WebKit bug 254901.
    let reason = fallback_reason(
        std::env::var_os("TINE_GPU").as_deref(),
        std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some(),
        std::path::Path::new("/sys/module/nvidia_drm").exists(),
        std::path::Path::new("/proc/driver/nvidia/version").exists(),
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
    fn nvidia_gets_a_working_default() {
        for (drm, version) in [(true, false), (false, true), (true, true)] {
            assert!(fallback_reason(None, false, drm, version).is_some());
        }
    }

    #[test]
    fn other_drivers_keep_gpu_rendering() {
        assert_eq!(fallback_reason(None, false, false, false), None);
    }

    #[test]
    fn explicit_overrides_win() {
        for drm in [false, true] {
            for version in [false, true] {
                for gpu in [
                    None,
                    Some(OsStr::new("0")),
                    Some(OsStr::new("1")),
                    Some(OsStr::new("")),
                ] {
                    assert_eq!(fallback_reason(gpu, true, drm, version), None);
                }
                for gpu in ["1", "", "custom"] {
                    assert_eq!(
                        fallback_reason(Some(OsStr::new(gpu)), false, drm, version),
                        None
                    );
                }
                assert_eq!(
                    fallback_reason(Some(OsStr::new("0")), false, drm, version),
                    Some(USER_FALLBACK)
                );
            }
        }
    }
}
