//! GJ5 equality oracle: a canonical text image of what the initial build
//! derives from a graph (every block projection and the snapshot indexes the
//! first publication builds). Two builds of one graph copy are equal exactly
//! when their images are byte-equal. Page slots and per-page name sets are
//! keyed by path and sorted, because the parallel load assigns slots in
//! completion order and `collect_document_referenced_names` returns its names
//! in hash order; neither order carries meaning.
use super::*;

fn block_lines(out: &mut String, block: &DocBlock, depth: usize) {
    out.push_str(&format!(
        "  block {depth} {:?} {:?} {}\n",
        block.uuid,
        block.raw(),
        serde_json::to_string(block.projection()).unwrap()
    ));
    for child in &block.children {
        block_lines(out, child, depth + 1);
    }
}

/// The canonical image of `graph`'s initial build.
pub(super) fn initial_build_image(graph: &Graph) -> String {
    graph.with_pages(|_| ());
    let snapshot = ReadSnapshot::capture(
        graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    let config = graph.config.clone();
    let mut out = String::new();
    let mut rel_by_slot = HashMap::new();
    let mut pages: Vec<(String, &PageEntry, &Arc<Document>)> = Vec::new();
    for (slot, (entry, doc)) in snapshot.pages.slots() {
        let rel = entry
            .rel_path
            .as_ref()
            .map(|rel| rel.as_str().to_owned())
            .unwrap_or_default();
        rel_by_slot.insert(slot, rel.clone());
        pages.push((rel, entry, doc));
    }
    pages.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, entry, doc) in &pages {
        out.push_str(&format!(
            "page {rel:?} {:?} {:?} pre={:?}\n",
            entry.kind, entry.name, doc.pre_block
        ));
        for block in &doc.roots {
            block_lines(&mut out, block, 0);
        }
        let mut referenced = collect_document_referenced_names(doc);
        referenced.sort();
        out.push_str(&format!("  referenced {referenced:?}\n"));
        out.push_str(&format!(
            "  explicit {:?}\n",
            crate::query::document_explicit_reference_names(entry, doc, &config)
        ));
        let mut counts: Vec<_> = document_block_ref_counts(doc).into_iter().collect();
        counts.sort();
        out.push_str(&format!("  block_ref_counts {counts:?}\n"));
    }
    for (shard, map) in snapshot.explicit_index.shards.iter().enumerate() {
        for (name, paths) in map.iter() {
            let paths: Vec<_> = paths.iter().map(|(path, _)| path).collect();
            out.push_str(&format!("explicit {shard} {name:?} {paths:?}\n"));
        }
    }
    let candidates = snapshot.reference_candidate_index.read().unwrap();
    out.push_str(&format!(
        "candidates page_count={} complete={}\n",
        candidates.page_count, candidates.complete
    ));
    let mut signatures: Vec<_> = candidates
        .signatures
        .iter()
        .map(|(slot, signatures)| (rel_by_slot[slot].clone(), signatures))
        .collect();
    signatures.sort_by(|a, b| a.0.cmp(&b.0));
    for (rel, signatures) in signatures {
        out.push_str(&format!(
            "signatures {rel:?} {}\n",
            serde_json::to_string(signatures.as_ref()).unwrap()
        ));
    }
    drop(candidates);
    out.push_str(&format!(
        "aliases {}\n",
        serde_json::to_string(snapshot.alias_index.get().unwrap()).unwrap()
    ));
    out.push_str(&format!(
        "block_ref_counts {}\n",
        serde_json::to_string(snapshot.block_ref_counts().as_ref()).unwrap()
    ));
    out.push_str(&format!(
        "real_page_names {}\n",
        serde_json::to_string(snapshot.real_page_names.as_ref()).unwrap()
    ));
    for (name, keys) in snapshot.transliteration_names.iter() {
        out.push_str(&format!("name_keys {name:?} {keys:?}\n"));
    }
    for (name, owners) in snapshot.transliteration_owners.iter() {
        out.push_str(&format!("name_owners {name:?} {owners}\n"));
    }
    // The icon index keys pages by slot; its slot-free projection is the
    // icon each page name resolves to.
    let names: Vec<String> = pages
        .iter()
        .map(|(_, entry, _)| entry.name.clone())
        .collect();
    let mut icons: Vec<_> = snapshot.page_icons(&names).into_iter().collect();
    icons.sort();
    out.push_str(&format!("icons {icons:?}\n"));
    out
}

/// `TINE_INDEX_ORACLE_GRAPH=<graph copy> TINE_INDEX_ORACLE_OUT=<file>
/// cargo test -p tine-store index_oracle_image -- --ignored` writes the image;
/// run it on the base and on the candidate and compare the files with `cmp`.
#[test]
#[ignore = "oracle: needs TINE_INDEX_ORACLE_GRAPH and TINE_INDEX_ORACLE_OUT"]
fn index_oracle_image() {
    let root = std::env::var("TINE_INDEX_ORACLE_GRAPH").expect("TINE_INDEX_ORACLE_GRAPH");
    let out = std::env::var("TINE_INDEX_ORACLE_OUT").expect("TINE_INDEX_ORACLE_OUT");
    let graph = Graph::open(&root);
    fs::write(out, initial_build_image(&graph)).unwrap();
}

/// More pages than the parallel threshold, with every explicit-name source
/// (links, tags, namespaces, page and block property keys and values, task
/// markers, aliases, Org) and Han names whose spellings share one page key.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(
        root.join("pages/kitchen-sink.md"),
        include_str!("../../../../src/fixtures/kitchen-sink.md"),
    )
    .unwrap();
    fs::write(
        root.join("pages/中文页面.md"),
        "alias:: 汉字别名, Über 中文\n\n- 读书 [[中文页面]] [[学习/数学]] #标签 [[豈]] [[\u{F900}]]\n  分类:: [[中文页面]]\n",
    )
    .unwrap();
    fs::write(
        root.join("pages/notes.org"),
        "#+title: Notes\n* TODO [[Org Target]] :orgtag:\n** child [[ns/a/b]]\n",
    )
    .unwrap();
    for i in 0..80 {
        fs::write(
            root.join(format!("pages/P{i:02}.md")),
            format!(
                "tags:: hub{}, [[Shared]]\n\n- TODO [[Topic {}]] #tag{} [[ns/a/{}]]\n  status:: open\n  Custom_Key:: [[Topic {}]]\n  - DONE child ((nope)) [[Topic {}]] [[P{:02}]]\n- [[topic {}]] again\n",
                i % 3,
                i % 7,
                i % 5,
                i % 4,
                i % 9,
                i % 7,
                (i + 1) % 80,
                i % 7
            ),
        )
        .unwrap();
    }
    fs::write(
        root.join("journals/2026_10_18.md"),
        "- [[P01]] [[中文页面]] [[Topic 3]] [[\u{8C48}]]\n",
    )
    .unwrap();
    dir
}

/// GJ5 S1/S4: the parallel bulk build of the explicit-reference index (and
/// the initial capture that uses it) equals inserting each page's names in
/// turn, the former serial build.
#[test]
fn explicit_index_build_equals_inserting_each_page() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    graph.with_pages(|_| ());
    let pages = graph.peek_pages().unwrap();
    assert!(pages.len() >= 64 && page_cache_worker_count() > 1);
    let mut reference = SnapshotExplicitIndex {
        shards: (0..SnapshotExplicitIndex::SHARDS)
            .map(|_| Arc::new(SharedMap::new()))
            .collect(),
    };
    for (entry, doc) in pages.iter() {
        let names = crate::query::document_explicit_reference_names(entry, doc, &graph.config);
        reference.update(&entry.path, &[], &names);
    }
    let reference = serde_json::to_string(&reference).unwrap();
    assert!(reference.contains("topic 3") && reference.contains("ns/a"));
    let built = SnapshotExplicitIndex::build(&pages, &graph.config);
    assert_eq!(serde_json::to_string(&built).unwrap(), reference);
    let snapshot = ReadSnapshot::capture(
        &graph,
        graph.config.clone(),
        Arc::new(EntryList::from(graph.list_pages_shared().as_slice())),
        None,
        &[],
    );
    assert_eq!(
        serde_json::to_string(&snapshot.explicit_index).unwrap(),
        reference
    );
}

/// GJ5 S6: transliteration ownership filters referenced names before copying
/// them; it must equal collecting every referenced name, then filtering.
#[test]
fn name_keys_equal_collecting_then_filtering() {
    let dir = fixture();
    let graph = Graph::open(dir.path());
    let image = initial_build_image(&graph);
    let pages = graph.peek_pages().unwrap();
    let mut reference: std::collections::BTreeMap<String, usize> = Default::default();
    for (page, doc) in pages.iter() {
        let names: std::collections::BTreeSet<String> = std::iter::once(page.name.clone())
            .chain(crate::query::document_alias_spellings(doc))
            .chain(collect_document_referenced_names(doc))
            .filter_map(|name| crate::transliteration::cache_identity(&name))
            .collect();
        for name in names {
            *reference.entry(name).or_default() += 1;
        }
    }
    assert!(reference.contains_key("\u{8C48}") && reference.contains_key("über 中文"));
    let owners: String = reference
        .iter()
        .map(|(name, owners)| format!("name_owners {name:?} {owners}\n"))
        .collect();
    let built: String = image
        .lines()
        .filter(|line| line.starts_with("name_owners "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(built, owners);
}
