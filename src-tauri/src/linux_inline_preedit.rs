//! Restore WebKitGTK's inline IME preedit for every webview, including windows
//! created after startup. Wry disables it for an older Fcitx popup-positioning
//! workaround (wry #955); without it the page receives no in-progress composition.

#[cfg(target_os = "linux")]
pub(crate) fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.plugin(
        tauri::plugin::Builder::<R, ()>::new("linux-inline-preedit")
            .on_webview_ready(|webview| {
                // Wry has already applied its settings. Use the existing GTK
                // context on the UI thread rather than replacing the IM module.
                if let Err(error) = webview.with_webview(|view| {
                    use webkit2gtk::{InputMethodContextExt, WebViewExt};
                    if let Some(context) = view.inner().input_method_context() {
                        context.set_enable_preedit(true);
                    }
                }) {
                    crate::debug::diag_private(
                        "linux-inline-preedit-unavailable",
                        error.to_string(),
                    );
                }
            })
            .build(),
    )
}

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    target_os = "android",
    target_os = "ios"
))]
pub(crate) fn configure<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_five_shipped_targets_have_an_explicit_preedit_policy() {
        let source = include_str!("linux_inline_preedit.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        for target in ["linux", "windows", "macos", "android", "ios"] {
            assert!(source.contains(&format!("target_os = \"{target}\"")));
        }
        assert_eq!(source.matches("target_os =").count(), 5);
    }
}
