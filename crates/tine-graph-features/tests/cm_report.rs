//! CM step 2, slice 2, item 2: O7's report (model r6,
//! `tine-agents/specs/notes/cm-cut-model-r6.md`; Lean `Report`,
//! `Disposition`, `SiteKind`). "The report lists every site it could not
//! spell (left as it was) and every property key it respelled, each with its
//! typed location." Each test renames through the app's entry
//! (`rename_page_as`) and asserts the report the dialog shows and the bytes
//! on disk.
use super::*;
use tine_core::refs::SiteKind;
use tine_graph_features::pages::{Disposition, RenameReport};

fn report(store: &Arc<Store>, old: &str, new: &str) -> RenameReport {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, RenameChoice::Rename, &[]).unwrap()
}

/// O7: "a site whose syntax cannot hold the new name stays as it is and is
/// reported". An Org headline tag cannot hold a name with a space: the
/// referrer is left byte-identical and the report names the site, its file
/// and its kind (a tag).
#[test]
fn o7_a_site_that_cannot_spell_the_new_name_is_reported_where_it_stands() {
    let referrer = "* heading :Old:\n";
    let (root, store) = fixture(
        "o7-unspellable",
        &[("pages/Old.md", "- old\n"), ("pages/r.org", referrer)],
    );
    let report = report(&store, "Old", "two words");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.org")).unwrap(),
        referrer
    );
    let [site] = report.site_exceptions.as_slice() else {
        panic!("one site exception: {:?}", report.site_exceptions);
    };
    assert_eq!(site.path, "pages/r.org");
    assert_eq!(site.kind, SiteKind::Tag);
    assert_eq!(site.disposition, Disposition::Unspellable);
    assert!(
        referrer[site.start..site.end].contains("Old"),
        "{:?}",
        &referrer[site.start..site.end]
    );
}

/// O7: "a property key is written in key spelling (lower-cased, whitespace
/// hyphenated), reported when that differs from the new name". The key is
/// rewritten and the report says it was respelled.
#[test]
fn o7_a_property_key_respelled_into_key_spelling_is_reported() {
    let (root, store) = fixture(
        "o7-key-spelling",
        &[
            ("pages/Old.md", "- old\n"),
            ("pages/r.md", "- Old:: value\n"),
        ],
    );
    let report = report(&store, "Old", "Two Words");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.md")).unwrap(),
        "- two-words:: value\n"
    );
    let [site] = report.site_exceptions.as_slice() else {
        panic!("one site exception: {:?}", report.site_exceptions);
    };
    assert_eq!(site.path, "pages/r.md");
    assert_eq!(site.kind, SiteKind::PropertyKey);
    assert_eq!(site.disposition, Disposition::NormalizedKey);
    assert_eq!(&"- Old:: value\n"[site.start..site.end], "Old");
}

/// O7 control: a rename every site can spell reports no exception (the key
/// spelling equals the new name).
#[test]
fn o7_a_rename_every_site_spells_reports_nothing() {
    let (_root, store) = fixture(
        "o7-clean",
        &[
            ("pages/Old.md", "- old\n"),
            ("pages/r.md", "- [[Old]] #Old\n- old:: value\n"),
        ],
    );
    let report = report(&store, "Old", "new");
    assert!(
        report.site_exceptions.is_empty(),
        "{:?}",
        report.site_exceptions
    );
}

/// O1 with O7: a property key is a typed name site and follows the whole
/// rename, namespace prefix included, "however it exists": renaming `A` to
/// `B` turns `a/x::` (a file-backed child) and `a/y::` (a child that exists
/// only by reference) into `b/x::` and `b/y::`, in Markdown and in an Org
/// property drawer, like a link to the same page; each respelled key is
/// reported (its key spelling differs from the new name `B/…`).
#[test]
fn o1_a_property_key_under_the_source_follows_the_namespace_prefix() {
    let md = "- a/x:: one\n- a/y:: two\n";
    let org = "* h\n:PROPERTIES:\n:a/x: one\n:a/y: two\n:END:\n";
    let (root, store) = fixture(
        "o1-key-prefix",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/child.md", "title:: A/x\n\n- child\n"),
            ("pages/r.md", md),
            ("pages/r.org", org),
        ],
    );
    let report = report(&store, "A", "B");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.md")).unwrap(),
        "- b/x:: one\n- b/y:: two\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("pages/r.org")).unwrap(),
        "* h\n:PROPERTIES:\n:b/x: one\n:b/y: two\n:END:\n"
    );
    let reported: Vec<(&str, &str, SiteKind, Disposition)> = report
        .site_exceptions
        .iter()
        .map(|site| {
            let text = if site.path == "pages/r.md" { md } else { org };
            (
                site.path.as_str(),
                &text[site.start..site.end],
                site.kind,
                site.disposition,
            )
        })
        .collect();
    assert_eq!(
        reported,
        [
            (
                "pages/r.md",
                "a/x",
                SiteKind::PropertyKey,
                Disposition::NormalizedKey
            ),
            (
                "pages/r.md",
                "a/y",
                SiteKind::PropertyKey,
                Disposition::NormalizedKey
            ),
            (
                "pages/r.org",
                "a/x",
                SiteKind::PropertyKey,
                Disposition::NormalizedKey
            ),
            (
                "pages/r.org",
                "a/y",
                SiteKind::PropertyKey,
                Disposition::NormalizedKey
            ),
        ]
    );
}
