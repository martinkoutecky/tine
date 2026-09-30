//! The app identity this binary ships as, fixed at compile time by the switch
//! `src-tauri/app-identity.json` (see docs/app-identity.md and build.rs).
//!
//! The OS app-data dir (settings, session, backups, plugins and, on Linux, the
//! WebKit localStorage holding the open graph and tabs), the desktop entry and
//! the Wayland app ID are all keyed by the identifier. Experiment builds ship
//! `page.tine.TineBeta` so they cannot read or rewrite the released Tine's state;
//! a release build ships the released identity and uses the released Tine's
//! app-data dir in place (no migration: the formats are shared).

/// The shipped identifier (`identifier` in tauri.conf.json, checked by build.rs).
pub(crate) const APP_IDENTIFIER: &str = env!("TINE_APP_IDENTIFIER");
/// The shipped product name (desktop-entry `Name=`).
pub(crate) const PRODUCT_NAME: &str = env!("TINE_PRODUCT_NAME");
/// The released Tine's identifier. Equal to [`APP_IDENTIFIER`] in a release build.
pub(crate) const RELEASE_IDENTIFIER: &str = env!("TINE_RELEASE_IDENTIFIER");

/// The app-data dir of `identifier`, resolved without a Tauri handle (settings
/// are read before the Builder exists). `dirs::data_dir()` is the base Tauri
/// v2's `app_data_dir()` joins the identifier onto.
pub(crate) fn app_data_dir_of(identifier: &str) -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|base| base.join(identifier))
}

pub(crate) fn current_app_data_dir() -> Option<std::path::PathBuf> {
    app_data_dir_of(APP_IDENTIFIER)
}

#[cfg(test)]
mod tests {
    #[test]
    fn tauri_config_accepts_the_preview_semver_without_changing_identity() {
        let mut conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        conf["version"] = serde_json::json!("0.7.0-beta.1");
        conf["bundle"]["android"]["versionCode"] = serde_json::json!(7001);
        let parsed: tauri::Config = serde_json::from_value(conf).unwrap();
        assert_eq!(parsed.version.as_deref(), Some("0.7.0-beta.1"));
        assert_eq!(parsed.identifier, super::APP_IDENTIFIER);
    }

    #[test]
    fn identity_matches_the_switch_and_tauri_conf() {
        let switch: serde_json::Value =
            serde_json::from_str(include_str!("../app-identity.json")).unwrap();
        let ship = switch["ship"].as_str().unwrap();
        let shipped = &switch["identities"][ship];
        assert_eq!(shipped["identifier"].as_str(), Some(super::APP_IDENTIFIER));
        assert_eq!(shipped["productName"].as_str(), Some(super::PRODUCT_NAME));
        assert_eq!(
            switch["identities"]["release"]["identifier"].as_str(),
            Some(super::RELEASE_IDENTIFIER)
        );
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"].as_str(), Some(super::APP_IDENTIFIER));
        // An experiment build must never share the released app-data dir.
        assert_eq!(
            ship == "release",
            super::APP_IDENTIFIER == super::RELEASE_IDENTIFIER
        );
    }
}
