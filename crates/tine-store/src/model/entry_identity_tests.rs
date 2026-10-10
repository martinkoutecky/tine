//! B1: one identity rule for held lookup, the host and the watcher. The
//! store-level tests use only APIs the pre-B1 tree also has (`hold_page`,
//! `Graph::source`, builds), so they run unchanged at the baseline.
//! Native coverage (macOS NFD normalization, Windows 8.3 short names,
//! per-directory case flags) runs only on those platforms.

use super::*;
use crate::model::{content_rev, Source};
use crate::store::Store;
use crate::PageId;
use std::sync::Arc;

/// A graph with `files`, opened on a non-canonical spelling of its root,
/// its whole graph loaded.
fn graph(files: &[(&str, &[u8])]) -> (tempfile::TempDir, Arc<Store>) {
    let temp = tempfile::tempdir().unwrap();
    for area in ["pages", "journals"] {
        fs::create_dir_all(temp.path().join(area)).unwrap();
    }
    for (rel, bytes) in files {
        fs::write(temp.path().join(rel), bytes).unwrap();
    }
    let given = temp.path().join("pages").join("..");
    let store = Arc::new(Store::open(&given, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    (temp, store)
}

fn is_held(source: Source) -> bool {
    !matches!(source, Source::Disk)
}

/// The rows a forced rebuild installs at `path`: their revisions.
fn rows(store: &Store, path: &Path) -> Vec<String> {
    assert!(store.graph.rebuild_cache_cancellable(|| false));
    store.graph.with_pages(|pages| {
        pages
            .iter()
            .filter(|(entry, _)| entry.path == path)
            .map(|(entry, _)| store.graph.cached_rev(&entry.path).unwrap_or_default())
            .collect()
    })
}

fn folds_case(dir: &Path) -> bool {
    let probe = dir.join(".Identity-Probe");
    fs::write(&probe, b"").unwrap();
    let folds = dir.join(".identity-probe").exists();
    fs::remove_file(&probe).unwrap();
    folds
}

/// An ancestor symlink or a `..` spelling of a held page's directory names
/// the held page: it is sourced from the owner's bytes, never the file.
#[cfg(unix)]
#[test]
fn an_ancestor_alias_names_the_held_page() {
    let (temp, store) = graph(&[("pages/a.md", b"- a\n")]);
    std::os::unix::fs::symlink(temp.path().join("pages"), temp.path().join("link")).unwrap();
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let root = &store.graph.root;
    for alias in [
        root.join("link/a.md"),
        root.join("journals/../pages/a.md"),
        temp.path().join("pages/../pages/a.md"),
    ] {
        assert!(
            is_held(store.graph.source(&alias)),
            "B1: {} reaches the held page through an ancestor alias but was sourced from disk",
            alias.display()
        );
    }
    store.close();
}

/// Two names a normalization-sensitive directory lists apart (NFC and NFD
/// `café`) are two pages: holding one never hides or substitutes the other.
#[cfg(target_os = "linux")]
#[test]
fn nfc_and_nfd_names_listed_apart_are_distinct_pages() {
    let (nfc, nfd) = ("pages/caf\u{e9}.md", "pages/cafe\u{301}.md");
    let (_temp, store) = graph(&[(nfc, b"- composed\n"), (nfd, b"- decomposed\n")]);
    store.hold_page(&PageId::from(nfc)).unwrap();
    let root = store.graph.root.clone();
    fs::write(root.join(nfd), "- decomposed, edited\n").unwrap();
    assert!(!is_held(store.graph.source(&root.join(nfd))));
    assert_eq!(rows(&store, &root.join(nfc)), [content_rev("- composed\n")]);
    assert_eq!(
        rows(&store, &root.join(nfd)),
        [content_rev("- decomposed, edited\n")],
        "B1: an NFD name listed apart from a held NFC name is its own page"
    );
    store.close();
}

/// Case-distinct files in a case-sensitive directory are distinct pages.
#[test]
fn case_distinct_files_in_a_sensitive_directory_are_distinct_pages() {
    let (temp, store) = graph(&[("pages/a.md", b"- lower\n")]);
    if folds_case(&temp.path().join("pages")) {
        eprintln!("SKIP: pages/ folds case on this volume");
        store.close();
        return;
    }
    fs::write(temp.path().join("pages/A.md"), "- upper\n").unwrap();
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let root = store.graph.root.clone();
    assert!(!is_held(store.graph.source(&root.join("pages/A.md"))));
    assert_eq!(
        rows(&store, &root.join("pages/A.md")),
        [content_rev("- upper\n")]
    );
    assert_eq!(
        rows(&store, &root.join("pages/a.md")),
        [content_rev("- lower\n")]
    );
    store.close();
}

/// Two separately listed hard links that collide with a held page's name
/// are one file under two entries: their identity is unknown, so the other
/// entry is never installed from disk while the page is held (B1).
#[cfg(unix)]
#[test]
fn a_colliding_hard_link_is_never_installed_from_disk() {
    let (temp, store) = graph(&[("pages/a.md", b"- a\n")]);
    if folds_case(&temp.path().join("pages")) {
        eprintln!("SKIP: pages/ folds case on this volume");
        store.close();
        return;
    }
    store.hold_page(&PageId::from("pages/a.md")).unwrap();
    let root = store.graph.root.clone();
    fs::hard_link(root.join("pages/a.md"), root.join("pages/A.md")).unwrap();
    fs::write(root.join("pages/A.md"), "- through the link\n").unwrap();
    assert!(
        is_held(store.graph.source(&root.join("pages/A.md"))),
        "B1: a colliding hard link of a held page was sourced from disk"
    );
    assert!(
        rows(&store, &root.join("pages/A.md")).is_empty(),
        "B1: an entry of unknown identity was installed from disk"
    );
    assert_eq!(
        rows(&store, &root.join("pages/a.md")),
        [content_rev("- a\n")]
    );
    store.close();
}

/// The rule itself, through `Graph::identify` (new in B1, so no baseline).
#[test]
fn identify_decides_by_entry() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("pages")).unwrap();
    fs::write(temp.path().join("pages/a.md"), "- a\n").unwrap();
    let graph = Graph::open(temp.path());
    let root = graph.root.clone();
    let spellings = Spellings::default();
    spellings.spell("pages/a.md", "pages/a.md");
    let registered = |fold: &str| spellings.candidates(fold, |_| true);
    let identify = |rel: &str| graph.identify(&root.join(rel), &registered);
    assert_eq!(identify("pages/a.md"), Identity::Key("pages/a.md".into()));
    assert_eq!(identify("pages/b.md"), Identity::New);
    assert_eq!(identify("journals/a.md"), Identity::New);
    assert_eq!(
        graph.identify(&root.join("../a.md"), &registered),
        Identity::Outside
    );
    assert_eq!(
        graph.identify(&root.join("../pages/a.md"), &registered),
        Identity::Outside
    );
    if folds_case(&root.join("pages")) {
        // A case alias of the entry on a folding volume: Q4's alias.
        assert_eq!(
            identify("pages/A.md"),
            Identity::Unknown {
                candidates: vec!["pages/a.md".into()],
                alias: Some("pages/a.md".into()),
            }
        );
    } else {
        // An absent colliding name: unknown, and no alias.
        assert_eq!(
            identify("pages/A.md"),
            Identity::Unknown {
                candidates: vec!["pages/a.md".into()],
                alias: None,
            }
        );
        fs::write(root.join("pages/A.md"), "- A\n").unwrap();
        assert_eq!(identify("pages/A.md"), Identity::New);
    }
    // A respelled key is found at its spelling, not at its key.
    spellings.spell("pages/a.md", "pages/x.md");
    assert_eq!(identify("pages/a.md"), Identity::New);
    fs::rename(root.join("pages/a.md"), root.join("pages/x.md")).unwrap();
    assert_eq!(identify("pages/x.md"), Identity::Key("pages/a.md".into()));
    // `ß` and `SS` meet in the collision filter.
    assert_eq!(
        fold_leaf(OsStr::new("STRASSE.md")),
        fold_leaf(OsStr::new("straße.md"))
    );
}

/// Native coverage of this rule runs only on Apple platforms; on Linux these
/// cases are reported, never counted as proof.
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[test]
fn macos_nfd_spelling_of_a_held_page_names_it() {
    let (_temp, store) = graph(&[("pages/caf\u{e9}.md", b"- a\n")]);
    store
        .hold_page(&PageId::from("pages/caf\u{e9}.md"))
        .unwrap();
    let alias = store.graph.root.join("pages/cafe\u{301}.md");
    assert!(is_held(store.graph.source(&alias)));
    store.close();
}
