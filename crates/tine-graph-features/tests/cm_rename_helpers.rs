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
    assert_eq!(dialog.component.len(), 1);
    let mut aliases = dialog.component[0].aliases.clone();
    aliases.sort();
    assert_eq!(aliases, ["AI", "ML", "Stats"]);
    assert_eq!(dialog.source.name, "ML notes");
    assert_eq!(dialog.source.aliases, ["Notes"]);

    assert_eq!(dialog.merges.len(), 1);
    let merge = dialog.merges.first().expect("the merge is offered");
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
    let merge = dialog.merges.into_iter().next().unwrap();
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
    assert_eq!(dialog.merges.len(), 1);

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
    let merge = dialog
        .merges
        .into_iter()
        .next()
        .expect("the reference merge is offered");
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
        "component",
        "source",
        "merges",
        "mergesRefused",
        "own",
        "ownRefused",
    ] {
        assert!(wire.get(key).is_some(), "missing {key}: {wire}");
    }
    assert_eq!(wire["merges"][0]["effect"]["retargeted"][0]["pageCount"], 1);
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

fn resolved_name(store: &Store, name: &str) -> String {
    let graph = store.whole_graph().unwrap();
    let id = match graph.resolve(name, false) {
        tine_store::Resolved::Existing { id, .. } => id,
        tine_store::Resolved::Alias { owners } => owners[0].clone(),
        other => panic!("{name} resolves to {other:?}"),
    };
    id.as_str().to_owned()
}

fn retargets(
    effect: &tine_graph_features::pages::Effect,
) -> Vec<(String, String, String, Vec<String>)> {
    effect
        .retargeted
        .iter()
        .map(|r| {
            (
                r.name.clone(),
                r.before.clone(),
                r.after.clone(),
                r.pages.clone(),
            )
        })
        .collect()
}

/// D-20261106-R13-UI / D-20261115-HELP: the dialog shows the alias
/// component, not only the direct declarers, and offers the ordinary merge
/// into every component page the model allows (each its own request).
#[test]
fn the_dialog_offers_a_merge_into_every_component_page() {
    let _guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        ("pages/A.md", "alias:: ML\n\n- a body\n"),
        ("pages/B.md", "alias:: ML, Stats\n\n- b body\n"),
        ("pages/C.md", "alias:: Stats\n\n- c body\n"),
        ("pages/Source.md", "- source body\n"),
    ];
    let (_root, store) = fixture("component", files);
    let dialog = alias_dialog(&store, "Source", "ML");
    let component: Vec<&str> = dialog.component.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(component, ["A", "B", "C"]);
    let into: Vec<&str> = dialog.merges.iter().map(|m| m.into.as_str()).collect();
    assert_eq!(into, ["pages/A.md", "pages/B.md", "pages/C.md"]);
    assert!(
        dialog.merges_refused.is_empty(),
        "{:?}",
        dialog.merges_refused
    );
    for target in ["B", "C"] {
        let (root, store) = fixture(&format!("component-{target}"), files);
        let dialog = alias_dialog(&store, "Source", "ML");
        let merge = dialog.merges.iter().find(|m| m.name == target).unwrap();
        assert_eq!(
            rename(
                &store,
                "Source",
                &merge.name,
                RenameChoice::MergeInto(&merge.into)
            ),
            RenameOutcome::Merged,
            "{target}"
        );
        let after = snapshot(&root);
        assert!(!after.contains_key("pages/Source.md"));
        assert!(after[&format!("pages/{target}.md")].contains("source body"));
    }
}

/// Each "links open a different page" row is the owner the name resolves to
/// before and after the request: a declarer with a lesser path keeps a
/// shared alias, and a real file keeps its own name.
#[test]
fn a_merge_retargets_only_names_whose_owner_changes() {
    let _guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        ("pages/A.md", "alias:: Shared\n\n- a body\n"),
        ("pages/Real.md", "- the real page\n"),
        (
            "pages/Source.md",
            "alias:: Shared, Real, Mine\n\n- source body\n",
        ),
        ("pages/Z.md", "alias:: ML\n\n- z links [[Mine]]\n"),
        ("pages/q.md", "- [[Shared]] [[Real]] [[Mine]]\n"),
    ];
    let (root, store) = fixture("merge-owners", files);
    let dialog = alias_dialog(&store, "Source", "ML");
    let merge = dialog.merges.iter().find(|m| m.name == "Z").unwrap();
    // Shared opens A before and after; Real is a real file; only Mine moves,
    // and the survivor's own link to it is listed.
    assert_eq!(
        retargets(&merge.effect),
        [(
            "Mine".to_owned(),
            "Source".to_owned(),
            "Z".to_owned(),
            vec!["Z".to_owned(), "q".to_owned()]
        )]
    );
    let (shared, real) = (
        resolved_name(&store, "Shared"),
        resolved_name(&store, "Real"),
    );
    assert_eq!(
        rename(&store, "Source", "Z", RenameChoice::MergeInto(&merge.into)),
        RenameOutcome::Merged
    );
    // The prediction is what the merge does.
    assert_eq!(resolved_name(&store, "Shared"), shared);
    assert_eq!(resolved_name(&store, "Real"), real);
    assert_eq!(resolved_name(&store, "Mine"), "pages/Z.md");
    assert!(snapshot(&root).contains_key("pages/A.md"));
}

/// "Use the name": the typed name moves to the source, and a shared alias
/// whose least-path declarer was the source passes to another declarer when
/// the source's file moves.
#[test]
fn use_the_name_retargets_shared_aliases_that_change_owner() {
    let _guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        ("pages/Aa.md", "alias:: Shared, Kept\n\n- source body\n"),
        ("pages/B.md", "alias:: Shared, ML\n\n- b body\n"),
        ("pages/q.md", "- [[Shared]] [[Kept]] [[ML]]\n"),
    ];
    let (root, store) = fixture("own-owners", files);
    let dialog = alias_dialog(&store, "Aa", "ML");
    let own = dialog.own.as_ref().expect("use the name is offered");
    let mut rows = retargets(&own.effect);
    rows.sort();
    assert_eq!(
        rows,
        [
            (
                "ML".to_owned(),
                "B".to_owned(),
                "ML".to_owned(),
                vec!["q".to_owned()]
            ),
            (
                "Shared".to_owned(),
                "Aa".to_owned(),
                "B".to_owned(),
                vec!["q".to_owned()]
            ),
        ]
    );
    let edit = &own.edits[0];
    fs::write(
        root.join(&edit.path),
        format!(
            "{}:: {}\n\n- b body\n",
            edit.key,
            edit.after.as_deref().unwrap()
        ),
    )
    .unwrap();
    assert_eq!(
        rename(&store, "Aa", "ML", RenameChoice::Rename),
        RenameOutcome::Renamed
    );
    assert_eq!(resolved_name(&store, "Shared"), "pages/B.md");
    assert_eq!(resolved_name(&store, "Kept"), "pages/ML.md");
    assert_eq!(resolved_name(&store, "ML"), "pages/ML.md");
}

/// One request of the alias dialog: rename `old` to `typed` (only an alias),
/// answered by merging into `into` (`Some`) or by "use the name" (`None`).
struct Case {
    label: &'static str,
    files: &'static [(&'static str, &'static str)],
    old: &'static str,
    typed: &'static str,
    into: Option<&'static str>,
    /// The rows the dialog must show: (name, before, after, pages).
    rows: &'static [(
        &'static str,
        &'static str,
        &'static str,
        &'static [&'static str],
    )],
    /// (row name, a spelling links use) for every name a surviving link
    /// names, and the files the request moves (before, after).
    probes: &'static [(&'static str, &'static str)],
    moves: &'static [(&'static str, &'static str)],
}

const SOURCE_PLAIN: (&str, &str) = ("pages/Source.md", "- s\n");
const Z_ML: (&str, &str) = ("pages/Z.md", "alias:: ML\n\n- z\n");
const MINE: &[(&str, &str, &str, &[&str])] = &[("Mine", "Source", "Z", &["Z"])];
const ML_TO_Z: &[(&str, &str, &str, &[&str])] = &[("ML", "Z", "ML", &["Z"])];
const OWN_MOVES: &[(&str, &str)] = &[("pages/Source.md", "pages/ML.md")];

/// The review of 88a0ab211 (finding 1): eight executed requests whose
/// surviving links change page, which the earlier prediction missed.
const CASES: &[Case] = &[
    Case {
        label: "merge-declarer-body",
        files: &[
            ("pages/Source.md", "alias:: Mine\n\n- s\n"),
            ("pages/Z.md", "alias:: ML, Mine\n\n- z links [[Mine]]\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.md"),
        rows: MINE,
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    Case {
        label: "merge-source-body",
        files: &[
            ("pages/Source.md", "alias:: Mine\n\n- s links [[Mine]]\n"),
            Z_ML,
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.md"),
        rows: MINE,
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    Case {
        label: "own-declarer-body",
        files: &[
            SOURCE_PLAIN,
            ("pages/Z.md", "alias:: ML\n\n- z links [[ML]]\n"),
        ],
        old: "Source",
        typed: "ML",
        into: None,
        rows: ML_TO_Z,
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "own-source-body",
        files: &[("pages/Source.md", "- s links [[ML]]\n"), Z_ML],
        old: "Source",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["ML"])],
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "own-declarer-body-crlf",
        files: &[
            SOURCE_PLAIN,
            ("pages/Z.md", "alias:: ML\r\n\r\n- z links [[ML]]\r\n"),
        ],
        old: "Source",
        typed: "ML",
        into: None,
        rows: ML_TO_Z,
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "own-declarer-body-page-link",
        files: &[
            SOURCE_PLAIN,
            ("pages/Z.md", "alias:: ML\n\n- z [see]([[ML]])\n"),
        ],
        old: "Source",
        typed: "ML",
        into: None,
        rows: ML_TO_Z,
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "own-declarer-body-tag",
        files: &[SOURCE_PLAIN, ("pages/Z.md", "alias:: ML\n\n- z #ML\n")],
        old: "Source",
        typed: "ML",
        into: None,
        rows: ML_TO_Z,
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "own-declarer-body-embed",
        files: &[
            SOURCE_PLAIN,
            ("pages/Z.md", "alias:: ML\n\n- z {{embed [[ML]]}}\n"),
        ],
        old: "Source",
        typed: "ML",
        into: None,
        rows: ML_TO_Z,
        probes: &[("ML", "ML")],
        moves: OWN_MOVES,
    },
    Case {
        label: "merge-survivor-body-org",
        files: &[
            ("pages/Source.org", "#+alias: Mine\n\n* s\n"),
            ("pages/Z.org", "#+alias: ML, Mine\n\n* z links [[Mine]]\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.org"),
        rows: MINE,
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    Case {
        label: "merge-day-spelling",
        files: &[
            ("pages/Source.md", "alias:: 2026-11-18\n\n- s\n"),
            Z_ML,
            ("pages/q.md", "- [[Nov 18th, 2026]]\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.md"),
        rows: &[("2026-11-18", "Source", "Z", &["q"])],
        probes: &[("2026-11-18", "Nov 18th, 2026")],
        moves: &[],
    },
    Case {
        label: "own-moved-child",
        files: &[
            ("pages/Aa.md", "- a\n"),
            ("pages/Aa%2Fx.md", "alias:: Shared\n\n- x\n"),
            ("pages/B.md", "alias:: Shared\n\n- b\n"),
            Z_ML,
            ("pages/q.md", "- [[Shared]]\n"),
        ],
        old: "Aa",
        typed: "ML",
        into: None,
        rows: &[("Shared", "Aa/x", "B", &["q"])],
        probes: &[("Shared", "Shared")],
        moves: &[
            ("pages/Aa.md", "pages/ML.md"),
            ("pages/Aa%2Fx.md", "pages/ML%2Fx.md"),
        ],
    },
    // The review of 0a3d95eae (finding 2): "use the name" for a source with
    // no file. The typed name's declarer releases it; the rename then gives
    // it to a page with no file (its links open that page) or, through the
    // respelled alias declaration, to the alias source's declarer.
    Case {
        label: "own-fileless-reference",
        files: &[Z_ML, ("pages/q.md", "- [[X]] and [[ML]]\n")],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-namespace",
        files: &[Z_ML, ("pages/q.md", "- [[N/c]] and [[ML]]\n")],
        old: "N",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias",
        files: &[
            ("pages/W.md", "alias:: X\n\n- w\n"),
            Z_ML,
            ("pages/q.md", "- [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "W", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    // The review of 0a3d95eae (finding 3): an Org headline tag is a site the
    // rename cannot spell, in the source, the survivor and another declarer.
    Case {
        label: "merge-org-headline-tag-source",
        files: &[
            ("pages/Source.org", "#+alias: Mine\n\n* s :Mine:\n"),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.org"),
        rows: MINE,
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    Case {
        label: "merge-org-headline-tag-survivor",
        files: &[
            ("pages/Source.org", "#+alias: Mine\n\n* s\n"),
            ("pages/Z.org", "#+alias: ML, Mine\n\n* z :Mine:\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.org"),
        rows: MINE,
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    Case {
        label: "merge-org-headline-tag-declarer",
        files: &[
            ("pages/Source.org", "#+alias: Mine\n\n* s\n"),
            ("pages/Y.org", "#+alias: Mine\n\n* y :Mine:\n"),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
        ],
        old: "Source",
        typed: "ML",
        into: Some("pages/Z.org"),
        rows: &[("Mine", "Source", "Y", &["Y"])],
        probes: &[("Mine", "Mine")],
        moves: &[],
    },
    // The review of 57755b314 (finding 1): the executor leaves a declarer with
    // VCS conflict markers unchanged, so it still declares the old name and
    // the preview must not count it. Six conflicted inputs, four controls.
    Case {
        label: "own-fileless-alias-conflicted-md",
        files: &[
            (
                "pages/W.md",
                "alias:: X\n\n- w\n\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.md", "alias:: ML\n\n- z\n"),
            ("pages/q.md", "- [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-conflicted-then-clean-md",
        files: &[
            (
                "pages/W.md",
                "alias:: X\n\n- w\n\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
            ),
            ("pages/Y.md", "alias:: X\n\n- y\n"),
            ("pages/Z.md", "alias:: ML\n\n- z\n"),
            ("pages/q.md", "- [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "Y", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-both-conflicted-md",
        files: &[
            (
                "pages/W.md",
                "alias:: X\n\n- w\n\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
            ),
            (
                "pages/Y.md",
                "alias:: X\n\n- y\n\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.md", "alias:: ML\n\n- z\n"),
            ("pages/q.md", "- [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-clean-then-conflicted-md",
        files: &[
            ("pages/W.md", "alias:: X\n\n- w\n"),
            (
                "pages/Y.md",
                "alias:: X\n\n- y\n\n<<<<<<< ours\n- x\n=======\n- y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.md", "alias:: ML\n\n- z\n"),
            ("pages/q.md", "- [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "W", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-org",
        files: &[
            ("pages/W.org", "#+alias: X\n\n* w\n"),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
            ("pages/q.org", "* q [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "W", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-conflicted-org",
        files: &[
            (
                "pages/W.org",
                "#+alias: X\n\n* w\n\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
            ("pages/q.org", "* q [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-conflicted-then-clean-org",
        files: &[
            (
                "pages/W.org",
                "#+alias: X\n\n* w\n\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
            ),
            ("pages/Y.org", "#+alias: X\n\n* y\n"),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
            ("pages/q.org", "* q [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "Y", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-both-conflicted-org",
        files: &[
            (
                "pages/W.org",
                "#+alias: X\n\n* w\n\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
            ),
            (
                "pages/Y.org",
                "#+alias: X\n\n* y\n\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
            ("pages/q.org", "* q [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "ML", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
    Case {
        label: "own-fileless-alias-clean-then-conflicted-org",
        files: &[
            ("pages/W.org", "#+alias: X\n\n* w\n"),
            (
                "pages/Y.org",
                "#+alias: X\n\n* y\n\n<<<<<<< ours\n* x\n=======\n* y\n>>>>>>> theirs\n",
            ),
            ("pages/Z.org", "#+alias: ML\n\n* z\n"),
            ("pages/q.org", "* q [[ML]]\n"),
        ],
        old: "X",
        typed: "ML",
        into: None,
        rows: &[("ML", "Z", "W", &["q"])],
        probes: &[("ML", "ML")],
        moves: &[],
    },
];

/// The effect the dialog shows for the case's request.
fn case_effect(store: &Store, case: &Case) -> tine_graph_features::pages::Effect {
    let dialog = alias_dialog(store, case.old, case.typed);
    match case.into {
        Some(into) => {
            let merge = dialog.merges.iter().find(|m| m.into == into);
            merge
                .unwrap_or_else(|| panic!("{}: {dialog:?}", case.label))
                .effect
                .clone()
        }
        None => {
            dialog
                .own
                .unwrap_or_else(|| panic!("{}", case.label))
                .effect
        }
    }
}

/// D-20261106-R13-UI: every surviving link that opens another page is
/// shown, wherever it is (a declarer's or the survivor's body, the source's
/// own body, a day's other spelling, a moved child's alias).
#[test]
fn every_surviving_link_that_opens_another_page_is_listed() {
    let _guard = crate::integration_lock();
    let mut wrong = Vec::new();
    for case in CASES {
        let (_root, store) = fixture(case.label, case.files);
        let expected: Vec<_> = (case.rows.iter())
            .map(|(name, before, after, pages)| {
                let pages = pages.iter().map(|page| page.to_string()).collect();
                (
                    name.to_string(),
                    before.to_string(),
                    after.to_string(),
                    pages,
                )
            })
            .collect();
        let shown = retargets(&case_effect(&store, case));
        if shown != expected {
            wrong.push((case.label, shown, expected));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The file and name `spelling` opens; a name no file claims or declares
/// opens its page with no file, shown by the name.
fn page_at(graph: &tine_store::WholeGraph, spelling: &str) -> (String, String) {
    let id = match graph.resolve(spelling, false) {
        tine_store::Resolved::Existing { id, .. } => id,
        tine_store::Resolved::Alias { owners } => owners[0].clone(),
        tine_store::Resolved::Absent { .. } => return (String::new(), spelling.to_owned()),
    };
    let name = graph.inventory(tine_store::InventoryScope::File(&id)).0[0]
        .name
        .clone();
    (id.as_str().to_owned(), name)
}

/// Apply the dialog's alias edit the way the window's property write does.
fn release(root: &Path, edit: &tine_graph_features::pages::AliasEdit) {
    let path = root.join(&edit.path);
    let text = fs::read_to_string(&path).unwrap();
    let org = edit.path.ends_with(".org");
    let line = |value: &str| match org {
        true => format!("#+{}: {value}", edit.key),
        false => format!("{}:: {value}", edit.key),
    };
    let old = line(&edit.before);
    let text = match &edit.after {
        Some(after) => text.replacen(&old, &line(after), 1),
        None => {
            let at = text.find(&old).unwrap();
            let end = text[at..].find('\n').map_or(text.len(), |n| at + n + 1);
            format!("{}{}", &text[..at], &text[end..])
        }
    };
    fs::write(path, text).unwrap();
}

/// The rows are the store's own answer: executing each request and asking
/// `resolve` before and after changes exactly the rows' names, from the
/// rows' page to the rows' page (a moved file is the same page).
#[test]
fn each_cases_rows_are_the_executed_resolution_diff() {
    let _guard = crate::integration_lock();
    let mut wrong = Vec::new();
    for case in CASES {
        let (root, store) = fixture(&format!("{}-run", case.label), case.files);
        let predicted: Vec<_> = (retargets(&case_effect(&store, case)).into_iter())
            .map(|(name, before, after, _)| (name, before, after))
            .collect();
        let before = store.whole_graph().unwrap();
        let was: Vec<_> = case
            .probes
            .iter()
            .map(|(_, s)| page_at(&before, s))
            .collect();
        let outcome = match case.into {
            Some(into) => {
                let dialog = alias_dialog(&store, case.old, case.typed);
                let name = &dialog.merges.iter().find(|m| m.into == into).unwrap().name;
                rename(&store, case.old, name, RenameChoice::MergeInto(into))
                    == RenameOutcome::Merged
            }
            None => {
                for edit in &alias_dialog(&store, case.old, case.typed)
                    .own
                    .unwrap()
                    .edits
                {
                    release(&root, edit);
                }
                rename(&store, case.old, case.typed, RenameChoice::Rename) == RenameOutcome::Renamed
            }
        };
        assert!(outcome, "{}", case.label);
        let after = store.whole_graph().unwrap();
        let executed: Vec<_> = (case.probes.iter().zip(was))
            .filter_map(|((label, spelling), (was_path, was_name))| {
                let (path, name) = page_at(&after, spelling);
                let moved = case.moves.iter().find(|(from, _)| *from == was_path);
                let same = moved.map_or(was_path.as_str(), |(_, to)| to) == path;
                (!same).then(|| (label.to_string(), was_name, name))
            })
            .collect();
        if predicted != executed {
            wrong.push((case.label, predicted, executed));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
