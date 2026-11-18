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
    report_as(store, old, new, RenameChoice::Rename)
}

fn report_as(store: &Arc<Store>, old: &str, new: &str, choice: RenameChoice<'_>) -> RenameReport {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, choice, &[]).unwrap()
}

/// Each exception as (path, the bytes of its span in `texts[path]`, kind,
/// disposition): slicing proves the span is in that file's text before the
/// rename, on character boundaries.
fn located<'t>(
    report: &RenameReport,
    texts: &[(&str, &'t str)],
) -> Vec<(String, &'t str, SiteKind, Disposition)> {
    report
        .site_exceptions
        .iter()
        .map(|site| {
            let (_, text) = texts
                .iter()
                .find(|(path, _)| *path == site.path)
                .unwrap_or_else(|| panic!("unexpected file {site:?}"));
            (
                site.path.clone(),
                &text[site.start..site.end],
                site.kind,
                site.disposition,
            )
        })
        .collect()
}

/// Review round 2 follow-up: a merge's exceptions are located in each of
/// the merge writer's two original inputs (the source's file and the
/// survivor's), by path and offset in that file's text before the merge.
#[test]
fn o7_a_merges_exceptions_are_located_in_each_original_file() {
    let _integration_guard = crate::integration_lock();
    let source = "* old heading :Old:\n";
    let survivor = "#+title: two words\n* kept :Old:\n";
    let (root, store) = fixture(
        "o7-merge-locations",
        &[("pages/Old.org", source), ("pages/two words.org", survivor)],
    );
    let report = report_as(
        &store,
        "Old",
        "two words",
        RenameChoice::MergeInto("pages/two words.org"),
    );
    assert_eq!(report.outcome, RenameOutcome::Merged);
    let sites = located(
        &report,
        &[("pages/Old.org", source), ("pages/two words.org", survivor)],
    );
    let mut paths: Vec<&str> = sites.iter().map(|(path, ..)| path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["pages/Old.org", "pages/two words.org"], "{sites:?}");
    for (path, span, kind, disposition) in &sites {
        assert!(span.contains("Old"), "{path}: {span:?}");
        assert_eq!(
            (*kind, *disposition),
            (SiteKind::Tag, Disposition::Unspellable),
            "{path}"
        );
    }
    assert!(fs::read_to_string(root.join("pages/two words.org"))
        .unwrap()
        .contains(":Old:"));
}

/// Review round 2 follow-up: offsets are bytes in the file's text before the
/// rename, so CRLF line ends and multibyte characters before a site count in
/// full; and one file reports two kinds of exception, in file order.
#[test]
fn o7_offsets_count_crlf_and_multibyte_bytes_and_one_file_reports_two_kinds() {
    let _integration_guard = crate::integration_lock();
    let org = "* žluťoučký kůň :Old:\r\n:PROPERTIES:\r\n:Old: v\r\n:END:\r\n";
    let md = "- žluťoučký [[Old]]\r\n- Old:: v\r\n";
    let (root, store) = fixture(
        "o7-offsets",
        &[
            ("pages/Old.md", "- old\n"),
            ("pages/r.org", org),
            ("pages/r.md", md),
        ],
    );
    let report = report(&store, "Old", "Two Words");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.md")).unwrap(),
        "- žluťoučký [[Two Words]]\r\n- two-words:: v\r\n"
    );
    let sites = located(&report, &[("pages/r.org", org), ("pages/r.md", md)]);
    let md_key = md.rfind("Old").unwrap();
    assert_eq!(
        report
            .site_exceptions
            .iter()
            .find(|site| site.path == "pages/r.md")
            .map(|site| site.start),
        Some(md_key),
        "{sites:?}"
    );
    let org_sites: Vec<_> = sites
        .iter()
        .filter(|(path, ..)| path == "pages/r.org")
        .map(|(_, span, kind, disposition)| (*kind, *disposition, span.contains("Old")))
        .collect();
    assert_eq!(
        org_sites,
        [
            (SiteKind::Tag, Disposition::Unspellable, true),
            (SiteKind::PropertyKey, Disposition::NormalizedKey, true),
        ],
        "{sites:?}"
    );
    assert_eq!(
        sites
            .iter()
            .filter(|(path, ..)| path == "pages/r.md")
            .map(|(_, span, kind, disposition)| (*span, *kind, *disposition))
            .collect::<Vec<_>>(),
        [("Old", SiteKind::PropertyKey, Disposition::NormalizedKey)]
    );
}

/// Review round 2 follow-up: property keys that do not name the renamed
/// page stay as they are and are not reported: a key that only shares a
/// prefix, a key inside a code block, a built-in key, and a day key (days
/// are pinned, O2).
#[test]
fn o7_keys_that_do_not_name_the_page_stay_and_are_not_reported() {
    let _integration_guard = crate::integration_lock();
    // (label, source file, rename, referrer, referrer after the rename)
    let day_format = (
        "logseq/config.edn",
        "{:journal/page-title-format \"ns/yyyy-MM-dd\"}\n",
    );
    let cases: [(&str, &[(&str, &str)], (&str, &str), &str, &str); 3] = [
        (
            "prefix-and-code",
            &[("pages/Old.md", "- old\n")],
            ("Old", "Two Words"),
            "- older:: one\n- old-x:: two\n- ```\n  Old:: three\n  ```\n",
            "- older:: one\n- old-x:: two\n- ```\n  Old:: three\n  ```\n",
        ),
        (
            "built-in",
            &[("pages/Collapsed.md", "- c\n")],
            ("Collapsed", "Folded"),
            "- block\n  collapsed:: true\n",
            "- block\n  collapsed:: true\n",
        ),
        (
            "day",
            &[day_format, ("pages/ns.md", "- ns\n")],
            ("ns", "nt"),
            // Not a first block of only properties (a page header).
            "- r\n- ns/2026-10-26:: v\n- [[ns/sub]]\n",
            "- r\n- ns/2026-10-26:: v\n- [[nt/sub]]\n",
        ),
    ];
    for (label, sources, (old, new), referrer, after) in cases {
        let mut files = sources.to_vec();
        files.push(("pages/r.md", referrer));
        let (root, store) = fixture(&format!("o7-key-{label}"), &files);
        let got = report(&store, old, new);
        assert_eq!(
            fs::read_to_string(root.join("pages/r.md")).unwrap(),
            after,
            "{label}"
        );
        assert!(
            got.site_exceptions.is_empty(),
            "{label}: {:?}",
            got.site_exceptions
        );
    }
}

/// O7: "a site whose syntax cannot hold the new name stays as it is and is
/// reported". An Org headline tag cannot hold a name with a space: the
/// referrer is left byte-identical and the report names the site, its file
/// and its kind (a tag).
#[test]
fn o7_a_site_that_cannot_spell_the_new_name_is_reported_where_it_stands() {
    let _integration_guard = crate::integration_lock();
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
/// rewritten and the report says it was respelled. (The property block is
/// not the file's first block: a first block of only properties is a page
/// header Tine's save rewrites, so a rename of it refuses with P3;
/// `cm_model_check::a_referrer_tine_would_reformat_is_refused_with_p3`.)
#[test]
fn o7_a_property_key_respelled_into_key_spelling_is_reported() {
    let _integration_guard = crate::integration_lock();
    let referrer = "- r\n- Old:: value\n";
    let (root, store) = fixture(
        "o7-key-spelling",
        &[("pages/Old.md", "- old\n"), ("pages/r.md", referrer)],
    );
    let report = report(&store, "Old", "Two Words");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.md")).unwrap(),
        "- r\n- two-words:: value\n"
    );
    let [site] = report.site_exceptions.as_slice() else {
        panic!("one site exception: {:?}", report.site_exceptions);
    };
    assert_eq!(site.path, "pages/r.md");
    assert_eq!(site.kind, SiteKind::PropertyKey);
    assert_eq!(site.disposition, Disposition::NormalizedKey);
    assert_eq!(&referrer[site.start..site.end], "Old");
}

/// O7 control: a rename every site can spell reports no exception (the key
/// spelling equals the new name).
#[test]
fn o7_a_rename_every_site_spells_reports_nothing() {
    let _integration_guard = crate::integration_lock();
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
    let _integration_guard = crate::integration_lock();
    // Not a first block of only properties (a page header; see
    // `o7_a_property_key_respelled_into_key_spelling_is_reported`).
    let md = "- r\n- a/x:: one\n- a/y:: two\n";
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
        "- r\n- b/x:: one\n- b/y:: two\n"
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
