//! Extend Tauri's existing macOS Edit menu; both entry methods use the
//! frontend's shared plain-text paste action, never WebKit's rich paste.

// Desktop unit builds also type-check the wiring on Linux/Windows, without
// needing a display or pretending to execute AppKit.
#[cfg(any(target_os = "macos", all(test, desktop)))]
mod macos {
    use tauri::{menu::MenuItem, Emitter, EventTarget, Manager};

    const ITEM: &str = "tine-paste-plain";

    fn paste_target(
        event: &tauri::menu::MenuEvent,
        focused: impl FnOnce() -> Option<String>,
    ) -> Option<EventTarget> {
        if event.id().as_ref() != ITEM {
            return None;
        }
        focused().map(EventTarget::webview_window)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn install(app: &tauri::App) -> tauri::Result<()> {
        let Some(menu) = app.menu() else {
            return Ok(());
        };
        // Tauri 2.12's default menu names this submenu "Edit" (it has no id).
        // Keep the rest of that native menu, including standard responder actions.
        for item in menu.items()? {
            if let Some(edit) = item.as_submenu() {
                if edit.text()? != "Edit" {
                    continue;
                }
                let locale = crate::i18n_shell::resolved_locale(
                    crate::settings::read_settings_value(app.handle()).as_ref(),
                );
                let paste = MenuItem::with_id(
                    app,
                    ITEM,
                    crate::i18n_shell::shell_text(&locale, "shell-edit-paste-match-style"),
                    true,
                    Some("Cmd+Shift+V"),
                )?;
                edit.insert(&paste, 6)?; // immediately after ordinary Paste
                break;
            }
        }
        app.on_menu_event(|app, event| {
            let target = paste_target(&event, || {
                app.webview_windows()
                    .into_values()
                    .find(|window| window.is_focused().unwrap_or(false))
                    .map(|window| window.label().to_string())
            });
            if let Some(target) = target {
                if let Err(error) = app.emit_to(target, "editor-paste-plain", ()) {
                    crate::debug::diag_private("paste-menu", error.to_string());
                }
            }
        });
        Ok(())
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(crate) fn relabel(app: &tauri::AppHandle, locale: &str) {
        let app_handle = app.clone();
        let label = crate::i18n_shell::shell_text(locale, "shell-edit-paste-match-style");
        let _ = app.run_on_main_thread(move || {
            if let Some(menu) = app_handle.menu() {
                for item in menu.items().unwrap_or_default() {
                    if let Some(edit) = item.as_submenu() {
                        if let Some(item) = edit.get(ITEM) {
                            if let Some(item) = item.as_menuitem() {
                                let _ = item.set_text(label);
                            }
                        }
                    }
                }
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use tauri::menu::MenuEvent;

        #[test]
        fn menu_paste_targets_one_focused_graph_and_never_broadcasts() {
            let event = MenuEvent { id: ITEM.into() };
            assert_eq!(
                paste_target(&event, || Some("graph-b".into())),
                Some(EventTarget::webview_window("graph-b")),
            );
            assert_eq!(paste_target(&event, || None), None);
            let other = MenuEvent {
                id: "other-menu-item".into(),
            };
            assert_eq!(
                paste_target(&other, || panic!("unrelated menu event")),
                None
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::{install, relabel};
