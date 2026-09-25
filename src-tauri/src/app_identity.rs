// The app identity of the `og` experiment build. It must differ from shipped
// Tine's `page.tine.Tine`: the OS app-data dir (settings, session, backups,
// plugins and, on Linux, the WebKit localStorage holding the open graph and
// tabs) is keyed by it, so sharing it would let this build read and rewrite
// the real Tine's state. This build has no predecessor identity and migrates
// nothing.

/// Must equal `identifier` in `tauri.conf.json` (guarded by the test below).
pub(crate) const APP_IDENTIFIER: &str = "page.tine.TineOG";

/// The app-data dir, resolved without a Tauri handle (settings are read
/// before the Builder exists). `dirs::data_dir()` is the base Tauri v2's
/// `app_data_dir()` joins the identifier onto.
pub(crate) fn current_app_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|base| base.join(APP_IDENTIFIER))
}

#[cfg(test)]
mod tests {
    #[test]
    fn identifier_matches_tauri_conf_and_is_not_shipped_tine() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"].as_str(), Some(super::APP_IDENTIFIER));
        assert_ne!(super::APP_IDENTIFIER, "page.tine.Tine");
    }
}
