//! Throwaway OG-SPIKEMW: native opener linkage, with env-gated evidence commands.
use tauri::{Manager, WebviewWindowBuilder};

pub fn create_main(app: &tauri::App) -> tauri::Result<()> {
    if std::env::var_os("TINE_SPIKE_MW").is_some() {
        if let Some(graph) = std::env::var_os("TINE_SPIKE_GRAPH") {
            std::env::set_var("TINE_GRAPH", graph);
        }
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .unwrap();
    eprintln!("SPIKE create_main from_config");
    let builder = WebviewWindowBuilder::from_config(app, config)?;
    #[cfg(desktop)]
    let builder = {
        let handle = app.handle().clone();
        builder.on_new_window(move |url, features| {
            eprintln!("SPIKE new-window URL={url}");
            // No popup behavior outside the explicitly enabled throwaway experiment.
            if std::env::var_os("TINE_SPIKE_MW").is_none() || url.as_str() != "about:blank" {
                return tauri::webview::NewWindowResponse::Deny;
            }
            let window =
                WebviewWindowBuilder::new(&handle, "spike-popup", tauri::WebviewUrl::External(url))
                    .window_features(features)
                    .title("Tine Spike Popup")
                    .decorations(true)
                    .inner_size(780.0, 800.0)
                    .position(1200.0, 50.0)
                    .on_document_title_changed(|window, title| {
                        let _ = window.set_title(&title);
                    })
                    .build();
            match window {
                Ok(window) => tauri::webview::NewWindowResponse::Create { window },
                Err(error) => {
                    eprintln!("spike popup: {error}");
                    tauri::webview::NewWindowResponse::Deny
                }
            }
        })
    };
    let builder = if std::env::var_os("TINE_SPIKE_MW").is_some() {
        builder
            .position(0.0, 50.0)
            .initialization_script("globalThis.__TINE_SPIKE_MW__ = true;")
    } else {
        builder
    };
    eprintln!("SPIKE create_main build");
    let main = builder.build()?;
    eprintln!("SPIKE create_main built");
    #[cfg(target_os = "linux")]
    if std::env::var_os("TINE_SPIKE_MW").is_some() {
        main.with_webview(|view| {
            use webkit2gtk::{SettingsExt, WebViewExt};
            if let Some(settings) = view.inner().settings() {
                eprintln!("SPIKE enabling javascript-can-open-windows-automatically");
                settings.set_javascript_can_open_windows_automatically(true);
            }
        })?;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = main;
    Ok(())
}

fn outdir() -> Result<std::path::PathBuf, String> {
    std::env::var_os("TINE_SPIKE_MW")
        .map(std::path::PathBuf::from)
        .ok_or("spike disabled".into())
}

#[tauri::command]
pub async fn spike_mw(
    app: tauri::AppHandle,
    action: String,
    value: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let out = outdir()?;
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    match action.as_str() {
        "log" => { eprintln!("SPIKE JS {}", value.unwrap_or_default()); Ok(serde_json::Value::Null) },
        "config" => Ok(serde_json::json!({"graph": std::fs::canonicalize(std::env::var("TINE_SPIKE_GRAPH").map_err(|e| e.to_string())?).map_err(|e| e.to_string())?.display().to_string(), "oskeys": std::env::var("TINE_SPIKE_MW_OSKEYS").as_deref() == Ok("1")})),
        "windows" => Ok(serde_json::json!(app.webview_windows().into_iter().map(|(label, w)| serde_json::json!({"label": label, "visible": w.is_visible().unwrap_or(false), "title": w.title().unwrap_or_default(), "decorated": w.is_decorated().unwrap_or(false)})).collect::<Vec<_>>())),
        "read" => {
            let root = std::env::var("TINE_SPIKE_GRAPH").map_err(|e| e.to_string())?;
            Ok(serde_json::json!(std::fs::read_to_string(std::path::Path::new(&root).join("pages/Spike.md")).map_err(|e| e.to_string())?))
        },
        "minimize" => { app.get_webview_window("main").ok_or("main missing")?.minimize().map_err(|e| e.to_string())?; Ok(serde_json::Value::Null) },
        "restore" => { app.get_webview_window("main").ok_or("main missing")?.unminimize().map_err(|e| e.to_string())?; Ok(serde_json::Value::Null) },
        "focus-popup" => { app.get_webview_window("spike-popup").ok_or("popup missing")?.set_focus().map_err(|e| e.to_string())?; Ok(serde_json::Value::Null) },
        "screenshot-result" => {
            let bytes = std::fs::read(out.join("screenshot.json")).map_err(|e| e.to_string())?;
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())
        },
        "close-popup" => { app.get_webview_window("spike-popup").ok_or("popup missing")?.close().map_err(|e| e.to_string())?; Ok(serde_json::Value::Null) },
        "screenshots" => {
            let titles: Vec<_> = app.webview_windows().into_iter().map(|(label, w)| serde_json::json!({"label": label, "title": w.title().unwrap_or_default()})).collect();
            std::fs::write(out.join("screenshots.ready"), serde_json::to_vec(&titles).unwrap()).map_err(|e| e.to_string())?;
            Ok(serde_json::Value::Null)
        },
        "ready" => { std::fs::write(out.join(format!("{action}.ready")), "ready").map_err(|e| e.to_string())?; Ok(serde_json::Value::Null) },
        "finish" => {
            let result = value.ok_or("result missing")?;
            std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&result).unwrap()).map_err(|e| e.to_string())?;
            app.get_webview_window("main").ok_or("main missing")?.close().map_err(|e| e.to_string())?;
            Ok(serde_json::Value::Null)
        },
        _ => Err("unknown spike action".into()),
    }
}

pub fn exit_code() -> Option<i32> {
    let out = outdir().ok()?;
    let pass = std::fs::read(out.join("result.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .map(|r| {
            r.as_object()
                .unwrap()
                .values()
                .all(|c| c["status"] == "pass")
        })
        .unwrap_or(false);
    Some(if pass { 0 } else { 1 })
}

pub fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) -> bool {
    let Ok(out) = outdir() else {
        return false;
    };
    if window.label() != "main" {
        return false;
    }
    if let tauri::WindowEvent::CloseRequested { .. } = event {
        let app = window.app_handle();
        let popup = app.get_webview_window("spike-popup");
        let existed = popup.is_some();
        let closed = popup.map(|w| w.destroy().is_ok()).unwrap_or(false);
        if let Ok(bytes) = std::fs::read(out.join("result.json")) {
            if let Ok(mut result) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                result["C7"]["mainClose"] =
                    serde_json::json!({"popupExisted": existed, "destroySucceeded": closed});
                if !existed || !closed {
                    result["C7"]["status"] = serde_json::json!("fail");
                }
                let _ = std::fs::write(
                    out.join("result.json"),
                    serde_json::to_vec_pretty(&result).unwrap(),
                );
            }
        }
    }
    if let tauri::WindowEvent::Destroyed = event {
        let pass = std::fs::read(out.join("result.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .map(|r| {
                r.as_object()
                    .unwrap()
                    .values()
                    .all(|c| c["status"] == "pass")
            })
            .unwrap_or(false);
        window.app_handle().exit(if pass { 0 } else { 1 });
        return true;
    }
    false
}
