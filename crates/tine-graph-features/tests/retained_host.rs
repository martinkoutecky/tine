//! Census writers with a page host (STEP3 §7, R6). With no unsaved input, a
//! writer under the host's reservations (or, for a single page's rename,
//! the host's own operation) leaves the graph byte-for-byte as the plain
//! write does and returns the same result; and a page another reservation
//! holds blocks the writer until it is released.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_core::model::PageKind;
use tine_core::pdf::{self, Highlight};
use tine_graph_features::{conflicts, guide, journals, pages, pdf as features_pdf};
use tine_store::{Input, PageHost, PageId, Store};

const CONFIG: &str = "{:file/name-format :triple-lowbar\n :default-home {:page \"Start\"}}\n";

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-retained-host-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn graph(label: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = scratch(label);
    for dir_name in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(dir.join(dir_name)).unwrap();
    }
    fs::write(dir.join("logseq/config.edn"), CONFIG).unwrap();
    for (rel, text) in files {
        fs::write(dir.join(rel), text).unwrap();
    }
    dir
}

/// The graph's text files and asset sidecars (trash locations carry stamps).
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for area in ["pages", "journals", "assets"] {
        for entry in fs::read_dir(dir.join(area)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let rel = format!("{area}/{}", path.file_name().unwrap().to_string_lossy());
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out.insert(
        "logseq/config.edn".into(),
        fs::read(dir.join("logseq/config.edn")).unwrap(),
    );
    out
}

/// Run `op` on two copies of one graph, without and with a running host,
/// and require the same result and the same bytes on disk.
fn same_with_a_host(
    label: &str,
    files: &[(&str, &str)],
    op: impl Fn(&Store, Option<&PageHost>) -> String,
) -> String {
    let plain = graph(&format!("{label}-plain"), files);
    let hosted = graph(&format!("{label}-hosted"), files);
    let app_data = scratch(&format!("{label}-app"));
    fs::create_dir_all(&app_data).unwrap();
    let store = Store::open(&plain, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let expected = op(&store, None);
    store.close();
    let store = Arc::new(Store::open(&hosted, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &app_data).unwrap();
    let got = op(&store, Some(&host));
    drop(host);
    store.close();
    assert_eq!(
        got, expected,
        "{label}: a writer's result changed under a host"
    );
    assert_eq!(
        tree(&hosted),
        tree(&plain),
        "{label}: a writer's bytes changed under a host"
    );
    for dir in [plain, hosted, app_data] {
        let _ = fs::remove_dir_all(dir);
    }
    expected
}

fn ok(result: String) {
    assert!(result.starts_with("Ok"), "{result}");
}

fn highlight(id: &str) -> Highlight {
    let rect = pdf::Rect {
        top: 1.0,
        left: 1.0,
        width: 2.0,
        height: 2.0,
        source_width: None,
        source_height: None,
    };
    Highlight {
        id: id.into(),
        page: 1,
        position: pdf::Position {
            page: 1,
            bounding: rect.clone(),
            rects: vec![rect],
        },
        color: "yellow".into(),
        text: Some("quoted".into()),
        image: None,
    }
}

#[test]
fn page_renames_with_a_host_write_what_the_plain_rename_writes() {
    let files = [
        ("pages/Start.md", "- home [[Other]]\n"),
        (
            "pages/Other.md",
            "- see [[Start]] and [[Ghost]] and [[Ns/child]]\n",
        ),
        ("pages/Ns.md", "- ns\n"),
        ("pages/Ns___child.md", "- child of [[Ns]]\n"),
        ("pages/Into.md", "- into\n"),
        ("pages/Merged.md", "alias:: Joined\n\n- merged body\n"),
    ];
    let rename = |old: &'static str, new: &'static str, into: Option<&'static str>| {
        move |store: &Store, host: Option<&PageHost>| {
            format!(
                "{:?}",
                pages::rename_or_merge_page(store, host, old, new, None, into, &[])
            )
        }
    };
    // A single page with a file: the host's own operation, then config.edn.
    let report = same_with_a_host("single", &files, rename("Start", "Begin", None));
    assert!(
        report.contains("Renamed") && report.contains("Begin"),
        "{report}"
    );
    // A name with no file: references only.
    ok(same_with_a_host(
        "ghost",
        &files,
        rename("Ghost", "Spirit", None),
    ));
    // A namespace and a merge: retained transactions under reservations.
    ok(same_with_a_host(
        "namespace",
        &files,
        rename("Ns", "Space", None),
    ));
    ok(same_with_a_host(
        "merge",
        &files,
        rename("Merged", "Into", Some("pages/Into.md")),
    ));
    // A target another page owns refuses alike.
    let refused = same_with_a_host("owned", &files, rename("Start", "Other", None));
    assert!(refused.starts_with("Err"), "{refused}");
}

#[test]
fn other_census_writers_with_a_host_write_what_the_plain_writers_write() {
    let files = [
        ("pages/A.md", "- a body\n"),
        ("pages/B.md", "- b body\n"),
        ("pages/Gone.md", "- going\n"),
        ("pages/stray name.md", "- stray\n"),
        (
            "pages/Marked.md",
            "- before\n<<<<<<< ours\n- mine\n=======\n- theirs\n>>>>>>> theirs\n",
        ),
        (
            "pages/A.sync-conflict-20240101-000000-ABCDEFG.md",
            "- copy\n",
        ),
        ("journals/Jun 19th, 2026.md", "- day\n"),
        ("journals/Jun 20th, 2026.md", "- other day\n"),
    ];
    ok(same_with_a_host("merge-pages", &files, |store, host| {
        format!(
            "{:?}",
            pages::merge_pages(store, host, "pages/A.md", "pages/B.md")
        )
    }));
    ok(same_with_a_host("delete", &files, |store, host| {
        format!(
            "{:?}",
            pages::delete_page_expected(store, host, "Gone", PageKind::Page, None, None)
        )
    }));
    ok(same_with_a_host("rescue", &files, |store, host| {
        format!(
            "{:?}",
            pages::rename_file_to_page(store, host, "pages/stray name.md", "Rescued")
        )
    }));
    ok(same_with_a_host("trash-copy", &files, |store, host| {
        format!(
            "{:?}",
            conflicts::trash_sync_conflict(
                store,
                host,
                "pages/A.sync-conflict-20240101-000000-ABCDEFG.md"
            )
        )
    }));
    ok(same_with_a_host("markers", &files, |store, host| {
        let diff = conflicts::vcs_marker_conflict_diff(store, "pages/Marked.md")
            .unwrap()
            .expect("markers");
        fn mine(rows: &[tine_core::sync_diff::DiffRow], out: &mut HashMap<String, String>) {
            for row in rows {
                out.insert(row.id.clone(), "mine".to_owned());
                mine(&row.children, out);
            }
        }
        let mut decisions = HashMap::new();
        mine(&diff.diff.rows, &mut decisions);
        let resolved = conflicts::resolve_vcs_marker_conflict(
            store,
            host,
            "pages/Marked.md",
            &decisions,
            &diff.diff.base_rev,
            "union",
        );
        assert!(resolved.is_ok(), "{resolved:?}");
        format!("{resolved:?}")
    }));
    ok(same_with_a_host("trash-journal", &files, |store, host| {
        format!(
            "{:?}",
            journals::trash_journal_file(store, host, "Jun 20th, 2026.md")
        )
    }));
    ok(same_with_a_host("migrate", &files, |store, host| {
        let listed = journals::journal_filename_migrations(store).unwrap();
        format!(
            "{:?}",
            journals::migrate_journal_filenames(store, host, &listed)
        )
    }));
    ok(same_with_a_host("highlights", &files, |store, host| {
        format!(
            "{:?}",
            features_pdf::write_highlights(
                store,
                host,
                "paper.pdf",
                "Paper",
                &[highlight("h1")],
                &[]
            )
        )
    }));
    ok(same_with_a_host("guide", &files, |store, host| {
        format!(
            "{:?}",
            guide::copy_guide_into_graph(store, host, "Tine Guide")
        )
    }));
}

/// A writer reserves the pages it touches: while another reservation holds
/// `page`, `write` waits, and it writes once that reservation is released;
/// `written` then holds.
fn waits_for_a_reservation(
    label: &str,
    files: &[(&str, &str)],
    page: &str,
    write: impl Fn(&Store, &PageHost) -> std::io::Result<()> + Sync,
    written: impl Fn(&Path) -> bool,
) {
    let dir = graph(label, files);
    let app_data = scratch(&format!("{label}-app"));
    fs::create_dir_all(&app_data).unwrap();
    let store = Arc::new(Store::open(&dir, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &app_data).unwrap();
    let held = host
        .reserve(|| vec![PageId::from(page)], Input::Refuse)
        .expect("nothing unsaved");
    let before = tree(&dir);
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host, write) = (&store, &host, &write);
        scope.spawn(move || {
            done.send(write(store, host).map_err(|error| error.to_string()))
                .unwrap();
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(500)).is_err(),
            "R6: {label} wrote while another reservation holds {page}"
        );
        assert_eq!(
            tree(&dir),
            before,
            "{label}: wrote under a held reservation"
        );
        drop(held);
        finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the writer runs once the page is released")
            .unwrap();
    });
    assert!(written(&dir), "{label}: the writer's write is missing");
    drop(host);
    store.close();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(app_data);
}

#[test]
fn a_census_writer_waits_for_a_reservation_of_its_page() {
    waits_for_a_reservation(
        "fence",
        &[("pages/A.md", "- a\n"), ("pages/B.md", "- b\n")],
        "pages/B.md",
        |store, host| pages::merge_pages(store, Some(host), "pages/A.md", "pages/B.md").map(drop),
        |dir| {
            fs::read_to_string(dir.join("pages/B.md"))
                .unwrap()
                .contains("- a")
        },
    );
}

/// A-K2: the Guide copy reserves the page it creates, so a page the host
/// holds as a fileless draft is not created behind it.
#[test]
fn the_guide_copy_waits_for_a_reservation_of_its_page() {
    let page = format!(
        "pages/{}.md",
        tine_core::model::encode_page_name(
            &tine_core::guide::guide_copy_page_name("Tine Guide"),
            tine_core::config::Config::parse(CONFIG).file_name_format
        )
    );
    waits_for_a_reservation(
        "guide-fence",
        &[],
        &page,
        |store, host| guide::copy_guide_into_graph(store, Some(host), "Tine Guide").map(drop),
        |dir| dir.join(&page).is_file(),
    );
}
