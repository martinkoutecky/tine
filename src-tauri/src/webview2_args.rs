//! WebView2 browser arguments for every window Tine creates on Windows.
//!
//! Wry normally supplies its own default arguments. Once a window provides
//! `additional_browser_args`, that value replaces wry's default rather than
//! extending it (wry 0.57.0 `webview2/mod.rs` `create_environment`), so Tine
//! passes wry's default plus its own flag.
//!
//! GH #804: WebView2 (like Edge) drew untagged images washed out against
//! other viewers on a reporter's display; `--force-color-profile=srgb` fixed it
//! there. Every window gets the same string: WebView2 windows that share a user
//! data folder must share one environment's arguments. Windows opened by
//! `window.open` (workspace windows) reuse their opener's environment.
//!
//! A user's own `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` still applies: WebView2
//! appends the variable to the arguments given here (Microsoft's
//! `CreateCoreWebView2EnvironmentWithOptions` reference), so for example
//! `--force-color-profile=<other>` there comes later on the command line.

/// Wry 0.57.0's default (no autoplay, no proxy: Tine sets neither).
pub(crate) const WRY_WINDOWS_DEFAULT_BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// GH #804: draw in sRGB, as the image viewers the reporter compared with.
pub(crate) const FORCE_SRGB_ARG: &str = "--force-color-profile=srgb";

/// The arguments for one window: the configured value (or wry's default),
/// Tine's colour-profile flag unless a profile is configured already, and,
/// only under explicit WebDriver automation, the inherited automation
/// arguments (E2E harness).
pub(crate) fn merged_windows_browser_args(
    configured: Option<&str>,
    automation_enabled: bool,
    inherited: Option<&str>,
) -> String {
    let mut arguments = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(WRY_WINDOWS_DEFAULT_BROWSER_ARGS)
        .to_string();
    if !arguments.contains("--force-color-profile") {
        arguments.push(' ');
        arguments.push_str(FORCE_SRGB_ARG);
    }
    if automation_enabled {
        if let Some(inherited) = inherited.map(str::trim).filter(|value| !value.is_empty()) {
            arguments.push(' ');
            arguments.push_str(inherited);
        }
    }
    arguments
}

/// Sets the arguments on every configured window; returns how many changed.
pub(crate) fn apply_windows_browser_args_policy(
    windows: &mut [tauri::utils::config::WindowConfig],
    automation_enabled: bool,
    inherited: Option<&str>,
) -> usize {
    let mut changed = 0;
    for window in windows {
        let arguments = merged_windows_browser_args(
            window.additional_browser_args.as_deref(),
            automation_enabled,
            inherited,
        );
        if window.additional_browser_args.as_deref() != Some(arguments.as_str()) {
            window.additional_browser_args = Some(arguments);
            changed += 1;
        }
    }
    changed
}

fn automation_enabled_from_env() -> bool {
    std::env::var("TAURI_WEBVIEW_AUTOMATION").as_deref() == Ok("true")
}

fn inherited_from_env() -> Option<String> {
    std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").ok()
}

/// The configured windows (main, capture) before Tauri builds them.
pub(crate) fn apply_to_config_windows(windows: &mut [tauri::utils::config::WindowConfig]) {
    apply_windows_browser_args_policy(
        windows,
        automation_enabled_from_env(),
        inherited_from_env().as_deref(),
    );
}

/// The arguments for a window built at runtime (extra graph windows).
pub(crate) fn windows_browser_args_from_env() -> String {
    merged_windows_browser_args(
        None,
        automation_enabled_from_env(),
        inherited_from_env().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_arguments_keep_wry_defaults_and_force_srgb() {
        let expected = format!("{WRY_WINDOWS_DEFAULT_BROWSER_ARGS} {FORCE_SRGB_ARG}");
        assert_eq!(merged_windows_browser_args(None, false, None), expected);
        assert_eq!(
            expected,
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --force-color-profile=srgb"
        );
        // Outside automation an inherited value is left to WebView2 itself,
        // which appends the environment variable on its own.
        assert_eq!(
            merged_windows_browser_args(None, false, Some("--remote-debugging-port=9222")),
            expected
        );
        assert_eq!(
            merged_windows_browser_args(None, true, Some("  ")),
            expected
        );
    }

    /// `WRY_WINDOWS_DEFAULT_BROWSER_ARGS` is copied from wry 0.57.0. A wry bump
    /// must re-read `webview2/mod.rs` `create_environment` and update it here.
    #[test]
    fn wry_default_copy_matches_the_pinned_wry_version() {
        let lock = include_str!("../../Cargo.lock");
        assert!(
            lock.contains("name = \"wry\"\nversion = \"0.57.0\"\n"),
            "wry moved: re-check its default WebView2 arguments"
        );
    }

    #[test]
    fn automation_arguments_preserve_wry_defaults_and_configured_values() {
        assert_eq!(
            merged_windows_browser_args(None, true, Some("--remote-debugging-port=9222")),
            format!(
                "{WRY_WINDOWS_DEFAULT_BROWSER_ARGS} {FORCE_SRGB_ARG} --remote-debugging-port=9222"
            )
        );
        assert_eq!(
            merged_windows_browser_args(
                Some("--disable-gpu"),
                true,
                Some("--remote-debugging-port=9222")
            ),
            format!("--disable-gpu {FORCE_SRGB_ARG} --remote-debugging-port=9222")
        );
        assert_eq!(
            merged_windows_browser_args(Some("--force-color-profile=display-p3-d65"), false, None),
            "--force-color-profile=display-p3-d65"
        );
    }

    #[test]
    fn policy_sets_every_configured_window_and_is_idempotent() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let windows = &mut context.config_mut().app.windows;
        assert!(windows.len() >= 2);
        let window_count = windows.len();
        assert_eq!(
            apply_windows_browser_args_policy(windows, false, None),
            window_count
        );
        assert!(windows
            .iter()
            .all(|window| window.additional_browser_args.as_deref()
                == Some(merged_windows_browser_args(None, false, None).as_str())));
        assert_eq!(apply_windows_browser_args_policy(windows, false, None), 0);
    }

    #[test]
    fn automation_policy_updates_every_configured_window() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let windows = &mut context.config_mut().app.windows;
        let window_count = windows.len();
        assert_eq!(
            apply_windows_browser_args_policy(windows, true, Some("--remote-debugging-port=9222")),
            window_count
        );
        assert!(windows.iter().all(|window| window
            .additional_browser_args
            .as_deref()
            .is_some_and(|args| args.contains("--remote-debugging-port=9222")
                && args.contains(FORCE_SRGB_ARG)
                && args.contains(WRY_WINDOWS_DEFAULT_BROWSER_ARGS))));
    }

    /// Main window and runtime graph windows must get the same string:
    /// WebView2 refuses a second environment with different arguments on one
    /// user data folder.
    #[test]
    fn runtime_windows_match_configured_windows() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let windows = &mut context.config_mut().app.windows;
        apply_windows_browser_args_policy(windows, false, None);
        let main = windows
            .iter()
            .find(|window| window.label == "main")
            .unwrap();
        assert_eq!(
            main.additional_browser_args.as_deref(),
            Some(merged_windows_browser_args(None, false, None).as_str())
        );
    }
}
