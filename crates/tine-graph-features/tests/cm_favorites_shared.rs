//! Conceptual model step 2, batch 4 (census Q12, harness gap H1): what the
//! favorites list is after a rename or merge. Shared by the Rust-entry check
//! (`cm_favorites_rename.rs`, this crate) and the headless CLI owner check
//! (src-tauri `owner/owner_tests/cm_favorites_tests.rs`, which includes this
//! file by path). The frontend check is `src/cmFavoritesRename.test.ts`.
//!
//! Scenarios and the proposed one answer (`target`) are in
//! `tests/fixtures/cm-favorites/scenarios.json`; each entry's differences from
//! `target` are pinned in `expected-differences.json` next to it, so a change
//! in what any entry answers fails until the file is refreshed
//! (`CM_REFRESH=1`). Uses only std, serde_json and tine-core.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join("crates/tine-graph-features").is_dir())
        .expect("the repository root")
        .join("crates/tine-graph-features/tests/fixtures/cm-favorites")
}

pub struct Scenario {
    pub id: String,
    pub config: String,
    pub files: Vec<(String, String)>,
    /// "rename" or "merge".
    pub kind: String,
    pub old: String,
    pub new: String,
    /// The survivor's path, for a merge.
    pub into: Option<String>,
    pub target: Value,
}

pub fn scenarios() -> Vec<Scenario> {
    let text = fs::read_to_string(fixture_dir().join("scenarios.json")).unwrap();
    let all: Value = serde_json::from_str(&text).unwrap();
    all["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Scenario {
            id: s["id"].as_str().unwrap().to_owned(),
            config: s["config"].as_str().unwrap().to_owned(),
            files: s["files"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                .collect(),
            kind: s["op"]["kind"].as_str().unwrap().to_owned(),
            old: s["op"]["old"].as_str().unwrap().to_owned(),
            new: s["op"]["new"].as_str().unwrap().to_owned(),
            into: s["op"]["into"].as_str().map(str::to_owned),
            target: s["target"].clone(),
        })
        .collect()
}

/// Write the scenario's config.edn and page files under `root`.
pub fn write_graph(root: &Path, scenario: &Scenario) {
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(root.join("logseq/config.edn"), &scenario.config).unwrap();
    for (path, text) in &scenario.files {
        fs::write(root.join(path), text).unwrap();
    }
}

/// What the graph says after the operation, compared with `target`: the
/// `:favorites` list, `:tine/favorites-page`, `:default-home` (when the
/// target names one) and each page the target lists, byte for byte.
pub fn differences(root: &Path, scenario: &Scenario, entry: &str) -> Vec<String> {
    let text = fs::read_to_string(root.join("logseq/config.edn")).unwrap_or_default();
    let config = tine_core::config::Config::parse(&text);
    let target = &scenario.target;
    let mut diffs = Vec::new();
    let want: Vec<String> = target["favorites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    if config.favorites != want {
        diffs.push(format!(
            "{entry} favorites: target {want:?}, got {:?}",
            config.favorites
        ));
    }
    let want_page = target["favoritesPage"].as_str().map(str::to_owned);
    if config.favorites_page != want_page {
        diffs.push(format!(
            "{entry} favorites-page: target {want_page:?}, got {:?}",
            config.favorites_page
        ));
    }
    if let Some(home) = target.get("home").and_then(Value::as_str) {
        if config.default_home.as_deref() != Some(home) {
            diffs.push(format!(
                "{entry} home: target {home:?}, got {:?}",
                config.default_home
            ));
        }
    }
    if let Some(pages) = target.get("pages").and_then(Value::as_object) {
        for (path, want) in pages {
            let got = fs::read_to_string(root.join(path)).ok();
            if got.as_deref() != want.as_str() {
                diffs.push(format!(
                    "{entry} page {path}: target {:?}, got {got:?}",
                    want.as_str().unwrap()
                ));
            }
        }
    }
    diffs
}

/// Compare one entry's differences (`id -> lines`; an entry that cannot run
/// a scenario says why in its lines) with `expected-differences.json`'s
/// `entry` part; `CM_REFRESH=1` rewrites that part only.
pub fn check_expected(entry: &str, found: BTreeMap<String, Vec<String>>) {
    let path = fixture_dir().join("expected-differences.json");
    let mut all: BTreeMap<String, Value> = fs::read_to_string(&path)
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap_or_default();
    let found: BTreeMap<String, Vec<String>> =
        found.into_iter().filter(|(_, v)| !v.is_empty()).collect();
    let now = serde_json::to_value(&found).unwrap();
    if std::env::var_os("CM_REFRESH").is_some() {
        all.insert(entry.to_owned(), now);
        fs::write(&path, serde_json::to_string_pretty(&all).unwrap() + "\n").unwrap();
        return;
    }
    let pinned = all
        .get(entry)
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    assert_eq!(
        now, pinned,
        "cm favorites: the {entry} entry's differences from the target changed; if intended, \
         refresh with CM_REFRESH=1 and say why in the commit"
    );
}
