//! host-only-writer P3 (W4): with a page host, the Guide copy creates each
//! page as one host operation over no file (DESIGN §3.1). A page that
//! already exists is skipped, never overwritten, also when its file appears
//! after the copy planned the page and before the host applied it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use tine_graph_features::guide;
use tine_store::{Input, PageHost, PageId, Store};

const CONFIG: &str = "{:file/name-format :triple-lowbar}\n";

struct Hosted {
    _dir: tempfile::TempDir,
    root: PathBuf,
    store: Arc<Store>,
    host: PageHost,
}

fn hosted(files: &[(&str, &str)]) -> Hosted {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("graph");
    for area in ["pages", "journals", "logseq", "assets"] {
        fs::create_dir_all(root.join(area)).unwrap();
    }
    fs::write(root.join("logseq/config.edn"), CONFIG).unwrap();
    for (rel, text) in files {
        fs::write(root.join(rel), text).unwrap();
    }
    let store = Arc::new(Store::open(&root, Default::default()).unwrap().0);
    store.whole_graph().unwrap();
    let host = PageHost::start_for_tests(&store, &dir.path().join("app")).unwrap();
    Hosted {
        _dir: dir,
        root,
        store,
        host,
    }
}

/// The copy of the Guide's first page, "Tine Guide": its name and file.
fn first_copy() -> (String, String) {
    let name = tine_core::guide::guide_copy_page_name("Tine Guide");
    let format = tine_core::config::Config::parse(CONFIG).file_name_format;
    let rel = format!(
        "pages/{}.md",
        tine_core::model::encode_page_name(&name, format)
    );
    (name, rel)
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

/// The normal outcome with a host: every absent Guide page is created (its
/// bytes equal the plain copy's: `retained_host.rs`), an existing one keeps
/// its bytes and is reported skipped, and a second copy creates nothing.
#[test]
fn with_a_host_the_guide_copy_creates_absent_pages_and_skips_existing_ones() {
    let _integration_guard = crate::integration_lock();
    let (name, rel) = first_copy();
    let g = hosted(&[(&rel, "- mine\n")]);
    let copied = guide::copy_guide_into_graph(&g.store, Some(&g.host), "Tine Guide").unwrap();
    assert_eq!(copied.skipped_pages, [name]);
    assert_eq!(
        copied.created_pages.len() + 1,
        tine_core::guide::GUIDE_TEMPLATES.len(),
        "{copied:?}"
    );
    assert!(copied.created);
    assert_eq!(read(&g.root, &rel), "- mine\n", "an existing page is kept");
    let format = tine_core::config::Config::parse(CONFIG).file_name_format;
    for created in &copied.created_pages {
        let text = read(
            &g.root,
            &format!(
                "pages/{}.md",
                tine_core::model::encode_page_name(created, format)
            ),
        );
        assert!(!text.is_empty(), "{created} is empty");
    }
    let again = guide::copy_guide_into_graph(&g.store, Some(&g.host), "Tine Guide").unwrap();
    assert!(again.created_pages.is_empty(), "{again:?}");
    assert!(!again.created, "{again:?}");
    assert_eq!(
        again.skipped_pages.len(),
        tine_core::guide::GUIDE_TEMPLATES.len()
    );
}

/// DESIGN §3.1, the create shape: the copy plans the first page's copy over
/// no file, and the host cannot apply it while another reservation holds
/// the page. Its file appears meanwhile (an external editor or a sync
/// delivery). Released, the operation finds the page no longer absent
/// (`Changed`), the copy replans, and reports the page skipped: the
/// file's bytes are never overwritten.
#[test]
fn a_guide_page_that_appears_after_the_plan_is_never_overwritten() {
    let _integration_guard = crate::integration_lock();
    let (name, rel) = first_copy();
    let g = hosted(&[]);
    let held = g
        .host
        .reserve(|| vec![PageId::from(rel.as_str())], Input::Refuse)
        .expect("nothing unsaved");
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host) = (&*g.store, &g.host);
        scope.spawn(move || {
            let copied = guide::copy_guide_into_graph(store, Some(host), "Tine Guide");
            done.send(copied.map_err(|error| error.to_string()))
                .unwrap();
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(500)).is_err(),
            "the copy wrote while another reservation holds {rel}"
        );
        assert!(!g.root.join(&rel).exists());
        fs::write(g.root.join(&rel), "- arrived\n").unwrap();
        drop(held);
        let copied = finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the copy runs once the page is released")
            .unwrap();
        assert!(copied.skipped_pages.contains(&name), "{copied:?}");
        assert!(!copied.created_pages.contains(&name), "{copied:?}");
    });
    assert_eq!(read(&g.root, &rel), "- arrived\n", "never overwritten");
}

/// `legacy_onboarding.rs`'s symlink swap with a host: a pages folder
/// swapped for a link to a folder outside the graph after the open refuses
/// the copy, and nothing is written outside the graph.
#[cfg(unix)]
#[test]
fn with_a_host_a_swapped_pages_folder_refuses_the_copy() {
    let _integration_guard = crate::integration_lock();
    let g = hosted(&[]);
    let outside = tempfile::tempdir().unwrap();
    fs::remove_dir(g.root.join("pages")).unwrap();
    std::os::unix::fs::symlink(outside.path(), g.root.join("pages")).unwrap();
    let copied = guide::copy_guide_into_graph(&g.store, Some(&g.host), "Tine Guide");
    assert!(copied.is_err(), "{copied:?}");
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

/// `client.rs`'s existing-path cases with a host: a folder where a Guide
/// page's file would go is an existing page, skipped as the transaction's
/// `IsADirectory` was.
#[test]
fn with_a_host_a_folder_at_a_guide_page_path_is_skipped() {
    let _integration_guard = crate::integration_lock();
    let (name, rel) = first_copy();
    let g = hosted(&[]);
    fs::create_dir(g.root.join(&rel)).unwrap();
    let copied = guide::copy_guide_into_graph(&g.store, Some(&g.host), "Tine Guide").unwrap();
    assert!(copied.skipped_pages.contains(&name), "{copied:?}");
    assert!(g.root.join(&rel).is_dir());
}

/// P3 review #2: malformed imported bytes can already own the Guide copy's
/// name. The hosted copy must preserve the transaction's refusal and leave
/// every destination absent.
#[test]
fn an_unreadable_owner_refuses_the_guide_copy_with_and_without_a_host() {
    let _integration_guard = crate::integration_lock();
    let (name, rel) = first_copy();
    for running in [false, true] {
        let g = hosted(&[]);
        let mut owner = format!("title:: {name}\n").into_bytes();
        owner.extend_from_slice(b"broken:: \xff\n\n- imported\n");
        fs::write(g.root.join("pages/Other.md"), &owner).unwrap();
        g.store.refresh(tine_store::Depth::Bytes).unwrap();
        g.store.whole_graph().unwrap();
        let copied =
            guide::copy_guide_into_graph(&g.store, running.then_some(&g.host), "Tine Guide");
        assert!(copied.is_err(), "host={running}: {copied:?}");
        assert_eq!(copied.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert!(!g.root.join(&rel).exists(), "host={running}: created {rel}");
        assert_eq!(fs::read(g.root.join("pages/Other.md")).unwrap(), owner);
        assert_eq!(fs::read_dir(g.root.join("pages")).unwrap().count(), 1);
    }
}

/// P3 review #3, DESIGN §3.1c C3: a twin delivered after planning must
/// invalidate admission before an operation is recorded. Replanning then
/// reports the Guide page skipped, rather than leaving unfinished work.
#[test]
fn a_guide_twin_that_arrives_after_the_plan_is_reported_skipped() {
    let _integration_guard = crate::integration_lock();
    let (name, rel) = first_copy();
    let g = hosted(&[]);
    let held = g
        .host
        .reserve(|| vec![PageId::from(rel.as_str())], Input::Refuse)
        .expect("nothing unsaved");
    let twin = Path::new(&rel).with_extension("org");
    std::thread::scope(|scope| {
        let (done, finished) = mpsc::channel();
        let (store, host) = (&*g.store, &g.host);
        scope.spawn(move || {
            let copied = guide::copy_guide_into_graph(store, Some(host), "Tine Guide");
            done.send(copied.map_err(|error| error.to_string()))
                .unwrap();
        });
        assert!(
            finished.recv_timeout(Duration::from_millis(500)).is_err(),
            "the copy wrote while another reservation holds {rel}"
        );
        assert!(!g.root.join(&rel).exists());
        fs::write(g.root.join(&twin), "* delivered twin\n").unwrap();
        drop(held);
        let copied = finished
            .recv_timeout(Duration::from_secs(20))
            .expect("the copy answers once the page is released")
            .expect("an arriving twin must be a skipped Guide page");
        assert!(copied.skipped_pages.contains(&name), "{copied:?}");
        assert!(!copied.created_pages.contains(&name), "{copied:?}");
    });
    assert!(!g.root.join(&rel).exists());
    assert_eq!(read(&g.root, twin.to_str().unwrap()), "* delivered twin\n");
}
