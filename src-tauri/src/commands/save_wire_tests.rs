//! Master 678830a086af (GH #538, #590): a failed save names the platform step
//! that failed, so an Android `EINVAL` from the no-replace rename can be told
//! from one raised while creating, writing or syncing the temporary file.
use super::save_pages_wire;
use std::fs;
use tine_store::{EditKind, FaultPoint, PageId, Store};

fn graph(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("tine-save-step-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    root
}

fn new_page(
    name: &str,
) -> (
    PageId,
    tine_core::model::PageDto,
    Option<String>,
    bool,
    Vec<EditKind>,
) {
    let dto = serde_json::from_value(serde_json::json!({
        "name": name, "kind": "page", "title": name,
        "blocks": [{ "id": "", "raw": "text", "collapsed": false, "children": [] }],
    }))
    .unwrap();
    (
        PageId::from(format!("pages/{name}.md")),
        dto,
        None,
        false,
        vec![EditKind::CreatePage],
    )
}

#[cfg(unix)]
#[test]
fn a_failed_save_names_the_platform_step_and_os_error() {
    use std::os::unix::fs::PermissionsExt;
    let root = graph("create");
    let store = Store::open(&root, Default::default()).unwrap().0;
    fs::set_permissions(root.join("pages"), fs::Permissions::from_mode(0o555)).unwrap();
    let wire = save_pages_wire(&store, &[new_page("Blocked")]);
    fs::set_permissions(root.join("pages"), fs::Permissions::from_mode(0o755)).unwrap();
    let encoded = serde_json::to_string(&wire).unwrap();
    assert!(
        encoded.contains(r#""family":"io:PermissionDenied""#)
            && encoded.contains(r#""operation":"create temporary file""#)
            && encoded.contains(&format!(r#""osError":{}"#, libc::EACCES)),
        "GH #538: a save failure names its fixed platform step and OS error; exemplar platform_step::at: {encoded}"
    );
    assert!(
        !encoded.contains(root.to_str().unwrap()),
        "I-5: no path on the save wire: {encoded}"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn a_directory_sync_failure_is_named_as_that_step() {
    let root = graph("dirsync");
    let store = Store::open(&root, Default::default()).unwrap().0;
    store.inject_fault(FaultPoint::DirectorySyncIo);
    let encoded = serde_json::to_string(&save_pages_wire(&store, &[new_page("Synced")])).unwrap();
    assert!(
        encoded.contains(r#""operation":"fsync directory""#),
        "GH #538: a directory sync failure is named as that step; exemplar directory_durability::failure_step: {encoded}"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_failure_without_a_platform_step_sends_no_step_fields() {
    let root = graph("clean");
    let store = Store::open(&root, Default::default()).unwrap().0;
    fs::write(root.join("pages/Taken.md"), "- other\n").unwrap();
    let encoded = serde_json::to_string(&save_pages_wire(&store, &[new_page("Taken")])).unwrap();
    assert!(
        !encoded.contains("operation") && !encoded.contains("osError"),
        "a refusal that is not a platform failure carries no step: {encoded}"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}
