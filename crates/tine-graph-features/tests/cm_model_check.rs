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
