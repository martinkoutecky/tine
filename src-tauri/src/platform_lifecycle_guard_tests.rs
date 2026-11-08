fn lib_source() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("read src-tauri/src/lib.rs")
}

/// GH #455: on Windows sign-out/shutdown tao runs the `RunEvent::Exit`
/// callback for WM_ENDSESSION but never leaves its message loop, so the
/// event loop must terminate the process itself on Windows.
#[test]
fn windows_session_end_exit_terminates_the_process() {
    let source = lib_source();
    let run = &source[source.find(".run(|app, event|").expect("the event loop")..];
    let run = &run[..run.find("});").expect("the end of the event loop")];
    assert!(
        run.contains("tauri::RunEvent::Exit")
            && run.contains(
                "#[cfg(target_os = \"windows\")]\n                std::process::exit(0);"
            ),
        "GH #455: the RunEvent::Exit arm must call std::process::exit(0) on Windows, \
         because WM_ENDSESSION does not break tao's message loop"
    );
}

/// Master d9763603: `App::run` never returns, so a clean-shutdown call
/// placed after it never runs and every relaunch reports an unclean exit.
/// The orderly end is the `RunEvent::Exit` arm, before Windows' exit(0).
#[test]
fn the_exit_arm_clears_the_unclean_exit_marker_before_terminating() {
    let source = lib_source();
    let run = &source[source.find(".run(|app, event|").expect("the event loop")..];
    let run = &run[..run.find("});").expect("the end of the event loop")];
    let clean = run
        .find("flight::mark_clean_shutdown();")
        .expect("RunEvent::Exit must call flight::mark_clean_shutdown()");
    assert!(clean < run.find("std::process::exit(0)").unwrap());
    assert!(source.contains("flight::persist_init(dir.join(\"diagnostics\"))"));
}

/// GH #446: the frontend's platform identity comes from the build
/// (`graph::app_platform`), injected before any frontend code runs, never
/// from the WebView user agent (iPadOS reports a Mac UA).
#[test]
fn frontend_platform_identity_is_injected_from_the_build() {
    let source = lib_source();
    assert!(
        source.contains(
            "\"globalThis.__TINE_PLATFORM__ = {:?};\",\n        crate::graph::app_platform()"
        ),
        "GH #446: lib.rs must inject __TINE_PLATFORM__ from crate::graph::app_platform()"
    );
}

/// GH #501/#492: the native touch-gesture E2E asks for a platform through
/// `TINE_E2E_TOUCH_GESTURES`; it reaches the frontend ONLY as
/// `__TINE_E2E_TOUCH_GESTURES__`, never as `__TINE_PLATFORM__`, so desktop
/// chrome keeps its real identity.
#[test]
fn e2e_touch_gesture_override_never_rewrites_platform_identity() {
    let source = lib_source();
    assert!(
        source.contains("globalThis.__TINE_E2E_TOUCH_GESTURES__ = {kind:?};")
            && source.contains("std::env::var(\"TINE_E2E_TOUCH_GESTURES\")"),
        "lib.rs must inject the touch-gesture E2E override as __TINE_E2E_TOUCH_GESTURES__"
    );
    let start = source
        .find("match e2e_touch_gestures_platform() {")
        .expect("override injection block");
    let block = &source[start..];
    let block = &block[..block.find("None => builder,").expect("block end")];
    assert!(
        !block.contains("__TINE_PLATFORM__"),
        "the E2E override must not set __TINE_PLATFORM__ (GH #446)"
    );
}

/// GH #607: the frontend's calendar is the backend's. The launch offset is
/// injected before frontend code runs, from the same zone source as
/// `JournalDate::today`; `local_clock` keeps it current.
#[test]
fn frontend_clock_correction_is_injected_from_the_backend_zone() {
    let source = lib_source();
    assert!(
        source.contains("JournalDate::local_utc_offset_now();")
            && source.contains("globalThis.__TINE_LOCAL_CLOCK__ = "),
        "GH #607: lib.rs must inject __TINE_LOCAL_CLOCK__ from JournalDate::local_utc_offset_now()"
    );
}

/// GH #572: apps get the WebKit that shipped with macOS (a Safari update
/// does not change it), and Tine needs the Safari 15.4 engine, which first
/// shipped in macOS 12.3. The bundle declares that floor so an older Mac is
/// refused at install/launch instead of running a half-working app.
#[test]
fn macos_bundle_declares_the_webkit_floor() {
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tauri.macos.conf.json"
        ))
        .expect("read src-tauri/tauri.macos.conf.json"),
    )
    .expect("tauri.macos.conf.json is JSON");
    assert_eq!(
        config["bundle"]["macOS"]["minimumSystemVersion"], "12.3",
        "GH #572: macOS bundles must require 12.3 (the Safari 15.4 engine)"
    );
}

/// GH #241: the updater ships on every desktop target (Windows, Linux,
/// macOS) and on no mobile target (Android, iOS). Windows uses native-tls
/// (Schannel) plus Reqwest's system-proxy reader; Linux/macOS keep rustls.
/// This pins the exact target sections so a cfg edit cannot silently drop
/// a platform's updater or its transport.
#[test]
fn updater_transport_is_declared_for_every_desktop_target() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read src-tauri/Cargo.toml");
    let mut section = "";
    let mut updater_sections = Vec::new();
    let mut windows_lines = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if line.starts_with("tauri-plugin-updater") {
            updater_sections.push((section, line));
        }
        if section == "[target.'cfg(windows)'.dependencies]" && !line.starts_with('#') {
            windows_lines.push(line);
        }
    }
    assert_eq!(
        updater_sections,
        vec![
            (
                "[target.'cfg(windows)'.dependencies]",
                "tauri-plugin-updater = { version = \"2\", default-features = false, features = [\"native-tls\", \"zip\"] }",
            ),
            (
                "[target.'cfg(all(not(target_os = \"android\"), not(target_os = \"ios\"), not(target_os = \"windows\")))'.dependencies]",
                "tauri-plugin-updater = \"2\"",
            ),
        ],
        "GH #241: the updater must be declared once for Windows (native-tls) and once for \
         Linux/macOS (rustls), and never for Android/iOS"
    );
    assert!(
        windows_lines.contains(
            &"reqwest = { version = \"0.13\", default-features = false, features = [\"native-tls\", \"system-proxy\"] }"
        ),
        "GH #241: the Windows updater must follow the system proxy via reqwest's system-proxy feature"
    );
}
