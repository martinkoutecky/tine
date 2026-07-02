use tauri::Manager;

/// Split a user-entered language string (e.g. "en_US, cs_CZ") into locale codes.
pub(crate) fn parse_spellcheck_langs(s: &str) -> Vec<String> {
    s.split([',', ';', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Enable/disable WebKitGTK spell checking on one webview and set its languages.
/// `langs` empty ⇒ leave WebKitGTK's default (the user's OS locale, like Logseq).
/// WebKitGTK checks a word against ALL given dictionaries, so listing several
/// (e.g. `en_US` + `cs_CZ`) accepts words from any of them — bilingual editing.
/// (Each language needs its hunspell dictionary installed; missing ones are
/// silently ignored.) The per-block `<textarea spellcheck>` attribute is the
/// other gate, so even an enabled context shows squiggles only while editing.
#[cfg(target_os = "linux")]
fn apply_spellcheck_to(window: &tauri::WebviewWindow, enabled: bool, langs: &[String]) {
    let langs: Vec<String> = langs.to_vec();
    let _ = window.with_webview(move |wv| {
        use webkit2gtk::{WebContextExt, WebViewExt};
        let webview = wv.inner();
        if let Some(ctx) = webview.web_context() {
            ctx.set_spell_checking_enabled(enabled);
            if enabled && !langs.is_empty() {
                let refs: Vec<&str> = langs.iter().map(String::as_str).collect();
                ctx.set_spell_checking_languages(&refs);
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn apply_spellcheck_to(_window: &tauri::WebviewWindow, _enabled: bool, _langs: &[String]) {
    // Windows (WebView2) and macOS (WKWebView) honour the textarea `spellcheck`
    // attribute with their own native checker; no context call is needed there.
}

/// Apply the spellcheck prefs to every window (main + capture). Called at startup
/// and live on every Settings change, so toggling/relanguaging takes effect
/// without a restart (Logseq needs a relaunch).
pub(crate) fn apply_spellcheck_all(app: &tauri::AppHandle, enabled: bool, langs: &[String]) {
    for (_label, window) in app.webview_windows() {
        apply_spellcheck_to(&window, enabled, langs);
    }
}

/// Live re-apply from the frontend (the Settings toggle / languages field). The
/// frontend persists the values itself via set_app_bool/_string; this just pushes
/// the current values onto the live webviews.
#[tauri::command]
pub(crate) fn apply_spellcheck(enabled: bool, languages: Vec<String>, app: tauri::AppHandle) {
    apply_spellcheck_all(&app, enabled, &languages);
}

/// Looks like a locale/dictionary code: "en", "en_US", "cs_CZ", "ca_ES_valencia".
fn is_locale_code(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 24
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Discover the spell-check dictionaries installed on this machine, so the UI can
/// offer them instead of making the user remember locale codes. Authoritative
/// source is enchant's own listing (it knows every backend + search path WebKitGTK
/// will actually use); if that CLI isn't present we scan the standard hunspell /
/// myspell directories for `*.dic`. Returns sorted, de-duplicated codes.
#[cfg(target_os = "linux")]
fn discover_dictionaries() -> Vec<String> {
    use std::collections::BTreeSet;
    let mut found: BTreeSet<String> = BTreeSet::new();

    // 1) `enchant-lsmod-2 -list-dicts` → lines like "en_US (hunspell)".
    for tool in ["enchant-lsmod-2", "enchant-lsmod"] {
        if let Ok(out) = std::process::Command::new(tool).arg("-list-dicts").output() {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    if let Some(code) = line.split_whitespace().next() {
                        if is_locale_code(code) {
                            found.insert(code.to_string());
                        }
                    }
                }
                if !found.is_empty() {
                    return found.into_iter().collect();
                }
            }
        }
    }

    // 2) Fallback: scan the standard hunspell / myspell dictionary dirs for *.dic.
    let mut dirs = vec![
        "/usr/share/hunspell".to_string(),
        "/usr/share/myspell/dicts".to_string(),
        "/usr/share/myspell".to_string(),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(format!("{}/.local/share/hunspell", home.to_string_lossy()));
    }
    if let Some(dicpath) = std::env::var_os("DICPATH") {
        for p in std::env::split_paths(&dicpath) {
            dirs.push(p.to_string_lossy().into_owned());
        }
    }
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("dic") {
                    continue;
                }
                // A real hunspell SPELL dictionary always ships a matching `.aff`
                // (affix-rules) file. Hyphenation dictionaries (hyph_*.dic) — which
                // also use the .dic extension and sit in the same dir — do NOT, so
                // requiring a sibling .aff cleanly excludes them (while keeping
                // genuine dicts like Thai th_TH that a prefix filter would mis-drop).
                if !p.with_extension("aff").exists() {
                    continue;
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    if is_locale_code(stem) {
                        found.insert(stem.to_string());
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

/// Installed spell-check dictionary codes (e.g. ["cs_CZ", "en_GB", "en_US"]). Empty
/// on non-Linux (those webviews use the OS checker, which the frontend handles by
/// falling back to a free-text language field).
#[tauri::command]
pub(crate) fn list_spellcheck_dictionaries() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        discover_dictionaries()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}
