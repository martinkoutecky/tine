//! `tine page rename` onto a date-shaped name (cli-rename-date-1109,
//! REG-CLI-RENAME-DATE-JOURNAL-REFUSAL), through the owner's `page.rename`
//! verb and the app's rename question (`rename_offer`), for the three
//! accepted spellings of one day, with and without that day's journal.
//!
//! Model (SPEC §2.1 `target`, §4.1 candidates, §4.3 unattended; KD): with no
//! journal file the only candidate renames the ordinary file to the literal
//! name under `pages/`, and `[[new]]` opens it while the other spellings keep
//! meaning the day. With the journal, every spelling has the journal in its
//! group, so a non-canonical spelling has two safe candidates and the
//! journal's own title has one (a merge); the CLI, which never merges,
//! refuses `page-exists` and writes nothing. The app's missing "own page"
//! option and the unattended singleton merge are the open conformance gaps
//! G3/B4 and G9/B6 (r13 fixtures `r13-rename-onto-date-*`).
use super::*;
use std::collections::BTreeMap;

const SPELLINGS: [&str; 3] = ["2026_11_02", "2026-11-02", "Nov 2nd, 2026"];
const JOURNAL: &str = "journals/2026_11_02.md";

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

fn date_graph(journal: bool) -> Graph {
    let mut extra = vec![
        ("pages/Foo.md", "- foo body\n"),
        ("pages/r.md", "- see [[Foo]]\n"),
    ];
    if journal {
        extra.push((JOURNAL, "- day body\n"));
    }
    Graph::seeded(Default::default(), &extra)
}

/// What `[[name]]` opens in the app afterwards: (name, is_journal).
fn opens(graph: &Graph, name: &str) -> (String, bool) {
    let target = graph.store.whole_graph().unwrap().link_target(name);
    (target.name, target.is_journal)
}

#[test]
fn rename_onto_a_free_day_spelling_makes_that_ordinary_page() {
    for new in SPELLINGS {
        let graph = date_graph(false);
        let offer = tine_graph_features::pages::rename_offer(&graph.store, "Foo", new, None);
        assert_eq!(
            offer.unwrap(),
            tine_graph_features::pages::RenameOffer::Rename,
            "{new}"
        );
        let slot = graph.slot();
        let renamed = verb(&slot, "page.rename", json!({"old":"Foo","new":new})).unwrap();
        assert_eq!(renamed["report"]["outcome"], "renamed", "{new}");
        let after = files(&graph.root);
        assert_eq!(
            after.get(&format!("pages/{new}.md")).map(String::as_str),
            Some("- foo body\n"),
            "{new}: {after:?}"
        );
        assert!(!after.contains_key("pages/Foo.md"), "{new}: {after:?}");
        assert!(!after.keys().any(|p| p.starts_with("journals/")), "{new}");
        assert_eq!(after["pages/r.md"], format!("- see [[{new}]]\n"));
        // KD: the literal spelling opens the ordinary page; the others still
        // mean the day.
        assert_eq!(opens(&graph, new), (new.to_owned(), false), "{new}");
        for other in SPELLINGS.iter().filter(|s| **s != new) {
            assert!(opens(&graph, other).1, "{new}: [[{other}]] means the day");
        }
    }
}

#[test]
fn rename_onto_a_day_with_a_journal_refuses_page_exists_and_writes_nothing() {
    for new in SPELLINGS {
        let graph = date_graph(true);
        let before = files(&graph.root);
        let offer = tine_graph_features::pages::rename_offer(&graph.store, "Foo", new, None);
        assert!(
            matches!(
                offer.unwrap(),
                tine_graph_features::pages::RenameOffer::Merge { ref into, journal: true, .. }
                    if into == JOURNAL
            ),
            "{new}: the app asks to merge into the journal"
        );
        let slot = graph.slot();
        let (code, detail) = error_code(verb(&slot, "page.rename", json!({"old":"Foo","new":new})));
        assert_eq!(
            (code.as_str(), &detail["reason"], &detail["admitted"]),
            ("refused", &json!("page-exists"), &json!(false)),
            "{new}: {detail}"
        );
        assert_eq!(files(&graph.root), before, "{new}: nothing written");
        for spelling in SPELLINGS {
            assert_eq!(
                opens(&graph, spelling),
                ("Nov 2nd, 2026".to_owned(), true),
                "{new}: [[{spelling}]] opens the journal"
            );
        }
    }
}
