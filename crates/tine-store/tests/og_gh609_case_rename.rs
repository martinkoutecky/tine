//! GH #609: one rename changes spelling without a second page identity.
use std::fs;
use tine_graph_features::pages::{rename_or_merge_page, RenameOutcome};
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

#[test]
fn case_only_rename_changes_filename_title_refs_and_namespace() {
    let _integration_guard = crate::integration_lock();
    for (ext, header, block) in [
        ("md", "title:: my note\n", "- "),
        ("org", "#+TITLE: my note\n", "* "),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::create_dir(dir.path().join("logseq")).unwrap();
        fs::write(
            dir.path().join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar :default-home {:page \"my note\"}}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join(format!("pages/my note.{ext}")),
            format!("{header}{block}[[my note]]\n"),
        )
        .unwrap();
        fs::write(dir.path().join("pages/my note___child.md"), "- child\n").unwrap();
        fs::write(
            dir.path().join("pages/Refs.md"),
            "- [[my note]] [[my note/child]]\n",
        )
        .unwrap();
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let report = hosted(&store, |host| {
            rename_or_merge_page(&store, host, "my note", "My Note", None, None, &[])
        })
        .unwrap();
        assert_eq!(report.outcome, RenameOutcome::Renamed);
        assert_eq!(report.home_page.as_deref(), Some("My Note"));
        let names: Vec<_> = fs::read_dir(dir.path().join("pages"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(names.contains(&format!("My Note.{ext}").into()));
        assert!(!names.contains(&format!("my note.{ext}").into()));
        assert!(names.contains(&"My Note___child.md".into()));
        let text = fs::read_to_string(dir.path().join(format!("pages/My Note.{ext}"))).unwrap();
        assert!(text.contains("My Note"));
        assert!(!text.contains("my note"));
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Refs.md")).unwrap(),
            "- [[My Note]] [[My Note/child]]\n"
        );
        let page = store
            .page_named("My Note", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap();
        assert_eq!(page.doc.name, "My Note");
        store.close();
        let reopened = Store::open(dir.path(), Default::default()).unwrap().0;
        assert_eq!(
            reopened
                .page_named("my note", tine_core::model::PageKind::Page)
                .unwrap()
                .unwrap()
                .doc
                .name,
            "My Note"
        );
        reopened.close();
    }
}

/// The page files in `pages/`: a killed write's hidden temporary
/// (`.{name}.{pid}.{seq}.tmp`) is crash debris, never a page.
fn pages(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir.join("pages"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| !(name.starts_with('.') && name.ends_with(".tmp")))
        .collect();
    names.sort();
    names
}

/// Whether `dir`'s volume folds case: `Old.md` then names `old.md`'s entry.
fn folds_case(dir: &std::path::Path) -> bool {
    fs::write(dir.join("case-probe"), "").unwrap();
    let folding = dir.join("CASE-PROBE").exists();
    fs::remove_file(dir.join("case-probe")).unwrap();
    folding
}

fn until(what: &str, test: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !test() {
        assert!(std::time::Instant::now() < deadline, "never: {what}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// A case rename that fails (GH #609). On a case-folding volume `Old.md` is
/// the source's own entry: the low-level transaction helper still has its
/// undo tests below; HostOp spelling failures are in binding_apply_tests.
/// On a case-sensitive volume the
/// two spellings are distinct pages and the rename is the page host's
/// (custody plus forward completion, STEP3-DESIGN's restated rollback tests):
/// a failure before application (its draft) changes nothing, and after
/// application the source is never trashed before `Old.md` publishes (SPEC-s3
/// s3.2), so the listing stays exactly `old.md` while the destination's
/// writes fail, and the rename completes forward once one succeeds.
#[test]
fn case_move_io_failure_keeps_the_source_and_completes_forward() {
    let _integration_guard = crate::integration_lock();
    let probe = tempfile::tempdir().unwrap();
    if folds_case(probe.path()) {
        return; // The internal transaction rollback cases run in tine-store unit tests.
    }
    use tine_store::host_faults::{fail, Phase};
    // A full disk or an app-data disk error: the operation's draft fails.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- [[old]]\n").unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    fail(Phase::DraftTemp, std::io::ErrorKind::Other, 10_000);
    assert!(hosted(&store, |host| rename_or_merge_page(
        &store,
        host,
        "old",
        "Old",
        None,
        None,
        &[]
    ))
    .is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "- [[old]]\n"
    );
    assert_eq!(pages(dir.path()), ["old.md"]);
    store.close();

    // A disk error fails the destination's first five writes; their backoff
    // outlasts the caller's 10 s wait.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- [[old]]\n").unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    fail(Phase::PageTemp, std::io::ErrorKind::Other, 5);
    let app_data = tempfile::tempdir().unwrap();
    let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
    assert!(rename_or_merge_page(&store, &host, "old", "Old", None, None, &[]).is_err());
    assert_eq!(pages(dir.path()), ["old.md"], "s3.2: src waits for dst");
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "- [[old]]\n"
    );
    until("the rename completes forward", || {
        pages(dir.path()) == ["Old.md"]
    });
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
        "- [[Old]]\n"
    );
    drop(host);
    store.close();
}
#[test]
fn distinct_case_twins_refuse_without_writing() {
    let _integration_guard = crate::integration_lock();
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "- first\n").unwrap();
    let Ok(mut twin) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.path().join("pages/Old.md"))
    else {
        return;
    };
    twin.write_all(b"- second\n").unwrap();
    drop(twin);
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    assert!(hosted(&store, |host| rename_or_merge_page(
        &store,
        host,
        "old",
        "Old",
        None,
        None,
        &[]
    ))
    .is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "- first\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
        "- second\n"
    );
    store.close();
}

#[test]
fn case_rename_crash_worker() {
    let _integration_guard = crate::integration_lock();
    let Ok(root) = std::env::var("TINE_QD1_CRASH_ROOT") else {
        return;
    };
    let store = std::sync::Arc::new(
        Store::open(std::path::Path::new(&root), Default::default())
            .unwrap()
            .0,
    );
    use tine_store::host_faults::{abort_before, Phase};
    match std::env::var("TINE_QD1_CRASH_POINT").unwrap().as_str() {
        "before" => store.inject_fault(tine_store::FaultPoint::AbortBeforeMoveRename),
        "publish" => store.inject_fault(tine_store::FaultPoint::AbortAfterMoveRename),
        "move-unsynced" => store.inject_fault(tine_store::FaultPoint::AbortBeforeMoveSync),
        "rewrite" => store.inject_fault(tine_store::FaultPoint::AbortAfterMoveRewrite),
        "host-dst" => abort_before(Phase::PageRename, 0),
        "host-sync" => abort_before(Phase::PageSync, 0),
        "host-trash" => abort_before(Phase::TrashMove, 0),
        "after" => {}
        _ => unreachable!(),
    }
    let app_data = std::env::var("TINE_QD1_APP_DATA").unwrap();
    let host =
        tine_store::PageHost::start_for_tests(&store, std::path::Path::new(&app_data)).unwrap();
    let result = rename_or_merge_page(&store, &host, "old", "Old", None, None, &[]);
    if std::env::var("TINE_QD1_CRASH_POINT").unwrap() == "after" {
        result.unwrap();
        std::process::abort();
    }
    panic!("case rename did not reach abort boundary");
}

/// Kill the case rename's worker at `point`, with its app data in `app_data`.
fn kill_case_rename(dir: &std::path::Path, app_data: &std::path::Path, point: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "og_gh609_case_rename::case_rename_crash_worker"])
        .env("TINE_QD1_CRASH_ROOT", dir)
        .env("TINE_QD1_CRASH_POINT", point)
        .env("TINE_QD1_APP_DATA", app_data)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("panicked at"),
        "must reach the injected abort, not panic before it ({point}): {output:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6),
            "must abort, not panic: {output:?}"
        );
    }
}

/// A case rename killed at each boundary (GH #609). On a case-sensitive
/// volume it is the page host's: across a crash only no loss and completion
/// hold (SPEC-s3 s3.2). Up to the kill the run kept its order, so the source
/// is still whole and live (its deletion waits for the destination); a
/// destination published before the kill is whole, a transient second
/// spelling (an accepted effect of write-before-delete). A relaunch on the
/// same app data completes the rename from its drafts, leaving exactly
/// `Old.md`. On a case-folding volume the entry moves first; a retry
/// finishes old text if no content draft was recorded yet.
#[test]
fn case_rename_kill_reopen_preserves_bytes_and_relaunch_completes() {
    let _integration_guard = crate::integration_lock();
    let probe = tempfile::tempdir().unwrap();
    if folds_case(probe.path()) {
        return transaction_case_rename_kills();
    }
    for (point, published) in [
        ("host-dst", false),
        ("host-sync", true),
        ("host-trash", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "title:: old\n- [[old]]\n").unwrap();
        kill_case_rename(dir.path(), app_data.path(), point);
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
            "title:: old\n- [[old]]\n",
            "{point}"
        );
        if published {
            assert_eq!(pages(dir.path()), ["Old.md", "old.md"], "{point}");
            assert_eq!(
                fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
                "title:: Old\n- [[Old]]\n",
                "{point}"
            );
        } else {
            assert_eq!(pages(dir.path()), ["old.md"], "{point}");
        }
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        until("the relaunch completes the rename", || {
            pages(dir.path()) == ["Old.md"]
        });
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            "title:: Old\n- [[Old]]\n",
            "{point}"
        );
        drop(host);
        store.close();
    }
}

/// The folding host path: entry first, then drafted content. Retry from the
/// explicit title actually displayed after relaunch, only if still old.
fn transaction_case_rename_kills() {
    for point in [
        "before",
        "move-unsynced",
        "publish",
        "host-dst",
        "host-sync",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        fs::write(dir.path().join("pages/old.md"), "title:: old\n- [[old]]\n").unwrap();
        let app_data = tempfile::tempdir().unwrap();
        kill_case_rename(dir.path(), app_data.path(), point);
        let drafted = matches!(point, "host-dst" | "host-sync");
        if drafted {
            assert!(
                fs::read_dir(app_data.path().join("drafts-v2/test-graph"))
                    .unwrap()
                    .any(|entry| entry
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "draft")),
                "{point}: the killed content operation must leave its recorded draft"
            );
        }
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        if drafted {
            until("recorded content completes on relaunch", || {
                fs::read_to_string(dir.path().join("pages/Old.md")).unwrap()
                    == "title:: Old\n- [[Old]]\n"
            });
        }
        assert_eq!(
            pages(dir.path()),
            [if point == "before" {
                "old.md"
            } else {
                "Old.md"
            }]
        );
        if !drafted {
            assert_eq!(
                fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
                "title:: old\n- [[old]]\n"
            );
            rename_or_merge_page(&store, &host, "old", "Old", None, None, &[]).unwrap();
        }
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            "title:: Old\n- [[Old]]\n"
        );
        assert_eq!(pages(dir.path()), ["Old.md"]);
        drop(host);
        store.close();
    }
}

/// An unchanged title is never an instruction to normalize the filename.
#[test]
fn same_name_rename_preserves_legitimate_filename_case() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/old.md"), "title:: Old\n- [[Old]]\n").unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    hosted(&store, |host| {
        rename_or_merge_page(&store, host, "Old", "Old", None, None, &[])
    })
    .unwrap();
    assert_eq!(pages(dir.path()), ["old.md"]);
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/old.md")).unwrap(),
        "title:: Old\n- [[Old]]\n"
    );
    let unchanged = hosted(&store, |host| {
        rename_or_merge_page(&store, host, "Old", "Old", None, None, &[])
    })
    .unwrap();
    assert!(unchanged.touched.is_empty());
    store.close();
}

#[test]
fn same_name_namespace_preserves_children_and_references() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::create_dir(dir.path().join("logseq")).unwrap();
    fs::write(
        dir.path().join("logseq/config.edn"),
        "{:file/name-format :triple-lowbar}\n",
    )
    .unwrap();
    for (file, body) in [
        ("Space.md", "title:: Space\n- root\n"),
        ("space___a.md", "- child\n"),
        ("custom.md", "title:: Space/b\n- custom\n"),
        ("Ref.md", "- [[space/a]]\n"),
    ] {
        fs::write(dir.path().join("pages").join(file), body).unwrap();
    }
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    hosted(&store, |host| {
        rename_or_merge_page(&store, host, "Space", "Space", None, None, &[])
    })
    .unwrap();
    assert_eq!(
        pages(dir.path()),
        ["Ref.md", "Space.md", "custom.md", "space___a.md"]
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/Ref.md")).unwrap(),
        "- [[space/a]]\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/custom.md")).unwrap(),
        "title:: Space/b\n- custom\n"
    );
    store.close();
}

/// Linux constructs the entry-first residues (a process kill cannot prove
/// power-loss durability). During content, use real draft/publication kills
/// at the already-moved path and relaunch with the same app data.
#[test]
fn ordinary_retry_recovers_every_entry_first_crash_boundary() {
    let _integration_guard = crate::integration_lock();
    for point in [
        "before-move",
        "move-unsynced",
        "before-content",
        "host-dst",
        "host-sync",
        "after",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let app_data = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let old = "title:: old\n- user text [[old]]\n";
        let new = "title:: Old\n- user text [[Old]]\n";
        let file = if point == "before-move" {
            "old.md"
        } else {
            "Old.md"
        };
        fs::write(dir.path().join("pages").join(file), old).unwrap();
        fs::write(dir.path().join("pages/Ref.md"), "- [[old]]\n").unwrap();
        if matches!(point, "host-dst" | "host-sync" | "after") {
            kill_case_rename(dir.path(), app_data.path(), point);
        }
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        let host = tine_store::PageHost::start_for_tests(&store, app_data.path()).unwrap();
        if matches!(point, "host-dst" | "host-sync" | "after") {
            until("recorded entry-first content finishes", || {
                fs::read_to_string(dir.path().join("pages/Old.md")).unwrap() == new
                    && fs::read_to_string(dir.path().join("pages/Ref.md")).unwrap() == "- [[Old]]\n"
            });
            // The displayed title is already the intended name; an unchanged
            // confirmation must remain harmless even after a real crash.
            let report =
                rename_or_merge_page(&store, &host, "Old", "Old", None, None, &[]).unwrap();
            assert!(report.touched.is_empty());
        } else {
            assert_eq!(
                fs::read_to_string(dir.path().join("pages").join(file)).unwrap(),
                old
            );
            rename_or_merge_page(&store, &host, "old", "Old", None, None, &[]).unwrap();
        }
        assert_eq!(pages(dir.path()), ["Old.md", "Ref.md"], "{point}");
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Old.md")).unwrap(),
            new,
            "{point}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("pages/Ref.md")).unwrap(),
            "- [[Old]]\n",
            "{point}"
        );
        drop(host);
        store.close();
    }
}

#[test]
fn same_name_rename_preserves_decomposed_filename() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    fs::write(
        dir.path().join("pages/cafe\u{301}.md"),
        "title:: café\n- user text\n",
    )
    .unwrap();
    let before = pages(dir.path());
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    let report = hosted(&store, |host| {
        rename_or_merge_page(&store, host, "café", "café", None, None, &[])
    })
    .unwrap();
    assert!(report.touched.is_empty());
    assert_eq!(pages(dir.path()), before);
    store.close();
}
#[test]
fn case_rename_uses_the_shared_unicode_identity_on_normalizing_filesystems() {
    let _integration_guard = crate::integration_lock();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("pages")).unwrap();
    // macOS can return a decomposed physical filename for a composed title.
    fs::write(
        dir.path().join("pages/cafe\u{301}.md"),
        "title:: café\n- [[café]]\n",
    )
    .unwrap();
    let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
    assert_eq!(
        hosted(&store, |host| rename_or_merge_page(
            &store,
            host,
            "café",
            "CAFÉ",
            None,
            None,
            &[]
        ))
        .unwrap()
        .outcome,
        RenameOutcome::Renamed
    );
    assert_eq!(
        store
            .page_named("CAFÉ", tine_core::model::PageKind::Page)
            .unwrap()
            .unwrap()
            .doc
            .name,
        "CAFÉ"
    );
    assert_eq!(fs::read_dir(dir.path().join("pages")).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("pages/CAFÉ.md")).unwrap(),
        "title:: CAFÉ\n- [[CAFÉ]]\n"
    );
    store.close();
}

#[test]
fn successful_case_rename_retains_only_the_existing_old_byte_copy() {
    let _integration_guard = crate::integration_lock();
    for blocks in [1, 60] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("pages")).unwrap();
        let old = "- [[old]]\n".repeat(blocks);
        fs::write(dir.path().join("pages/old.md"), &old).unwrap();
        let store = std::sync::Arc::new(Store::open(dir.path(), Default::default()).unwrap().0);
        hosted(&store, |host| {
            rename_or_merge_page(&store, host, "old", "Old", None, None, &[])
        })
        .unwrap();
        let copies: Vec<_> = fs::read_dir(dir.path().join("logseq/.tine-trash/pages"))
            .unwrap()
            .map(|entry| fs::read(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(copies, [old.as_bytes()]);
        let live = fs::read(dir.path().join("pages/Old.md")).unwrap();
        assert_eq!(live, "- [[Old]]\n".repeat(blocks).as_bytes());
        eprintln!("case rename blocks={blocks}: live={} B, old-copy={} B, retained copies=1, staging copies=0", live.len(), old.len());
        store.close();
    }
}
