//! NUI-LSDOC-0511: an Org `[[…]]` link inside a `#[[…]]` tag or inside
//! emphasis, at top level and inside a `#+BEGIN_QUOTE` body, LF and CRLF.
//! lsdoc v0.5.10 gave that inner link no source span at top level and a
//! made-up zero-length span at the quote body start inside the quote (a
//! debug build aborted the parse on a CRLF quote), so Linked References did
//! not mark the emphasised link, or marked an empty place. Rename and merge
//! go through the real path, `pages::rename_or_merge_page` on a page host
//! (the audited save path), and assert the referrer's bytes on disk.
use std::fs;
use tine_store::Store;

fn hosted<T>(store: &std::sync::Arc<Store>, run: impl FnOnce(&tine_store::PageHost) -> T) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

/// The referrer page: every form at top level and inside a quote body. An Org
/// labelled link `[[name][label]]` is left out on this line: its rename is a
/// separate, span-independent gap of the text scanner here.
fn referrer(eol: &str, name: &str) -> String {
    [
        format!("* top #[[{name}]] and *[[{name}]]*"),
        "* quoted".to_string(),
        "#+BEGIN_QUOTE".to_string(),
        "a".to_string(),
        format!("#[[{name}]] and *[[{name}]]*"),
        "#+END_QUOTE".to_string(),
        String::new(),
    ]
    .join(eol)
}

fn rename_case(eol: &str, merge: bool) {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("pages")).unwrap();
    fs::write(root.path().join("pages/c d.org"), "* body of c d\n").unwrap();
    if merge {
        fs::write(root.path().join("pages/g h.org"), "* survivor\n").unwrap();
    }
    fs::write(root.path().join("pages/ref.org"), referrer(eol, "c d")).unwrap();
    let store = std::sync::Arc::new(Store::open(root.path(), Default::default()).unwrap().0);
    let (to, into) = if merge {
        ("g h", Some("pages/g h.org"))
    } else {
        ("e f", None)
    };
    hosted(&store, |host| {
        tine_graph_features::pages::rename_or_merge_page(&store, host, "c d", to, None, into, &[])
    })
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("pages/ref.org")).unwrap(),
        referrer(eol, to),
        "every reference to `c d` names `{to}`, each in its own place (eol {eol:?}, merge {merge})"
    );
}

#[test]
fn rename_respells_org_links_in_tags_and_emphasis_lf() {
    rename_case("\n", false);
}

#[test]
fn rename_respells_org_links_in_tags_and_emphasis_crlf() {
    rename_case("\r\n", false);
}

#[test]
fn merge_respells_org_links_in_tags_and_emphasis_lf() {
    rename_case("\n", true);
}

#[test]
fn merge_respells_org_links_in_tags_and_emphasis_crlf() {
    rename_case("\r\n", true);
}

/// Linked References mark each mention of the page in a referring block; the
/// emphasised link is one, and every mark covers its own bytes.
fn evidence_case(eol: &str) {
    let _integration_guard = crate::integration_lock();
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("pages")).unwrap();
    fs::write(root.path().join("pages/c d.org"), "* body of c d\n").unwrap();
    fs::write(root.path().join("pages/ref.org"), referrer(eol, "c d")).unwrap();
    let store = Store::open(root.path(), Default::default()).unwrap().0;
    let groups = store.whole_graph().unwrap().backlinks("c d").unwrap();
    assert_eq!(groups.len(), 1, "the referrer links `c d` (eol {eol:?})");
    let group = &groups[0];
    assert_eq!(group.blocks.len(), 2, "both referrer blocks link `c d`");
    for block in &group.blocks {
        let text: Vec<u16> = block.raw.encode_utf16().collect();
        let evidence = group
            .evidence
            .iter()
            .find(|e| e.block_id == block.id)
            .unwrap_or_else(|| panic!("evidence for {:?}", block.raw));
        let marks: Vec<String> = evidence
            .occurrences
            .iter()
            .map(|o| String::from_utf16(&text[o.span.start..o.span.end]).unwrap())
            .collect();
        let emphasised = block.raw.find("*[[c d]]*").unwrap() + 1;
        assert!(
            evidence
                .occurrences
                .iter()
                .any(|o| o.span.start == emphasised && o.span.end == emphasised + "[[c d]]".len()),
            "the emphasised link is marked where it stands (eol {eol:?}): {marks:?} in {:?}",
            block.raw
        );
        assert!(
            marks.iter().all(|m| m.contains("c d")),
            "every mark covers a mention (eol {eol:?}): {marks:?}"
        );
    }
}

#[test]
fn linked_reference_marks_org_links_in_tags_and_emphasis_lf() {
    evidence_case("\n");
}

#[test]
fn linked_reference_marks_org_links_in_tags_and_emphasis_crlf() {
    evidence_case("\r\n");
}
