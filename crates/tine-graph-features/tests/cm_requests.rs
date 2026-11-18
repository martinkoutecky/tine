//! CM step 2, slice 1: the request-level rules of the confirmed model
//! (`tine-agents/specs/notes/cm-cut-model-r6.md`, D-20261116-CM536). One test
//! per rule, each through the app's rename question (`pages::rename_offer`)
//! and the rename with its answer (`pages::rename_page_as`), asserting what
//! the user gets: the outcome, the refusal and its reason, the file bytes.
use super::*;

#[test]
fn a_stationary_title_refusal_names_its_file_as_data() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "title-refusal-context",
        &[
            ("pages/Old.md", "- old\n"),
            ("pages/child.md", "title:: Else/[[Old]]\n\n- child\n"),
        ],
    );
    let before = snapshot(&root);
    let error = rename(&store, "Old", "New", RenameChoice::Rename).unwrap_err();
    let refusal =
        tine_graph_features::pages::MergeRefused::of(&error).expect("typed title refusal");
    assert_eq!(refusal.file, "pages/child.md");
    assert_eq!(
        refusal.reason,
        tine_graph_features::pages::merge_payload::RealizationError::Trace(
            tine_core::text_trace::TraceError::TitleIdentity
        )
    );
    assert_eq!(snapshot(&root), before);
}

#[test]
fn remaining_rewrite_refusals_keep_context_and_leave_files_unchanged() {
    use tine_graph_features::pages::{merge_pages, RewriteBlocked as R};
    let _guard = crate::integration_lock();
    for eol in ["\n", "\r\n"] {
        let duplicate = format!("title:: A{eol}{eol}- a{eol}");
        let (root, store) = fixture(
            "remaining-duplicate-refusal",
            &[("pages/one.md", &duplicate), ("pages/two.md", &duplicate)],
        );
        let before = snapshot(&root);
        let error = rename(&store, "A", "B", RenameChoice::Rename).unwrap_err();
        assert_eq!(
            R::of(&error),
            Some(&R::DuplicateFiles {
                files: vec!["pages/one.md".into(), "pages/two.md".into()]
            })
        );
        assert_eq!(snapshot(&root), before);
        for (source, target, into, expected) in [
            (
                "pages/A.md",
                "pages/B.org",
                "pages/B.org",
                R::DifferentFormats {
                    source: "pages/A.md".into(),
                    target: "pages/B.org".into(),
                },
            ),
            (
                "pages/A.md",
                "pages/B.md",
                "pages/A.md",
                R::SameFile {
                    file: "pages/A.md".into(),
                },
            ),
        ] {
            let (root, store) = fixture(
                "remaining-file-refusals",
                &[(source, "- a\n"), (target, "* b\n")],
            );
            let before = snapshot(&root);
            let host = crate::test_host(&store);
            let error = merge_pages(&store, &host, source, into).unwrap_err();
            assert_eq!(R::of(&error), Some(&expected), "{error}");
            assert_eq!(snapshot(&root), before);
        }
        for marked_source in [false, true] {
            let marked =
                "- a\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n".replace('\n', eol);
            let files = if marked_source {
                [("pages/A.md", marked.as_str()), ("pages/B.md", "- b\n")]
            } else {
                [("pages/A.md", "- a\n"), ("pages/B.md", marked.as_str())]
            };
            let (root, store) = fixture("remaining-conflict-refusal", &files);
            let before = snapshot(&root);
            let host = crate::test_host(&store);
            let error = merge_pages(&store, &host, "pages/A.md", "pages/B.md").unwrap_err();
            assert_eq!(
                R::of(&error),
                Some(&R::MergeConflictMarkers {
                    file: if marked_source {
                        "pages/A.md"
                    } else {
                        "pages/B.md"
                    }
                    .into(),
                })
            );
            assert_eq!(snapshot(&root), before);
        }
        let (root, store) = fixture(
            "remaining-selection-refusals",
            &[("pages/A.md", "- a\n"), ("pages/B.md", "- b\n")],
        );
        let before = snapshot(&root);
        let host = crate::test_host(&store);
        let error = rename_page_as(
            &store,
            &host,
            "A",
            "B",
            None,
            RenameChoice::MergeInto("pages/missing.md"),
            &[],
        )
        .unwrap_err();
        assert_eq!(
            R::of(&error),
            Some(&R::StaleMerge {
                file: "pages/missing.md".into()
            })
        );
        let error = rename_page_as(
            &store,
            &host,
            "A",
            "C",
            Some("pages/missing.md"),
            RenameChoice::Rename,
            &[],
        )
        .unwrap_err();
        assert_eq!(
            R::of(&error),
            Some(&R::StaleSource {
                file: "pages/missing.md".into()
            })
        );
        assert_eq!(snapshot(&root), before);

        let org = "#+title: A\n* parent\n*** child\n".replace('\n', eol);
        let (root, store) = fixture(
            "remaining-org-refusal",
            &[("pages/A.org", &org), ("pages/B.org", "* b\n")],
        );
        let before = snapshot(&root);
        let host = crate::test_host(&store);
        let error = merge_pages(&store, &host, "pages/A.org", "pages/B.org").unwrap_err();
        assert_eq!(
            R::of(&error),
            Some(&R::ReadOnlyOrg {
                file: "pages/A.org".into()
            })
        );
        assert_eq!(snapshot(&root), before);
    }
}

fn reason(store: &Store, old: &str, new: &str) -> serde_json::Value {
    let offer = serde_json::to_value(rename_offer(store, old, new, None).unwrap()).unwrap();
    // The model's refusal; the window may offer a helper with it (a day move,
    // the contested-name merge: D-20261115-HELP), carrying the same reason.
    assert!(
        ["refused", "contested", "dayMove"].contains(&offer["kind"].as_str().unwrap_or_default()),
        "{old} → {new}: {offer}"
    );
    offer["reason"].clone()
}

/// O0: "A is renamable if it is a page (it has a file, or is referenced, or
/// is a namespace parent) or an alias. Any other name is refused." A name
/// that only a stored query mentions is no reference (O1 lists "every
/// reference" and "names inside stored queries" as different sites).
#[test]
fn o0_a_name_that_is_no_page_and_no_alias_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "o0",
        &[
            ("pages/other.md", "- [[Real]] [[Parent/child]]\n"),
            ("pages/q.md", "- {{query (page \"QueryOnly\")}}\n"),
            ("pages/owner.md", "alias:: Nick\n\n- owner\n"),
        ],
    );
    let before = snapshot(&root);
    for ghost in ["Ghost", "QueryOnly"] {
        let reason = reason(&store, ghost, "X");
        assert_eq!(reason["kind"], "notAPage", "{ghost}: {reason}");
        assert_eq!(reason["name"], ghost);
        let error = rename(&store, ghost, "X", RenameChoice::Rename).unwrap_err();
        assert!(
            matches!(
                RenameBlocked::of(&error),
                Some(RenameBlocked::NotAPage { .. })
            ),
            "{ghost}: {error}"
        );
        assert!(error.to_string().contains(ghost), "{error}");
        assert_eq!(snapshot(&root), before, "{ghost}: a refusal writes nothing");
    }
    for page in ["Real", "Parent", "Nick", "owner"] {
        assert_eq!(
            rename_offer(&store, page, "Renamed", None).unwrap(),
            RenameOffer::Rename,
            "{page}"
        );
    }
    rename(&store, "Parent", "P2", RenameChoice::Rename).unwrap();
    assert_eq!(
        snapshot(&root)["pages/other.md"],
        "- [[Real]] [[P2/child]]\n"
    );
}

/// O2: "A day cannot be the source of a rename." O0: "A day is refused."
/// By the day's identity, whatever its spelling and wherever its file is
/// (journals, pages, or no file at all).
#[test]
fn o2_a_day_is_not_a_rename_source() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "day-source",
        &[
            ("journals/2026_10_26.md", "- day\n"),
            ("pages/Oct 27th, 2026.md", "- stray date page\n"),
            (
                "pages/p.md",
                "- [[Oct 26th, 2026]] [[Oct 27th, 2026]] [[Oct 28th, 2026]]\n",
            ),
        ],
    );
    let before = snapshot(&root);
    for day in [
        "Oct 26th, 2026",
        "2026-10-26",
        "2026_10_26",
        "Oct 27th, 2026",
        "Oct 28th, 2026",
    ] {
        let reason = reason(&store, day, "Renamed day");
        assert_eq!(reason["kind"], "day", "{day}: {reason}");
        assert_eq!(reason["name"], day);
        let error = rename(&store, day, "Renamed day", RenameChoice::Rename).unwrap_err();
        assert!(
            matches!(RenameBlocked::of(&error), Some(RenameBlocked::Day { .. })),
            "{day}: {error}"
        );
        assert_eq!(snapshot(&root), before, "{day}: a refusal writes nothing");
    }
}

/// O2: "Days ... are never respelled." O4: "A day under the source is not
/// touched: it stays (O2), and the rename proceeds." (D-20261106-R13 C2C3:
/// no journal-reference guard.) With the title format `ns/yyyy-MM-dd` every
/// day is spelled under `ns`: renaming `ns` moves the page and its own
/// references, while every day site (a link in two spellings, an alias
/// declaration, a property value, a reference inside a title) stays and the
/// day still opens its journal. The same holds for a merge of `ns`.
#[test]
fn o4_a_day_under_the_source_stays_and_the_rename_proceeds() {
    let _guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        (
            "logseq/config.edn",
            "{:journal/page-title-format \"ns/yyyy-MM-dd\"}\n",
        ),
        ("journals/2026_10_26.md", "- day\n"),
        ("pages/ns.md", "- ns page [[ns/2026-10-26]]\n"),
        (
            "pages/p.md",
            "- [[ns/2026-10-26]] and [[ns]] and [[ns/2026-10-6]] and [[ns/sub]]\n",
        ),
        ("pages/alias.md", "alias:: ns/2026-10-26\n\n- a\n"),
        ("pages/prop.md", "- when:: [[ns/2026-10-26]]\n"),
        ("pages/titled.md", "title:: Log [[ns/2026-10-26]]\n\n- t\n"),
    ];
    let unchanged = [
        "journals/2026_10_26.md",
        "pages/alias.md",
        "pages/prop.md",
        "pages/titled.md",
    ];
    let (root, store) = fixture("day-under-source", files);
    let before = snapshot(&root);
    assert_eq!(
        rename_offer(&store, "ns", "new", None).unwrap(),
        RenameOffer::Rename
    );
    assert_eq!(
        rename(&store, "ns", "new", RenameChoice::Rename).unwrap(),
        RenameOutcome::Renamed
    );
    let after = snapshot(&root);
    assert_eq!(after["pages/new.md"], "- ns page [[ns/2026-10-26]]\n");
    assert!(!after.contains_key("pages/ns.md"));
    assert_eq!(
        after["pages/p.md"],
        "- [[ns/2026-10-26]] and [[new]] and [[ns/2026-10-6]] and [[new/sub]]\n"
    );
    for path in unchanged {
        assert_eq!(
            after[path], before[path],
            "{path}: a day site is never respelled"
        );
    }
    let opened = store.whole_graph().unwrap().link_target("ns/2026-10-26");
    assert!(
        opened.is_journal,
        "the day still opens its journal: {opened:?}"
    );

    let mut merged = files.to_vec();
    merged.push(("pages/target.md", "- target\n"));
    let (root, store) = fixture("day-under-merge-source", &merged);
    let before = snapshot(&root);
    assert_eq!(
        rename(
            &store,
            "ns",
            "target",
            RenameChoice::MergeInto("pages/target.md")
        )
        .unwrap(),
        RenameOutcome::Merged
    );
    let after = snapshot(&root);
    assert!(
        after["pages/target.md"].contains("ns page [[ns/2026-10-26]]"),
        "{}",
        after["pages/target.md"]
    );
    assert_eq!(
        after["pages/p.md"],
        "- [[ns/2026-10-26]] and [[target]] and [[ns/2026-10-6]] and [[target/sub]]\n"
    );
    for path in unchanged {
        assert_eq!(
            after[path], before[path],
            "{path}: a day site is never respelled"
        );
    }
}

/// O1 and O3 with D-20261106-R13 (no journal-reference guard): a journal is
/// a file like any other. Its alias declaration and the links that reach a
/// journal through a renamed name are respelled by O1 for every kind of
/// source; the outcome is O3's (free name: rename; one holder: merge after
/// the confirm; an alias of another page: refused naming that page).
#[test]
fn journal_files_follow_the_rename_like_any_other_file() {
    let _guard = crate::integration_lock();
    for source in ["real", "reference-only", "alias-only"] {
        for target in ["free", "page", "alias"] {
            let mut files = vec![
                ("journals/2026_10_26.md", "alias:: old/sub\n\n- day\n"),
                ("pages/p.md", "- [[old/sub]] and [[old/sub]]\n"),
            ];
            match source {
                "real" => files.push(("pages/old.md", "- source\n")),
                "alias-only" => files.push(("pages/owner.md", "alias:: old\n\n- owner\n")),
                _ => {}
            }
            match target {
                "page" => files.push(("pages/new.md", "- target\n")),
                "alias" => files.push(("pages/target.md", "alias:: new\n\n- target\n")),
                _ => {}
            }
            let (root, store) = fixture(&format!("journal-{source}-{target}"), &files);
            let label = format!("{source}/{target}");
            let choice = match (rename_offer(&store, "old", "new", None).unwrap(), target) {
                (RenameOffer::Rename, "free") => RenameChoice::Rename,
                (RenameOffer::Merge { into, .. }, "page") => {
                    assert_eq!(into, "pages/new.md", "{label}");
                    RenameChoice::MergeInto("pages/new.md")
                }
                (RenameOffer::Alias(dialog), "alias") => {
                    let reason = dialog.reason;
                    assert!(
                        matches!(&reason, RenameBlocked::AliasHeld { page, .. } if page == "target"),
                        "{label}: {reason:?}"
                    );
                    continue;
                }
                (offer, _) => panic!("{label}: {offer:?}"),
            };
            rename(&store, "old", "new", choice).unwrap();
            let after = snapshot(&root);
            assert_eq!(
                after["journals/2026_10_26.md"], "alias:: new/sub\n\n- day\n",
                "{label}"
            );
            assert_eq!(
                after["pages/p.md"], "- [[new/sub]] and [[new/sub]]\n",
                "{label}"
            );
        }
    }
    // A journal's declaration in each header form, and a namespace parent
    // that is a day's title (format "ns": every day is titled `ns`).
    for (label, files, old, new, path, expected) in [
        (
            "md",
            vec![("journals/2026_10_26.md", "alias:: old\n\n- day\n")],
            "old",
            "new",
            "journals/2026_10_26.md",
            "alias:: new\n\n- day\n",
        ),
        (
            "org",
            vec![("journals/2026_10_26.org", "#+ALIASES: old\n\n* day\n")],
            "old",
            "new",
            "journals/2026_10_26.org",
            "#+ALIASES: new\n\n* day\n",
        ),
        (
            "day-parent",
            vec![
                ("journals/2026_10_26.md", "- day\n"),
                ("pages/p.md", "- [[ns/sub]] [[ns]]\n"),
                ("logseq/config.edn", "{:journal/page-title-format \"ns\"}\n"),
            ],
            "ns/sub",
            "new",
            "pages/p.md",
            "- [[new]] [[ns]]\n",
        ),
    ] {
        let (root, store) = fixture(&format!("journal-header-{label}"), &files);
        assert_eq!(
            rename_offer(&store, old, new, None).unwrap(),
            RenameOffer::Rename,
            "{label}"
        );
        rename(&store, old, new, RenameChoice::Rename).unwrap();
        assert_eq!(snapshot(&root)[path], expected, "{label}");
    }
    // The declaration in a first root of only properties: Tine's save writes
    // that block as the page header (`aliases:: new\n\n- day\n`), so the
    // renamed bytes would not survive its next save. P3 (check slice B):
    // refused before any write, naming the journal.
    let first_root = "- aliases:: old\n- day\n";
    let (root, store) = fixture(
        "journal-header-first-root",
        &[("journals/2026_10_26.md", first_root)],
    );
    let error = rename(&store, "old", "new", RenameChoice::Rename).unwrap_err();
    assert_eq!(
        tine_graph_features::pages::CheckRefused::of(&error),
        Some(&tine_graph_features::pages::CheckRefused {
            guarantee: tine_store::Guarantee::P3,
            file: "journals/2026_10_26.md".into(),
        }),
        "{error}"
    );
    assert_eq!(snapshot(&root)["journals/2026_10_26.md"], first_root);
}

/// O3: "No holder, and a page other than the source declares B as an alias,
/// and the source's own file does not claim B: refused, naming the page B
/// opens. This holds when several pages declare B, and when the source has
/// no file." "An answer where nothing was asked is refused." What a name
/// opens: "the claim of the least-path direct declarer".
#[test]
fn o3_a_name_held_as_another_pages_alias_is_refused_naming_the_page_it_opens() {
    let _guard = crate::integration_lock();
    let cases: &[(&str, &[(&str, &str)], &str, &str, &str)] = &[
        (
            "one-declarer",
            &[
                ("pages/Machine Learning.md", "alias:: ML\n\n- ml\n"),
                ("pages/ML notes.md", "- notes\n"),
            ],
            "ML notes",
            "Machine Learning",
            "pages/Machine Learning.md",
        ),
        (
            "several-declarers",
            &[
                ("pages/b.md", "alias:: ML\n\n- b\n"),
                ("pages/a.md", "alias:: ML\n\n- a\n"),
                ("pages/src.md", "- s\n"),
            ],
            "src",
            "a",
            "pages/a.md",
        ),
        (
            "file-less-source",
            &[
                ("pages/owner.md", "alias:: ML\n\n- owner\n"),
                ("pages/refs.md", "- [[src]]\n"),
            ],
            "src",
            "owner",
            "pages/owner.md",
        ),
        (
            "empty-source",
            &[
                ("pages/src.md", ""),
                ("pages/owner.md", "alias:: ML\n\n- owner\n"),
            ],
            "src",
            "owner",
            "pages/owner.md",
        ),
        (
            "source-declares-it-too",
            &[
                ("pages/src.md", "alias:: ML\n\n- body\n"),
                ("pages/other.md", "alias:: ML\n\n- other\n"),
            ],
            "src",
            "other",
            "pages/other.md",
        ),
    ];
    for (label, files, old, page, path) in cases {
        let (root, store) = fixture(&format!("alias-held-{label}"), files);
        let before = snapshot(&root);
        let offer = rename_offer(&store, old, "ML", None).unwrap();
        let RenameOffer::Alias(dialog) = &offer else {
            panic!("{label}: {offer:?}");
        };
        let (reason, message) = (&dialog.reason, &dialog.message);
        assert_eq!(
            reason,
            &RenameBlocked::AliasHeld {
                alias: "ML".into(),
                page: (*page).into(),
                path: (*path).into(),
            },
            "{label}"
        );
        assert!(message.contains(page), "{label}: {message}");
        let error = rename(&store, old, "ML", RenameChoice::Rename).unwrap_err();
        assert_eq!(RenameBlocked::of(&error), Some(reason), "{label}: {error}");
        assert!(
            rename(&store, old, "ML", RenameChoice::MergeInto(path)).is_err(),
            "{label}: an answer where nothing was asked is refused"
        );
        assert_eq!(snapshot(&root), before, "{label}: a refusal writes nothing");
    }
}

/// O3: "No holder otherwise (... or is an alias declared only by the source
/// itself): rename in one step. A respelling moves the file. The source's
/// own declaration of B stays as it is (P1)." No question is asked, so a
/// merge answer is one nobody asked for, and refused.
#[test]
fn o3_renaming_a_page_to_its_own_alias_is_one_step() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "own-alias",
        &[
            ("pages/source.md", "alias:: Short, Other\n\n- body\n"),
            ("pages/r.md", "- [[source]] [[Short]]\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "source", "Short", None).unwrap(),
        RenameOffer::Rename
    );
    let before = snapshot(&root);
    assert!(rename(
        &store,
        "source",
        "Short",
        RenameChoice::MergeInto("pages/source.md")
    )
    .is_err());
    assert_eq!(snapshot(&root), before, "an unasked answer writes nothing");
    assert_eq!(
        rename(&store, "source", "Short", RenameChoice::Rename).unwrap(),
        RenameOutcome::Renamed
    );
    let after = snapshot(&root);
    assert!(!after.contains_key("pages/source.md"));
    assert_eq!(after["pages/Short.md"], "alias:: Short, Other\n\n- body\n");
    assert_eq!(after["pages/r.md"], "- [[Short]] [[Short]]\n");
    match resolve(&store, "Short") {
        Resolved::Existing { id, .. } => assert_eq!(id.as_str(), "pages/Short.md"),
        other => panic!("Short is the page now: {other:?}"),
    }
}

/// O4: "a moved file with conflict markers needs no rewrite (it moves
/// verbatim)"; otherwise the outcome is unavailable and refused with the
/// reason. A marked file the rename moves without changing a byte still
/// moves. The refusal comes from the rename itself: deciding it needs the
/// moved file's bytes, which the rename question does not read.
#[test]
fn o4_a_moved_file_with_conflict_markers_that_needs_a_rewrite_is_refused() {
    let _guard = crate::integration_lock();
    const MARKED: &str = "<<<<<<< a\n- x\n=======\n- y\n>>>>>>> b\n";
    for (label, files, old, path) in [
        (
            "own-title",
            vec![("pages/Old.md", format!("title:: Old\n{MARKED}"))],
            "Old",
            "pages/Old.md",
        ),
        (
            "own-reference",
            vec![("pages/Old.md", format!("- [[Old]]\n{MARKED}"))],
            "Old",
            "pages/Old.md",
        ),
        (
            "namespace-child",
            vec![
                ("pages/Old.md", "- parent\n".to_owned()),
                ("pages/Old___x.md", format!("title:: Old/x\n{MARKED}")),
            ],
            "Old",
            "pages/Old___x.md",
        ),
    ] {
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
        let (root, store) = fixture(&format!("marked-{label}"), &files);
        let before = snapshot(&root);
        let error = rename(&store, old, "New", RenameChoice::Rename)
            .err()
            .unwrap_or_else(|| panic!("{label}: renamed: {:?}", snapshot(&root)));
        assert_eq!(
            RenameBlocked::of(&error),
            Some(&RenameBlocked::ConflictMarkers { file: path.into() }),
            "{label}: {error}"
        );
        assert_eq!(snapshot(&root), before, "{label}: a refusal writes nothing");
    }
    let (root, store) = fixture(
        "marked-verbatim",
        &[("pages/Old.md", MARKED), ("pages/r.md", "- [[Old]]\n")],
    );
    rename(&store, "Old", "New", RenameChoice::Rename).unwrap();
    let after = snapshot(&root);
    assert_eq!(after["pages/New.md"], MARKED, "moved verbatim");
    assert_eq!(after["pages/r.md"], "- [[New]]\n");
}

/// O0, O2, O3 hold for a request that names the source again (round 2
/// finding 2): "Any other name is refused", "A day cannot be the source of a
/// rename", "An answer where nothing was asked is refused". None has a
/// same-spelling exemption. A valid page renamed to its own spelling is
/// unchanged and writes nothing.
#[test]
fn a_request_to_the_same_name_is_checked_like_any_other() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "same-name",
        &[
            ("pages/A.md", "- [[A]] [[2026-10-27]]\n"),
            ("pages/unrelated.md", "- other\n"),
        ],
    );
    let before = snapshot(&root);
    for (name, kind) in [
        ("Ghost", "notAPage"),
        ("2026-10-26", "day"),
        ("2026-10-27", "day"),
    ] {
        assert_eq!(reason(&store, name, name)["kind"], kind, "{name}");
        let error = rename(&store, name, name, RenameChoice::Rename).unwrap_err();
        assert!(RenameBlocked::of(&error).is_some(), "{name}: {error}");
    }
    assert!(
        rename(
            &store,
            "A",
            "A",
            RenameChoice::MergeInto("pages/unrelated.md")
        )
        .is_err(),
        "an answer where nothing was asked is refused"
    );
    assert_eq!(snapshot(&root), before, "a refusal writes nothing");
    assert_eq!(
        rename_offer(&store, "A", "A", None).unwrap(),
        RenameOffer::Rename
    );
    assert_eq!(
        rename(&store, "A", "A", RenameChoice::Rename).unwrap(),
        RenameOutcome::Unchanged
    );
    assert_eq!(
        snapshot(&root),
        before,
        "an unchanged rename writes nothing"
    );
}

/// A request is "this page, that new spelling": the source may be typed in
/// any case. `foo → foo` on page `Foo` respells it to `foo`, and `foo → Bar`
/// renames it, each exactly as the request typed in the page's own spelling
/// does (round 2 correction).
#[test]
fn the_source_typed_in_another_case_is_the_same_request() {
    let _guard = crate::integration_lock();
    let files = [
        ("pages/Foo.md", "- body\n"),
        ("pages/r.md", "- [[Foo]] [[foo]]\n"),
    ];
    for (label, new) in [("respell", "foo"), ("rename", "Bar")] {
        let (typed_root, typed) = fixture(&format!("typed-case-{label}"), &files);
        let (shown_root, shown) = fixture(&format!("shown-case-{label}"), &files);
        let want = rename(&shown, "Foo", new, RenameChoice::Rename).unwrap();
        assert_eq!(want, RenameOutcome::Renamed, "Foo → {new}");
        assert_eq!(
            rename(&typed, "foo", new, RenameChoice::Rename).unwrap(),
            want,
            "foo → {new}"
        );
        assert_eq!(snapshot(&typed_root), snapshot(&shown_root), "foo → {new}");
    }
}

/// A valid request for the page's own spelling is unchanged and decided
/// without a walk of the graph (no listing, no stamps), unlike a refusal.
#[test]
fn a_request_for_the_pages_own_spelling_is_unchanged_without_a_walk() {
    let _guard = crate::exclusive_integration_lock();
    let (root, store) = fixture("own-spelling", &[("pages/Foo.md", "- body\n")]);
    let before = snapshot(&root);
    store.whole_graph().unwrap();
    let app = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app.path()).unwrap();
    while !host.quiescent_for_tests() {
        std::thread::yield_now();
    }
    tine_store::cost_counters::reset();
    let report = rename_page_as(&store, &host, "Foo", "Foo", None, RenameChoice::Rename, &[]);
    let unchanged = tine_store::cost_counters::snapshot();
    assert_eq!(report.unwrap().outcome, RenameOutcome::Unchanged);
    assert_eq!(
        (unchanged.listing_stamps, unchanged.stamps_by_path),
        (0, 0),
        "no walk for an unchanged name: {unchanged:?}"
    );
    assert_eq!(snapshot(&root), before);
    // The control: a refusal is decided on a walked view, which lists.
    tine_store::cost_counters::reset();
    let refused = rename_page_as(&store, &host, "Ghost", "X", None, RenameChoice::Rename, &[]);
    assert!(refused.is_err());
    let walked = tine_store::cost_counters::snapshot();
    assert!(walked.listing_stamps > 0, "a walk lists: {walked:?}");
}
