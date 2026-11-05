//! Conceptual model step 2, batch 12: rename and merge follow model round 9
//! (divergences R4, R10, R11, R12, R14 in
//! `tine-agents/subagent-tasks/notes/cm-conformance-divergences.md`). One
//! test per rule, each asserting the user-visible outcome on disk and in the
//! graph, through the app's own rename question (`pages::rename_offer`, the
//! planner's answer) and its answer (`pages::rename_page_as`).
//!
//! - R4 / M-Q5 (D-20261102-MC5): a name that is only an alias offers a merge
//!   into its owner AND "make it its own page" (still an alias of the owner).
//! - R10 / M-Q7: a journal day is the claimant of its day under every date
//!   spelling, so a rename onto a day offers a merge into the journal.
//! - R11 / ruling (a): a merge through an alias gives query arguments the
//!   survivor's own name; links and child names keep the typed name (b).
//! - R12 / M-Q6, CM-OWN-CHILD (D-20261103-CMA): no rename or merge onto a
//!   used name inside the page's own namespace; a merely linked one is free.
//! - R14: a case-only rename onto a file another page holds refuses with
//!   that reason (not a retry hint) and writes nothing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_graph_features::pages::{
    rename_offer, rename_page_as, RenameBlocked, RenameChoice, RenameOffer, RenameOutcome,
};
use tine_store::{Resolved, Store};

fn fixture(label: &str, files: &[(&str, &str)]) -> (PathBuf, Arc<Store>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-cm-merge-rules-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        fs::write(root.join(rel), body).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

/// Every page and journal file under the graph, path → bytes.
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

fn rename(
    store: &Arc<Store>,
    old: &str,
    new: &str,
    choice: RenameChoice<'_>,
) -> std::io::Result<RenameOutcome> {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    rename_page_as(store, &host, old, new, None, choice, &[]).map(|report| report.outcome)
}

fn resolve(store: &Store, name: &str) -> Resolved {
    store.whole_graph().unwrap().resolve(name, false)
}

fn backlink_pages(store: &Store, name: &str) -> Vec<String> {
    let view = store.whole_graph().unwrap();
    let mut pages: Vec<String> = view
        .backlinks(name)
        .unwrap()
        .iter()
        .map(|group| group.page.clone())
        .collect();
    pages.sort();
    pages
}

const ML: &[(&str, &str)] = &[
    ("pages/Machine Learning.md", "alias:: ML\n\n- ml\n"),
    ("pages/ML notes.md", "- notes\n"),
    ("pages/ML notes%2Fx.md", "- child\n"),
    (
        "pages/q.md",
        "- {{query (page \"ML notes\")}}\n- see [[ML notes]]\n- see [[ML notes/x]]\n",
    ),
];

/// CM-R9-EMPTYDIALOG: an empty alias target must offer the owner, retain the
/// empty file on merge, and fill that exact file on OwnPage.
#[test]
fn empty_alias_target_dialog_and_both_answers() {
    let _guard = crate::integration_lock();
    for own in [false, true] {
        let mut files = ML.to_vec();
        files.push(("pages/ML.md", "- \n"));
        let (root, store) = fixture("empty-target", &files);
        let offer =
            serde_json::to_value(rename_offer(&store, "ML notes", "ML", None).unwrap()).unwrap();
        assert_eq!(offer["kind"], "aliasOf", "{offer}");
        assert_eq!(offer["owner"], "pages/Machine Learning.md");
        assert_eq!(offer["ownPage"], true);
        let before = snapshot(&root);
        assert!(rename(&store, "ML notes", "ML", RenameChoice::Rename).is_err());
        assert_eq!(snapshot(&root), before, "no answer writes nothing");
        rename(
            &store,
            "ML notes",
            "ML",
            if own {
                RenameChoice::OwnPage
            } else {
                RenameChoice::MergeInto("pages/Machine Learning.md")
            },
        )
        .unwrap();
        let after = snapshot(&root);
        assert!(!after.contains_key("pages/ML notes.md"));
        assert!(after["pages/ML%2Fx.md"].contains("child"));
        assert!(after["pages/Machine Learning.md"].contains("alias:: ML"));
        if own {
            assert!(after["pages/ML.md"].contains("notes"));
            assert!(after["pages/q.md"].contains("(page \"ML\")"));
        } else {
            assert_eq!(after["pages/ML.md"], "- \n");
            assert!(after["pages/Machine Learning.md"].contains("notes"));
            assert!(after["pages/q.md"].contains("(page \"Machine Learning\")"));
        }
        assert!(after["pages/q.md"].contains("see [[ML]]"));
        assert!(backlink_pages(&store, "Machine Learning").contains(&"q".to_owned()));
    }
}

/// R10-Q4: the API rejects OwnPage for a file-less source, while a real
/// content-free source still gets both answers.
#[test]
fn fileless_alias_offer_rejects_own_page() {
    let _guard = crate::integration_lock();
    for empty_target in [false, true] {
        let mut files = vec![
            ("pages/owner.md", "alias:: ML\n\n- owner\n"),
            ("pages/refs.md", "- [[B]]\n"),
        ];
        if empty_target {
            files.push(("pages/ML.md", ""));
        }
        let (root, store) = fixture("fileless-alias", &files);
        let offer = serde_json::to_value(rename_offer(&store, "B", "ML", None).unwrap()).unwrap();
        assert_eq!(offer["kind"], "aliasOf");
        assert_eq!(offer["merge"], true);
        assert_eq!(offer["ownPage"], false);
        let before = snapshot(&root);
        assert!(rename(&store, "B", "ML", RenameChoice::OwnPage).is_err());
        assert_eq!(snapshot(&root), before);
        rename(&store, "B", "ML", RenameChoice::MergeInto("pages/owner.md")).unwrap();
        assert!(snapshot(&root)["pages/refs.md"].contains("[[ML]]"));
    }
    let (root, store) = fixture(
        "empty-source",
        &[
            ("pages/B.md", ""),
            ("pages/owner.md", "alias:: ML\n\n- owner\n"),
        ],
    );
    let offer = serde_json::to_value(rename_offer(&store, "B", "ML", None).unwrap()).unwrap();
    assert_eq!(offer["ownPage"], true);
    rename(&store, "B", "ML", RenameChoice::OwnPage).unwrap();
    assert!(snapshot(&root).contains_key("pages/ML.md"));
}

#[test]
fn own_alias_offers_only_own_page_and_conflicted_empty_target_refuses() {
    let _guard = crate::integration_lock();
    let (_, store) = fixture(
        "self-alias",
        &[("pages/source.md", "alias:: Short\n\n- body\n")],
    );
    assert!(matches!(
        rename_offer(&store, "source", "Short", None).unwrap(),
        RenameOffer::AliasOf {
            merge: false,
            own_page: true,
            ..
        }
    ));
    assert!(rename(&store, "source", "Short", RenameChoice::Rename).is_err());
    rename(&store, "source", "Short", RenameChoice::OwnPage).unwrap();
    let (root, store) = fixture(
        "empty-twins",
        &[
            ("pages/source.md", "- body\n"),
            ("pages/owner.md", "alias:: ML\n\n- owner\n"),
            ("pages/ML.md", ""),
            ("pages/ML.org", ""),
        ],
    );
    let before = snapshot(&root);
    assert!(rename_offer(&store, "source", "ML", None).is_err());
    assert!(rename(&store, "source", "ML", RenameChoice::OwnPage).is_err());
    assert_eq!(snapshot(&root), before);
}

/// R4 / M-Q5: renaming onto an alias-only name offers both answers; "make it
/// its own page" leaves `ML.md` a real page while `ML` stays an alias of
/// Machine Learning, so `[[ML]]` still shows in its Linked References.
#[test]
fn alias_only_name_offers_merge_and_own_page() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture("own-page", ML);
    assert_eq!(
        rename_offer(&store, "ML notes", "ML", None).unwrap(),
        RenameOffer::AliasOf {
            owner: "pages/Machine Learning.md".into(),
            name: "Machine Learning".into(),
            merge: true,
            own_page: true,
        }
    );
    // Without an answer, the alias-only name is still not taken silently.
    let held = rename(&store, "ML notes", "ML", RenameChoice::Rename).unwrap_err();
    assert!(matches!(
        RenameBlocked::of(&held),
        Some(RenameBlocked::AliasOf { merge: true, .. })
    ));
    assert_eq!(
        rename(&store, "ML notes", "ML", RenameChoice::OwnPage).unwrap(),
        RenameOutcome::Renamed
    );
    let files = snapshot(&root);
    assert_eq!(
        files.get("pages/ML.md").map(String::as_str),
        Some("- notes\n")
    );
    assert!(!files.contains_key("pages/ML notes.md"));
    assert_eq!(
        files["pages/Machine Learning.md"], "alias:: ML\n\n- ml\n",
        "the owner keeps its alias"
    );
    assert!(
        files["pages/q.md"].contains("see [[ML]]"),
        "{}",
        files["pages/q.md"]
    );
    match resolve(&store, "ML") {
        Resolved::Existing { id, .. } => assert_eq!(id.as_str(), "pages/ML.md"),
        other => panic!("ML is its own page now: {other:?}"),
    }
    assert!(
        backlink_pages(&store, "Machine Learning").contains(&"q".to_owned()),
        "[[ML]] still counts for Machine Learning (ML is its alias)"
    );
}

/// R11 / rulings (a)(b): the merge answer through the alias. Query arguments
/// take the survivor's own name; links keep the typed name; children move to
/// the typed name's namespace.
#[test]
fn merge_through_alias_names_the_survivor_in_queries() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture("through-alias", ML);
    assert_eq!(
        rename(
            &store,
            "ML notes",
            "ML",
            RenameChoice::MergeInto("pages/Machine Learning.md")
        )
        .unwrap(),
        RenameOutcome::Merged
    );
    let files = snapshot(&root);
    let q = &files["pages/q.md"];
    assert!(q.contains("(page \"Machine Learning\")"), "{q}");
    assert!(q.contains("see [[ML]]"), "{q}");
    assert!(q.contains("see [[ML/x]]"), "{q}");
    assert!(!files.contains_key("pages/ML notes.md"));
    assert!(files["pages/Machine Learning.md"].contains("- notes"));
    assert!(matches!(resolve(&store, "ML/x"), Resolved::Existing { .. }));
}

/// R10 / M-Q7: a journal day is the claimant of `2026-10-26` (a date spelling
/// that is not the journal's own title), so the rename offers a merge into
/// the journal, and the merge lands there.
#[test]
fn rename_onto_a_day_offers_a_merge_into_its_journal() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "journal",
        &[
            ("journals/2026_10_26.md", "- day\n"),
            ("pages/meeting.md", "- notes\n"),
            (
                "pages/p.md",
                "- see [[meeting]]\n- {{query (page \"meeting\")}}\n",
            ),
        ],
    );
    let offer = rename_offer(&store, "meeting", "2026-10-26", None).unwrap();
    let RenameOffer::Merge { into, journal, .. } = &offer else {
        panic!("a merge into the journal is offered: {offer:?}");
    };
    assert_eq!((into.as_str(), *journal), ("journals/2026_10_26.md", true));
    // The unanswered rename does not quietly create a second claimant.
    assert!(rename(&store, "meeting", "2026-10-26", RenameChoice::Rename).is_err());
    assert!(!snapshot(&root).contains_key("pages/2026-10-26.md"));
    assert_eq!(
        rename(
            &store,
            "meeting",
            "2026-10-26",
            RenameChoice::MergeInto(into)
        )
        .unwrap(),
        RenameOutcome::Merged
    );
    let files = snapshot(&root);
    assert!(files["journals/2026_10_26.md"].contains("- notes"));
    assert!(!files.contains_key("pages/meeting.md"));
    assert!(!files.contains_key("pages/2026-10-26.md"));
}

/// R12 / M-Q6, CM-OWN-CHILD: the own namespace child is refused when used
/// (a file claims it, or a page, the renamed one included, declares it as an
/// alias), with the reason, whatever the answer; nothing is written. A
/// merely linked child name is free.
#[test]
fn own_namespace_child_refuses_when_used() {
    let _guard = crate::integration_lock();
    let cases: &[(&str, &[(&str, &str)], &str, &str)] = &[
        (
            "page",
            &[
                ("pages/Foo.md", "- foo\n"),
                ("pages/Foo%2Fbar.md", "- bar\n"),
                ("pages/p.md", "- see [[Foo]] and [[Foo/bar]]\n"),
            ],
            "Foo",
            "Foo/bar",
        ),
        (
            "alias",
            &[
                ("pages/a.md", "- ay\n"),
                ("pages/o.md", "alias:: a/x\n\n- oh\n"),
                ("pages/p.md", "- see [[a]] and [[a/x]]\n"),
            ],
            "a",
            "a/x",
        ),
        (
            "own-alias",
            &[
                ("pages/Foo.md", "alias:: Foo/bar\n\n- foo\n"),
                ("pages/p.md", "- see [[Foo]]\n"),
            ],
            "Foo",
            "Foo/bar",
        ),
    ];
    for (label, files, old, new) in cases {
        let (root, store) = fixture(label, files);
        let before = snapshot(&root);
        let offer = rename_offer(&store, old, new, None).unwrap();
        let RenameOffer::Refused { reason, message } = &offer else {
            panic!("{label}: refused, got {offer:?}");
        };
        assert_eq!(
            reason,
            &RenameBlocked::OwnChild {
                old: (*old).into(),
                new: (*new).into()
            },
            "{label}"
        );
        assert!(message.contains("inside"), "{label}: {message}");
        for choice in [RenameChoice::Rename, RenameChoice::OwnPage] {
            let error = rename(&store, old, new, choice).unwrap_err();
            assert!(
                matches!(
                    RenameBlocked::of(&error),
                    Some(RenameBlocked::OwnChild { .. })
                ),
                "{label}: {error}"
            );
        }
        let survivor = match resolve(&store, new) {
            Resolved::Existing { id, .. } => id.as_str().to_owned(),
            Resolved::Alias { owners } => owners[0].as_str().to_owned(),
            other => panic!("{label}: {other:?}"),
        };
        let error = rename(&store, old, new, RenameChoice::MergeInto(&survivor)).unwrap_err();
        assert!(
            matches!(
                RenameBlocked::of(&error),
                Some(RenameBlocked::OwnChild { .. })
            ),
            "{label}: the merge is refused too: {error}"
        );
        assert_eq!(snapshot(&root), before, "{label}: nothing written");
    }
    // Merely linked: unused, a plain rename.
    let (root, store) = fixture(
        "unused",
        &[
            ("pages/Foo.md", "- foo\n"),
            ("pages/p.md", "- see [[Foo]] and [[Foo/bar]]\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "Foo", "Foo/bar", None).unwrap(),
        RenameOffer::Rename
    );
    assert_eq!(
        rename(&store, "Foo", "Foo/bar", RenameChoice::Rename).unwrap(),
        RenameOutcome::Renamed
    );
    assert!(!snapshot(&root).contains_key("pages/Foo.md"));
}

/// S15 (model round 10, OG `namespace-page?`): `./x` and `https://a.b/c` are
/// not namespace names, so they are no children of the pages `.` and `https:`.
/// Renaming `.` leaves the page `./x` and its links alone (OG
/// `rename-namespace-pages!` selects children by `:block/namespace`, never set
/// for `./x`), and renaming `.` onto `./x` is a merge into that page, not a
/// refusal as "inside its own namespace". The same for a URL name.
#[test]
fn relative_and_url_names_are_no_namespace_children() {
    let _guard = crate::integration_lock();
    let files: &[(&str, &str)] = &[
        ("pages/dot.md", "title:: .\n\n- dot\n"),
        ("pages/dotx.md", "title:: ./x\n\n- dot x\n"),
        ("pages/scheme.md", "title:: https:\n\n- scheme\n"),
        ("pages/url.md", "title:: https://a.b/c\n\n- url\n"),
        (
            "pages/p.md",
            "- see [[.]] and [[./x]]\n- see [[https:]] and [[https://a.b/c]]\n",
        ),
    ];
    for (old, child, child_path, new) in [
        (".", "./x", "pages/dotx.md", "z"),
        ("https:", "https://a.b/c", "pages/url.md", "w"),
    ] {
        // Onto the other page: a merge is offered, not an own-child refusal.
        let (_root, store) = fixture("relative-offer", files);
        assert_eq!(
            rename_offer(&store, old, child, None).unwrap(),
            RenameOffer::Merge {
                into: child_path.into(),
                name: child.into(),
                journal: false,
            },
            "{old} -> {child}"
        );
        // A plain rename moves `old` only.
        let (root, store) = fixture("relative-rename", files);
        let before = snapshot(&root);
        assert_eq!(
            rename(&store, old, new, RenameChoice::Rename).unwrap(),
            RenameOutcome::Renamed,
            "{old} -> {new}"
        );
        let after = snapshot(&root);
        assert_eq!(
            after.get(child_path),
            before.get(child_path),
            "{old} -> {new}: {child} is no child of {old}"
        );
        assert!(
            after["pages/p.md"].contains(&format!("[[{child}]]")),
            "{old} -> {new}: {}",
            after["pages/p.md"]
        );
        assert!(
            after["pages/p.md"].contains(&format!("[[{new}]]")),
            "{old} -> {new}: {}",
            after["pages/p.md"]
        );
        match resolve(&store, child) {
            Resolved::Existing { id, .. } => assert_eq!(id.as_str(), child_path),
            other => panic!("{old} -> {new}: {child} keeps its page: {other:?}"),
        }
    }
}

/// R14: the case-only destination file `pages/n%2Fy.md` is page B's. The
/// refusal names that (a retry cannot help) and every file is unchanged.
#[test]
fn case_only_rename_onto_another_pages_file_refuses_with_the_reason() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "case-taken",
        &[
            ("pages/N%2Fy.md", "- upper\n"),
            ("pages/n%2Fy.md", "title:: B\n\n- lower\n"),
            ("pages/p.md", "- see [[N/y]] and [[B]]\n"),
        ],
    );
    let before = snapshot(&root);
    let offer = rename_offer(&store, "N/y", "n/y", None).unwrap();
    assert_eq!(
        offer,
        RenameOffer::Refused {
            reason: RenameBlocked::FileTaken {
                file: "pages/n%2Fy.md".into(),
                page: "B".into()
            },
            message: "cannot rename: the file pages/n%2Fy.md belongs to the page “B”; rename or merge that page first".into(),
        }
    );
    let error = rename(&store, "N/y", "n/y", RenameChoice::Rename).unwrap_err();
    assert_ne!(error.kind(), std::io::ErrorKind::WouldBlock, "{error}");
    assert!(matches!(
        RenameBlocked::of(&error),
        Some(RenameBlocked::FileTaken { .. })
    ));
    assert_eq!(snapshot(&root), before, "nothing written");
}
