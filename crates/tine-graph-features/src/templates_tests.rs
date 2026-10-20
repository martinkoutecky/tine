use super::*;
use std::path::Path;
use std::time::Duration;
use tine_store::OpenOptions;

fn graph(files: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for dir in ["pages", "journals", "logseq"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for (rel, text) in files {
        let path = root.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

/// Graph-wide discovery's answer, as the daily template path used it:
/// the first listed template whose name equals `name`.
fn discovered(store: &Store, name: &str) -> Option<TemplateDto> {
    store
        .whole_graph()
        .unwrap()
        .templates()
        .into_iter()
        .find(|t| t.name == name)
}

fn settled(store: &Store, name: &str) -> Option<TemplateDto> {
    match named_template_from_files(store, name, || false).unwrap() {
        NamedTemplate::Settled(found) => found,
        NamedTemplate::Unsettled => panic!("{name:?} was not settled from files"),
    }
}

fn json(found: &Option<TemplateDto>) -> serde_json::Value {
    serde_json::to_value(found).unwrap()
}

/// One definition, none, and the per-block rules (key case, Org drawers,
/// parent inclusion, ids, nested templates, exact name) match graph-wide
/// discovery block for block.
#[test]
fn a_single_or_absent_definition_matches_graph_wide_discovery() {
    let root = graph(&[
        (
            "pages/Daily templates.md",
            "- Daily\n  Template:: Daily Plan\n  template-including-parent:: false\n  - Prepare <%today%>\n    id:: 6500aaaa-0000-4000-8000-000000000001\n  - Notes\n    collapsed:: true\n    - hidden\n",
        ),
        (
            "pages/Org templates.org",
            "* Weekly\n:PROPERTIES:\n:template: Weekly Review\n:id: 6500aaaa-0000-4000-8000-000000000002\n:END:\n** Look back\n",
        ),
        (
            "pages/Nested.md",
            "- outer\n  template:: Outer\n  - inner\n    template:: Inner one\n",
        ),
        ("journals/2026_01_02.md", "- journal\n  template:: From a journal\n  - row\n"),
        ("pages/Mentions.md", "- this page talks about a template but defines none\n"),
        ("pages/Plain.md", "- nothing here\n"),
    ]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    for name in [
        "Daily Plan",
        "Weekly Review",
        "Outer",
        "Inner one",
        "From a journal",
        "daily plan",
        "Daily  Plan",
        "Absent",
    ] {
        let early = settled(&store, name);
        assert_eq!(json(&early), json(&discovered(&store, name)), "{name}");
    }
    let daily = settled(&store, "Daily Plan").unwrap();
    assert_eq!(daily.page, "Daily templates");
    assert_eq!(daily.blocks.len(), 2, "children only, the root is excluded");
    assert_eq!(daily.blocks[0].raw, "Prepare <%today%>", "id:: is dropped");
    assert!(daily.blocks[1].collapsed);
    assert!(
        settled(&store, "daily plan").is_none(),
        "names are case-sensitive"
    );
    store.close();
}

/// Two definitions of one name: graph-wide discovery takes the first in the
/// graph's page order, which the files do not show, so the files do not
/// settle it (the caller waits for the graph, as before).
#[test]
fn two_definitions_of_the_name_are_left_to_the_graph() {
    let root = graph(&[
        ("pages/A.md", "- one\n  template:: Daily\n  - from A\n"),
        ("pages/B.md", "- two\n  template:: Daily\n  - from B\n"),
        ("pages/C.md", "- three\n  template:: Other\n"),
    ]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    assert!(matches!(
        named_template_from_files(&store, "Daily", || false).unwrap(),
        NamedTemplate::Unsettled
    ));
    let root2 = graph(&[(
        "pages/Twice.md",
        "- one\n  template:: Daily\n- two\n  template:: Daily\n",
    )]);
    let store2 = Store::open(root2.path(), OpenOptions::default()).unwrap().0;
    assert!(matches!(
        named_template_from_files(&store2, "Daily", || false).unwrap(),
        NamedTemplate::Unsettled
    ));
    // A different name in the same graph is still settled.
    assert_eq!(
        json(&settled(&store, "Other")),
        json(&discovered(&store, "Other"))
    );
    store.close();
    store2.close();
}

/// A definition in a duplicate-day journal (a stray the graph does not list
/// as a page), or a file that cannot be read, leaves the answer to the graph.
#[test]
fn a_stray_day_or_an_unreadable_candidate_is_left_to_the_graph() {
    let root = graph(&[
        ("journals/2026_01_02.md", "- canonical\n"),
        ("journals/2026-01-02.md", "- stray\n  template:: Daily\n"),
    ]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    assert!(matches!(
        named_template_from_files(&store, "Daily", || false).unwrap(),
        NamedTemplate::Unsettled
    ));
    store.close();
    let root = graph(&[("pages/Bad.md", "- x\n  template:: Daily\n")]);
    std::fs::write(
        root.path().join("pages/Bad.md"),
        b"- x\n  template:: Daily caf\xe9\n",
    )
    .unwrap();
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    assert!(matches!(
        named_template_from_files(&store, "Daily", || false).unwrap(),
        NamedTemplate::Unsettled
    ));
    store.close();
}

#[test]
fn a_raised_stop_is_unsettled() {
    let root = graph(&[("pages/A.md", "- one\n  template:: Daily\n")]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    assert!(matches!(
        named_template_from_files(&store, "Daily", || true).unwrap(),
        NamedTemplate::Unsettled
    ));
    store.close();
}

/// BqbeQ fail-before: the old lookup was graph-wide discovery, which waits for
/// the initial parse. The file lookup answers while that parse is held.
#[test]
fn the_named_template_is_found_while_the_initial_parse_is_held() {
    let root = graph(&[
        (
            "pages/Daily templates.md",
            "- Daily\n  template:: Daily\n  - row\n",
        ),
        ("pages/Other.md", "- other\n"),
    ]);
    let hold = root.path().join(".tine-test-pause-load");
    std::fs::write(&hold, "").unwrap();
    let store = std::sync::Arc::new(Store::open(root.path(), OpenOptions::default()).unwrap().0);
    assert!(matches!(store.is_graph_ready(), Ok(false)));
    let (send, receive) = std::sync::mpsc::channel();
    let asking = std::sync::Arc::clone(&store);
    std::thread::spawn(move || {
        let _ = send.send(named_template_from_files(&asking, "Daily", || false).unwrap());
    });
    let early = receive.recv_timeout(Duration::from_secs(5));
    let still_held = matches!(store.is_graph_ready(), Ok(false));
    std::fs::remove_file(&hold).unwrap();
    let early = early.expect("the template lookup waited for the graph-wide parse");
    assert!(still_held);
    let NamedTemplate::Settled(Some(found)) = early else {
        panic!("not settled: {early:?}")
    };
    assert_eq!(json(&Some(found)), json(&discovered(&store, "Daily")));
    store.close();
}

/// The equivalence oracle on a real-shaped graph: `TINE_TEMPLATE_ORACLE_GRAPH`
/// names a graph directory, which is copied first (the original is never
/// opened). Every template name in it, and one absent name, gets the
/// graph-wide answer or is left unsettled only for a repeated name.
#[test]
#[ignore = "needs TINE_TEMPLATE_ORACLE_GRAPH; run by hand on the corpora"]
fn named_template_from_files_matches_discovery_on_a_corpus() {
    let source = std::env::var("TINE_TEMPLATE_ORACLE_GRAPH").expect("TINE_TEMPLATE_ORACLE_GRAPH");
    let copy = tempfile::tempdir().unwrap();
    copy_tree(Path::new(&source), copy.path());
    let store = Store::open(copy.path(), OpenOptions::default()).unwrap().0;
    let all = store.whole_graph().unwrap().templates();
    let mut names: Vec<String> = all.iter().map(|t| t.name.clone()).collect();
    names.sort();
    names.dedup();
    names.push("no template has this name 7f3c".into());
    let (mut settled_count, mut unsettled_count) = (0, 0);
    for name in &names {
        let count = all.iter().filter(|t| &t.name == name).count();
        match named_template_from_files(&store, name, || false).unwrap() {
            NamedTemplate::Settled(found) => {
                assert!(count <= 1, "{name:?} is defined {count} times");
                assert_eq!(json(&found), json(&discovered(&store, name)), "{name:?}");
                settled_count += 1;
            }
            NamedTemplate::Unsettled => {
                assert!(
                    count > 1,
                    "{name:?} (defined {count} times) was not settled"
                );
                unsettled_count += 1;
            }
        }
    }
    eprintln!(
        "template oracle: {} names, {settled_count} settled, {unsettled_count} repeated",
        names.len()
    );
    store.close();
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            std::fs::create_dir_all(&target).unwrap();
            copy_tree(&entry.path(), &target);
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// GH discussion #729 (OG parity): a `template::` in a page's property header
/// (the leading properties block of a Markdown page, the `#+KEY:` / head-drawer
/// header of an Org page) defines a template, as OG's `get-all-templates` /
/// `get-template-by-name` match any entity with `:block/properties :template`,
/// the page's pre-block included. The listing and the file lookup agree.
#[test]
fn a_page_property_header_template_is_listed_and_found_from_files() {
    let root = graph(&[
        (
            "pages/Usual note.md",
            "tags::  \nlink:: \ncreate-date:: <% today %>\ntemplate:: Usual note\n",
        ),
        (
            "pages/With rows.md",
            "template:: Page with rows\nicon:: x\n\n- first row\n- second row\n",
        ),
        (
            "pages/Parentless.md",
            "template:: No parent\ntemplate-including-parent:: false\n\n- row\n",
        ),
        (
            "pages/Org header.org",
            "#+TITLE: Org header\n#+TEMPLATE: Org page template\n\n* Row\n",
        ),
        (
            "pages/Org drawer.org",
            ":PROPERTIES:\n:template: Org drawer template\n:END:\n#+TITLE: Org drawer\n\n* Row\n",
        ),
        ("pages/Empty.md", "template::\n\n- row\n"),
    ]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    let all = store.whole_graph().unwrap().templates();
    let mut names: Vec<&str> = all.iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "No parent",
            "Org drawer template",
            "Org page template",
            "Page with rows",
            "Usual note"
        ]
    );
    for name in &names {
        assert_eq!(
            json(&settled(&store, name)),
            json(&discovered(&store, name)),
            "{name}"
        );
    }
    let usual = discovered(&store, "Usual note").unwrap();
    assert_eq!(usual.page, "Usual note");
    assert_eq!(usual.blocks.len(), 1);
    // The header block, `template::` stripped; the dynamic variable is left for
    // the insertion step (the frontend expands `<% today %>`).
    assert_eq!(
        usual.blocks[0].raw,
        "tags:: \nlink:: \ncreate-date:: <% today %>"
    );
    // OG's `get-template-by-name` resolves a page-header template to the
    // pre-block entity, which has no children: only the header is inserted.
    let rows = discovered(&store, "Page with rows").unwrap();
    assert_eq!(rows.blocks.len(), 1);
    assert_eq!(rows.blocks[0].raw, "icon:: x");
    assert!(rows.blocks[0].children.is_empty());
    // `template-including-parent:: false` drops the root: nothing remains.
    assert!(discovered(&store, "No parent").unwrap().blocks.is_empty());
    store.close();
}

/// The bullet forms of the same report already worked; they stay pinned: a
/// bulleted first block with an empty property line before `template::`.
#[test]
fn a_bulleted_first_block_with_an_empty_property_line_is_a_template() {
    let root = graph(&[
        ("pages/Bullet.md", "- tags::  \n  template:: Bulleted\n"),
        (
            "pages/Empty first.md",
            "- hello\n  tags::  \n  template:: Empty first\n  - child\n",
        ),
    ]);
    let store = Store::open(root.path(), OpenOptions::default()).unwrap().0;
    for name in ["Bulleted", "Empty first"] {
        let found = discovered(&store, name).unwrap_or_else(|| panic!("{name} not listed"));
        assert_eq!(json(&Some(found)), json(&settled(&store, name)), "{name}");
    }
    store.close();
}
