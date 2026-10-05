//! Name-discovery transport: readable names plus the unreadable paths.
use super::{ResolvedWire, WholeGraph};
use serde::Serialize;
use tine_store::Resolved;

#[derive(Serialize)]
pub(crate) struct PageInventoryWire {
    /// The `GraphRev` the inventory was read at. The frontend drops a response
    /// older than one it already holds.
    rev: String,
    entries: Vec<PageInventoryEntryWire>,
    /// Graph-relative paths whose page name could not be read.
    unreadable: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct PageInventoryEntryWire {
    /// `refs::page_key(name)`: the frontend looks names up by this key only.
    key: String,
    name: String,
    is_journal: bool,
    day: Option<i64>,
    target: ResolvedWire,
}

// One unreadable file never blocks name answers for the rest of the graph
// (I-22): readable claims resolve and a miss is `Absent`. The unreadable paths
// travel on the inventory so the frontend reports them (I-2).
pub(super) fn resolve_name(view: WholeGraph, name: &str, journal: bool) -> ResolvedWire {
    view.resolve(name, journal).into()
}

pub(super) fn page_inventory_wire(view: &WholeGraph) -> PageInventoryWire {
    let rev = view.rev();
    PageInventoryWire {
        rev: rev.into(),
        entries: view
            .inventory(tine_store::InventoryScope::All)
            .0
            .iter()
            .map(|entry| PageInventoryEntryWire {
                key: tine_core::refs::page_key(&entry.name),
                name: entry.name.clone(),
                is_journal: entry.is_journal,
                day: entry.day.map(|day| day.0),
                target: ResolvedWire::from(&entry.target),
            })
            .collect(),
        unreadable: view
            .unreadable_files()
            .iter()
            .map(|(id, _)| id.as_str().to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod inventory_adapter_tests {
    use super::*;
    use tine_store::{PageId, SaveBase, SaveOutcome};

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tine-inventory-adapter-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn open(root: &std::path::Path, files: &[(&str, &str)]) -> tine_store::Store {
        for (path, body) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        tine_store::Store::open(root, tine_store::OpenOptions::default())
            .unwrap()
            .0
    }

    fn rows(wire: &PageInventoryWire) -> Vec<String> {
        wire.entries
            .iter()
            .map(|entry| serde_json::to_string(entry).unwrap())
            .collect()
    }

    /// The one inventory wire carries effective page names from Markdown and
    /// Org titles, with physical paths retained as ids. The frontend views
    /// over it are pinned in `src/pageIndex.test.ts`.
    #[test]
    fn page_inventory_wire_matches_legacy_fixture() {
        let root = temp_root("legacy");
        let store = open(
            &root,
            &[
                (
                    "pages/Alpha.md",
                    "title:: Display Alpha\nalias:: Shared\n- [[Only Linked]]\n",
                ),
                (
                    "pages/Beta.org",
                    // Org page properties are `#+KEY:` directives; mldoc parses a bare
                    // `alias::` line in Org as a paragraph (og 8e, OG graph-parser).
                    "#+TITLE: Display Beta\n#+ALIAS: Shared\n* [[Only Linked]]\n",
                ),
                ("pages/nested/Alpha.org", "* nested twin\n"),
                ("pages/Team%2FChild.md", "- [[Another Ref]]\n"),
                ("journals/2026_06_26.md", "- canonical\n"),
                ("journals/Jun 26th, 2026.org", "* duplicate day\n"),
            ],
        );
        let view = store.whole_graph().unwrap();
        let wire = page_inventory_wire(&view);
        assert_eq!(wire.rev, String::from(view.rev()));
        assert_eq!(
            rows(&wire),
            vec![
                r#"{"key":"alpha","name":"Alpha","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/nested/Alpha.org","others":[]}}"#,
                r#"{"key":"another ref","name":"Another Ref","is_journal":false,"day":null,"target":{"kind":"absent","id":"pages/Another Ref.md"}}"#,
                r#"{"key":"display alpha","name":"Display Alpha","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Alpha.md","others":[]}}"#,
                r#"{"key":"display beta","name":"Display Beta","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Beta.org","others":[]}}"#,
                r#"{"key":"jun 26th, 2026","name":"Jun 26th, 2026","is_journal":true,"day":20260626,"target":{"kind":"existing","id":"journals/2026_06_26.md","others":["journals/Jun 26th, 2026.org"]}}"#,
                r#"{"key":"only linked","name":"Only Linked","is_journal":false,"day":null,"target":{"kind":"absent","id":"pages/Only Linked.md"}}"#,
                r#"{"key":"shared","name":"Shared","is_journal":false,"day":null,"target":{"kind":"alias","owners":["pages/Alpha.md","pages/Beta.org"]}}"#,
                r#"{"key":"team/child","name":"Team/Child","is_journal":false,"day":null,"target":{"kind":"existing","id":"pages/Team%2FChild.md","others":[]}}"#,
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Cost probe for batch-1 B16a, not a gate: `page_inventory` on a real
    /// graph. Run on a COPY: `TINE_INVENTORY_PROBE=<dir> cargo test --release
    /// -p tine page_inventory_cost_probe -- --ignored --nocapture`. Reports the
    /// JSON payload size and the median command time over 20 runs, both warm
    /// and after an ordinary content save (the refresh a save triggers).
    #[test]
    #[ignore]
    fn page_inventory_cost_probe() {
        let root = std::path::PathBuf::from(
            std::env::var("TINE_INVENTORY_PROBE").expect("TINE_INVENTORY_PROBE=<graph copy>"),
        );
        let (store, _, _) =
            tine_store::Store::open(&root, tine_store::OpenOptions::default()).unwrap();
        let time = |store: &tine_store::Store| {
            let start = std::time::Instant::now();
            let bytes =
                serde_json::to_vec(&page_inventory_wire(&store.whole_graph().unwrap())).unwrap();
            (start.elapsed(), bytes.len())
        };
        let median = |mut runs: Vec<std::time::Duration>| {
            runs.sort();
            runs[runs.len() / 2]
        };
        let (first, bytes) = time(&store);
        let wire = page_inventory_wire(&store.whole_graph().unwrap());
        let warm: Vec<_> = (0..20).map(|_| time(&store).0).collect();
        let target = wire
            .entries
            .iter()
            .find_map(|entry| match &entry.target {
                ResolvedWire::Existing { id, .. } if !entry.is_journal && id.ends_with(".md") => {
                    let id = PageId::from(id.clone());
                    let read = store.page(&id).ok()?;
                    (read.read_only.is_none() && !read.doc.blocks.is_empty()).then_some(id)
                }
                _ => None,
            })
            .expect("a Markdown page");
        let mut after_save = Vec::new();
        for i in 0..20 {
            let read = store.page(&target).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw.push_str(&format!(" probe{i}"));
            let outcome = store.save(
                tine_store::EditKind::ReplacePage,
                &target,
                SaveBase::Existing(read.rev),
                &doc,
            );
            assert!(matches!(outcome, SaveOutcome::Saved(_)), "probe save");
            after_save.push(time(&store).0);
        }
        println!(
            "page_inventory: entries={} payload_bytes={bytes} first={first:?} warm_median={:?} after_save_median={:?} after_save_max={:?}",
            wire.entries.len(),
            median(warm),
            median(after_save.clone()),
            after_save.iter().max().unwrap(),
        );
    }

    /// Existing files beat a colliding alias (v0.6.5 `load_named`): the wire has
    /// one entry for the name, and its target is the file, as `resolve` says.
    #[test]
    fn page_inventory_a_file_beats_a_colliding_alias() {
        let root = temp_root("collide");
        let store = open(
            &root,
            &[
                ("pages/Owner.md", "alias:: Real, Cafe\u{301}\n- body\n"),
                ("pages/Real.md", "- a real page\n"),
                ("pages/Café.md", "- NFC file\n"),
            ],
        );
        let view = store.whole_graph().unwrap();
        let wire = page_inventory_wire(&view);
        for key in ["real", "café"] {
            let entries: Vec<_> = wire.entries.iter().filter(|e| e.key == key).collect();
            assert_eq!(entries.len(), 1, "one entry for {key}");
            let resolved = ResolvedWire::from(view.resolve(&entries[0].name, false));
            assert_eq!(
                serde_json::to_string(&entries[0].target).unwrap(),
                serde_json::to_string(&resolved).unwrap(),
            );
            assert!(matches!(entries[0].target, ResolvedWire::Existing { .. }));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn one_unreadable_name_is_listed_and_never_blocks_the_graph() {
        let root = temp_root("partial");
        let store = open(&root, &[("pages/Good.md", "- good\n")]);
        std::fs::write(root.join("pages/Bad.md"), b"title:: unknown \xff\n").unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        let wire = page_inventory_wire(&store.whole_graph().unwrap());
        assert!(wire.entries.iter().any(|entry| entry.name == "Good"));
        assert_eq!(wire.unreadable, vec!["pages/Bad.md".to_owned()]);
        assert!(matches!(
            resolve_name(store.whole_graph().unwrap(), "Good", false),
            ResolvedWire::Existing { .. }
        ));
        assert!(matches!(
            resolve_name(store.whole_graph().unwrap(), "Unknown", false),
            ResolvedWire::Absent { .. }
        ));
        std::fs::write(root.join("pages/Bad.md"), "- repaired\n").unwrap();
        store.refresh(tine_store::Depth::Stamps).unwrap();
        assert!(page_inventory_wire(&store.whole_graph().unwrap())
            .unreadable
            .is_empty());
        store.close();
        std::fs::remove_dir_all(root).unwrap();
    }
}
