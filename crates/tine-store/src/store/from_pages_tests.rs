//! `WholeGraph::from_pages` answers as a store opened over the same files.
use crate::{InventoryScope, OpenOptions, Store, WatchMode, WholeGraph};
use std::path::Path;

/// Every file under `root`, graph-relative, with its bytes and time (hidden
/// and non-page files included: the listing must leave them out itself).
fn files_under(root: &Path) -> Vec<(String, Vec<u8>, Option<std::time::SystemTime>)> {
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
                let modified = std::fs::metadata(&path).unwrap().modified().ok();
                out.push((rel, std::fs::read(&path).unwrap(), modified));
            }
        }
    }
    out
}

/// What the two views answer, with each view's root replaced by `<root>`.
fn answers(graph: &WholeGraph, root: &str) -> Vec<String> {
    let clean = |text: String| text.replace(root, "<root>");
    let inventory = graph.inventory(InventoryScope::All);
    let mut out = vec![clean(format!("{inventory:?}"))];
    let mut modified = graph.page_modified();
    modified.sort();
    out.push(format!("{modified:?}"));
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

fn assert_same_answers(root: &Path) -> Vec<String> {
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
    let (memory, meta) = WholeGraph::from_pages(files_under(root)).unwrap();
    assert_eq!(meta.root, crate::model::file_source::MEMORY_ROOT);
    let (disk, memory) = (
        answers(&disk, &disk_root),
        answers(&memory, crate::model::file_source::MEMORY_ROOT),
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
    assert_same_answers(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/demo-graph"));
}

#[test]
fn naming_edge_cases_answer_alike() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
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
    // Unreadable bodies after a settled title: the launch pass refuses the
    // read and the file keeps its file-name claim, in memory as on disk.
    let mut broken = b"title:: Broken Title\n\n- fine\n- body \xff\xfe\n".to_vec();
    std::fs::write(root.join("pages/Broken.md"), &broken).unwrap();
    broken = b"#+title: Broken Org\r\n* fine\r\n* body \xff\r\n".to_vec();
    std::fs::write(root.join("pages/BrokenOrg.org"), &broken).unwrap();
    let answers = assert_same_answers(root);
    // The answers exercised what the fixture is for: titles and org titles
    // name pages, aliases resolve, and the listing leaves out hidden,
    // backup, dot-directory, conflict and non-page files.
    let pages = &answers[2];
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

#[test]
fn an_unsafe_layout_is_refused_as_a_store_open_refuses_it() {
    for config in [
        "{:pages-directory \"../outside\"}",
        "{:journals-directory \"/abs\"}",
        "{:pages-directory \"\"}",
        "{:pages-directory \"a\\\\b\"}",
    ] {
        let files = vec![(
            "logseq/config.edn".to_owned(),
            config.as_bytes().to_vec(),
            None,
        )];
        let error = WholeGraph::from_pages(files).err().expect(config);
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{config}");
    }
    let nested = "{:pages-directory \"notes/pages\"}";
    let files = vec![(
        "logseq/config.edn".to_owned(),
        nested.as_bytes().to_vec(),
        None,
    )];
    assert!(WholeGraph::from_pages(files).is_ok());
}

/// A flat directory of 20k pages lists in time linear in its size (each
/// file joins its directory once; a sibling search took 7 s here).
#[test]
fn a_flat_directory_builds_in_linear_time() {
    let files: Vec<_> = (0..20_000)
        .map(|n| (format!("pages/p{n}.md"), b"- x\n".to_vec(), None))
        .collect();
    let began = std::time::Instant::now();
    let built = crate::model::file_source::Files::new(Path::new("/r"), files);
    let elapsed = began.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
    drop(built);
}
