//! Opt-in S2 acceptance and I-25 comparison. The legacy commit helper is
//! test-only: production page writes always use the host.
use super::*;
use std::{fs, path::Path, sync::Arc, time::Duration};

fn legacy(store: &Store, old: &str, new: &str) {
    let mut plan = plan_rename(store, old, new, None, None, &|| {}).unwrap();
    plan.pages(store).unwrap();
    assert!(commit_rename(store, old, plan, &[]).unwrap().is_some());
}

fn hosted(store: &Arc<Store>, app: &Path, old: &str, new: &str) {
    let host = PageHost::start_for_tests(store, app).unwrap();
    assert!(host.quiesce_for_tests(Duration::from_secs(60)));
    rename_page_expected(store, &host, old, new, None).unwrap();
    assert!(host.quiesce_for_tests(Duration::from_secs(60)));
    drop(host);
}

#[test]
#[ignore = "I-25 comparison; run alone with --ignored --nocapture --test-threads=1"]
fn namespace_old_and_host_unit_cost() {
    for (children, blocks, unit) in [(2, 1, true), (2, 60, true), (2, 1, false), (22, 1, false)] {
        for host_path in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("graph");
            fs::create_dir_all(root.join("pages")).unwrap();
            let body = |text: &str| format!("- {text}\n").repeat(blocks);
            fs::write(root.join("pages/Ns.md"), body("ns")).unwrap();
            for i in 0..children {
                let (name, text) = if unit {
                    let name = ["a", "b"][i];
                    (name.to_owned(), name)
                } else {
                    (format!("c{i:03}"), "child")
                };
                fs::write(root.join(format!("pages/Ns%2F{name}.md")), body(text)).unwrap();
            }
            let linked = if unit { "a" } else { "c000" };
            fs::write(
                root.join("pages/Ref.md"),
                body(&format!("links [[Ns/{linked}]]")),
            )
            .unwrap();
            let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
            store.whole_graph().unwrap();
            let host = host_path
                .then(|| PageHost::start_for_tests(&store, &temp.path().join("app")).unwrap());
            if let Some(host) = &host {
                assert!(host.quiesce_for_tests(Duration::from_secs(60)));
            }
            tine_store::cost_counters::reset();
            if let Some(host) = &host {
                rename_page_expected(&store, host, "Ns", "Space", None).unwrap();
                assert!(host.quiesce_for_tests(Duration::from_secs(60)));
            } else {
                legacy(&store, "Ns", "Space");
            }
            let cost = tine_store::cost_counters::snapshot();
            eprintln!("I25 unit={unit} children={children} blocks={blocks} host={host_path}: files={} bytes={} fsyncs={} transport=0", cost.files_written, cost.bytes_written, cost.fsyncs);
            assert_eq!(
                fs::read_to_string(root.join("pages/Ref.md")).unwrap(),
                body(&format!("links [[Space/{linked}]]"))
            );
            drop(host);
            store.close();
        }
    }
}

/// Do not print names or contents from the anonymized graph. The caller
/// prepares two copies under this worktree and supplies their parent.
#[test]
#[ignore = "requires TINE_HOST_WRITER_ACCEPTANCE with two prepared graph copies"]
fn anonymized_namespace_matches_legacy_bytes() {
    let parent = std::path::PathBuf::from(
        std::env::var_os("TINE_HOST_WRITER_ACCEPTANCE").expect("prepared copies required"),
    );
    let root = parent.join("anon-copy");
    let old_root = parent.join("anon-old-copy");
    assert!(root.is_dir() && old_root.is_dir());
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let old_store = Store::open(&old_root, Default::default()).unwrap().0;
    let graph = store.whole_graph().unwrap();
    let entries = graph.inventory(tine_store::InventoryScope::All);
    let prefixes: std::collections::BTreeSet<_> = entries
        .0
        .iter()
        .flat_map(|e| {
            e.name
                .match_indices('/')
                .map(|(i, _)| e.name[..i].to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    let new = "TineHostWriterAcceptanceDestination";
    let mut max_moves = 0;
    let selected = prefixes.iter().find(|name| {
        let prefix = format!("{}/", refs::normalize(name));
        let descendants: Vec<_> = entries
            .0
            .iter()
            .filter(|e| refs::normalize(&e.name).starts_with(&prefix))
            .collect();
        let plan = plan_rename(&store, name, new, None, None, &|| {}).unwrap();
        let moves = plan.moves.len();
        max_moves = max_moves.max(moves);
        moves >= 1 && plan.map.0.len() > 1 && !descendants.is_empty()
    });
    eprintln!("anonymized inventory: {} entries, {} namespace prefixes, maximum physical moves {max_moves}", entries.0.len(), prefixes.len());
    let old = selected
        .expect("a namespace with physical page moves is required")
        .clone();
    assert!(matches!(graph.resolve(new, false), Resolved::Absent { .. }));
    // This anonymized fixture has no physical namespace with an existing
    // indexed referrer. Seed the same controlled referrer in both copies,
    // after closing their stores, without changing any original page bytes.
    let planned = plan_rename(&store, &old, new, None, None, &|| {}).unwrap();
    let linked = planned.map.0[0].0.clone();
    eprintln!("acceptance physical moves={}, mapped identities={}; one identical controlled referrer seeded per copy", planned.moves.len(), planned.map.0.len());
    store.close();
    old_store.close();
    let fixture = format!("- [[{old}]] [[{linked}]]\n");
    for copy in [&root, &old_root] {
        let path = copy.join("pages/TineHostWriterAcceptanceReferrer.md");
        assert!(!path.exists());
        fs::write(path, &fixture).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    let old_store = Store::open(&old_root, Default::default()).unwrap().0;
    assert!(!store
        .whole_graph()
        .unwrap()
        .explicit_referrers(&[old.clone()])
        .is_empty());
    hosted(&store, &parent.join("anon-app"), &old, new);
    legacy(&old_store, &old, new);
    store.close();
    old_store.close();

    fn files(
        root: &Path,
        dir: &Path,
        out: &mut std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
        trash: &mut (usize, usize),
    ) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let rel = entry.path().strip_prefix(root).unwrap().to_owned();
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() || rel.starts_with(".git") {
                continue;
            }
            if kind.is_dir() {
                files(root, &entry.path(), out, trash);
            } else if kind.is_file() {
                let bytes = fs::read(entry.path()).unwrap();
                if rel.starts_with("logseq/.tine-trash") {
                    trash.0 += 1;
                    trash.1 += bytes.len();
                } else {
                    out.insert(rel, bytes);
                }
            }
        }
    }
    let (mut actual, mut expected) = (
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::new(),
    );
    let (mut host_trash, mut old_trash) = ((0, 0), (0, 0));
    files(&root, &root, &mut actual, &mut host_trash);
    files(&old_root, &old_root, &mut expected, &mut old_trash);
    assert!(
        actual == expected,
        "graph regular-file set or bytes differ outside recovery trash"
    );
    eprintln!("anonymized namespace acceptance: {} regular files identical; host trash files/bytes={host_trash:?}, legacy={old_trash:?}; host deletions retain moved originals", actual.len());
}
