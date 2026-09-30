use std::fs;
use tine_store::{Area, Store};

#[test]
fn favorites_setter_and_reload_read_the_same_root_entry() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    let original = "{:extension {:favorites [\"Shadow\"]} :favorites [\"Real\"]}\n";
    fs::write(dir.path().join("logseq/config.edn"), original).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(store.config().config.favorites, ["Real"]);
    tine_graph_features::config::set_favorites(&store, &["New".into()], None).unwrap();
    let id = store.file_id(Area::Meta, "config.edn").unwrap();
    let written = String::from_utf8(store.read(&id, None).unwrap().0).unwrap();
    assert_eq!(written, original.replace("[\"Real\"]", "[\"New\"]"));
    assert_eq!(store.config().config.favorites, ["New"]);
    store.close();
}
