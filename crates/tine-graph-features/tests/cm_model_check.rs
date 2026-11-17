//! CM step 2, check slice B, part 1 (design `cm-step2-p2p3-design.md` items
//! 3 and 4): every path by which a rename or merge reaches a write is checked
//! against the model (P2, P3) on its final bytes, before any write.
//!
//! The writer bug is a test fault (`TEST_RENAME_WRITER_BUG_SENTINEL`): a
//! rewrite or move step whose final bytes hold the sentinel loses it after
//! the writer and before the check. A path whose step skipped the check
//! would write the damaged file; a checked one refuses with P2, names the
//! file, and writes nothing. Each test pins the file the refusal names, so a
//! path that skipped its own check cannot pass on another step's.
use super::*;
use tine_graph_features::pages::CheckRefused;
use tine_store::Guarantee;

const BUG: &str = tine_store::model::TEST_RENAME_WRITER_BUG_SENTINEL;

#[test]
fn macro_tags_decide_nesting_before_selecting_links() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        for macro_name in ["renderer", "cloze"] {
            for (old, argument) in [
                ("Old", "#[[Old]]Tail"),
                ("Old", "#prefix[[Old]]"),
                ("Old", "#[[Old]][[Other]]"),
                ("a [[Old]]", "#[[a [[Old]]]]"),
                ("Old", "#[[Old]] Tail"),
                ("Old", "[[a [[Old]]]]"),
            ] {
                for merge in [false, true] {
                    for eol in ["\n", "\r\n"] {
                        let ext = if org { "org" } else { "md" };
                        let bullet = if org { "*" } else { "-" };
                        let source_path = format!("pages/source.{ext}");
                        let target_path = format!("pages/New.{ext}");
                        let ref_path = format!("pages/r.{ext}");
                        let body = format!("{bullet} {{{{{macro_name} {argument}}}}}{eol}");
                        let source = if org {
                            format!("#+title: {old}{eol}{body}")
                        } else {
                            format!("title:: {old}{eol}{eol}{body}")
                        };
                        let target = format!("{bullet} target{eol}");
                        let mut files = vec![
                            (source_path.as_str(), source.as_str()),
                            (ref_path.as_str(), body.as_str()),
                        ];
                        if merge {
                            files.push((target_path.as_str(), target.as_str()));
                        }
                        let (root, store) = fixture("check-macro-tag-nesting", &files);
                        let app_data = tempfile::tempdir().unwrap();
                        let host =
                            tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
                        let choice = if merge {
                            RenameChoice::MergeInto(&target_path)
                        } else {
                            RenameChoice::Rename
                        };
                        let case = format!(
                            "org={org}, {macro_name}, {argument}, merge={merge}, eol={eol:?}"
                        );
                        match rename_page_as(&store, &host, old, "New", None, choice, &[]) {
                            Ok(report) => {
                                let actual = fs::read_to_string(root.join(&ref_path)).unwrap();
                                let survivor = fs::read_to_string(root.join(&target_path)).unwrap();
                                let protected = argument != "#[[Old]] Tail";
                                let expected = if protected {
                                    body.clone()
                                } else {
                                    body.replace("Old", "New")
                                };
                                if actual != expected
                                    || !survivor.contains(&expected)
                                    || report.site_exceptions.len() != if protected { 2 } else { 0 }
                                {
                                    failures.push(format!("{case}: referrer={actual:?}, survivor={survivor:?}, exceptions={:?}", report.site_exceptions));
                                    continue;
                                }
                                for (path, before) in [(&source_path, &source), (&ref_path, &body)]
                                    .into_iter()
                                    .filter(|_| protected)
                                {
                                    let site = report
                                        .site_exceptions
                                        .iter()
                                        .find(|site| &site.path == path)
                                        .unwrap();
                                    assert_eq!(&before[site.start..site.end], old, "{case}");
                                    assert_eq!(site.kind, tine_core::refs::SiteKind::MacroArg);
                                    assert_eq!(
                                        site.disposition,
                                        tine_graph_features::pages::Disposition::Unspellable
                                    );
                                }
                            }
                            Err(error) => failures.push(format!("{case}: {error}")),
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn enclosing_link_candidates_keep_only_the_models_inner_sites() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for org in [false, true] {
        for to in ["b [[Y]]", "b [[X]]"] {
            for (prefix, suffix) in [
                ("", ""),
                ("{{renderer ", "}}"),
                ("{{cloze ", "}}"),
                ("body\n  tags:: ", ""),
            ] {
                let bullet = if org { "*" } else { "-" };
                let (prefix, suffix) = if org && prefix.starts_with("body") {
                    ("body\n:PROPERTIES:\n:tags: ", "\n:END:")
                } else {
                    (prefix, suffix)
                };
                let before = format!("{bullet} {prefix}[[a [[X]]]]{suffix}\n");
                let path = if org { "pages/r.org" } else { "pages/r.md" };
                for merge in [false, true] {
                    let ext = if org { "org" } else { "md" };
                    let page_path = |name| {
                        format!(
                            "pages/{}.{ext}",
                            tine_core::model::encode_page_name(
                                name,
                                tine_core::config::FileNameFormat::TripleLowbar
                            )
                        )
                    };
                    let source_path = page_path("a [[X]]");
                    let target_path = page_path(to);
                    let source = before.clone();
                    let target = format!("{bullet} target\n");
                    let mut files = vec![
                        (path, before.as_str()),
                        (source_path.as_str(), source.as_str()),
                    ];
                    if merge {
                        files.push((target_path.as_str(), target.as_str()));
                    }
                    let (root, store) = fixture("check-inner-spelling", &files);
                    let app_data = tempfile::tempdir().unwrap();
                    let host =
                        tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
                    let case = format!("org={org}, to={to}, merge={merge}, input={before:?}");
                    let choice = if merge {
                        RenameChoice::MergeInto(&target_path)
                    } else {
                        RenameChoice::Rename
                    };
                    match rename_page_as(&store, &host, "a [[X]]", to, None, choice, &[]) {
                        Ok(report) => {
                            let actual = fs::read_to_string(root.join(path)).unwrap();
                            if org {
                                assert_eq!(actual, before, "{case}");
                                assert_eq!(report.site_exceptions.len(), 2, "{case}");
                                assert!(fs::read_to_string(root.join(&target_path))
                                    .unwrap()
                                    .ends_with(&before));
                                for (site_path, text) in
                                    [(path, &before), (source_path.as_str(), &source)]
                                {
                                    let site = report
                                        .site_exceptions
                                        .iter()
                                        .find(|site| site.path == site_path)
                                        .unwrap();
                                    assert_eq!(&text[site.start..site.end], "a [[X]]");
                                    assert_eq!(
                                        site.disposition,
                                        tine_graph_features::pages::Disposition::Unspellable
                                    );
                                }
                            } else {
                                let expected = before.replace(
                                    "a [[X]]",
                                    &to.replace('[', "\\[").replace(']', "\\]"),
                                );
                                assert_eq!(actual, expected, "{case}");
                                assert!(fs::read_to_string(root.join(&target_path))
                                    .unwrap()
                                    .ends_with(&expected));
                                assert!(report.site_exceptions.is_empty(), "{case}");
                                let report = rename_page_as(
                                    &store,
                                    &host,
                                    to,
                                    "Final",
                                    None,
                                    RenameChoice::Rename,
                                    &[],
                                )
                                .unwrap();
                                assert!(report.site_exceptions.is_empty());
                                assert_eq!(
                                    fs::read_to_string(root.join(path)).unwrap(),
                                    before.replace("a [[X]]", "Final")
                                );
                                assert!(fs::read_to_string(root.join(page_path("Final")))
                                    .unwrap()
                                    .ends_with(&before.replace("a [[X]]", "Final")));
                            }
                        }
                        Err(error) => failures.push(format!("{case}: {error}")),
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failing cases:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn macro_property_and_query_readings_keep_their_own_context() {
    let _guard = crate::integration_lock();
    for (path, before, after, reports) in [
        (
            "pages/r.md",
            "- lead\n  {{renderer #[[Old]]Tail}}\n- [[Old]]\n",
            "- lead\n  {{renderer #[[Old]]Tail}}\n- [[New]]\n",
            1,
        ),
        (
            "pages/r.md",
            "- body\n  tags:: {{renderer #[[Old]]Tail}}\n- [[Old]]\n",
            "- body\n  tags:: {{renderer #[[Old]]Tail}}\n- [[New]]\n",
            1,
        ),
        (
            "pages/r.org",
            "* body\n:PROPERTIES:\n:tags: {{cloze #[[Old]]Tail}}\n:END:\n* [[Old]]\n",
            "* body\n:PROPERTIES:\n:tags: {{cloze #[[Old]]Tail}}\n:END:\n* [[New]]\n",
            1,
        ),
        (
            "pages/r.md",
            "- {{query (and #[[Old]] Tail)}} [[Old]]\n",
            "- {{query (and #New Tail)}} [[New]]\n",
            0,
        ),
        (
            "pages/r.org",
            "* {{query (and #[[Old]] Tail)}} [[Old]]\n",
            "* {{query (and #New Tail)}} [[New]]\n",
            0,
        ),
        (
            "pages/r.md",
            "- [#[[Old]]Tail](file:x.md) [[Old]]\n",
            "- [#[[Old]]Tail](file:x.md) [[New]]\n",
            0,
        ),
        (
            "pages/r.org",
            "* [[file:x.org][#[[Old]]Tail]] [[Old]]\n",
            "* [[file:x.org][#[[Old]]Tail]] [[New]]\n",
            0,
        ),
        ("pages/r.org", "* head :Old:\n", "* head :New:\n", 0),
    ] {
        let (root, store) = fixture(
            "check-reader-contexts",
            &[("pages/Old.md", "- source\n"), (path, before)],
        );
        let report = nested_rename(&store, "Old", "New");
        assert_eq!(
            fs::read_to_string(root.join(path)).unwrap(),
            after,
            "{before:?}"
        );
        assert_eq!(report.site_exceptions.len(), reports, "{before:?}");
        for site in report.site_exceptions {
            assert_eq!(site.path, path);
            assert_eq!(&before[site.start..site.end], "Old");
            assert_eq!(site.kind, tine_core::refs::SiteKind::MacroArg);
        }
    }
}

fn nested_rename(
    store: &Arc<Store>,
    old: &str,
    new: &str,
) -> tine_graph_features::pages::RenameReport {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, RenameChoice::Rename, &[])
        .unwrap_or_else(|e| panic!("{old:?} -> {new:?}: {e}"))
}

/// O7: changing an inner name would repoint the enclosing reference to a
/// different page. Leave it, report its source location, and still rename.
#[test]
fn nested_names_are_left_and_reported_by_the_transaction() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for (org, old, body) in [
        (false, "Old", "[[a [[Old]]]]"),
        (false, "Old", "#[[a [[Old]]]]"),
        (false, "Old Name", "#[[Old Name]]Tail"),
        (true, "Old Name", "#[[Old Name]]Tail"),
        (false, "Old", "body\n  tags:: [[a [[Old]]]]"),
        (false, "Old", "body\n  alias:: [[a [[Old]]]]"),
        (false, "Old", "{{renderer [[a [[Old]]]]}}"),
        (true, "Old", "[[a [[Old]]]]"),
        (
            true,
            "Old",
            "body\n:PROPERTIES:\n:tags: [[a [[Old]]]]\n:END:",
        ),
        (
            true,
            "Old",
            "body\n:PROPERTIES:\n:alias: [[a [[Old]]]]\n:END:",
        ),
    ] {
        for ordinary in [false, true] {
            let path = if org { "pages/r.org" } else { "pages/r.md" };
            let prefix = if org { "* " } else { "- " };
            let tail = if ordinary {
                format!("\n{prefix}[[{old}]]")
            } else {
                String::new()
            };
            let before = format!("{prefix}{body}{tail}\n");
            let (root, store) = fixture(
                "check-nested",
                &[
                    ("pages/source.md", &format!("title:: {old}\n\n- source\n")),
                    (path, &before),
                ],
            );
            let app_data = tempfile::tempdir().unwrap();
            let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
            let report =
                match rename_page_as(&store, &host, old, "New", None, RenameChoice::Rename, &[]) {
                    Ok(report) => report,
                    Err(error) => {
                        failures.push(format!("{before:?}: refused: {error}"));
                        continue;
                    }
                };
            let expected = if ordinary {
                format!("{prefix}{body}\n{prefix}[[New]]\n")
            } else {
                before.clone()
            };
            let actual = fs::read_to_string(root.join(path)).unwrap();
            if actual != expected {
                failures.push(format!(
                    "{before:?}: wrote {actual:?}, expected {expected:?}"
                ));
            }
            // A property value's members are its top-level links only, as in
            // Logseq: the inner name of a nested member is no reference to
            // `old`, so the member is left and nothing is listed.
            if body.contains("tags:") || body.contains("alias:") {
                if !report.site_exceptions.is_empty() {
                    failures.push(format!(
                        "{before:?}: expected no report: {:?}",
                        report.site_exceptions
                    ));
                }
                continue;
            }
            if report.site_exceptions.len() != 1 {
                failures.push(format!(
                    "{before:?}: expected one report: {:?}",
                    report.site_exceptions
                ));
                continue;
            }
            let site = &report.site_exceptions[0];
            assert_eq!(site.path, path);
            assert_eq!(&before[site.start..site.end], old);
            assert_eq!(site.start, before.find(old).unwrap());
            assert_eq!(
                site.kind,
                if body.contains("renderer") {
                    tine_core::refs::SiteKind::MacroArg
                } else {
                    tine_core::refs::SiteKind::Link
                }
            );
            assert_eq!(
                site.disposition,
                tine_graph_features::pages::Disposition::Unspellable
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn escaped_names_can_be_renamed_again_by_the_transaction() {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    let old = r"New\\Tail";
    for (before, after) in [
        ("- [[New\\\\\\\\Tail]]\n", "- [[Final]]\n"),
        ("- [New\\\\\\\\Tail](file:x.md)\n", "- [Final](file:x.md)\n"),
        ("- #[[New\\\\\\\\Tail]]\n", "- #[[Final]]\n"),
        (
            "- body\n  tags:: [[New\\\\\\\\Tail]]\n",
            "- body\n  tags:: [[Final]]\n",
        ),
        (
            "alias:: [[New\\\\\\\\Tail]]\n\n- body\n",
            "alias:: [[Final]]\n\n- body\n",
        ),
        (
            "alias:: New\\\\\\\\Tail\n\n- body\n",
            "alias:: Final\n\n- body\n",
        ),
        (
            "- body\n  tags:: New\\\\\\\\Tail\n",
            "- body\n  tags:: Final\n",
        ),
    ] {
        let (root, store) = fixture(
            "check-escaped-input",
            &[
                ("pages/source.md", &format!("title:: {old}\n\n- source\n")),
                ("pages/r.md", before),
            ],
        );
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        match rename_page_as(&store, &host, old, "Final", None, RenameChoice::Rename, &[]) {
            Ok(report) => {
                let actual = fs::read_to_string(root.join("pages/r.md")).unwrap();
                if actual != after || !report.site_exceptions.is_empty() {
                    failures.push(format!(
                        "{before:?}: wrote {actual:?}, expected {after:?}; report {:?}",
                        report.site_exceptions
                    ));
                }
            }
            Err(error) => failures.push(format!("{before:?}: refused: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// O7 applies even when no site can take the destination. A reference-only
/// source avoids a title/file-name question: each escaped input must be
/// found and reported, though no single-line syntax can hold this new name.
#[test]
fn escaped_unspellable_sites_are_reported_by_the_transaction() {
    let _guard = crate::integration_lock();
    for before in [
        "- [[New\\\\\\\\Tail]]\n",
        "- [New\\\\\\\\Tail](file:x.md)\n",
        "- #[[New\\\\\\\\Tail]]\n",
        "- body\n  tags:: [[New\\\\\\\\Tail]]\n",
        "alias:: [[New\\\\\\\\Tail]]\n\n- body\n",
        "- body\n  tags:: New\\\\\\\\Tail\n",
        "alias:: New\\\\\\\\Tail\n\n- body\n",
    ] {
        let (root, store) = fixture("check-escaped-unspellable", &[("pages/r.md", before)]);
        let report = nested_rename(&store, r"New\\Tail", "Two\nLines");
        assert_eq!(fs::read_to_string(root.join("pages/r.md")).unwrap(), before);
        assert_eq!(report.site_exceptions.len(), 1, "{before:?}");
        let site = &report.site_exceptions[0];
        assert_eq!(site.path, "pages/r.md");
        assert!(before[site.start..site.end].contains("Tail"));
        assert_eq!(
            site.disposition,
            tine_graph_features::pages::Disposition::Unspellable
        );
    }
}

/// O1 maps the enclosing name itself; O7 retains inner references in the
/// namespace suffix. A complete enclosing-name rename can replace it.
#[test]
fn enclosing_names_follow_their_own_map_by_the_transaction() {
    let _guard = crate::integration_lock();
    for (old, before, after, count) in [
        ("Old", "- [[Old/[[Old]]]]\n", "- [[New/[[Old]]]]\n", 1),
        ("a [[Old]]", "- [[a [[Old]]]]\n", "- [[New]]\n", 0),
        ("a [[Old]]", "- #[[a [[Old]]]]\n", "- #[[a [[Old]]]]\n", 1),
        ("[[a [[Old]]]]", "- #[[a [[Old]]]]\n", "- #[[New]]\n", 0),
        (
            "Old",
            "- {{renderer [[Old/[[Old]]]]}}\n",
            "- {{renderer [[New/[[Old]]]]}}\n",
            1,
        ),
    ] {
        let (root, store) = fixture(
            "check-enclosing-name",
            &[
                ("pages/source.md", &format!("title:: {old}\n\n- source\n")),
                ("pages/r.md", before),
            ],
        );
        let report = nested_rename(&store, old, "New");
        assert_eq!(fs::read_to_string(root.join("pages/r.md")).unwrap(), after);
        assert_eq!(report.site_exceptions.len(), count, "{before:?}");
    }
}

#[test]
fn nested_sites_in_a_merge_referrer_and_source_are_left_and_reported() {
    let _guard = crate::integration_lock();
    for (org, source, target, referrer, merged) in [
        (
            false,
            "- [[a [[Old]]]] [[Old]]\n",
            "- target\n",
            "- #[[Old]]Tail [[Old]]\n",
            "- target\n- [[a [[Old]]]] [[New]]\n",
        ),
        (
            true,
            "* [[a [[Old]]]] [[Old]]\n",
            "* target\n",
            "* #[[Old]]Tail [[Old]]\n",
            "* target\n* [[a [[Old]]]] [[New]]\n",
        ),
    ] {
        let source_path = if org { "pages/Old.org" } else { "pages/Old.md" };
        let target_path = if org { "pages/New.org" } else { "pages/New.md" };
        let ref_path = if org { "pages/r.org" } else { "pages/r.md" };
        let (root, store) = fixture(
            "check-nested-merge",
            &[
                (source_path, source),
                (target_path, target),
                (ref_path, referrer),
            ],
        );
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let report = rename_page_as(
            &store,
            &host,
            "Old",
            "New",
            None,
            RenameChoice::MergeInto(target_path),
            &[],
        )
        .unwrap();
        assert_eq!(report.outcome, RenameOutcome::Merged);
        assert_eq!(fs::read_to_string(root.join(target_path)).unwrap(), merged);
        assert_eq!(
            fs::read_to_string(root.join(ref_path)).unwrap(),
            referrer.replace(" [[Old]]\n", " [[New]]\n")
        );
        assert_eq!(report.site_exceptions.len(), 2);
        for (path, text) in [(source_path, source), (ref_path, referrer)] {
            let site = report
                .site_exceptions
                .iter()
                .find(|s| s.path == path)
                .unwrap();
            assert_eq!(&text[site.start..site.end], "Old");
            assert_eq!(
                site.disposition,
                tine_graph_features::pages::Disposition::Unspellable
            );
        }
    }
}

#[test]
fn unglued_tags_and_glued_plain_links_still_follow_the_transaction() {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        let path = if org { "pages/r.org" } else { "pages/r.md" };
        let prefix = if org { "*" } else { "-" };
        let before = format!("{prefix} #[[Old Name]] Tail [[Old Name]]Tail\n");
        let (root, store) = fixture(
            "check-unglued",
            &[("pages/Old Name.md", "- source\n"), (path, &before)],
        );
        let report = nested_rename(&store, "Old Name", "New Name");
        assert!(report.site_exceptions.is_empty());
        assert_eq!(
            fs::read_to_string(root.join(path)).unwrap(),
            before.replace("Old Name", "New Name")
        );
    }
}

/// These look nested in the source but are not inner references in their
/// respective readers: labels retain literal brackets, the query parser
/// reads one argument, and the Org suffix is not a headline tag.
#[test]
fn labels_queries_and_org_suffixes_follow_the_reader_by_the_transaction() {
    let _guard = crate::integration_lock();
    for (org, before, after) in [
        (
            false,
            "- [a [[Old]]](file:x.md) [[Old]]\n",
            "- [a [[Old]]](file:x.md) [[New]]\n",
        ),
        (
            true,
            "* [[file:x.org][a [[Old]]]] [[Old]]\n",
            "* [[file:x.org][a [[Old]]]] [[New]]\n",
        ),
        (
            false,
            "- {{query [[a [[Old]]]]}} [[Old]]\n",
            "- {{query [[a [[Old]]]]}} [[New]]\n",
        ),
        (true, "* head :a[[Old]]:\n", "* head :a[[New]]:\n"),
    ] {
        let path = if org { "pages/r.org" } else { "pages/r.md" };
        let (root, store) = fixture(
            "check-reader-nesting-controls",
            &[("pages/Old.md", "- source\n"), (path, before)],
        );
        let report = nested_rename(&store, "Old", "New");
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), after);
        assert!(report.site_exceptions.is_empty());
    }
}

#[test]
fn nested_name_reports_keep_crlf_file_locations() {
    let _guard = crate::integration_lock();
    let before = "- lead\r\n- #[[Old Name]]Tail\r\n- [[Old Name]]\r\n";
    let (root, store) = fixture(
        "check-nested-crlf",
        &[
            ("pages/Old Name.md", "- source\r\n"),
            ("pages/r.md", before),
        ],
    );
    let report = nested_rename(&store, "Old Name", "New Name");
    assert_eq!(
        fs::read_to_string(root.join("pages/r.md")).unwrap(),
        "- lead\r\n- #[[Old Name]]Tail\r\n- [[New Name]]\r\n"
    );
    let [site] = report.site_exceptions.as_slice() else {
        panic!("{:?}", report.site_exceptions)
    };
    assert_eq!(site.start, before.find("Old Name").unwrap());
    assert_eq!(&before[site.start..site.end], "Old Name");
}

#[test]
fn glued_tag_sites_are_reported_for_each_destination_form() {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        for to in ["New Name", "zz-new", "2026-11-02"] {
            let path = if org { "pages/r.org" } else { "pages/r.md" };
            let before = if org {
                "* #[[Old Name]]Tail\n"
            } else {
                "- #[[Old Name]]Tail\n"
            };
            let (root, store) = fixture(
                "check-glued-destinations",
                &[("pages/Old Name.md", "- source\n"), (path, before)],
            );
            let report = nested_rename(&store, "Old Name", to);
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), before);
            assert_eq!(report.site_exceptions.len(), 1);
            let site = &report.site_exceptions[0];
            assert_eq!(site.path, path);
            assert_eq!(&before[site.start..site.end], "Old Name");
            assert_eq!(
                site.disposition,
                tine_graph_features::pages::Disposition::Unspellable
            );
        }
    }
}

/// An embed that is a linkable property's member keeps its `{{embed [[..]]}}`
/// form when its page is renamed; the check expects that native form.
#[test]
fn embed_members_of_linkable_properties_rename_in_place() {
    let _guard = crate::integration_lock();
    for (path, before, after) in [
        (
            "pages/r.md",
            "- body\n  tags:: {{embed [[a]]}}\n- [[a]]\n",
            "- body\n  tags:: {{embed [[New]]}}\n- [[New]]\n",
        ),
        (
            "pages/r.md",
            "- body\n  alias:: {{embed [[a]]}}\n- [[a]]\n",
            "- body\n  alias:: {{embed [[New]]}}\n- [[New]]\n",
        ),
        (
            "pages/r.md",
            "tags:: {{embed [[a]]}}\n\n- [[a]]\n",
            "tags:: {{embed [[New]]}}\n\n- [[New]]\n",
        ),
        (
            "pages/r.org",
            "* body\n:PROPERTIES:\n:tags: {{embed [[a]]}}\n:END:\n* [[a]]\n",
            "* body\n:PROPERTIES:\n:tags: {{embed [[New]]}}\n:END:\n* [[New]]\n",
        ),
        (
            "pages/r.org",
            "* body\n:PROPERTIES:\n:alias: {{embed [[a]]}}\n:END:\n* [[a]]\n",
            "* body\n:PROPERTIES:\n:alias: {{embed [[New]]}}\n:END:\n* [[New]]\n",
        ),
        (
            "pages/r.org",
            "#+tags: {{embed [[a]]}}\n* [[a]]\n",
            "#+tags: {{embed [[New]]}}\n* [[New]]\n",
        ),
    ] {
        let (root, store) = fixture(
            "check-embed-member",
            &[("pages/a.md", "- source\n"), (path, before)],
        );
        let report = nested_rename(&store, "a", "New");
        assert_eq!(
            fs::read_to_string(root.join(path)).unwrap(),
            after,
            "{before:?}"
        );
        assert!(report.site_exceptions.is_empty(), "{before:?}");
    }
}

#[test]
fn nested_page_alias_members_are_left_unlisted() {
    let _guard = crate::integration_lock();
    for (path, before, after) in [
        (
            "pages/r.md",
            "alias:: [[a [[Old]]]]\n\n- [[Old]]\n",
            "alias:: [[a [[Old]]]]\n\n- [[New]]\n",
        ),
        (
            "pages/r.org",
            "#+alias: [[a [[Old]]]]\n* [[Old]]\n",
            "#+alias: [[a [[Old]]]]\n* [[New]]\n",
        ),
    ] {
        let (root, store) = fixture(
            "check-nested-page-alias",
            &[("pages/Old.md", "- source\n"), (path, before)],
        );
        let report = nested_rename(&store, "Old", "New");
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), after);
        // The alias is `a [[Old]]`; its inner name is no reference to `Old`
        // (Logseq reads a property value's top-level links only).
        assert!(report.site_exceptions.is_empty());
    }
}

#[test]
fn org_enclosing_names_follow_their_own_map_by_the_transaction() {
    let _guard = crate::integration_lock();
    for (old, before, after, reports) in [
        ("Old", "* [[Old/[[Old]]]]\n", "* [[New/[[Old]]]]\n", 1),
        ("a [[Old]]", "* [[a [[Old]]]]\n", "* [[New]]\n", 0),
        (
            "a [[Old]]",
            "* [[file:x.org][a [[Old]]]]\n",
            "* [[file:x.org][New]]\n",
            0,
        ),
    ] {
        let (root, store) = fixture(
            "check-org-enclosing",
            &[
                ("pages/source.md", &format!("title:: {old}\n\n- source\n")),
                ("pages/r.org", before),
            ],
        );
        let report = nested_rename(&store, old, "New");
        assert_eq!(fs::read_to_string(root.join("pages/r.org")).unwrap(), after);
        assert_eq!(report.site_exceptions.len(), reports);
    }
}

#[test]
fn code_and_literal_file_labels_follow_the_reader() {
    let _guard = crate::integration_lock();
    for (path, old, new, before, after) in [
        (
            "pages/r.md",
            "Old",
            "New",
            "- [`Old`](file:x.md)\n",
            "- [`New`](file:x.md)\n",
        ),
        (
            "pages/r.org",
            "=Old=",
            "=New=",
            "* [[file:x.org][=Old=]]\n",
            "* [[file:x.org][=New=]]\n",
        ),
    ] {
        let (root, store) = fixture(
            "check-code-label",
            &[
                ("pages/source.md", &format!("title:: {old}\n\n- source\n")),
                (path, before),
            ],
        );
        let report = nested_rename(&store, old, new);
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), after);
        assert!(report.site_exceptions.is_empty());
    }
}

/// The rename `old` → `new` as the app answers it, refused by the check:
/// the refusal, with nothing on disk changed.
fn refused(
    root: &Path,
    store: &Arc<Store>,
    old: &str,
    new: &str,
    choice: RenameChoice<'_>,
) -> CheckRefused {
    let before = snapshot(root);
    let error = rename(store, old, new, choice).unwrap_err();
    let refused = CheckRefused::of(&error)
        .unwrap_or_else(|| panic!("{old} → {new}: not the model check's refusal: {error}"))
        .clone();
    assert_eq!(
        snapshot(root),
        before,
        "{old} → {new}: a refusal writes nothing"
    );
    refused
}

fn p2(file: &str) -> CheckRefused {
    CheckRefused {
        guarantee: Guarantee::P2,
        file: file.to_owned(),
    }
}

/// O1/O7 and P2: the real transaction admits each faithful label spelling,
/// keeps its formatting, and the reader still names the destination.
fn label_spellings(prefix: &str, suffix: &str, org: bool) {
    let _guard = crate::integration_lock();
    let mut failures = Vec::new();
    for (to, md, org_spelling) in [
        ("a]b", r"a\]b", Some("a]b")),
        (r"New\Tail", r"New\Tail", Some(r"New\Tail")),
        (r"New\\Tail", r"New\\\\Tail", Some(r"New\\Tail")),
        (r"a\*b", r"a\\\*b", Some(r"a\*b")),
        (r"a\#b", r"a\\\#b", Some(r"a\#b")),
        ("a*b", "a*b", Some("a*b")),
        ("a#b", "a#b", Some("a#b")),
        ("a_b", "a_b", Some("a_b")),
        ("a`b", "a`b", Some("a`b")),
        ("a`b`c", "Old", Some("a`b`c")),
        ("New", "New", Some("New")),
        (" New ", "New", Some("New")),
    ] {
        let before = format!("{prefix}Old{suffix}");
        let path = if org { "pages/r.org" } else { "pages/r.md" };
        let (root, store) = fixture(
            "check-label-spelling",
            &[("pages/Old.md", "- source\n"), (path, &before)],
        );
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        match rename_page_as(&store, &host, "Old", to, None, RenameChoice::Rename, &[]) {
            Err(error) => failures.push(format!("{before:?} -> {to:?}: {error}")),
            Ok(report) => {
                let spelling = if org
                    && ((prefix.ends_with('*') && to == "a*b")
                        || (!prefix.ends_with('*') && matches!(to, r"New\Tail" | "a_b")))
                    || (!org && to == "a`b`c")
                {
                    None
                } else if org {
                    org_spelling
                } else {
                    Some(md)
                };
                let expected =
                    spelling.map_or_else(|| before.clone(), |s| format!("{prefix}{s}{suffix}"));
                let actual = fs::read_to_string(root.join(path)).unwrap();
                if actual != expected {
                    failures.push(format!(
                        "{before:?} -> {to:?}: actual {actual:?}, expected {expected:?}"
                    ));
                }
                let parsed = tine_core::render::parse_projection(&actual, org);
                let names: Vec<_> =
                    tine_core::reference_evidence::project(&actual, org, &parsed.blocks)
                        .explicit
                        .into_iter()
                        .filter(|r| r.rule == "explicit_link")
                        .map(|r| r.name)
                        .collect();
                assert_eq!(names, [if actual == before { "Old" } else { to.trim() }]);
                assert_eq!(report.outcome, RenameOutcome::Renamed);
                if report.site_exceptions.len() != usize::from(spelling.is_none()) {
                    failures.push(format!(
                        "{before:?} -> {to:?}: reports {:?}, actual {actual:?}",
                        report.site_exceptions
                    ));
                }
                if spelling.is_some() {
                    // Read the published bytes as a new graph, so this step
                    // tests consuming the emitted spelling, not publication
                    // timing in the previous store instance.
                    let again = Arc::new(Store::open(&root, Default::default()).unwrap().0);
                    let second = nested_rename(&again, to.trim(), "Final");
                    assert!(second.site_exceptions.is_empty(), "{before:?} via {to:?}");
                    assert_eq!(
                        fs::read_to_string(root.join(path)).unwrap(),
                        format!("{prefix}Final{suffix}"),
                        "{before:?} via {to:?}"
                    );
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn plain_markdown_labels_pass_the_model_check() {
    label_spellings("- [", "](file:x.md)\n", false);
}

#[test]
fn formatted_markdown_labels_pass_the_model_check() {
    label_spellings("- [**", "**](file:x.md)\n", false);
}

#[test]
fn image_labels_pass_the_model_check() {
    label_spellings("- ![", "](file:x.md)\n", false);
}

#[test]
fn markdown_links_inside_emphasis_pass_the_model_check() {
    label_spellings("- **[", "](file:x.md)**\n", false);
}

#[test]
fn org_labels_pass_the_model_check() {
    label_spellings("* h [[file:x.org][", "]]\n", true);
}

#[test]
fn emphasized_org_labels_pass_the_model_check() {
    label_spellings("* h [[file:x.org][*", "*]]\n", true);
}

/// A split name has no contiguous replacement run: O7 leaves and reports
/// it while admitting the rename. Spaces around a requested name are trimmed
/// by the existing name-request boundary, before either writer or check.
#[test]
fn unspellable_labels_are_left_and_reported_by_the_transaction() {
    let _guard = crate::integration_lock();
    for (path, before) in [
        ("pages/r.md", "- [**Ol**_d_](file:x.md)\n"),
        ("pages/r.org", "* h [[file:x.org][*Ol*d]]\n"),
    ] {
        let (root, store) = fixture(
            "check-split-label",
            &[("pages/Old.md", "- source\n"), (path, before)],
        );
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let report = rename_page_as(
            &store,
            &host,
            "Old",
            " New ",
            None,
            RenameChoice::Rename,
            &[],
        )
        .unwrap();
        assert!(root.join("pages/New.md").exists());
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), before);
        let [site] = report.site_exceptions.as_slice() else {
            panic!("{:?}", report.site_exceptions)
        };
        assert_eq!(site.path, path);
        assert_eq!(site.kind, tine_core::refs::SiteKind::Link);
        assert_eq!(
            site.disposition,
            tine_graph_features::pages::Disposition::Unspellable
        );
    }
}

/// The same parser-checked spelling boundary fixes the non-label siblings.
/// These assert bytes, actual reader names and O7 reports through the app's
/// transaction rather than accepting agreement between two spelling lists.
#[test]
fn nonlabel_spellings_keep_the_readers_name_or_report_the_site() {
    let _guard = crate::integration_lock();
    for (org, before, to, after, name, exceptions) in [
        (
            false,
            "- #Old\n",
            r"New\\Tail",
            "- #[[New\\\\\\\\Tail]]\n",
            r"New\\Tail",
            0,
        ),
        (
            false,
            "- #[[Old]]\n",
            r"New\\Tail",
            "- #[[New\\\\\\\\Tail]]\n",
            r"New\\Tail",
            0,
        ),
        (
            false,
            "- body\n  tags:: Old\n",
            "a#b",
            "- body\n  tags:: [[a#b]]\n",
            "a#b",
            0,
        ),
        (
            false,
            "alias:: Old\n\n- body\n",
            "a#b",
            "alias:: [[a#b]]\n\n- body\n",
            "a#b",
            0,
        ),
        (
            false,
            "- [[Old]]\n",
            r"New\\Tail",
            "- [[New\\\\\\\\Tail]]\n",
            r"New\\Tail",
            0,
        ),
        (
            false,
            "- {{renderer [[Old]]}}\n",
            r"New\\Tail",
            "- {{renderer [[New\\\\\\\\Tail]]}}\n",
            r"New\\Tail",
            0,
        ),
        (
            true,
            "* heading :Old:\n",
            "a_b",
            "* heading :Old:\n",
            "Old",
            1,
        ),
        (
            true,
            "* [[Old][label]]\n",
            "a]b",
            "* [[Old][label]]\n",
            "Old",
            1,
        ),
        (
            true,
            "* [[Old][label]]\n",
            "a:b",
            "* [[Old][label]]\n",
            "Old",
            1,
        ),
        (
            true,
            "* [[file:Old.org][label]]\n",
            "a]b",
            "* [[file:Old.org][label]]\n",
            "label",
            1,
        ),
    ] {
        let path = if org { "pages/r.org" } else { "pages/r.md" };
        let (root, store) = fixture(
            "check-nonlabel-spelling",
            &[("pages/Old.md", "- source\n"), (path, before)],
        );
        let app_data = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        let report =
            rename_page_as(&store, &host, "Old", to, None, RenameChoice::Rename, &[]).unwrap();
        assert_eq!(report.outcome, RenameOutcome::Renamed);
        assert_eq!(
            fs::read_to_string(root.join(path)).unwrap(),
            after,
            "{before:?} -> {to:?}"
        );
        assert_eq!(
            report.site_exceptions.len(),
            exceptions,
            "{before:?} -> {to:?}"
        );
        for site in &report.site_exceptions {
            assert_eq!(site.path, path);
            assert_eq!(
                site.disposition,
                tine_graph_features::pages::Disposition::Unspellable
            );
        }
        let reading = tine_core::cm_check::observe(
            after,
            if org {
                tine_core::model::Format::Org
            } else {
                tine_core::model::Format::Md
            },
            tine_core::config::FileNameFormat::Legacy,
        );
        let sites: Vec<_> = reading
            .header
            .iter()
            .flat_map(|h| h.text.iter())
            .chain(reading.blocks.iter().flat_map(|b| b.body.iter()))
            .filter_map(|a| match a {
                tine_core::cm_check::Atom::Site(s) => Some(s),
                _ => None,
            })
            .collect();
        assert!(
            sites.iter().any(|s| s.name == name),
            "{before:?}: {sites:?}"
        );
        if exceptions == 0 {
            let again = Arc::new(Store::open(&root, Default::default()).unwrap().0);
            let second = nested_rename(&again, to, "Final");
            assert!(second.site_exceptions.is_empty());
            let expected = match before {
                "- #Old\n" | "- #[[Old]]\n" => "- #[[Final]]\n".to_owned(),
                "- body\n  tags:: Old\n" => "- body\n  tags:: [[Final]]\n".to_owned(),
                "alias:: Old\n\n- body\n" => "alias:: [[Final]]\n\n- body\n".to_owned(),
                _ => before.replace("Old", "Final"),
            };
            assert_eq!(
                fs::read_to_string(root.join(path)).unwrap(),
                expected,
                "{before:?} via {to:?}"
            );
        }
    }
}

/// O7's location kind comes from the enclosing macro even when the shared
/// link spelling cannot produce an edit. A target-only page is still reported.
#[test]
fn an_unspellable_macro_link_is_reported_as_a_macro_argument() {
    let _guard = crate::integration_lock();
    let before = "* {{renderer [[Old][label]]}}\n";
    let (root, store) = fixture(
        "check-macro-spelling-kind",
        &[("pages/Old.md", "- source\n"), ("pages/r.org", before)],
    );
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let report =
        rename_page_as(&store, &host, "Old", "a:b", None, RenameChoice::Rename, &[]).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/r.org")).unwrap(),
        before
    );
    let [site] = report.site_exceptions.as_slice() else {
        panic!("{:?}", report.site_exceptions)
    };
    assert_eq!(site.path, "pages/r.org");
    assert_eq!(&before[site.start..site.end], "Old");
    assert_eq!(site.kind, tine_core::refs::SiteKind::MacroArg);
    assert_eq!(
        site.disposition,
        tine_graph_features::pages::Disposition::Unspellable
    );
}

/// File labels have the same reader-owned name run inside macro arguments,
/// even when the target's stem also contains the old name. The macro brace
/// guard still applies before label spelling, including in a checked file
/// whose ordinary link does change.
#[test]
fn macro_file_labels_share_the_name_run_and_keep_the_macro_guard() {
    let _guard = crate::integration_lock();
    for (org, before, new_text, guarded_text) in [
        (
            false,
            "- {{renderer [**Old**](file:Old.md)}}\n- [[Old]]\n",
            "- {{renderer [**New**](file:Old.md)}}\n- [[New]]\n",
            "- {{renderer [**Old**](file:Old.md)}}\n- [[bad}name]]\n",
        ),
        (
            true,
            "* {{renderer [[file:Old.org][*Old*]]}}\n* [[Old]]\n",
            "* {{renderer [[file:Old.org][*New*]]}}\n* [[New]]\n",
            "* {{renderer [[file:Old.org][*Old*]]}}\n* [[bad}name]]\n",
        ),
    ] {
        for (to, expected, exceptions) in [("New", new_text, 0), ("bad}name", guarded_text, 1)] {
            let path = if org { "pages/r.org" } else { "pages/r.md" };
            let (root, store) = fixture(
                "check-macro-file-label",
                &[("pages/Old.md", "- source\n"), (path, before)],
            );
            let app_data = tempfile::tempdir().unwrap();
            let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
            let report =
                rename_page_as(&store, &host, "Old", to, None, RenameChoice::Rename, &[]).unwrap();
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), expected);
            assert_eq!(report.site_exceptions.len(), exceptions);
            for site in &report.site_exceptions {
                assert_eq!(site.kind, tine_core::refs::SiteKind::MacroArg);
            }
        }
    }
}

/// A referrer's rewrite (`rewrite_refs_prepared`, the planner's proposal).
#[test]
fn a_referrer_rewrite_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-referrer",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/r.md", &format!("- [[A]] {BUG}\n")),
        ],
    );
    let refusal = refused(&root, &store, "A", "C", RenameChoice::Rename);
    assert_eq!(refusal, p2("pages/r.md"));
}

/// Negative control through the transaction: after the shared spelling
/// decision, the writer fault removes a portion of the new label's name.
/// That still parses as a link, but to another page, and P2 refuses it.
#[test]
fn a_damaged_labels_actual_name_is_refused_before_writing() {
    let _guard = crate::integration_lock();
    let old = format!("Old{BUG}");
    let new = format!("New{BUG}");
    for (prefix, suffix, org) in [
        ("- [", "](file:x.md)\n", false),
        ("- [**", "**](file:x.md)\n", false),
        ("- ![", "](file:x.md)\n", false),
        ("* h [[file:x.org][", "]]\n", true),
        ("* h [[file:x.org][*", "*]]\n", true),
    ] {
        let src = format!("pages/{old}.md");
        let path = if org { "pages/r.org" } else { "pages/r.md" };
        let before = format!("{prefix}{old}{suffix}");
        let broken = format!("{prefix}New{suffix}");
        let parsed = tine_core::render::parse_projection(&broken, org);
        let names: Vec<_> = tine_core::reference_evidence::project(&broken, org, &parsed.blocks)
            .explicit
            .into_iter()
            .filter(|r| r.rule == "explicit_link")
            .map(|r| r.name)
            .collect();
        assert_eq!(names, ["New"]);
        assert_ne!(names[0], new);
        let (root, store) = fixture(
            "check-damaged-label",
            &[(&src, "- source\n"), (path, &before)],
        );
        assert_eq!(
            refused(&root, &store, &old, &new, RenameChoice::Rename),
            p2(path)
        );
    }
}

#[test]
fn a_namespace_descendants_move_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-descendant",
        &[
            ("logseq/config.edn", "{:file/name-format :triple-lowbar}"),
            ("pages/A.md", "- source\n"),
            ("pages/A___child.md", &format!("- child {BUG}\n")),
        ],
    );
    assert_eq!(
        refused(&root, &store, "A", "C", RenameChoice::Rename),
        p2("pages/A___child.md")
    );
}

#[test]
fn a_journal_referrers_rewrite_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-journal",
        &[
            ("pages/A.md", "- source\n"),
            ("journals/2026_11_02.md", &format!("- [[A]] {BUG}\n")),
        ],
    );
    assert_eq!(
        refused(&root, &store, "A", "C", RenameChoice::Rename),
        p2("journals/2026_11_02.md")
    );
}

/// Explicitly exercise preflight's fallback, without a prepared proposal.
/// The fault reaches the same check before PagePlan can construct a host op.
#[test]
fn a_fallback_rewrite_move_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-move-fallback",
        &[("pages/A.md", &format!("- [[A]] {BUG}\n"))],
    );
    let before = snapshot(&root);
    let src = store.file_id(tine_store::Area::Pages, "A.md").unwrap();
    let dst = store.file_id(tine_store::Area::Pages, "C.md").unwrap();
    let (_, rev) = store.read(&src, None).unwrap();
    let mut tx = store.page_plan(Some(tine_store::EditKind::RenamePage));
    tx.move_file(
        &src,
        rev,
        &dst,
        tine_store::MoveRewrite::Rewrite(Arc::new(tine_store::RenameMap(
            vec![("A".into(), "C".into())],
            vec![],
        ))),
    );
    tx.check(
        &src,
        tine_store::RenameCheck {
            plan: Arc::new(tine_store::CheckPlan {
                old: Some("A".into()),
                destination: "C".into(),
                days: tine_core::date::JournalFormat::default(),
                names: tine_core::config::FileNameFormat::Legacy,
            }),
            page: "A".into(),
            fixed: false,
            observed: None,
            source: None,
        },
    );
    let error = match tx.plan() {
        Err(error) => error,
        Ok(_) => panic!("damaged move admitted"),
    };
    assert!(
        matches!(error, tine_store::Why::Refused(tine_store::Refusal::ModelCheck { guarantee: Guarantee::P2, ref file }) if file.as_str() == "pages/A.md"),
        "{error:?}"
    );
    assert_eq!(snapshot(&root), before);
}

/// The renamed page's move with its own references rewritten (the
/// planner's proposal, carried to the move step: `MoveRewrite::Prepared`).
#[test]
fn a_rewritten_move_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-move-prepared",
        &[("pages/A.md", &format!("- [[A]] {BUG}\n"))],
    );
    let refusal = refused(&root, &store, "A", "C", RenameChoice::Rename);
    assert_eq!(refusal, p2("pages/A.md"));
}

/// The renamed page's move with no reference to rewrite: its bytes move as
/// they are (`MoveRewrite::Verbatim` is what a marked page takes; an
/// unmarked one with nothing to rewrite takes the prepared move with equal
/// bytes). Both are moves, both checked.
#[test]
fn a_move_with_unchanged_bytes_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-move-plain",
        &[("pages/A.md", &format!("- a {BUG}\n"))],
    );
    let refusal = refused(&root, &store, "A", "C", RenameChoice::Rename);
    assert_eq!(refusal, p2("pages/A.md"));
}

/// A page carrying VCS conflict markers moves verbatim
/// (`MoveRewrite::Verbatim`, R-VCS-MARKERS): checked as well.
#[test]
fn a_verbatim_move_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-move-verbatim",
        &[(
            "pages/A.md",
            &format!("- a {BUG}\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n"),
        )],
    );
    let refusal = refused(&root, &store, "A", "C", RenameChoice::Rename);
    assert_eq!(refusal, p2("pages/A.md"));
}

/// The same marked page with no writer bug moves (SPEC O4: "a moved file with
/// conflict markers needs no rewrite (it moves verbatim)"): its unchanged
/// save refuses (R-VCS-MARKERS) and publishes nothing, which leaves its bytes
/// and its path. The one moved file P3 lets through a refusing save; it is
/// open with the manager (receipt Q9), since ruling Q6 names no exception.
#[test]
fn a_marked_page_still_moves_verbatim() {
    let _guard = crate::integration_lock();
    let body = "- a\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n";
    let (root, store) = fixture("check-move-verbatim-ok", &[("pages/A.md", body)]);
    rename(&store, "A", "C", RenameChoice::Rename).unwrap();
    assert_eq!(fs::read_to_string(root.join("pages/C.md")).unwrap(), body);
}

/// An interrupted rename's title rebind in place (`rewrite_refs`,
/// `TitleRebind::Own`): `pages/New.md` still says `title:: Old`.
#[test]
fn a_title_rebind_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-rebind",
        &[("pages/New.md", &format!("title:: Old\n\n- {BUG}\n"))],
    );
    let refusal = refused(&root, &store, "Old", "New", RenameChoice::Rename);
    assert_eq!(refusal, p2("pages/New.md"));
}

/// A rename that merges: the survivor's write (the merge writer's proposal,
/// `rewrite_refs_prepared` with the source's and the survivor's readings).
#[test]
fn a_rename_merge_survivor_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-survivor",
        &[
            ("pages/A.md", &format!("- a {BUG}\n")),
            ("pages/B.md", "- b\n"),
        ],
    );
    let refusal = refused(
        &root,
        &store,
        "A",
        "B",
        RenameChoice::MergeInto("pages/B.md"),
    );
    assert_eq!(refusal, p2("pages/B.md"));
}

/// The survivor's united alias members (O6), which the merge writer used to
/// re-read itself (`AliasField`, deleted): a member damaged after the writer
/// is refused by the check, in both formats.
#[test]
fn a_merge_survivors_united_aliases_are_checked() {
    let _guard = crate::integration_lock();
    for (ext, source, target) in [
        ("md", format!("alias:: X{BUG}\n- a\n"), "alias:: Y\n- b\n"),
        (
            "org",
            format!("#+alias: X{BUG}\n* a\n"),
            "#+alias: Y\n* b\n",
        ),
    ] {
        let (src, dst) = (format!("pages/A.{ext}"), format!("pages/B.{ext}"));
        let (root, store) = fixture(
            &format!("check-aliases-{ext}"),
            &[(src.as_str(), source.as_str()), (dst.as_str(), target)],
        );
        let refusal = refused(&root, &store, "A", "B", RenameChoice::MergeInto(&dst));
        assert_eq!(refusal, p2(&dst), "{ext}");
    }
}

/// The review's Ghost mutation, at the transaction: a survivor whose united
/// alias declaration names a member the union does not hold
/// (`alias:: X, B, Ghost`). The writer's own acceptance compares placement
/// with the header intent it emitted, so a fault in the field emitter would
/// corrupt both alike; only the independent model check certifies the union's
/// meaning, and it refuses the step with P2 before any write.
#[test]
fn a_survivor_alias_member_outside_the_union_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-ghost-member",
        &[
            ("pages/A.md", &format!("alias:: B, Gh{BUG}ost\n- source\n")),
            ("pages/B.md", "alias:: X\n- target\n"),
        ],
    );
    let before = snapshot(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let error = tine_graph_features::pages::merge_pages(&store, &host, "pages/A.md", "pages/B.md")
        .unwrap_err();
    assert_eq!(CheckRefused::of(&error), Some(&p2("pages/B.md")), "{error}");
    assert_eq!(snapshot(&root), before, "a refusal writes nothing");
}

/// The merge that renames nothing (`pages::merge_pages`, the model's empty
/// plan): its survivor is checked the same way.
#[test]
fn a_plain_merge_survivor_is_checked() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-merge-pages",
        &[
            ("pages/A.md", &format!("- a {BUG}\n")),
            ("pages/B.md", "- b\n"),
        ],
    );
    let before = snapshot(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let error = tine_graph_features::pages::merge_pages(&store, &host, "pages/A.md", "pages/B.md")
        .unwrap_err();
    assert_eq!(CheckRefused::of(&error), Some(&p2("pages/B.md")), "{error}");
    assert_eq!(snapshot(&root), before, "a refusal writes nothing");
}

/// The same files with no writer bug rename and merge as before: the check
/// passes what the writers write.
#[test]
fn the_check_passes_the_writers_output() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-passes",
        &[
            ("pages/A.md", "- [[A]] a\n"),
            ("pages/B.md", "- b\n"),
            ("pages/r.md", "- [[A]] and #A\n"),
            ("pages/New.md", "title:: Old\n\n- n\n"),
        ],
    );
    rename(&store, "A", "C", RenameChoice::Rename).unwrap();
    assert_eq!(snapshot(&root)["pages/r.md"], "- [[C]] and #C\n");
    rename(&store, "Old", "New", RenameChoice::Rename).unwrap();
    rename(&store, "C", "B", RenameChoice::MergeInto("pages/B.md")).unwrap();
    assert!(snapshot(&root)["pages/B.md"].contains("[[B]] a"));
}

/// P3: a referrer whose first block holds `tags::` reads as a page property
/// block that Tine's own save writes as the page's header (an extra blank
/// line). The rename's rewrite of it would not survive Tine's next save
/// unchanged, so the rename refuses, names the file, and says how to fix it.
#[test]
fn a_referrer_tine_would_reformat_is_refused_with_p3() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "check-p3",
        &[("pages/A.md", "- a\n"), ("pages/r.md", "- tags:: [[A]]\n")],
    );
    let refusal = refused(&root, &store, "A", "C", RenameChoice::Rename);
    assert_eq!(
        refusal,
        CheckRefused {
            guarantee: Guarantee::P3,
            file: "pages/r.md".into(),
        }
    );
    let message = refusal.to_string();
    assert!(
        message.contains("pages/r.md") && message.contains("save it once"),
        "{message}"
    );
}

/// Manager ruling Q6: a moved file is changed (Lean `changed G H`: its path
/// is new), so P3 checks it even when its bytes move as they are. The
/// renamed page, whose only block is a properties-only bullet (Tine's save
/// makes it the page header, as `a_childless_properties_only_first_bullet_does_not_survive`
/// pins), is refused naming the file at its old path; nothing is written. So
/// is a namespace child of that shape, moved verbatim; without the
/// properties the child moves (control).
#[test]
fn a_moved_page_tine_would_reformat_is_refused_with_p3() {
    let _guard = crate::integration_lock();
    let p3 = |file: &str| CheckRefused {
        guarantee: Guarantee::P3,
        file: file.to_owned(),
    };
    // `A___b.md` is the namespace child `A/b` under this codec.
    let codec = ("logseq/config.edn", "{:file/name-format :triple-lowbar}");
    // The referrer shape of `a_referrer_tine_would_reformat_is_refused_with_p3`.
    let (root, store) = fixture("check-p3-move", &[codec, ("pages/A.md", "- tags:: x\n")]);
    assert_eq!(
        refused(&root, &store, "A", "C", RenameChoice::Rename),
        p3("pages/A.md")
    );
    let (root, store) = fixture(
        "check-p3-child",
        &[
            codec,
            ("pages/A.md", "- a\n"),
            ("pages/A___b.md", "- tags:: x\n"),
        ],
    );
    assert_eq!(
        refused(&root, &store, "A", "C", RenameChoice::Rename),
        p3("pages/A___b.md")
    );
    let (root, store) = fixture(
        "check-p3-move-ok",
        &[codec, ("pages/A.md", "- a\n"), ("pages/A___b.md", "- b\n")],
    );
    rename(&store, "A", "C", RenameChoice::Rename).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pages/C___b.md")).unwrap(),
        "- b\n"
    );
}
