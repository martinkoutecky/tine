//! og checkpoint 5, packet P2: page rename/merge keeps page identity and literal
//! preamble text (REG-OG-C5-L03-S1, REG-OG-C5-L03-S2, REG-OG-C5-L03-S3).
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::pages;
use tine_store::Store;

/// Renames are the page host's (STEP3 §7): each runs through a host started
/// for it under its own app data, as the app's graph binding runs one.
fn hosted<T>(
    store: &std::sync::Arc<tine_store::Store>,
    run: impl FnOnce(&tine_store::PageHost) -> T,
) -> T {
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(store, app_data.path()).unwrap();
    run(&host)
}

fn fixture(label: &str, files: &[(&str, &str)]) -> (PathBuf, std::sync::Arc<Store>) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-og-c5-p2-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(&root, Default::default()).unwrap().0);
    (root, store)
}

const LOWBAR: &str = "{:file/name-format :triple-lowbar}\n";

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

fn owner(store: &Store, name: &str) -> Option<String> {
    match store.whole_graph().unwrap().resolve(name, false) {
        tine_store::Resolved::Existing { id, .. } => Some(id.as_str().to_owned()),
        tine_store::Resolved::Alias { owners } => Some(owners[0].as_str().to_owned()),
        tine_store::Resolved::Absent { .. } => None,
    }
}

/// L03-S1: the parent's identity matches a descendant by its normalized key, so
/// the descendant's suffix must come from the descendant's own namespace
/// structure, never from the parent's unnormalized character count. A
/// decomposed parent (`e` + U+0301) with a composed child (`é/x`) renamed to
/// `New` must move the child to `New/x`, not `Newx`. Same for a parent typed
/// with a boundary slash, which the page key strips.
#[test]
fn namespace_rename_keeps_the_child_suffix_across_spellings() {
    for (label, parent_title, child_title, typed_old) in [
        ("nfd-parent", "e\u{301}", "\u{e9}/x", "e\u{301}"),
        ("nfc-parent", "\u{e9}", "e\u{301}/x", "\u{e9}"),
        ("case", "PARENT", "parent/x", "PARENT"),
        ("boundary-slash", "Parent", "Parent/x", "/Parent"),
    ] {
        let (root, store) = fixture(
            label,
            &[
                ("logseq/config.edn", LOWBAR),
                (
                    "pages/Parent.md",
                    &format!("title:: {parent_title}\n\n- parent\n"),
                ),
                (
                    "pages/Child.md",
                    &format!("title:: {child_title}\n\n- child\n"),
                ),
                ("pages/Ref.md", &format!("- see [[{child_title}]]\n")),
            ],
        );
        hosted(&store, |host| {
            pages::rename_page_expected(&store, host, typed_old, "New", None)
        })
        .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(
            owner(&store, "New/x").as_deref(),
            Some("pages/New___x.md"),
            "{label}: the descendant must become New/x"
        );
        assert!(
            owner(&store, "Newx").is_none(),
            "{label}: the descendant must not become Newx"
        );
        let child = read(&root, "pages/New___x.md");
        assert!(child.contains("title:: New/x"), "{label}: {child}");
        assert_eq!(read(&root, "pages/Ref.md"), "- see [[New/x]]\n", "{label}");
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}

/// L03-S2: a fenced `key:: value` example in the source preamble is literal
/// text (parser-owned region), never a header property: it is neither dropped
/// because the survivor has the same property, nor hoisted out of its fence
/// into the survivor's header. Both entrances: the rename-merge and the plain
/// (duplicate-journal) merge.
#[test]
fn merge_keeps_a_fenced_preamble_line_inside_its_fence() {
    let fenced = "```\nnote:: keep me\n```";
    for (label, survivor) in [
        ("equal", "note:: keep me\n\n- destination body\n"),
        ("absent", "- destination body\n"),
    ] {
        for plain in [false, true] {
            let (root, store) = fixture(
                label,
                &[
                    ("pages/Old.md", &format!("{fenced}\n- source body\n")),
                    ("pages/New.md", survivor),
                ],
            );
            if plain {
                pages::merge_pages(&store, None, "pages/Old.md", "pages/New.md").unwrap();
            } else {
                hosted(&store, |host| {
                    pages::rename_or_merge_page(
                        &store,
                        host,
                        "Old",
                        "New",
                        None,
                        Some("pages/New.md"),
                        &[],
                    )
                })
                .unwrap();
            }
            let merged = read(&root, "pages/New.md");
            let at = format!("{label}, plain merge {plain}");
            // Block text is written under its bullet, indented: compare the
            // block's own lines.
            let unindented: Vec<&str> = merged
                .lines()
                .map(|line| {
                    let line = line.trim_start();
                    line.strip_prefix("- ").unwrap_or(line)
                })
                .collect();
            assert!(
                unindented.join("\n").contains(fenced),
                "{at}: the fenced example must survive inside its fence:\n{merged}"
            );
            let header_lines = merged
                .lines()
                .filter(|line| *line == "note:: keep me")
                .count();
            assert_eq!(
                header_lines,
                usize::from(label == "equal"),
                "{at}: no fenced line may become a header property:\n{merged}"
            );
            assert!(merged.contains("- source body"), "{at}:\n{merged}");
            assert!(merged.contains("- destination body"), "{at}:\n{merged}");
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

/// L03-S2, Org sibling: a `#+KEY:` line inside an Org literal block is not a
/// directive, so the merge never drops it as equal to the survivor's directive.
#[test]
fn org_merge_keeps_a_directive_example_inside_its_block() {
    let example = "#+BEGIN_EXAMPLE\n#+CATEGORY: work\n#+END_EXAMPLE";
    let (root, store) = fixture(
        "org-example",
        &[
            ("pages/Old.org", &format!("{example}\n* source block\n")),
            ("pages/New.org", "#+CATEGORY: work\n* survivor block\n"),
        ],
    );
    hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Old", "New", None, Some("pages/New.org"), &[])
    })
    .unwrap();
    let merged = read(&root, "pages/New.org");
    assert!(merged.contains(example), "{merged}");
    assert!(merged.starts_with("#+CATEGORY: work\n"), "{merged}");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

/// L03-S3: the rename-merge appends every source block, as OG `merge-pages!`
/// does, even when the survivor already holds an identical block (Martin,
/// 2026-10-05, option (a)). The old crash-retry skip treated a source whose
/// blocks equal the survivor's trailing blocks as already merged, and silently
/// dropped the source's copy on a first merge.
#[test]
fn rename_merge_keeps_a_source_block_equal_to_a_survivor_block() {
    let (root, store) = fixture(
        "identical-block",
        &[
            ("pages/Old.md", "- repeated\n"),
            ("pages/New.md", "- only in new\n- repeated\n"),
        ],
    );
    hosted(&store, |host| {
        pages::rename_or_merge_page(&store, host, "Old", "New", None, Some("pages/New.md"), &[])
    })
    .unwrap();
    let merged = read(&root, "pages/New.md");
    assert_eq!(merged.matches("- repeated").count(), 2, "{merged}");
    assert!(merged.contains("- only in new"), "{merged}");
    drop(store);
    fs::remove_dir_all(root).unwrap();
}
