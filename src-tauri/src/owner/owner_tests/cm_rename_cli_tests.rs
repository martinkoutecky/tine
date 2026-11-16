//! `tine page rename` against model r6 (CM step 2 slice 1; r3 findings #15,
//! #16), through the owner's `page.rename` verb. The CLI is a caller that
//! cannot ask (O5), so it gets an outcome only when there is nothing to ask,
//! and every refusal the planner makes before writing is a `refused` reply
//! with `admitted: false`, not `uncertain`.
use super::*;
use std::collections::BTreeMap;

#[test]
fn cli_check_refusal_carries_the_file_and_save_fix_as_data_and_prose() {
    let graph = Graph::seeded(
        Default::default(),
        &[
            ("pages/Old.md", "- body\n"),
            ("pages/r.md", "- tags:: [[Old]]\n"),
        ],
    );
    let before = files(&graph.root);
    let body = verb(
        &graph.slot(),
        "page.rename",
        json!({"old":"Old","new":"New"}),
    )
    .unwrap_err();
    let detail = body.detail.unwrap();
    assert_eq!(body.code, "refused");
    assert_eq!(detail["admitted"], false);
    assert_eq!(detail["reason"], "rename-refused");
    assert_eq!(detail["guarantee"], "P3");
    assert_eq!(detail["file"], "pages/r.md");
    assert!(
        body.message
            .contains("open pages/r.md in the Tine app and save it once"),
        "{}",
        body.message
    );
    assert_eq!(files(&graph.root), before);
}

#[test]
fn cli_failed_writer_check_is_refused_with_file_and_a_bug_report_request() {
    let referrer = format!(
        "- {} [[Old]]\n",
        tine_store::model::TEST_RENAME_WRITER_BUG_SENTINEL
    );
    let graph = Graph::seeded(
        Default::default(),
        &[("pages/Old.md", "- body\n"), ("pages/r.md", &referrer)],
    );
    let before = files(&graph.root);
    let body = verb(
        &graph.slot(),
        "page.rename",
        json!({"old":"Old","new":"New"}),
    )
    .unwrap_err();
    let detail = body.detail.unwrap();
    assert_eq!(body.code, "refused");
    assert_eq!(detail["admitted"], false);
    assert_eq!(detail["reason"], "rename-refused");
    assert_eq!(detail["guarantee"], "P2");
    assert_eq!(detail["file"], "pages/r.md");
    assert!(
        body.message.contains("bug in Tine, please report"),
        "{}",
        body.message
    );
    assert_eq!(files(&graph.root), before);
}

/// Every page and journal file, path → text.
fn files(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for dir in ["pages", "journals"] {
        for entry in std::fs::read_dir(root.join(dir)).into_iter().flatten() {
            let entry = entry.unwrap();
            let rel = format!("{dir}/{}", entry.file_name().to_string_lossy());
            out.insert(rel, std::fs::read_to_string(entry.path()).unwrap());
        }
    }
    out
}

/// O3: an alias "declared only by the source itself: rename in one step ...
/// The source's own declaration of B stays as it is (P1)." Through the CLI
/// too: before, the owner verb surfaced the planner's question as
/// `uncertain` (r3 #15).
#[test]
fn cli_renames_a_page_to_its_own_alias_in_one_step() {
    let graph = Graph::seeded(
        Default::default(),
        &[
            ("pages/source.md", "alias:: Short\n\n- body\n"),
            ("pages/r.md", "- [[source]]\n"),
        ],
    );
    let slot = graph.slot();
    let renamed = verb(&slot, "page.rename", json!({"old":"source","new":"Short"})).unwrap();
    assert_eq!(renamed["kind"], "page-renamed", "{renamed}");
    let after = files(&graph.root);
    assert_eq!(after["pages/Short.md"], "alias:: Short\n\n- body\n");
    assert!(!after.contains_key("pages/source.md"));
    assert_eq!(after["pages/r.md"], "- [[Short]]\n");
}

/// O0: "A is renamable if it is a page ... or an alias", and the rename
/// substitutes A everywhere (O1). An alias name typed as the source is that
/// alias: its declaration is respelled and the owning page keeps its name
/// and file (before, the CLI resolved the alias to its owner and renamed the
/// owner, r3 #16).
#[test]
fn cli_renames_an_alias_itself_not_its_owner() {
    let graph = Graph::seeded(
        Default::default(),
        &[
            ("pages/Project.md", "alias:: Shortcut\n\n- project\n"),
            ("pages/Links.md", "- [[Shortcut]] and [[Project]]\n"),
        ],
    );
    let slot = graph.slot();
    let renamed = verb(
        &slot,
        "page.rename",
        json!({"old":"Shortcut","new":"Quick"}),
    )
    .unwrap();
    assert_eq!(renamed["old"], "Shortcut", "{renamed}");
    let after = files(&graph.root);
    assert_eq!(after["pages/Project.md"], "alias:: Quick\n\n- project\n");
    assert_eq!(after["pages/Links.md"], "- [[Quick]] and [[Project]]\n");
    assert!(!after.contains_key("pages/Quick.md"));
}

/// O3 ("No holder, and a page other than the source declares B as an alias
/// ...: refused, naming the page B opens"), O0 ("Any other name is
/// refused"), O2 ("A day cannot be the source of a rename") and O5 (a caller
/// that cannot ask is refused whenever something would be asked, here a
/// merge into one holder): each is `refused` with `admitted: false`, the
/// model's message, and nothing written (before: `uncertain` with
/// `admitted: null` for every planner refusal, r3 #15).
#[test]
fn cli_planner_refusals_are_refused_before_any_write() {
    let graph = Graph::seeded(
        Default::default(),
        &[
            ("pages/owner.md", "alias:: Destination\n\n- owner\n"),
            ("journals/2026_11_02.md", "- day\n"),
        ],
    );
    let slot = graph.slot();
    let before = files(&graph.root);
    for (old, new, reason, says) in [
        (
            "a",
            "Destination",
            "page-exists",
            "alias of the page “owner”",
        ),
        ("a", "b", "page-exists", "already owns the destination"),
        (
            "Ghost",
            "New",
            "rename-refused",
            "no page or alias has that name",
        ),
        ("Nov 2nd, 2026", "New", "rename-refused", "journal day"),
        ("ns", "ns/child", "rename-refused", "inside “ns”"),
    ] {
        let result = verb(&slot, "page.rename", json!({"old":old,"new":new}));
        let body = result.expect_err("refused");
        let detail = body.detail.clone().unwrap_or(Value::Null);
        assert_eq!(
            (body.code.as_str(), &detail["reason"], &detail["admitted"]),
            ("refused", &json!(reason), &json!(false)),
            "{old} → {new}: {} {detail}",
            body.message
        );
        assert!(
            body.message.contains(says),
            "{old} → {new}: {}",
            body.message
        );
        assert_eq!(files(&graph.root), before, "{old} → {new}: nothing written");
    }
}

/// O2 asks of the name the user typed (round 2 finding 3): under a
/// yearless title format, `2026-10-26` names the 2026 journal, whose page is
/// titled `10-26`; the rename is the Day refusal, not `uncertain` from the
/// yearless title the owner used to pass on.
#[test]
fn cli_a_typed_day_is_refused_under_a_yearless_title_format() {
    let graph = Graph::seeded(
        Default::default(),
        &[
            (
                "logseq/config.edn",
                "{:file/name-format :triple-lowbar :journal/page-title-format \"MM-dd\"}\n",
            ),
            ("journals/2026_10_26.md", "- day\n"),
            ("pages/Links.md", "- [[10-26]]\n"),
        ],
    );
    let slot = graph.slot();
    let before = files(&graph.root);
    let body = verb(
        &slot,
        "page.rename",
        json!({"old":"2026-10-26","new":"New"}),
    )
    .expect_err("refused");
    let detail = body.detail.clone().unwrap_or(Value::Null);
    assert_eq!(
        (body.code.as_str(), &detail["reason"], &detail["admitted"]),
        ("refused", &json!("rename-refused"), &json!(false)),
        "{} {detail}",
        body.message
    );
    assert!(body.message.contains("journal day"), "{}", body.message);
    assert_eq!(files(&graph.root), before, "nothing written");
}
