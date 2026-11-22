//! A memory graph of every file answers as the store does on disk: built by
//! `WholeGraph::subgraph` from the store's view (the export) or by
//! `WholeGraph::from_files` from the bytes alone (the browser).
use crate::{InventoryScope, OpenOptions, Store, WatchMode, WholeGraph};
use std::path::Path;
use std::sync::Arc;
use tine_core::doc::Document;

/// Every file under `root`, graph-relative, with its bytes (hidden and
/// non-page files included: the listing must leave them out itself).
fn files_under(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    out
}

/// Each file parsed as the export hands its documents to `subgraph`.
fn documents(files: Vec<(String, Vec<u8>)>) -> Vec<(String, Arc<Document>)> {
    (files.into_iter())
        .map(|(rel, bytes)| {
            let text = String::from_utf8_lossy(&bytes);
            let document = match tine_core::model::Format::from_path(Path::new(&rel)) {
                tine_core::model::Format::Org => tine_core::org::parse_org(&text),
                _ => tine_core::doc::parse(&text),
            };
            (rel, Arc::new(document))
        })
        .collect()
}

/// What the two views answer, with each view's root replaced by `<root>`
/// (page times only when the memory graph carries them).
fn answers(graph: &WholeGraph, root: &str, times: bool) -> Vec<String> {
    let clean = |text: String| text.replace(root, "<root>");
    let inventory = graph.inventory(InventoryScope::All);
    let mut out = vec![clean(format!("{inventory:?}"))];
    if times {
        let mut modified = graph.page_modified();
        modified.sort();
        out.push(format!("{modified:?}"));
    }
    for active in [true, false] {
        let corpus = graph.corpus(None, active);
        let mut pages: Vec<_> = (corpus.pages.iter())
            .map(|page| format!("{} {} {:?}", page.id.as_str(), page.name, page.kind))
            .collect();
        pages.sort();
        out.push(pages.join("\n"));
    }
    for entry in &inventory.0 {
        out.push(clean(format!(
            "{} -> {:?} / {:?}",
            entry.name,
            graph.resolve(&entry.name, entry.is_journal),
            graph.backlinks(&entry.name).map(|groups| groups.len())
        )));
    }
    for text in ["a", "p", "2026", "fold"] {
        let names: Vec<_> = (graph
            .complete_page_names(text, 50, None, false, None)
            .into_iter())
        .map(|(entry, class)| format!("{} {class:?}", entry.name))
        .collect();
        out.push(names.join(", "));
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Build {
    Subgraph,
    Files,
}

fn assert_same_answers(root: &Path, build: Build) -> Vec<String> {
    let (store, _, _) = Store::open(
        root,
        OpenOptions {
            approved_external_assets: None,
            watch: WatchMode::Poll,
            launch_checkpoint: None,
        },
    )
    .unwrap();
    let disk = store.whole_graph().unwrap();
    let disk_root = Store::canonical_root(root).unwrap().display().to_string();
    let (memory, meta) = match build {
        Build::Subgraph => disk.subgraph(documents(files_under(root))),
        Build::Files => WholeGraph::from_files(files_under(root)),
    };
    assert_eq!(meta.root, crate::model::file_source::MEMORY_ROOT);
    let times = build == Build::Subgraph;
    let (disk, memory) = (
        answers(&disk, &disk_root, times),
        answers(&memory, crate::model::file_source::MEMORY_ROOT, times),
    );
    store.close();
    assert!(disk.len() > 4, "the fixture has pages");
    for (disk, memory) in disk.iter().zip(&memory) {
        assert_eq!(disk, memory);
    }
    assert_eq!(disk.len(), memory.len());
    memory
}

#[test]
fn the_demo_graph_answers_alike() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph");
    assert_same_answers(&root, Build::Subgraph);
    assert_same_answers(&root, Build::Files);
}

#[test]
fn naming_edge_cases_answer_alike() {
    let temp = tempfile::tempdir().unwrap();
    naming_fixture(temp.path());
    for build in [Build::Subgraph, Build::Files] {
        check_naming(&assert_same_answers(temp.path(), build));
    }
}

/// The browser's bytes reach the same unreadable-file answers as the disk:
/// a body that is not UTF-8 after a settled title, and an unreadable
/// preferred journal with a same-day claimant.
#[test]
fn unreadable_files_answer_alike_from_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    naming_fixture(root);
    let mut broken = b"title:: Broken Title\n\n- fine\n- body \xff\xfe\n".to_vec();
    std::fs::write(root.join("pages/Broken.md"), &broken).unwrap();
    broken = b"#+title: Broken Org\r\n* fine\r\n* body \xff\r\n".to_vec();
    std::fs::write(root.join("pages/BrokenOrg.org"), &broken).unwrap();
    std::fs::write(root.join("journals/2026_11_05.md"), b"- fine\n- day \xff\n").unwrap();
    let answers = assert_same_answers(root, Build::Files);
    check_naming(&answers);
    assert!(answers[0].contains("Broken"), "{}", answers[0]);
}

fn naming_fixture(root: &Path) {
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "logseq/config.edn",
        "{:journal/page-title-format \"yyyy-MM-dd\" :hidden [\"private\"] :file/name-format :triple-lowbar}",
    );
    write(
        "journals/2026_11_02.md",
        "- a day linking [[Alpha]] and #Beta\n",
    );
    write("journals/2026_11_03.md", "- another day [[Gamma]]\n");
    write(
        "pages/Alpha.md",
        "alias:: First, Premier\n\n- alpha links [[Beta]]\n",
    );
    write("pages/beta.md", "- lower-case file, [[alpha]]\n");
    write(
        "pages/Titled.md",
        "title:: Real Name\n\n- [[Premier]] via alias\n",
    );
    write(
        "pages/2026-11-02.md",
        "- a page whose file name is a date\n",
    );
    write(
        "pages/Day.md",
        "title:: 2026-11-04\n\n- a day by its title\n",
    );
    write("pages/ns___child.md", "- a namespace child of [[ns]]\n");
    write(
        "pages/Org page.org",
        "#+title: Org Title\n* org block linking [[Alpha]]\n",
    );
    write("pages/Café.md", "- composed é [[Cafe\u{301}]]\n");
    write(
        "pages/Alpha.sync-conflict-20261101-120000-ABCDEFG.md",
        "- a conflict copy\n",
    );
    write("pages/notes.txt", "not a page\n");
    write("private/Hidden.md", "- hidden by config\n");
    write(".hidden/Dot.md", "- in a dot directory\n");
    write("logseq/bak/pages/Alpha.md", "- a backup\n");
    write("pages/deep/nested/Leaf.md", "- nested [[Alpha]]\n");
    // A same-day claimant beside the preferred journal. (An unreadable file
    // cannot reach `subgraph`, which takes parsed documents.)
    write("journals/2026_11_05.md", "- fine\n- day\n");
    write("pages/2026-11-05.md", "- the same day as a page\n");
}

/// The answers exercised what the fixture is for: titles and org titles name
/// pages, aliases resolve, and the listing leaves out hidden, backup,
/// dot-directory, conflict and non-page files.
fn check_naming(answers: &[String]) {
    let pages = answers.iter().find(|a| a.starts_with("journals/")).unwrap();
    for named in ["Real Name", "Org Title", "Leaf", "2026-11-04"] {
        assert!(pages.contains(named), "{named} missing from\n{pages}");
    }
    for absent in ["Hidden", "Dot", "sync-conflict", "notes", "bak"] {
        assert!(!pages.contains(absent), "{absent} listed in\n{pages}");
    }
    assert!(
        answers[0].contains("Premier"),
        "alias missing: {}",
        answers[0]
    );
}

/// A flat directory of 20k pages lists in time linear in its size (each
/// file joins its directory once; a sibling search took 7 s here).
#[test]
fn a_flat_directory_builds_in_linear_time() {
    let files: Vec<_> = (0..20_000)
        .map(|n| {
            (
                format!("pages/p{n}.md"),
                crate::model::file_source::Content::Bytes(Arc::from(&b"- x\n"[..])),
                None,
            )
        })
        .collect();
    let began = std::time::Instant::now();
    let built = crate::model::file_source::Files::new(Path::new("/r"), files);
    let elapsed = began.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
    drop(built);
}
