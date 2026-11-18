//! CM step 2, slice 2, item 1: O4 over every *touched* name (model r6,
//! `tine-agents/specs/notes/cm-cut-model-r6.md`, D-20261116-CM536; Lean
//! `touchedNames`, `captureClaimed`, `preSafety`). "Touched names are the
//! source, every name under it (`A/…`) however it exists (a file, a
//! reference, an alias that any page declares), the aliases the source's file
//! declares, and the images of all of these under O1; a name is touched
//! whether or not the request changes its spelling." Each test goes through
//! the app's question (`rename_offer`) and the rename with an answer
//! (`rename_page_as`), and asserts what the user gets: the refusal and its
//! reason, and the files untouched.
use super::*;

fn refusal(store: &Store, old: &str, new: &str) -> serde_json::Value {
    let offer = serde_json::to_value(rename_offer(store, old, new, None).unwrap()).unwrap();
    // The model's refusal; the window may offer a helper with it (a day move,
    // the contested-name merge: D-20261115-HELP), carrying the same reason.
    assert!(
        ["refused", "contested", "dayMove"].contains(&offer["kind"].as_str().unwrap_or_default()),
        "{old} → {new}: {offer}"
    );
    offer["reason"].clone()
}

fn refused_with(
    root: &Path,
    store: &Arc<Store>,
    old: &str,
    new: &str,
    choice: RenameChoice<'_>,
) -> RenameBlocked {
    let before = snapshot(root);
    let error = rename(store, old, new, choice).unwrap_err();
    let blocked = RenameBlocked::of(&error)
        .unwrap_or_else(|| panic!("{old} → {new}: untyped refusal {error}"))
        .clone();
    assert_eq!(
        snapshot(root),
        before,
        "{old} → {new}: a refusal writes nothing"
    );
    blocked
}

/// O4 capture: "no touched name newly lands on a claimed name other than
/// the merge survivor". A reference-only child `A/x` (no file) would become
/// `B/x`, which a file claims: the rename would fold one page into another
/// without asking. Refused, for a rename and for a merge into `B` alike
/// (`B/x` is not the survivor).
#[test]
fn o4_a_reference_only_child_landing_on_a_claimed_name_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-ref-child",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/r.md", "- [[A/x]]\n"),
            ("pages/bx.md", "title:: B/x\n\n- bx\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "capture", "{reason}");
    assert_eq!(reason["name"], "A/x");
    assert_eq!(reason["image"], "B/x");
    assert_eq!(reason["page"], "B/x");
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Capture { .. }
    ));

    let (root, store) = fixture(
        "touched-ref-child-merge",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/B.md", "- b\n"),
            ("pages/r.md", "- [[A/x]]\n"),
            ("pages/bx.md", "title:: B/x\n\n- bx\n"),
        ],
    );
    assert_eq!(refusal(&store, "A", "B")["kind"], "capture");
    assert!(matches!(
        refused_with(
            &root,
            &store,
            "A",
            "B",
            RenameChoice::MergeInto("pages/B.md")
        ),
        RenameBlocked::Capture { .. }
    ));
}

/// O4: a name under the source that exists only as another page's alias is
/// touched ("an alias that any page declares"): `A/sub`, declared by `Y`,
/// would become `B/sub`, which a file claims. Refused.
#[test]
fn o4_an_alias_under_the_source_landing_on_a_claimed_name_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-alias-child",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/Y.md", "alias:: A/sub\n\n- y\n"),
            ("pages/bsub.md", "title:: B/sub\n\n- bs\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "capture", "{reason}");
    assert_eq!(reason["name"], "A/sub");
    assert_eq!(reason["image"], "B/sub");
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Capture { .. }
    ));
}

/// O4, the namespace parent: `A/x` exists only because the file `A/x/y`
/// lies under it ("every name under it however it exists"). It would become
/// `B/x`, a file's page. Refused.
#[test]
fn o4_a_namespace_parent_under_the_source_landing_on_a_claimed_name_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-ns-parent",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/axy.md", "title:: A/x/y\n\n- axy\n"),
            ("pages/bx.md", "title:: B/x\n\n- bx\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "capture", "{reason}");
    assert_eq!(reason["name"], "A/x");
    assert_eq!(reason["image"], "B/x");
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Capture { .. }
    ));
}

/// O4 (Lean `captureClaimed`): "Only the source concept may land on its own
/// alias. A child concept must still respect that alias claim, even when the
/// parent file declared it." `A` declares `B/x`; its child `A/x` would land
/// on it. Refused. Control: the source landing on its own alias is the O3
/// one-step rename.
#[test]
fn o4_a_child_landing_on_an_alias_the_source_declares_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-own-alias-child",
        &[
            ("pages/A.md", "alias:: B/x\n\n- a\n"),
            ("pages/r.md", "- [[A/x]]\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "capture", "{reason}");
    assert_eq!(reason["name"], "A/x");
    assert_eq!(reason["image"], "B/x");
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Capture { .. }
    ));
    let (_, store) = fixture(
        "touched-own-alias-source",
        &[("pages/A.md", "alias:: B\n\n- a\n")],
    );
    assert_eq!(
        rename_offer(&store, "A", "B", None).unwrap(),
        RenameOffer::Rename
    );
}

/// O4 contested: "no touched name is contested". The source's own alias `X`
/// is claimed by two files (Lean fixture 87,
/// `unchanged_alias_contested_refused`), and the image `B/x` of a
/// reference-only child is claimed by two files. Both refused, naming the
/// name and its files, though neither name changes spelling in the first.
#[test]
fn o4_a_contested_touched_name_is_refused() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-contested-alias",
        &[
            ("pages/A.md", "alias:: X\n- source\n"),
            ("pages/P.md", "title:: X\n- p\n"),
            ("pages/Q.md", "title:: X\n- q\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "contested", "{reason}");
    assert_eq!(reason["name"], "X");
    assert_eq!(
        reason["files"],
        serde_json::json!(["pages/P.md", "pages/Q.md"])
    );
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Contested { .. }
    ));

    let (root, store) = fixture(
        "touched-contested-image",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/r.md", "- [[A/x]]\n"),
            ("pages/P.md", "title:: B/x\n- p\n"),
            ("pages/Q.md", "title:: B/x\n- q\n"),
        ],
    );
    let reason = refusal(&store, "A", "B");
    assert_eq!(reason["kind"], "contested", "{reason}");
    assert_eq!(reason["name"], "B/x");
    assert!(matches!(
        refused_with(&root, &store, "A", "B", RenameChoice::Rename),
        RenameBlocked::Contested { .. }
    ));
}

/// Review round 2, finding 4, with the reviewer's input: an image that is a
/// day is touched (Lean `touchedNames` appends `touched.map nameMap`
/// unfiltered; only days already written are left out), so contest is
/// checked on it before capture. Title format `ns/yyyy-MM-dd`: the child
/// `A/2026-10-26` is no day, but its image `ns/2026-10-26` is, and two
/// journal files claim that day. `A → ns` is refused as contested, naming
/// both files, not as a capture naming one.
#[test]
fn o4_a_day_image_claimed_by_two_journal_files_is_contested() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-contested-day-image",
        &[
            (
                "logseq/config.edn",
                "{:journal/page-title-format \"ns/yyyy-MM-dd\"}\n",
            ),
            ("pages/A.md", "- a\n"),
            ("pages/r.md", "- [[A/2026-10-26]]\n"),
            ("journals/2026_10_26.md", "- one\n"),
            ("journals/2026-10-26.md", "- two\n"),
        ],
    );
    let reason = refusal(&store, "A", "ns");
    assert_eq!(reason["kind"], "contested", "{reason}");
    assert_eq!(reason["name"], "ns/2026-10-26");
    assert_eq!(
        reason["files"],
        serde_json::json!(["journals/2026-10-26.md", "journals/2026_10_26.md"])
    );
    assert!(matches!(
        refused_with(&root, &store, "A", "ns", RenameChoice::Rename),
        RenameBlocked::Contested { .. }
    ));
}

/// O4: "a name that is only referenced is free, here as in O3"
/// (D-20261105-CM11). A file child `A/x` and a reference-only child `A/y`
/// land on `B/x` and `B/y`, which are only linked: the rename proceeds and
/// the links follow.
#[test]
fn o4_a_touched_name_landing_on_a_referenced_name_proceeds() {
    let _guard = crate::integration_lock();
    let (root, store) = fixture(
        "touched-free",
        &[
            ("pages/A.md", "- a\n"),
            ("pages/ax.md", "title:: A/x\n\n- ax\n"),
            ("pages/r.md", "- [[A/y]] [[B/x]] [[B/y]]\n"),
        ],
    );
    assert_eq!(
        rename(&store, "A", "B", RenameChoice::Rename).unwrap(),
        RenameOutcome::Renamed
    );
    let after = snapshot(&root);
    assert_eq!(after["pages/r.md"], "- [[B/y]] [[B/x]] [[B/y]]\n");
    assert!(
        after.values().any(|text| text == "title:: B/x\n\n- ax\n"),
        "{after:?}"
    );
}

/// The cost of the touched names (PRINCIPLES 2, 3): the checks need every
/// touched name once, and the reference and alias names under the source
/// come from ordered indexes (O(log N + k)), so the names visited do not grow
/// with unrelated names in the graph.
#[test]
fn o4_touched_names_are_found_without_visiting_unrelated_names() {
    let _guard = crate::exclusive_integration_lock();
    let visits = |unrelated: usize| {
        let mut files = vec![
            ("pages/A.md".to_owned(), "- a\n".to_owned()),
            ("pages/r.md".to_owned(), "- [[A/x]] [[A/y/z]]\n".to_owned()),
            ("pages/Y.md".to_owned(), "alias:: A/w\n\n- y\n".to_owned()),
        ];
        for index in 0..unrelated {
            files.push((
                format!("pages/u{index:04}.md"),
                format!("alias:: Alias{index:04}\n\n- [[Ref{index:04}]] [[N{index:04}/c]]\n"),
            ));
        }
        let borrowed: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, body)| (path.as_str(), body.as_str()))
            .collect();
        let (_, store) = fixture(&format!("touched-cost-{unrelated}"), &borrowed);
        store.whole_graph().unwrap();
        tine_store::cost_counters::reset();
        assert_eq!(
            rename_offer(&store, "A", "B", None).unwrap(),
            RenameOffer::Rename
        );
        tine_store::cost_counters::snapshot().namespace_name_visits
    };
    let few = visits(5);
    let many = visits(200);
    assert_eq!(few, many, "touched-name lookups grew with unrelated names");
    // `A/x`, `A/y/z` (reference index) and `A/w` (alias index): each visited.
    assert!(
        few >= 3,
        "the touched names under the source are visited: {few}"
    );
}
