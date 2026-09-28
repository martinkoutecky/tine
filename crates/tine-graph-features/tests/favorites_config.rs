//! Family 22: `:tine/favorites-page` is written with membership in ONE guarded
//! config write, byte-identical to master's `set_favorites_page` then
//! `set_favorites` (20d986e67), and read back through GraphMeta.
use std::fs;
use tine_graph_features::config;
use tine_store::Store;

fn graph(label: &str, config_edn: Option<&str>) -> (std::path::PathBuf, Store) {
    let root = std::env::temp_dir().join(format!("tine-fav-config-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    if let Some(text) = config_edn {
        fs::write(root.join("logseq/config.edn"), text).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

fn written(root: &std::path::Path) -> String {
    fs::read_to_string(root.join("logseq/config.edn")).unwrap()
}

#[test]
fn favorites_page_key_is_written_with_membership_like_master() {
    // Master's two writes on `{}`: the page key inserted after `{`, then the
    // membership vector inserted after `{` in front of it.
    let (root, store) = graph("empty", Some("{}\n"));
    config::set_favorites(&store, &["A".into()], Some("Favorites")).unwrap();
    assert_eq!(
        written(&root),
        "{\n :favorites [\"A\"]\n\n :tine/favorites-page \"Favorites\"\n}\n"
    );
    let config = store.config().config;
    let format = tine_core::date::JournalFormat::new(None, None);
    let meta = tine_core::model::GraphMeta::from_config(String::new(), &config, &format);
    assert_eq!(meta.favorites, vec!["A".to_string()]);
    assert_eq!(meta.favorites_page.as_deref(), Some("Favorites"));

    // Replacing keeps one key, the rest of the file, and escapes the name.
    config::set_favorites(&store, &["A".into(), "B".into()], Some("od\"d")).unwrap();
    let text = written(&root);
    assert_eq!(
        text,
        "{\n :favorites [\"A\" \"B\"]\n\n :tine/favorites-page \"od\\\"d\"\n}\n"
    );
    assert_eq!(text.matches(":tine/favorites-page").count(), 1);
    assert_eq!(
        tine_core::config::Config::parse(&text)
            .favorites_page
            .as_deref(),
        Some("od\"d")
    );

    // Membership-only writes leave the page key alone.
    config::set_favorites(&store, &[], None).unwrap();
    assert_eq!(
        written(&root),
        "{\n :favorites []\n\n :tine/favorites-page \"od\\\"d\"\n}\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorites_page_key_preserves_unknown_keys_and_comments() {
    let input = "{ ; keep me\n :favorites [\"Old\"]\n :unrelated 42}\n";
    let (root, store) = graph("typical", Some(input));
    config::set_favorites(&store, &["New".into()], Some("Favorites")).unwrap();
    assert_eq!(
        written(&root),
        "{\n :tine/favorites-page \"Favorites\"\n ; keep me\n :favorites [\"New\"]\n :unrelated 42}\n"
    );
    // A blank value reads as "no arrangement page".
    assert_eq!(
        tine_core::config::Config::parse("{:tine/favorites-page \"  \"}").favorites_page,
        None
    );
    fs::remove_dir_all(root).unwrap();
}
