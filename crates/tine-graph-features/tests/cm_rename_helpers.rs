//! The rename dialog's interface helpers (D-20261115-HELP, D-20261116-TAGH;
//! model r6 "Outside the model"), read through the app's own question
//! (`pages::rename_offer`): what the alias dialog shows and offers when the
//! typed name is only another page's alias, and the tag union a merge's
//! helper writes before the merge. Nothing here writes: each offered step is
//! an ordinary request (`rename_page_as`) or an ordinary page edit, which a
//! test performs the way the window would (the edit as the file's new text)
//! to check that the read matches what the request then does.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tine_graph_features::pages::{
    rename_offer, rename_page_as, AliasDialog, OwnRefusal, RenameBlocked, RenameChoice,
    RenameOffer, RenameOutcome, TagUnion,
};
use tine_store::Store;

fn fixture(label: &str, files: &[(&str, &str)]) -> (PathBuf, Arc<Store>) {
    let root = crate::test_support::fresh_dir(&format!("tine-cm-helpers-{label}-"));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

fn snapshot(root: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for dir in ["pages", "journals"] {
        for entry in fs::read_dir(root.join(dir)).unwrap().flatten() {
            let rel = format!("{dir}/{}", entry.file_name().to_string_lossy());
            files.insert(rel, fs::read_to_string(entry.path()).unwrap());
        }
    }
    files
}

fn rename(store: &Arc<Store>, old: &str, new: &str, choice: RenameChoice<'_>) -> RenameOutcome {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, choice, &[])
        .unwrap()
        .outcome
}

fn alias_dialog(store: &Store, old: &str, new: &str) -> AliasDialog {
    match rename_offer(store, old, new, None).unwrap() {
        RenameOffer::Alias(dialog) => dialog,
        other => panic!("the alias dialog for {old} -> {new}, got {other:?}"),
    }
}

const ML: &[(&str, &str)] = &[
    (
        "pages/Machine Learning.md",
        "alias:: AI, [[ML]], Stats\n\n- ml body\n",
    ),
    ("pages/ML notes.md", "alias:: Notes\n\n- notes body\n"),
    (
        "pages/q.md",
        "- see [[ML notes]]\n- also [[ML]] here\n- via [[Notes]]\n",
    ),
];

/// D-20261115-HELP: "make it visible to the user what they are actually
/// asking". The typed name ML is only Machine Learning's alias (O3 refuses
/// it); the dialog shows the component and both legal requests with what
/// each changes, including links that open another page with their text
/// unchanged. Reading it writes nothing.
#[test]
fn the_alias_dialog_shows_the_component_and_each_requests_effect() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture("dialog", ML);
    let before = snapshot(&root);
    let dialog = alias_dialog(&store, "ML notes", "ML");
    assert_eq!(
        dialog.reason,
        RenameBlocked::AliasHeld {
            alias: "ML".into(),
            page: "Machine Learning".into(),
            path: "pages/Machine Learning.md".into(),
        }
    );
    assert_eq!(dialog.opens.name, "Machine Learning");
    assert_eq!(dialog.declarers.len(), 1);
    let mut aliases = dialog.declarers[0].aliases.clone();
    aliases.sort();
    assert_eq!(aliases, ["AI", "ML", "Stats"]);
    assert_eq!(dialog.source.name, "ML notes");
    assert_eq!(dialog.source.aliases, ["Notes"]);

    let merge = dialog.merge.as_ref().expect("the merge is offered");
    assert_eq!(merge.into, "pages/Machine Learning.md");
    assert_eq!(merge.name, "Machine Learning");
    assert!(
        merge.effect.pages.contains(&"q".to_owned()),
        "{:?}",
        merge.effect
    );
    // The source's own alias joins the owner: [[Notes]] opens it, text unchanged.
    assert_eq!(merge.effect.retargeted.len(), 1);
    let notes = &merge.effect.retargeted[0];
    assert_eq!(
        (
            notes.name.as_str(),
            notes.before.as_str(),
            notes.after.as_str()
        ),
        ("Notes", "ML notes", "Machine Learning")
    );
    assert_eq!(notes.pages, ["q"]);

    let own = dialog
        .own
        .as_ref()
        .expect("use the name for this page is offered");
    assert!(dialog.own_refused.is_none());
    assert_eq!(own.edits.len(), 1);
    let edit = &own.edits[0];
    assert_eq!(
        (edit.page.as_str(), edit.key.as_str(), edit.before.as_str()),
        ("Machine Learning", "alias", "AI, [[ML]], Stats")
    );
    assert_eq!(edit.after.as_deref(), Some("AI, Stats"));
    // [[ML]] in q opens this page afterwards, its text unchanged.
    let ml = &own.effect.retargeted[0];
    assert_eq!(
        (ml.name.as_str(), ml.before.as_str(), ml.after.as_str()),
        ("ML", "Machine Learning", "ML")
    );
    assert_eq!(ml.pages, ["q"]);
    assert!(own.effect.pages.contains(&"Machine Learning".to_owned()));
    assert_eq!(snapshot(&root), before, "the dialog's read wrote nothing");
}

/// The dialog's two steps are ordinary: the edit the dialog names, then the
/// plain rename it planned, which the model now allows.
#[test]
fn use_the_name_for_this_page_is_the_named_edit_then_the_planned_rename() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture("own", ML);
    let dialog = alias_dialog(&store, "ML notes", "ML");
    let edit = &dialog.own.as_ref().unwrap().edits[0];
    fs::write(
        root.join(&edit.path),
        format!(
            "{}:: {}\n\n- ml body\n",
            edit.key,
            edit.after.as_deref().unwrap()
        ),
    )
    .unwrap();
    assert_eq!(
        rename(&store, "ML notes", "ML", RenameChoice::Rename),
        RenameOutcome::Renamed
    );
    let after = snapshot(&root);
    assert_eq!(
        after["pages/Machine Learning.md"],
        "alias:: AI, Stats\n\n- ml body\n"
    );
    assert_eq!(after["pages/ML.md"], "alias:: Notes\n\n- notes body\n");
    assert_eq!(
        after["pages/q.md"],
        "- see [[ML]]\n- also [[ML]] here\n- via [[Notes]]\n"
    );
    assert!(!after.contains_key("pages/ML notes.md"));
}

/// The merge the dialog offers is the ordinary request with its confirm.
#[test]
fn the_dialogs_merge_is_the_ordinary_merge_request() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture("merge", ML);
    let dialog = alias_dialog(&store, "ML notes", "ML");
    let merge = dialog.merge.unwrap();
    assert_eq!(
        rename(
            &store,
            "ML notes",
            &merge.name,
            RenameChoice::MergeInto(&merge.into)
        ),
        RenameOutcome::Merged
    );
    let after = snapshot(&root);
    assert!(!after.contains_key("pages/ML notes.md"));
    assert!(after["pages/Machine Learning.md"].contains("notes body"));
    assert_eq!(
        after["pages/q.md"],
        "- see [[Machine Learning]]\n- also [[ML]] here\n- via [[Notes]]\n"
    );
}

/// A request the model refuses is not offered; the dialog says why.
#[test]
fn a_refused_step_is_not_offered_and_says_why() {
    let _guard = crate::integration_lock();
    // The alias line repeats its key: the window's property edit would
    // replace both lines, so the user edits it.
    let (_root, store) = fixture(
        "alias-line",
        &[
            (
                "pages/Machine Learning.md",
                "alias:: ML\nalias:: AI\n\n- ml body\n",
            ),
            ("pages/ML notes.md", "- notes body\n"),
        ],
    );
    let dialog = alias_dialog(&store, "ML notes", "ML");
    assert!(dialog.own.is_none());
    assert_eq!(
        dialog.own_refused,
        Some(OwnRefusal::AliasLine {
            page: "Machine Learning".into(),
            path: "pages/Machine Learning.md".into(),
        })
    );
    assert!(dialog.merge.is_some());

    // The rename after the edit would capture an existing page (O4).
    let (_root, store) = fixture(
        "capture",
        &[
            ("pages/Machine Learning.md", "alias:: ML\n\n- ml body\n"),
            ("pages/ML notes.md", "- notes body\n"),
            ("pages/ML notes%2Fx.md", "- child\n"),
            ("pages/ML%2Fx.md", "- taken\n"),
        ],
    );
    let dialog = alias_dialog(&store, "ML notes", "ML");
    assert!(dialog.own.is_none());
    assert!(
        matches!(&dialog.own_refused, Some(OwnRefusal::Plan { reason: RenameBlocked::Capture { image, .. } }) if image == "ML/x"),
        "{:?}",
        dialog.own_refused
    );
}

/// A file-less source (only linked) still gets both requests.
#[test]
fn a_fileless_source_is_offered_the_reference_merge() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "ghost",
        &[
            ("pages/Owner Two.md", "alias:: Short Two\n\n- owner two\n"),
            ("pages/ghost referrer.md", "- [[Ghost]]\n"),
        ],
    );
    let dialog = alias_dialog(&store, "Ghost", "Short Two");
    assert_eq!(dialog.source.path, None);
    let merge = dialog.merge.expect("the reference merge is offered");
    assert_eq!(merge.into, "pages/Owner Two.md");
    assert_eq!(merge.effect.pages, ["ghost referrer"]);
    assert_eq!(
        rename(
            &store,
            "Ghost",
            &merge.name,
            RenameChoice::MergeInto(&merge.into)
        ),
        RenameOutcome::Merged
    );
    assert_eq!(
        snapshot(&root)["pages/ghost referrer.md"],
        "- [[Owner Two]]\n"
    );
}

/// D-20261116-TAGH: when both merged pages have `tags::`, the merge offer
/// carries the union the window writes first: the kept page's line gains the
/// source's missing tags (as the source spells them), the source's line goes.
#[test]
fn a_merge_of_two_tagged_pages_offers_the_tag_union() {
    let _guard = crate::integration_lock();
    for (label, org, survivor, source, expected) in [
        (
            "md",
            false,
            "tags:: b, c\n\n- kept\n",
            "tags:: a, [[B]], [[d e]]\n\n- gone\n",
            Some(("b, c", "a, [[B]], [[d e]]", "b, c, a, [[d e]]")),
        ),
        (
            "org",
            true,
            "#+tags: b, c\n\n* kept\n",
            "#+tags: a, b\n\n* gone\n",
            Some(("b, c", "a, b", "b, c, a")),
        ),
        (
            "source-untagged",
            false,
            "tags:: b\n\n- kept\n",
            "- gone\n",
            None,
        ),
        // Two tags lines in the kept page: the helper does not guess.
        (
            "two-lines",
            false,
            "tags:: b\ntags:: c\n\n- kept\n",
            "tags:: a\n\n- gone\n",
            None,
        ),
    ] {
        let ext = if org { "org" } else { "md" };
        let (old, new) = (format!("pages/Old.{ext}"), format!("pages/New.{ext}"));
        let (_root, store) = fixture(label, &[(&new, survivor), (&old, source)]);
        let offer = rename_offer(&store, "Old", "New", None).unwrap();
        let RenameOffer::Merge { tags, .. } = offer else {
            panic!("{label}: {offer:?}");
        };
        assert_eq!(
            tags,
            expected.map(|(survivor, source, united)| TagUnion {
                survivor: survivor.into(),
                source: source.into(),
                united: united.into(),
            }),
            "{label}"
        );
    }
}

/// The union's edits, then the ordinary merge: one tags line, no leftover.
#[test]
fn the_tag_union_then_the_merge_leaves_one_tags_line() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "union-merge",
        &[
            ("pages/New.md", "tags:: b, c\n\n- kept\n"),
            ("pages/Old.md", "tags:: a, b\n\n- gone\n"),
        ],
    );
    let RenameOffer::Merge { into, tags, .. } = rename_offer(&store, "Old", "New", None).unwrap()
    else {
        panic!("merge");
    };
    let tags = tags.unwrap();
    fs::write(
        root.join("pages/New.md"),
        format!("tags:: {}\n\n- kept\n", tags.united),
    )
    .unwrap();
    fs::write(root.join("pages/Old.md"), "- gone\n").unwrap();
    assert_eq!(
        rename(&store, "Old", "New", RenameChoice::MergeInto(&into)),
        RenameOutcome::Merged
    );
    let merged = &snapshot(&root)["pages/New.md"];
    assert_eq!(merged.matches("tags::").count(), 1, "{merged}");
    assert!(merged.starts_with("tags:: b, c, a\n"), "{merged}");
    assert!(
        merged.contains("- kept") && merged.contains("- gone"),
        "{merged}"
    );
}

/// The wire the window reads (`AliasDialogOffer`, `TagUnion` in src/types.ts).
#[test]
fn the_dialog_and_the_union_keep_the_windows_wire_names() {
    let _guard = crate::integration_lock();
    let (_root, store) = fixture("wire", ML);
    let wire = serde_json::to_value(rename_offer(&store, "ML notes", "ML", None).unwrap()).unwrap();
    assert_eq!(wire["kind"], "alias");
    assert_eq!(wire["reason"]["kind"], "aliasHeld");
    for key in [
        "message",
        "opens",
        "declarers",
        "source",
        "merge",
        "mergeRefused",
        "own",
        "ownRefused",
    ] {
        assert!(wire.get(key).is_some(), "missing {key}: {wire}");
    }
    assert_eq!(wire["merge"]["effect"]["retargeted"][0]["pageCount"], 1);
    assert_eq!(wire["own"]["edits"][0]["after"], "AI, Stats");
    assert!(wire["own"]["effect"]["pageCount"].is_u64());
    let (_root, store) = fixture(
        "wire-refused",
        &[
            (
                "pages/Machine Learning.md",
                "alias:: ML\nalias:: AI\n\n- ml body\n",
            ),
            ("pages/ML notes.md", "- notes body\n"),
        ],
    );
    let wire = serde_json::to_value(rename_offer(&store, "ML notes", "ML", None).unwrap()).unwrap();
    assert_eq!(wire["ownRefused"]["kind"], "aliasLine");
    assert_eq!(wire["ownRefused"]["page"], "Machine Learning");
    let (_root, store) = fixture(
        "wire-tags",
        &[
            ("pages/New.md", "tags:: b\n\n- kept\n"),
            ("pages/Old.md", "tags:: a\n\n- gone\n"),
        ],
    );
    let wire = serde_json::to_value(rename_offer(&store, "Old", "New", None).unwrap()).unwrap();
    assert_eq!(wire["kind"], "merge");
    assert_eq!(
        wire["tags"],
        serde_json::json!({"survivor": "b", "source": "a", "united": "b, a"})
    );
}
