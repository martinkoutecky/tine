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
