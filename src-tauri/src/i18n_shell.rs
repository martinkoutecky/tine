//! Native shell text in the UI language (tine#107, docs/contracts/localization.md).
//! The frontend is the only message formatter: it resolves the locale and stores
//! it under `ui_locale_resolved`; native code only selects literal strings from
//! the generated table (`i18n_shell.gen.rs`, from `src/i18n/locales/*/shell.ftl`),
//! with English where a locale lacks one.

include!("i18n_shell.gen.rs");

/// The device-settings key holding the locale the frontend last resolved.
pub(crate) const UI_LOCALE_KEY: &str = "ui_locale_resolved";

/// Shell message `id` in `locale`, else English, else the id itself.
pub(crate) fn shell_text(locale: &str, id: &'static str) -> &'static str {
    shell_literal(locale, id)
        .or_else(|| shell_literal("en", id))
        .unwrap_or(id)
}

/// The resolved UI locale stored in `settings` (a `tine-settings.json` value).
pub(crate) fn resolved_locale(settings: Option<&serde_json::Value>) -> String {
    settings
        .and_then(|value| value.get(UI_LOCALE_KEY))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("en")
        .to_string()
}

/// The boot card's text at launch, as a script for every webview (`index.html`
/// shows it until the frontend mounts). Read where the app-data dir is known
/// before the app is built (desktop); elsewhere the English text stays.
pub(crate) fn boot_text_script() -> String {
    let settings = crate::app_identity::current_app_data_dir()
        .and_then(|dir| crate::settings::read_settings_value_at(&dir.join("tine-settings.json")));
    boot_text_assignment("__TINE_BOOT_TEXT__", settings.as_ref())
}

/// The boot card's text for one window built later (a graph window), read when
/// it is built, so a language chosen since launch shows on its boot card
/// (review CODE-2 C). `index.html` prefers it over the launch text.
pub(crate) fn window_boot_text_script(app: &tauri::AppHandle) -> String {
    boot_text_assignment(
        "__TINE_WINDOW_BOOT_TEXT__",
        crate::settings::read_settings_value(app).as_ref(),
    )
}

fn boot_text_assignment(global: &str, settings: Option<&serde_json::Value>) -> String {
    let text = shell_text(&resolved_locale(settings), "shell-boot-opening");
    format!(
        "globalThis.{global} = {};",
        serde_json::Value::String(text.to_string())
    )
}

/// Serialises `set_ui_locale` from its comparison to its tray relabel, so the
/// stored locale and the tray's labels change in one order (review CODE-1 F4):
/// the relabel is queued on the main thread, which runs its queue in order.
static PUBLISH: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Store the frontend's resolved locale and relabel native shell text. Ordered
/// per webview (`ORDERED_COMMANDS`); across webviews the frontends converge on
/// one choice through the change broadcast, so the last call carries it.
#[tauri::command]
pub(crate) async fn set_ui_locale(locale: String, app: tauri::AppHandle) -> Result<(), String> {
    let store = app.clone();
    crate::state::off_ui(move || {
        let _publishing = PUBLISH
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = std::cell::Cell::new(false);
        crate::settings::update_settings(&store, |json| {
            changed.set(resolved_locale(Some(json)) != locale);
            json[UI_LOCALE_KEY] = serde_json::Value::String(locale.clone());
        })?;
        #[cfg(desktop)]
        if changed.get() {
            crate::tray::relabel(&store, &locale);
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graph_window_boot_card_reads_the_language_when_it_is_built() {
        // Review CODE-2 C: a graph window opened after a language change shows
        // the current language, not the launch's; index.html prefers it.
        let zh = serde_json::json!({ UI_LOCALE_KEY: "zh-CN" });
        let script = boot_text_assignment("__TINE_WINDOW_BOOT_TEXT__", Some(&zh));
        assert!(script.starts_with("globalThis.__TINE_WINDOW_BOOT_TEXT__ = "));
        assert!(script.contains(shell_text("zh-CN", "shell-boot-opening")));
        assert_ne!(
            shell_text("zh-CN", "shell-boot-opening"),
            shell_text("en", "shell-boot-opening")
        );
        assert!(include_str!("graph.rs").contains("window_boot_text_script(app)"));
        assert!(include_str!("../../index.html").contains("globalThis.__TINE_WINDOW_BOOT_TEXT__"));
    }

    #[test]
    fn a_locale_without_a_shell_message_falls_back_to_english() {
        assert_eq!(shell_text("en", "shell-tray-quit"), "Quit");
        assert_eq!(shell_text("xx-unknown", "shell-tray-quit"), "Quit");
        assert_ne!(shell_text("zh-CN", "shell-tray-quit"), "Quit");
        assert_eq!(shell_text("en", "shell-no-such-id"), "shell-no-such-id");
    }

    #[test]
    fn the_resolved_locale_defaults_to_english() {
        assert_eq!(resolved_locale(None), "en");
        let stored = serde_json::json!({ UI_LOCALE_KEY: "zh-CN" });
        assert_eq!(resolved_locale(Some(&stored)), "zh-CN");
    }
}
