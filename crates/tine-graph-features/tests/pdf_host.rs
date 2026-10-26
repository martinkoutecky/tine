//! W5 `write_highlights` with a page host (host-only-writer P3, DESIGN §3.1,
//! §3.4, §3.7). The sidecar EDN is written first, then the annotation page
//! (and a migrated legacy page's deletion) as one host operation. With no
//! unsaved input the graph's bytes and the result equal the plain writer's;
//! the page the writer actually writes, under any spelling, is the host's to
//! admit; and a page that changes after the plan is never overwritten.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use tine_core::pdf::{self, Highlight};
use tine_graph_features::pdf as features_pdf;
use tine_store::{Input, PageHost, PageId, Store};

const CONFIG: &str = "{:file/name-format :triple-lowbar}\n";

fn graph(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for sub in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), CONFIG).unwrap();
    for (rel, text) in files {
        fs::write(root.join(rel), text).unwrap();
    }
    (dir, root)
}

fn open(root: &Path) -> Arc<Store> {
    let store = Arc::new(Store::open(root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    store
}

/// The top-level files of the page and asset folders (trash locations
/// carry stamps).
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for area in ["pages", "assets"] {
        for entry in fs::read_dir(root.join(area)).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                let rel = format!("{area}/{}", path.file_name().unwrap().to_string_lossy());
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    out
}

fn highlight(id: &str, color: &str, image: Option<i64>) -> Highlight {
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
        color: color.into(),
        text: image.is_none().then(|| format!("text of {id}")),
        image,
    }
}

const A: &str = "00000000-0000-4000-8000-00000000000a";
const B: &str = "00000000-0000-4000-8000-00000000000b";
const C: &str = "00000000-0000-4000-8000-00000000000c";

fn sidecar(entries: &[(&str, &str)]) -> String {
    let entries: Vec<String> = entries
        .iter()
        .map(|(id, text)| {
            format!(
                r#"{{:id #uuid "{id}" :page 1 :position {{:page 1 :bounding {{:top 1 :left 1 :width 2 :height 2}} :rects ({{:top 1 :left 1 :width 2 :height 2}})}} :content {{:text "{text}"}} :properties {{:color "yellow"}}}}"#
            )
        })
        .collect();
    format!("{{:highlights [{}] :extra {{}}}}\n", entries.join(" "))
}

fn page(pdf: &str, ids: &[(&str, &str)]) -> String {
    let mut page = format!("file:: [Paper](../assets/{pdf})\nfile-path:: ../assets/{pdf}\n\n");
    for (id, text) in ids {
        page.push_str(&format!(
            "- {text}\n  hl-page:: 1\n  hl-color:: yellow\n  ls-type:: annotation\n  id:: {id}\n\t- my note on {text}\n"
        ));
    }
    page
}

/// Run `write` on two copies of one graph, without and with a running host;
/// the result and the bytes on disk are the same.
fn same_with_a_host(
    label: &str,
    files: &[(&str, &str)],
    write: impl Fn(&Store, Option<&PageHost>, &Path) -> std::io::Result<Vec<Highlight>>,
) -> String {
    let (_plain_dir, plain) = graph(files);
    let (_hosted_dir, hosted) = graph(files);
    let app_data = tempfile::tempdir().unwrap();
    let store = open(&plain);
    let expected = format!("{:?}", write(&store, None, &plain));
    store.close();
    let store = open(&hosted);
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let got = format!("{:?}", write(&store, Some(&host), &hosted));
    drop(host);
    store.close();
    assert_eq!(got, expected, "{label}: the result changed under a host");
    assert_eq!(
        tree(&hosted),
        tree(&plain),
        "{label}: the bytes changed under a host"
    );
    expected
}

/// The writer's outcomes with a host equal the plain writer's: an addition
/// and a recolour on a page with the user's notes, a deleted area highlight
/// whose crop is retired, a migration from the legacy key (its sidecar
/// retired, its page's text carried to the new name and the old page
/// deleted), and a non-round-tripping Org page refused with nothing written.
#[test]
fn highlight_writes_with_a_host_equal_the_plain_writer() {
    let _integration_guard = crate::integration_lock();
    let paper = [
        ("assets/paper.edn", sidecar(&[(A, "aye")])),
        ("pages/hls__paper.md", page("paper.pdf", &[(A, "aye")])),
    ];
    let paper: Vec<(&str, &str)> = paper.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let loaded = vec![highlight(A, "yellow", None)];
    let result = same_with_a_host("add", &paper, |store, host, _root| {
        let next = vec![highlight(A, "red", None), highlight(B, "yellow", None)];
        features_pdf::write_highlights(store, host, "paper.pdf", "Paper", &next, &loaded)
    });
    assert!(result.starts_with("Ok"), "{result}");

    let crop = "assets/paper/1_".to_owned() + C + "_42.png";
    let area = highlight(C, "yellow", Some(42));
    let with_area = [
        (
            "assets/paper.edn",
            pdf::write_highlights(std::slice::from_ref(&area), ""),
        ),
        ("pages/hls__paper.md", page("paper.pdf", &[])),
    ];
    let with_area: Vec<(&str, &str)> = with_area.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let result = same_with_a_host("crop", &with_area, |store, host, root| {
        fs::create_dir_all(root.join("assets/paper")).unwrap();
        fs::write(root.join(&crop), b"png").unwrap();
        let written = features_pdf::write_highlights(
            store,
            host,
            "paper.pdf",
            "Paper",
            &[],
            std::slice::from_ref(&area),
        );
        assert!(!root.join(&crop).exists(), "the crop is retired");
        written
    });
    assert!(result.starts_with("Ok"), "{result}");

    let legacy = [
        ("assets/my_paper.edn", sidecar(&[(A, "aye")])),
        (
            "pages/hls__my_paper.md",
            page("My Paper.pdf", &[(A, "aye")]),
        ),
    ];
    let legacy: Vec<(&str, &str)> = legacy.iter().map(|(r, t)| (*r, t.as_str())).collect();
    let result = same_with_a_host("legacy", &legacy, |store, host, root| {
        let loaded = features_pdf::read_highlights_checked(store, "My Paper.pdf").unwrap();
        let mut next = loaded.clone();
        next.push(highlight(B, "yellow", None));
        let written =
            features_pdf::write_highlights(store, host, "My Paper.pdf", "Paper", &next, &loaded);
        assert!(!root.join("pages/hls__my_paper.md").exists());
        assert!(!root.join("assets/my_paper.edn").exists());
        let moved = fs::read_to_string(root.join("pages/hls__My Paper.md")).unwrap();
        assert!(
            moved.contains("my note on aye") && moved.contains(B),
            "{moved}"
        );
        written
    });
    assert!(result.starts_with("Ok"), "{result}");

    let org = [("pages/hls__paper.org", "* a\n*** skipped level\n")];
    let result = same_with_a_host("org", &org, |store, host, _root| {
        let next = vec![highlight(A, "yellow", None)];
        features_pdf::write_highlights(store, host, "paper.pdf", "Paper", &next, &[])
    });
    assert!(result.contains("PermissionDenied"), "{result}");
}

/// Hold `held` under another reservation while `write` runs on another
/// thread: it does not write `held` (and has not returned) within 500 ms;
/// `during` then runs, the reservation is released, and `write` finishes.
fn while_held(
    host: &PageHost,
    root: &Path,
    held: &str,
    write: impl FnOnce() -> std::io::Result<Vec<Highlight>> + Send,
    during: impl FnOnce(),
) {
    let reservation = host
        .reserve(|| vec![PageId::from(held)], Input::Refuse)
        .expect("nothing unsaved");
    let before = fs::read_to_string(root.join(held)).ok();
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        scope.spawn(move || {
            let _ = done.send(write().map_err(|error| error.to_string()));
        });
        let early = finished.recv_timeout(Duration::from_millis(500));
        assert_eq!(
            fs::read_to_string(root.join(held)).ok(),
            before,
            "{held} was written while another reservation holds it"
        );
        assert!(
            early.is_err(),
            "the writer returned while {held} is held: {early:?}"
        );
        during();
        drop(reservation);
        finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the writer finishes once the page is released")
            .unwrap();
    });
}

/// DESIGN §3.7 (CENSUS W5 [ambiguous]): for a PDF key that the page file
/// name encodes (`%` here), the annotation page is created under its
/// encoded file, outside the unencoded names the writer used to reserve.
/// With a host, the operation names the page it writes, so the host admits
/// that page: a reservation of it holds the writer's page write until it
/// is released.
#[test]
fn the_annotation_page_under_its_encoded_name_is_admitted_by_the_host() {
    let _integration_guard = crate::integration_lock();
    let (_dir, root) = graph(&[]);
    let store = open(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let next = vec![highlight(A, "yellow", None)];
    while_held(
        &host,
        &root,
        ENCODED,
        || features_pdf::write_highlights(&store, Some(&host), "half%.pdf", "Half", &next, &[]),
        || {},
    );
    let written = fs::read_to_string(root.join(ENCODED)).unwrap();
    assert!(written.contains(A), "{written}");
    assert!(!root.join("pages/hls__half%.md").exists());
    drop(host);
    store.close();
}

/// The encoded annotation page of a PDF with a `%` in its key.
const ENCODED: &str = "pages/hls__half%25.md";

/// Two honest savers: A saves unchanged yellow and parks, B saves blue and
/// publishes, then A resumes. A must merge B's sidecar again on its replan.
#[test]
fn a_page_replan_merges_the_other_savers_sidecar_again() {
    let _integration_guard = crate::integration_lock();
    let loaded = vec![highlight(A, "yellow", None)];
    let (_dir, root) = graph(&[
        ("assets/half%.edn", &pdf::write_highlights(&loaded, "")),
        (ENCODED, &page("half%.pdf", &[(A, "text of a")])),
    ]);
    let store = open(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    while_held(
        &host,
        &root,
        ENCODED,
        || {
            features_pdf::write_highlights(
                &store,
                Some(&host),
                "half%.pdf",
                "Half",
                &loaded,
                &loaded,
            )
        },
        || {
            let other = open(&root);
            let blue = vec![highlight(A, "blue", None)];
            features_pdf::write_highlights(&other, None, "half%.pdf", "Half", &blue, &loaded)
                .unwrap();
            other.close();
        },
    );
    let sidecar = features_pdf::read_highlights_checked(&store, "half%.pdf").unwrap();
    assert_eq!(sidecar[0].color, "blue");
    let written = fs::read_to_string(root.join(ENCODED)).unwrap();
    assert!(
        written.contains("hl-color:: blue"),
        "sidecar blue must have a blue annotation: {written}"
    );
}

/// A second highlight save to a PDF whose annotation page has an encoded
/// file name updates that page, with and without a host. The page is found
/// under its encoded name only by resolving the page name, and the writer
/// used to drop its text there, so it planned a creation over the existing
/// file and every save after the first failed.
#[test]
fn a_second_highlight_save_to_an_encoded_annotation_page_succeeds() {
    let _integration_guard = crate::integration_lock();
    let result = same_with_a_host("second save", &[], |store, host, root| {
        let first = vec![highlight(A, "yellow", None)];
        features_pdf::write_highlights(store, host, "half%.pdf", "Half", &first, &[])
            .expect("the first save creates the page");
        let mut page = fs::read_to_string(root.join(ENCODED)).unwrap();
        page.push_str("- my own block\n");
        fs::write(root.join(ENCODED), page).unwrap();
        let next = vec![highlight(A, "red", None), highlight(B, "yellow", None)];
        let written =
            features_pdf::write_highlights(store, host, "half%.pdf", "Half", &next, &first);
        let page = fs::read_to_string(root.join(ENCODED)).unwrap();
        assert!(
            page.contains(A) && page.contains(B) && page.contains("- my own block"),
            "{page}"
        );
        assert!(!root.join("pages/hls__half%.md").exists());
        written
    });
    assert!(result.starts_with("Ok"), "{result}");
}

/// The same page open in the window before it has a file (the host holds
/// it): the first save creates it through the host's operation.
#[test]
fn a_first_highlight_save_with_its_encoded_annotation_page_open_succeeds() {
    let _integration_guard = crate::integration_lock();
    let (_dir, root) = graph(&[]);
    let store = open(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let reloaded = serde_json::to_value(host.window_reloaded()).unwrap();
    let session = reloaded["session"].as_u64().unwrap();
    let id = reloaded["nextId"].as_u64().unwrap();
    host.open(session, id, &PageId::from(ENCODED), "hls__half%")
        .unwrap();
    let next = vec![highlight(A, "yellow", None)];
    let saved =
        features_pdf::write_highlights(&store, Some(&host), "half%.pdf", "Half", &next, &[]);
    assert_eq!(saved.map_err(|error| error.to_string()), Ok(next));
    let written = fs::read_to_string(root.join(ENCODED)).unwrap();
    assert!(written.contains(A), "{written}");
    drop(host);
    store.close();
}

/// DESIGN §3.1, §3.4: the sidecar (the highlight data) is saved before the
/// annotation page's operation, and the page's operation carries the bytes
/// it was planned from. A page an external editor changes while the
/// operation waits for admission refuses it (`Changed`); the writer plans
/// again from the changed page, so the edit survives beside the new block.
#[test]
fn an_annotation_page_changed_after_the_plan_is_not_overwritten() {
    let _integration_guard = crate::integration_lock();
    let (_dir, root) = graph(&[
        ("assets/paper.edn", &sidecar(&[(A, "aye")])),
        ("pages/hls__paper.md", &page("paper.pdf", &[(A, "aye")])),
    ]);
    let store = open(&root);
    let app_data = tempfile::tempdir().unwrap();
    let host = PageHost::start_for_tests(&store, app_data.path()).unwrap();
    let loaded = vec![highlight(A, "yellow", None)];
    let next = vec![highlight(A, "yellow", None), highlight(B, "yellow", None)];
    let held = "pages/hls__paper.md";
    while_held(
        &host,
        &root,
        held,
        || {
            features_pdf::write_highlights(
                &store,
                Some(&host),
                "paper.pdf",
                "Paper",
                &next,
                &loaded,
            )
        },
        || {
            // `while_held`'s timeout only proves the writer has not returned;
            // under load it may not have reached the sidecar save yet.
            let deadline = Instant::now() + Duration::from_secs(20);
            let saved = loop {
                let saved = fs::read_to_string(root.join("assets/paper.edn")).unwrap();
                if saved.contains(B) || Instant::now() >= deadline {
                    break saved;
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(
                saved.contains(B),
                "§3.4 pre: the sidecar is saved before the page's operation"
            );
            let mut edited = fs::read_to_string(root.join(held)).unwrap();
            edited.push_str("- an external edit\n");
            fs::write(root.join(held), edited).unwrap();
        },
    );
    let written = fs::read_to_string(root.join(held)).unwrap();
    assert!(
        written.contains("- an external edit") && written.contains("my note on aye"),
        "the change after the plan was overwritten: {written}"
    );
    assert!(written.contains(B), "{written}");
    drop(host);
    store.close();
}
