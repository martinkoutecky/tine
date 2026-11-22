//! Name-discovery transport: readable names plus the unreadable paths, and
//! the wire form of one name's resolution. The row, resolution and alias
//! group shapes live in `tine_graph_features::inventory_wire`, shared with the
//! published snapshot's `answers` (one serializer per answer, I-12).
use serde::Serialize;
use tine_graph_features::inventory_wire::{entry_wire, PageInventoryEntryWire};

/// The wire form of one publication's inventory delta (cost-model audit
/// 2026-11-02, row 1): the page keys whose alias and reference-only entries
/// the window drops, and the entries it adds in their place, in the same
/// entry shape as `page_inventory_wire`. Claimed (file) entries never travel
/// here: a claim change publishes no delta and the window refetches.
#[derive(Serialize)]
pub(crate) struct InventoryDeltaWire {
    keys: Vec<String>,
    entries: Vec<PageInventoryEntryWire>,
}

pub(crate) fn inventory_delta_wire(
    (keys, entries): (&[String], &tine_store::Inventory),
) -> InventoryDeltaWire {
    InventoryDeltaWire {
        keys: keys.to_vec(),
        entries: entries.0.iter().map(entry_wire).collect(),
    }
}

#[cfg(test)]
#[path = "../../../crates/tine-store/src/test_host_writer.rs"]
mod test_host_writer;
#[cfg(test)]
use tine_store as store_crate;

#[cfg(test)]
mod inventory_adapter_tests {
    use super::*;
    use tine_graph_features::inventory_wire::{
        page_inventory_wire, page_modified_wire, resolve_name, PageInventoryWire, ResolvedWire,
    };
    use tine_store::WholeGraph;
    use tine_store::{PageId, SaveBase};

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
    fn page_inventory_wire_matches_the_shared_fixture() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/browser-core-inventory.json"
        ))
        .unwrap();
        let files: Vec<(String, String)> =
            serde_json::from_value(fixture["files"].clone()).unwrap();
        let files: Vec<(&str, &str)> = (files.iter())
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        let root = temp_root("legacy");
        let store = open(&root, &files);
        let view = store.whole_graph().unwrap();
        let wire = page_inventory_wire(&view);
        assert_eq!(wire.rev, String::from(view.rev()));
        let rows: Vec<serde_json::Value> = (rows(&wire).iter())
            .map(|row| serde_json::from_str(row).unwrap())
            .collect();
        assert_eq!(serde_json::Value::from(rows), fixture["rows"]);
        assert_eq!(serde_json::json!(wire.unreadable), fixture["unreadable"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// GH #335: the All pages view's Updated column. One wire of
    /// `[path, unix seconds]` pairs at the view's rev, read from the times the
    /// store already holds; it adds no stat and nothing persisted.
    #[test]
    fn page_modified_wire_lists_each_page_file_with_its_time() {
        let root = temp_root("modified");
        let store = open(
            &root,
            &[
                ("pages/Alpha.md", "- a\n"),
                ("journals/2026_06_26.md", "- j\n"),
            ],
        );
        for (path, secs) in [
            ("pages/Alpha.md", 1_700_000_000u64),
            ("journals/2026_06_26.md", 1_700_000_100),
        ] {
            std::fs::File::options()
                .write(true)
                .open(root.join(path))
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .unwrap();
        }
        drop(store);
        let (store, _, _) =
            tine_store::Store::open(&root, tine_store::OpenOptions::default()).unwrap();
        let view = store.whole_graph().unwrap();
        let wire = page_modified_wire(&view);
        assert_eq!(wire.rev, String::from(view.rev()));
        assert_eq!(
            serde_json::to_string(&wire.modified).unwrap(),
            r#"[["journals/2026_06_26.md",1700000100],["pages/Alpha.md",1700000000]]"#
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
        let store = std::sync::Arc::new(store);
        let app = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app.path()).unwrap();
        let mut after_save = Vec::new();
        for i in 0..20 {
            let read = store.page(&target).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw.push_str(&format!(" probe{i}"));
            test_host_writer::save_on(
                &host,
                &store,
                tine_store::EditKind::ReplacePage,
                &target,
                SaveBase::Existing(read.rev),
                &doc,
            )
            .expect("probe save");
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

    /// Cost probe for the og-perf-names lane (D-20261102-PERF rows 1 and 2),
    /// not a gate. Run on a COPY of a generated graph:
    /// `TINE_NAMES_PROBE=<dir> cargo test --release -p tine og_perf_names_probe
    /// -- --ignored --nocapture`. Types `[[<existing page>]]` into a block one
    /// character per save (each autopaired intermediate `[[prefix]]` is a new
    /// name, the last one an existing page) and reports what the window pays
    /// per save: full inventory refetches (count, bytes, time) or deltas
    /// (count, bytes). Then the `[[` completion and the property-name
    /// completion per keystroke.
    #[test]
    #[ignore]
    fn og_perf_names_probe() {
        let root = std::path::PathBuf::from(
            std::env::var("TINE_NAMES_PROBE").expect("TINE_NAMES_PROBE=<graph copy>"),
        );
        let (store, _, _) =
            tine_store::Store::open(&root, tine_store::OpenOptions::default()).unwrap();
        let refetch = |store: &tine_store::Store| {
            let start = std::time::Instant::now();
            let bytes =
                serde_json::to_vec(&page_inventory_wire(&store.whole_graph().unwrap())).unwrap();
            (start.elapsed(), bytes.len())
        };
        let median = |mut runs: Vec<std::time::Duration>| {
            runs.sort();
            runs[runs.len() / 2]
        };
        let wire = page_inventory_wire(&store.whole_graph().unwrap());
        let (target, existing) = {
            let mut target = None;
            let mut existing = None;
            for entry in &wire.entries {
                if let ResolvedWire::Existing { id, .. } = &entry.target {
                    if entry.is_journal || !id.ends_with(".md") {
                        continue;
                    }
                    if existing.is_none()
                        && entry.name.is_ascii()
                        && entry.name.len() >= 10
                        && !entry.name.contains('/')
                    {
                        existing = Some(entry.name.clone());
                    } else if target.is_none() {
                        let id = PageId::from(id.clone());
                        if let Ok(read) = store.page(&id) {
                            if read.read_only.is_none() && !read.doc.blocks.is_empty() {
                                target = Some(id);
                            }
                        }
                    }
                }
            }
            (
                target.expect("a Markdown page"),
                existing.expect("a page name"),
            )
        };
        let store = std::sync::Arc::new(store);
        let app = tempfile::tempdir().unwrap();
        let host = tine_store::PageHost::start_for_tests(&store, app.path()).unwrap();
        let sub = store.subscribe();
        let original = store.page(&target).unwrap().doc.blocks[0].raw.clone();
        let (mut saves, mut changed, mut refetches, mut deltas) = (0, 0, 0, 0);
        let (mut refetch_bytes, mut delta_bytes) = (0usize, 0usize);
        let mut refetch_time = std::time::Duration::ZERO;
        for k in 1..=existing.len() {
            let read = store.page(&target).unwrap();
            let mut doc = read.doc;
            doc.blocks[0].raw = format!("{original} [[{}]]", &existing[..k]);
            test_host_writer::save_on(
                &host,
                &store,
                tine_store::EditKind::ReplacePage,
                &target,
                SaveBase::Existing(read.rev),
                &doc,
            )
            .expect("probe save");
            saves += 1;
            while let Some(change) = sub.try_recv().unwrap() {
                let answers = serde_json::to_value(&change).unwrap();
                if answers["inventoryChanged"] != true {
                    continue;
                }
                changed += 1;
                match probe_inventory_delta(&change) {
                    Some(bytes) => {
                        deltas += 1;
                        delta_bytes += bytes;
                    }
                    None => {
                        let (time, bytes) = refetch(&store);
                        refetches += 1;
                        refetch_bytes += bytes;
                        refetch_time += time;
                    }
                }
            }
        }
        println!(
            "typing [[{existing}]]: saves={saves} inventory_changed={changed} full_refetches={refetches} refetch_bytes={refetch_bytes} refetch_time={refetch_time:?} deltas={deltas} delta_bytes={delta_bytes}"
        );
        let read = store.page(&target).unwrap();
        let (page, block) = (
            store
                .whole_graph()
                .unwrap()
                .inventory(tine_store::InventoryScope::File(&target))
                .0[0]
                .name
                .clone(),
            read.doc.blocks[0].id.clone(),
        );
        let raw = read.doc.blocks[0].raw.clone();
        let mut per_keystroke = Vec::new();
        for k in 1..=6 {
            let view = store.whole_graph().unwrap();
            let start = std::time::Instant::now();
            let hits = view.complete_page_names(
                &existing[..k],
                20,
                Some((&page, &block, &raw)),
                false,
                None,
            );
            per_keystroke.push(start.elapsed());
            assert!(!hits.is_empty());
        }
        let property = (0..6)
            .map(|_| {
                let view = store.whole_graph().unwrap();
                let start = std::time::Instant::now();
                let bytes = probe_property_names(&view);
                (start.elapsed(), bytes)
            })
            .collect::<Vec<_>>();
        println!(
            "[[ completion per keystroke: median={:?} max={:?}; property-name completion per trigger: median={:?} bytes={}",
            median(per_keystroke.clone()),
            per_keystroke.iter().max().unwrap(),
            median(property.iter().map(|(time, _)| *time).collect()),
            property[0].1,
        );
        drop(host);
    }

    /// Delta bytes the window receives instead of a refetch (`None`: refetch).
    fn probe_inventory_delta(change: &tine_store::Change) -> Option<usize> {
        change.inventory_delta().1.map(|delta| {
            serde_json::to_vec(&inventory_delta_wire(delta))
                .unwrap()
                .len()
        })
    }

    /// One property-name completion session's key fetch: IPC payload bytes
    /// (the window fetches keys once per trigger session, values per chosen
    /// key; before, the full facets on every trigger update).
    fn probe_property_names(view: &WholeGraph) -> usize {
        serde_json::to_vec(&view.property_facets(tine_store::FacetPolicy::CompletionKeys))
            .unwrap()
            .len()
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
        assert_eq!(wire.unreadable.len(), 1);
        assert!(wire.unreadable[0].starts_with("pages/Bad.md: "));
        assert!(wire.unreadable[0].to_ascii_lowercase().contains("utf-8"));
        assert!(matches!(
            resolve_name(&store.whole_graph().unwrap(), "Good", false),
            ResolvedWire::Existing { .. }
        ));
        assert!(matches!(
            resolve_name(&store.whole_graph().unwrap(), "Unknown", false),
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
