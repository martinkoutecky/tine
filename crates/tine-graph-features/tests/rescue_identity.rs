// Master 46a0e8c27: "Rename file to page" (rescuing a stray or duplicate-day
// journal file) must refuse when ANOTHER file already owns that page identity,
// not only when a file with the same encoded filename exists. The owner may be
// a legacy-encoded filename (`pages/A:B.md`), the other text extension, or an
// explicit `title::`. The title case passed on og already (the view resolves
// parsed identity); the two legacy-filename cases failed before the port: og's
// view excludes non-portable names, so the rescue published `A%3AB.md` beside
// `A:B.md`, which OG then loads as two files for one page.
// In-scope scenario for the refusal: honest multi-device divergence or an
// external editor leaves two files claiming one page; a rescue that published
// a second claimant would split the page's identity.
use std::fs;
use std::io;
use std::path::PathBuf;

use tine_graph_features::pages;
use tine_store::Store;

fn graph(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-rescue-identity-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    root
}

/// Rescue `stray` as `name`; assert the refusal kind and that no byte moved.
fn assert_rescue_refused(root: &PathBuf, incumbent: &str, stray: &str, name: &str, absent: &str) {
    let incumbent_bytes = fs::read(root.join(incumbent)).unwrap();
    let stray_bytes = fs::read(root.join(stray)).unwrap();
    let store = Store::open(root, Default::default()).unwrap().0;
    let _ = store.whole_graph().unwrap();
    let error = pages::rename_file_to_page(&store, stray, name).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(fs::read(root.join(incumbent)).unwrap(), incumbent_bytes);
    assert_eq!(fs::read(root.join(stray)).unwrap(), stray_bytes);
    assert!(
        !root.join(absent).exists(),
        "a second claimant was published"
    );
    store.close();
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg_attr(windows, ignore = "':' cannot appear in a Windows filename")]
fn rescue_refuses_legacy_page_identity_collision() {
    let root = graph("legacy");
    fs::write(
        root.join("pages/A:B.md"),
        "- authoritative historical page\n",
    )
    .unwrap();
    fs::write(root.join("journals/Loose.md"), "- loose journal stray\n").unwrap();
    assert_rescue_refused(
        &root,
        "pages/A:B.md",
        "journals/Loose.md",
        "A:B",
        "pages/A%3AB.md",
    );
}

#[test]
#[cfg_attr(windows, ignore = "':' cannot appear in a Windows filename")]
fn rescue_refuses_legacy_identity_in_other_text_extension() {
    let root = graph("extension");
    fs::write(
        root.join("pages/B:C.markdown"),
        "- authoritative legacy markdown page\n",
    )
    .unwrap();
    fs::write(
        root.join("journals/Loose.org"),
        "* loose org journal stray\n",
    )
    .unwrap();
    assert_rescue_refused(
        &root,
        "pages/B:C.markdown",
        "journals/Loose.org",
        "B:C",
        "pages/B%3AC.org",
    );
}

#[test]
fn rescue_refuses_effective_markdown_title_collision() {
    let root = graph("title");
    fs::write(
        root.join("pages/Other.md"),
        "title:: Named Page\n\n- authoritative page\n",
    )
    .unwrap();
    fs::write(root.join("journals/Loose.md"), "- loose journal stray\n").unwrap();
    assert_rescue_refused(
        &root,
        "pages/Other.md",
        "journals/Loose.md",
        "Named Page",
        "pages/Named Page.md",
    );
}

#[test]
#[cfg_attr(windows, ignore = "':' cannot appear in a Windows filename")]
fn a_legacy_filename_for_another_name_does_not_block_a_rescue() {
    let root = graph("unrelated");
    fs::write(root.join("pages/A:B.md"), "- legacy page\n").unwrap();
    fs::write(root.join("journals/Loose.md"), "- loose journal stray\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let _ = store.whole_graph().unwrap();
    pages::rename_file_to_page(&store, "journals/Loose.md", "A:C").unwrap();
    assert!(root.join("pages/A%3AC.md").exists());
    assert!(!root.join("journals/Loose.md").exists());
    assert!(root.join("pages/A:B.md").exists());
    store.close();
    let _ = fs::remove_dir_all(&root);
}
