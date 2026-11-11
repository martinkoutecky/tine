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
//! - B4 / accepted SPEC §5.3: a merge through an alias substitutes the
//!   selected member's spelling in every site and namespace child.
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

#[test]
fn journal_reference_loss_refuses_every_source_and_answer_without_writing() {
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
            let before = snapshot(&root);
            let offer = rename_offer(&store, "old", "new", None).unwrap();
            assert!(
                matches!(offer, RenameOffer::Refused { .. }),
                "{source}/{target}: {offer:?}"
            );
            let wire = serde_json::to_value(&offer).unwrap();
            assert_eq!(wire["reason"]["kind"], "journalReferenceLoss");
            assert_eq!(
                wire["reason"]["journals"],
                serde_json::json!(["Oct 26th, 2026"])
            );
            assert!(wire["message"].as_str().unwrap().contains("Oct 26th, 2026"));
            for choice in [
                RenameChoice::Rename,
                RenameChoice::OwnPage,
                RenameChoice::MergeInto(if target == "page" {
                    "pages/new.md"
                } else {
                    "pages/target.md"
                }),
            ] {
                let error = rename(&store, "old", "new", choice).unwrap_err();
                assert!(
                    RenameBlocked::of(&error).is_some(),
                    "{source}/{target}: {error}"
                );
                assert_eq!(
                    snapshot(&root),
                    before,
                    "{source}/{target}: refusal wrote files"
                );
            }
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn journal_reference_loss_checks_each_owner_occurrence_and_direct_ownership() {
    let _guard = crate::integration_lock();
    for (label, journal_name, referring, old, new, blocked) in [
        ("unused", "ns/yyyy-MM-dd", "- [[ns]]\n", "ns", "new", false),
        (
            "retained-prefix",
            "ns",
            "- [[ns/sub]]\n",
            "ns/sub",
            "ns/other",
            false,
        ),
        (
            "lost-prefix-outside-subtree",
            "ns",
            "- [[ns/sub]]\n",
            "ns/sub",
            "new",
            true,
        ),
        (
            "per-occurrence",
            "ns",
            "- [[ns/sub]] [[ns]]\n",
            "ns/sub",
            "new",
            true,
        ),
        (
            "fixed-marker",
            "TODO",
            "- TODO fixed\n",
            "TODO",
            "new",
            false,
        ),
        ("fixed-priority", "A", "- [#A] fixed\n", "A", "new", false),
        (
            "query-only",
            "ns/yyyy-MM-dd",
            "- {{query (page ns/2026-10-26)}}\n",
            "ns",
            "new",
            false,
        ),
    ] {
        let config = format!("{{:journal/page-title-format \"{journal_name}\"}}\n");
        let (root, store) = fixture(
            label,
            &[
                ("journals/2026_10_26.md", "- day\n"),
                ("pages/p.md", referring),
                ("logseq/config.edn", &config),
            ],
        );
        let offer = rename_offer(&store, old, new, None).unwrap();
        assert_eq!(
            matches!(offer, RenameOffer::Refused { .. }),
            blocked,
            "{label}: {offer:?}"
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    // The owner file is p.md; an ignored twin's occurrence must not refuse.
    let (root, store) = fixture(
        "non-owner",
        &[
            ("journals/2026_10_26.md", "- day\n"),
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"ns/yyyy-MM-dd\"}\n",
            ),
            ("pages/p.md", "- owner\n"),
            ("pages/twin.md", "title:: p\n\n- [[ns/2026-10-26]]\n"),
            ("pages/q.md", "- [[ns]]\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "ns", "new", None).unwrap(),
        RenameOffer::Rename
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn journal_reference_loss_includes_headers_but_not_transitive_alias_ownership() {
    let _guard = crate::integration_lock();
    for files in [
        vec![("journals/2026_10_26.md", "alias:: old\n\n- day\n")],
        vec![("journals/2026_10_26.org", "#+ALIASES: old\n\n* day\n")],
        vec![("journals/2026_10_26.md", "- aliases:: old\n- day\n")],
    ] {
        let (root, store) = fixture("declaration-only", &files);
        assert!(matches!(
            rename_offer(&store, "old", "new", None).unwrap(),
            RenameOffer::Refused { .. }
        ));
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    let (root, store) = fixture(
        "transitive-alias",
        &[
            ("journals/2026_10_26.md", "alias:: stable\n\n- day\n"),
            ("pages/owner.md", "alias:: stable, old/sub\n\n- owner\n"),
            ("pages/p.md", "- [[old/sub]]\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "old", "new", None).unwrap(),
        RenameOffer::Rename
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
    for (disabled, expected) in [(false, true), (true, false)] {
        let config = format!(
            "{{:journal/page-title-format \"ns\" :property-pages/enabled? {}}}\n",
            !disabled
        );
        let (root, store) = fixture(
            "property-gate",
            &[
                ("journals/2026_10_26.md", "- day\n"),
                ("pages/p.md", "- ns:: value\n"),
                ("logseq/config.edn", &config),
            ],
        );
        assert_eq!(
            matches!(
                rename_offer(&store, "ns", "new", None).unwrap(),
                RenameOffer::Refused { .. }
            ),
            expected
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
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

/// CM12/CM13: an alias-only source changes every declarer, including files
/// with no ordinary link. Reload must retain the renamed alias group.
#[test]
fn alias_source_rewrites_all_declarers_and_survives_reload() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "alias-source",
        &[
            ("pages/owner.md", "alias:: ML\n\n- owner\n"),
            ("pages/other.md", "aliases:: ML, keep\n\n- other\n"),
            ("pages/sub.md", "alias:: ML/sub，[[ML/other]]\n\n- sub\n"),
            ("pages/q.md", "- [[ML]] [[ML]]\n- {{query (page \"ML\")}}\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "ML", "AI", None).unwrap(),
        RenameOffer::Rename
    );
    assert!(rename(&store, "ML", "AI", RenameChoice::OwnPage).is_err());
    rename(&store, "ML", "AI", RenameChoice::Rename).unwrap();
    assert_eq!(snapshot(&root)["pages/owner.md"], "alias:: AI\n\n- owner\n");
    assert_eq!(
        snapshot(&root)["pages/other.md"],
        "aliases:: AI, keep\n\n- other\n"
    );
    assert_eq!(
        snapshot(&root)["pages/sub.md"],
        "alias:: AI/sub，[[AI/other]]\n\n- sub\n"
    );
    assert!(!snapshot(&root).contains_key("pages/AI.md"));
    store.close();
    let reloaded = Store::open(&root, Default::default()).unwrap().0;
    assert!(matches!(resolve(&reloaded, "AI"), Resolved::Alias { owners } if owners.len() == 2));
    assert!(!matches!(resolve(&reloaded, "ML"), Resolved::Alias { .. }));
    assert!(backlink_pages(&reloaded, "owner").contains(&"q".to_owned()));
}

/// C6/C7/C13: no fake OwnPage, the shared empty-target owner selection,
/// survivor query name, and same-key respelling through the real API.
#[test]
fn alias_source_empty_target_and_same_key() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "alias-empty-target",
        &[
            ("pages/owner.md", "alias:: ML\n\n- owner\n"),
            ("pages/target.md", "alias:: AI\n\n- target\n"),
            ("pages/AI.md", ""),
            ("pages/q.md", "- [[ML]]\n- {{query (page \"ML\")}}\n"),
        ],
    );
    assert!(
        matches!(rename_offer(&store, "ML", "AI", None).unwrap(), RenameOffer::Merge {
        into, ..
    } if into == "pages/AI.md")
    );
    assert!(rename(&store, "ML", "AI", RenameChoice::OwnPage).is_err());
    rename(&store, "ML", "AI", RenameChoice::MergeInto("pages/AI.md")).unwrap();
    let after = snapshot(&root);
    assert_eq!(after["pages/AI.md"], "");
    assert_eq!(after["pages/owner.md"], "alias:: AI\n\n- owner\n");
    assert_eq!(after["pages/q.md"], "- [[AI]]\n- {{query (page \"AI\")}}\n");
    assert_eq!(after["pages/target.md"], "alias:: AI\n\n- target\n");
    assert_eq!(
        rename_offer(&store, "AI", "ai", None).unwrap(),
        RenameOffer::Rename
    );
    // This source now has an actual empty file; exercise alias-only respelling
    // on a separate source, where no owner choice can resolve the source tie.
    let (root, store) = fixture(
        "alias-respell",
        &[
            ("pages/a.md", "alias:: ML\n"),
            ("pages/b.md", "alias:: ML\n"),
        ],
    );
    assert_eq!(
        rename_offer(&store, "ML", "ml", None).unwrap(),
        RenameOffer::Rename
    );
    rename(&store, "ML", "ml", RenameChoice::Rename).unwrap();
    assert_eq!(snapshot(&root)["pages/a.md"], "alias:: ml\n");
    assert_eq!(snapshot(&root)["pages/b.md"], "alias:: ml\n");
}

/// R13-C4: an empty real target receives content through the guarded merge.
#[test]
fn empty_alias_target_dialog_and_both_answers() {
    let _guard = crate::integration_lock();
    for text in ["", "- \n", "- existing\n"] {
        let mut files = ML.to_vec();
        files.push(("pages/ML.md", text));
        let (root, store) = fixture("empty-target", &files);
        let offer =
            serde_json::to_value(rename_offer(&store, "ML notes", "ML", None).unwrap()).unwrap();
        assert_eq!(offer["kind"], "merge", "{offer}");
        assert_eq!(offer["into"], "pages/ML.md");
        let before = snapshot(&root);
        assert!(rename(&store, "ML notes", "ML", RenameChoice::Rename).is_err());
        assert_eq!(snapshot(&root), before, "no answer writes nothing");
        rename(
            &store,
            "ML notes",
            "ML",
            RenameChoice::MergeInto("pages/ML.md"),
        )
        .unwrap();
        let after = snapshot(&root);
        assert!(!after.contains_key("pages/ML notes.md"));
        assert!(after["pages/ML%2Fx.md"].contains("child"));
        assert!(after["pages/Machine Learning.md"].contains("alias:: ML"));
        assert!(after["pages/ML.md"].contains("notes"));
        assert!(!after["pages/Machine Learning.md"].contains("notes"));
        assert!(after["pages/q.md"].contains("(page \"ML\")"));
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
        assert_eq!(
            offer["kind"],
            if empty_target { "merge" } else { "aliasOf" }
        );
        if !empty_target {
            assert_eq!(offer["merge"], true);
            assert_eq!(offer["ownPage"], false);
        }
        let before = snapshot(&root);
        assert!(rename(&store, "B", "ML", RenameChoice::OwnPage).is_err());
        assert_eq!(snapshot(&root), before);
        rename(
            &store,
            "B",
            "ML",
            RenameChoice::MergeInto(if empty_target {
                "pages/ML.md"
            } else {
                "pages/owner.md"
            }),
        )
        .unwrap();
        assert_eq!(
            snapshot(&root)["pages/refs.md"],
            if empty_target {
                "- [[ML]]\n"
            } else {
                "- [[owner]]\n"
            }
        );
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

/// B4: the merge answer through an alias uses the selected member in
/// queries, links and namespace children.
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
    assert!(q.contains("see [[Machine Learning]]"), "{q}");
    assert!(q.contains("see [[Machine Learning/x]]"), "{q}");
    assert!(!files.contains_key("pages/ML notes.md"));
    assert!(files["pages/Machine Learning.md"].contains("- notes"));
    assert!(matches!(
        resolve(&store, "Machine Learning/x"),
        Resolved::Existing { .. }
    ));
}

// CM B4 / SPEC §5.3: the selected member supplies one substitution at
// every site, including fileless sources and namespace query arguments.
fn selected_spelling_oracle(label: &str, files: &BTreeMap<String, String>) {
    if let Ok(output) = std::env::var("CM_MERGE_ORACLE") {
        fs::write(
            Path::new(&output).join(format!("selected-{label}.json")),
            serde_json::to_string_pretty(files).unwrap(),
        )
        .unwrap();
    }
}

fn selected_spelling_save(store: &Arc<Store>, root: &Path, files: &BTreeMap<String, String>) {
    let host = crate::test_host(store);
    for (path, bytes) in files {
        let id = tine_core::model::PageId::from(path.as_str());
        let page = store.page(&id).unwrap();
        crate::test_host_writer::save_on(
            &host,
            store,
            tine_store::EditKind::SaveBlock,
            &id,
            tine_store::SaveBase::Existing(page.rev),
            &page.doc,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), *bytes);
    }
}

fn selected_spelling_at(site: &str) {
    let _guard = crate::integration_lock();
    for org in [false, true] {
        for fileless in [false, true] {
            let ext = if org { "org" } else { "md" };
            let (target, referrer, child, source) = if org {
                (
                    "#+alias: ML\n* target\n",
                    "#+alias: B\n* [[B]] #B [[B/sub]] #B/sub\n* {{query (page \"B\")}}\n* {{query (page \"B/sub\")}}\n",
                    "#+TITLE: B/sub\n* child\n",
                    "* source [[B]]\n",
                )
            } else {
                (
                    "alias:: ML\n- target\n",
                    "alias:: B\n- [[B]] #B [[B/sub]] #B/sub\n- {{query (page \"B\")}}\n- {{query (page \"B/sub\")}}\n",
                    "title:: B/sub\n- child\n",
                    "- source [[B]]\n",
                )
            };
            let target_path = format!("pages/Machine Learning.{ext}");
            let referrer_path = format!("pages/referrer.{ext}");
            let child_path = format!("pages/child.{ext}");
            let source_path = format!("pages/B.{ext}");
            let mut files = vec![
                (target_path.as_str(), target),
                (referrer_path.as_str(), referrer),
                (child_path.as_str(), child),
            ];
            if !fileless {
                files.push((source_path.as_str(), source));
            }
            let (root, store) = fixture("selected-spelling", &files);
            rename(&store, "B", "ML", RenameChoice::MergeInto(&target_path)).unwrap();
            let files = snapshot(&root);
            selected_spelling_oracle(&format!("{site}-{org}-{fileless}"), &files);
            let text = &files[&referrer_path];
            match site {
                "aliases" => assert!(
                    text.contains("alias: Machine Learning")
                        || text.contains("alias:: Machine Learning"),
                    "{text}"
                ),
                "links" => {
                    for expected in [
                        "[[Machine Learning]]",
                        "#[[Machine Learning]]",
                        "[[Machine Learning/sub]]",
                        "#[[Machine Learning/sub]]",
                    ] {
                        assert!(text.contains(expected), "missing {expected}: {text}");
                    }
                }
                "queries" => {
                    for expected in [
                        "(page \"Machine Learning\")",
                        "(page \"Machine Learning/sub\")",
                    ] {
                        assert!(text.contains(expected), "missing {expected}: {text}");
                    }
                }
                "namespace" => {
                    let view = store.whole_graph().unwrap();
                    let Resolved::Existing { id, .. } = view.resolve("Machine Learning/sub", false)
                    else {
                        panic!("the child must move under the selected member: {files:?}");
                    };
                    assert_eq!(store.page(&id).unwrap().doc.name, "Machine Learning/sub");
                    assert!(!files.contains_key(&child_path));
                }
                _ => unreachable!(),
            }
            // The ordinary guarded save and a fresh reader must preserve
            // every emitted file, including moved namespace titles.
            selected_spelling_save(&store, &root, &files);
            let fresh = Store::open(&root, Default::default()).unwrap().0;
            assert!(matches!(
                resolve(&fresh, "Machine Learning/sub"),
                Resolved::Existing { .. }
            ));
        }
    }
}

#[test]
fn selected_spelling_in_aliases() {
    selected_spelling_at("aliases");
}

#[test]
fn selected_spelling_in_links_and_tags() {
    selected_spelling_at("links");
}

#[test]
fn selected_spelling_in_queries() {
    selected_spelling_at("queries");
}

#[test]
fn selected_spelling_in_namespace_files() {
    selected_spelling_at("namespace");
}

#[test]
fn selected_spelling_in_native_reference_sites() {
    let _guard = crate::integration_lock();
    for (ext, source, target, before, expected) in [
        (
            "md", "- source\n", "alias:: typed\n- target\n",
            "alias:: B\n- block\n  B:: value\n  tags:: B\n- {{renderer [[B]]}}\n- {{embed [[B]]}}\n- `[[B]]` TODO [#B]\n",
            "alias:: chosen\n- block\n  chosen:: value\n  tags:: chosen\n- {{renderer [[chosen]]}}\n- {{embed [[chosen]]}}\n- `[[B]]` TODO [#B]\n",
        ),
        (
            "org", "* source\n", "#+alias: typed\n* target\n",
            "#+alias: B\n* heading :B:\n:PROPERTIES:\n:B: value\n:tags: B\n:END:\n* [[B][label]] [[file:./pages/B.org][file label]]\n* {{renderer [[B]]}}\n* {{embed [[B]]}}\n",
            "#+alias: chosen\n* heading :chosen:\n:PROPERTIES:\n:chosen: value\n:tags: chosen\n:END:\n* [[chosen][label]] [[file:./pages/chosen.org][file label]]\n* {{renderer [[chosen]]}}\n* {{embed [[chosen]]}}\n",
        ),
    ] {
        let source_path = format!("pages/B.{ext}");
        let target_path = format!("pages/chosen.{ext}");
        let referrer_path = format!("pages/referrer.{ext}");
        let (root, store) = fixture("selected-native-sites", &[
            (&source_path, source), (&target_path, target), (&referrer_path, before),
        ]);
        rename(&store, "B", "typed", RenameChoice::MergeInto(&target_path)).unwrap();
        let files = snapshot(&root);
        selected_spelling_oracle(&format!("native-{ext}"), &files);
        assert_eq!(files[&referrer_path], expected);
        selected_spelling_save(&store, &root, &files);
        let fresh = Store::open(&root, Default::default()).unwrap().0;
        assert_eq!(fresh.page(&tine_core::model::PageId::from(referrer_path.as_str())).unwrap().doc.name, "referrer");
    }
}

#[test]
fn selected_spelling_identifies_a_day_with_an_ambiguous_label() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "selected-day",
        &[
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"MM-dd\"}",
            ),
            ("pages/B.md", "- source\n"),
            ("journals/2026_10_26.md", "alias:: PickedDay\n- 2026\n"),
            ("journals/2027_10_26.md", "- 2027\n"),
            (
                "pages/referrer.md",
                "alias:: B\n- [[B]]\n- {{query (page \"B\")}}\n",
            ),
        ],
    );
    rename(
        &store,
        "B",
        "PickedDay",
        RenameChoice::MergeInto("journals/2026_10_26.md"),
    )
    .unwrap();
    let files = snapshot(&root);
    selected_spelling_oracle("ambiguous-day", &files);
    assert_eq!(
        files["pages/referrer.md"],
        "alias:: 2026_10_26\n- [[2026_10_26]]\n- {{query (page \"2026_10_26\")}}\n"
    );
    assert_eq!(files["journals/2027_10_26.md"], "- 2027\n");
    selected_spelling_save(&store, &root, &files);
    let fresh = Store::open(&root, Default::default()).unwrap().0;
    for year in [2026, 2027] {
        let id = tine_core::model::PageId::from(format!("journals/{year}_10_26.md").as_str());
        let page = fresh.page(&id).unwrap();
        assert_eq!(page.doc.name, "10-26");
        assert_eq!(page.doc.blocks[0].raw, year.to_string());
    }
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
    let listed: Vec<_> = fs::read_dir(root.join("pages"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    if !["N%2Fy.md", "n%2Fy.md"]
        .iter()
        .all(|name| listed.iter().any(|listed| listed == name))
    {
        // A case-insensitive volume (NTFS, APFS) holds one file for both
        // names, so no other page can own the case-only destination there.
        eprintln!("SKIP: pages/ folds case on this volume");
        store.close();
        return;
    }
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
