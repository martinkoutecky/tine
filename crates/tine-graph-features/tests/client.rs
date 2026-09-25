use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::pdf::{Highlight, Position, Rect};
use tine_graph_features::{assets, config, conflicts, guide, journals, pages, pdf};
use tine_store::{model::Graph, Area, Content, Day, FaultPoint, Store};

fn disk_tree(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &path, out);
            } else {
                let mut rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.contains("/.tine-trash/") {
                    if let Some((prefix, name)) = rel.rsplit_once("__") {
                        if let Some((parent, _)) = prefix.rsplit_once('/') {
                            rel = format!("{parent}/__{name}");
                        }
                    }
                }
                out.push((rel, fs::read(path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-client-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("assets")).unwrap();
    (
        root.clone(),
        Store::from_legacy(Arc::new(Graph::open(&root))),
    )
}

#[test]
fn source_scan_guard_clients_touch_no_path() {
    for (name, source) in [
        ("lib", include_str!("../src/lib.rs")),
        ("assets", include_str!("../src/assets.rs")),
        ("conflicts", include_str!("../src/conflicts.rs")),
        ("config", include_str!("../src/config.rs")),
        ("journals", include_str!("../src/journals.rs")),
        ("pages", include_str!("../src/pages.rs")),
        ("pdf", include_str!("../src/pdf.rs")),
        ("guide", include_str!("../src/guide.rs")),
    ] {
        for forbidden in [
            "std::fs",
            "std::path",
            "File::open",
            "File::create",
            "OpenOptions",
            "read_dir",
            "canonicalize",
            ".join(",
            "tine_store::model",
        ] {
            if name == "config" && forbidden == ".join(" {
                continue; // EDN favorites join strings, never paths.
            }
            if name == "guide" && forbidden == "std::path" {
                continue; // Public API accepts the device parent folder for graph creation.
            }
            assert!(
                !source.contains(forbidden),
                "Clients touch no path: {name} contains {forbidden}"
            );
        }
    }
}

#[test]
fn guide_creation_matches_legacy_tree_and_folder_choice() {
    use tine_store::onboarding::create_demo_graph as old_create;
    let (empty, _) = fixture("demo-empty-new");
    let (old, _) = fixture("demo-empty-old");
    fs::remove_dir(empty.join("pages")).unwrap();
    fs::remove_dir(empty.join("assets")).unwrap();
    assert_eq!(guide::create_demo_graph(&empty).unwrap(), empty);
    old_create(&old).unwrap();
    assert_eq!(disk_tree(&empty), disk_tree(&old));

    let (parent, _) = fixture("demo-parent");
    fs::write(parent.join("keep"), b"keep").unwrap();
    let first = guide::create_demo_graph(&parent).unwrap();
    assert_eq!(first, parent.join("tine-demo"));
    assert_eq!(disk_tree(&first), disk_tree(&old));
    let second = guide::create_demo_graph(&parent).unwrap();
    assert_eq!(second, parent.join("tine-demo-2"));
    assert_eq!(disk_tree(&second), disk_tree(&old));
    assert_eq!(fs::read(parent.join("keep")).unwrap(), b"keep");

    let file = parent.join("file");
    fs::write(&file, b"x").unwrap();
    assert!(matches!(
        Store::create_graph(&file, &[]),
        Err(tine_store::OpenError::NotAFolder(_))
    ));
    assert!(matches!(
        Store::create_graph(std::path::Path::new(""), &[]),
        Err(tine_store::OpenError::NotAFolder(_))
    ));

    let (collision, _) = fixture("demo-collision");
    let duplicate = [
        (
            Area::Meta,
            "config.edn".to_string(),
            b"{:custom true}\n".to_vec(),
        ),
        (Area::Assets, "same.bin".to_string(), b"first".to_vec()),
        (Area::Assets, "same.bin".to_string(), b"second".to_vec()),
    ];
    assert!(matches!(
        Store::create_graph(&collision, &duplicate),
        Err(tine_store::OpenError::CreateFailed { .. })
    ));
    assert_eq!(
        fs::read(collision.join("tine-demo/assets/same.bin")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(collision.join("tine-demo/logseq/config.edn")).unwrap(),
        b"{:custom true}\n"
    );
}

#[test]
fn guide_copy_matches_legacy_independent_steps() {
    use tine_store::onboarding::copy_guide_into_graph as old_copy;
    for case in ["empty", "page", "asset", "asset_dir"] {
        let (new_root, store) = fixture(&format!("guide-{case}-new"));
        let (old_root, _) = fixture(&format!("guide-{case}-old"));
        if case == "page" {
            let name = tine_core::guide::guide_copy_page_name("Features/Sheets");
            let file = format!(
                "{}.md",
                tine_core::model::encode_page_name(
                    &name,
                    tine_core::config::Config::default().file_name_format
                )
            );
            for root in [&new_root, &old_root] {
                fs::write(root.join("pages").join(&file), b"existing").unwrap();
            }
        }
        if case == "asset" {
            for root in [&new_root, &old_root] {
                fs::write(root.join("assets/quick-capture.png"), b"existing").unwrap();
            }
        }
        if case == "asset_dir" {
            for root in [&new_root, &old_root] {
                fs::create_dir(root.join("assets/quick-capture.png")).unwrap();
            }
        }
        let old_graph = Graph::open(&old_root);
        let actual = guide::copy_guide_into_graph(&store, "Features/Sheets").unwrap();
        let expected = old_copy(&old_graph, "Features/Sheets").unwrap();
        if case == "page" {
            assert!(actual
                .skipped_pages
                .contains(&tine_core::guide::guide_copy_page_name("Features/Sheets")));
        }
        if case == "asset" || case == "asset_dir" {
            assert!(!actual
                .copied_assets
                .contains(&"quick-capture.png".to_string()));
        }
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "{case}"
        );
        assert_eq!(disk_tree(&new_root), disk_tree(&old_root), "{case}");
    }
    let (_, store) = fixture("guide-unknown");
    assert_eq!(
        guide::copy_guide_into_graph(&store, "missing")
            .unwrap_err()
            .to_string(),
        "unknown bundled guide page"
    );
}

#[test]
fn config_setters_match_legacy_values_and_bytes() {
    type New = fn(&Store) -> std::io::Result<()>;
    type Old = fn(&Graph) -> std::io::Result<()>;
    let operations: [(&str, New, Old); 11] = [
        (
            "favorites",
            |s| config::set_favorites(s, &["A] B".into()]),
            |g| g.set_favorites(&["A] B".into()]),
        ),
        (
            "workflow",
            |s| config::set_preferred_workflow(s, "todo"),
            |g| g.set_preferred_workflow("todo"),
        ),
        (
            "timetracking",
            |s| config::set_timetracking_enabled(s, false),
            |g| g.set_timetracking_enabled(false),
        ),
        (
            "brackets",
            |s| config::set_show_brackets(s, false),
            |g| g.set_show_brackets(false),
        ),
        (
            "doc_mode",
            |s| config::set_doc_mode_enter_for_new_block(s, true),
            |g| g.set_doc_mode_enter_for_new_block(true),
        ),
        (
            "outdenting",
            |s| config::set_logical_outdenting(s, true),
            |g| g.set_logical_outdenting(true),
        ),
        (
            "guide",
            |s| config::set_guide_announced(s, true),
            |g| g.set_guide_announced(true),
        ),
        (
            "format",
            |s| config::set_preferred_format(s, tine_core::model::Format::Org),
            |g| g.set_preferred_format(tine_core::model::Format::Org),
        ),
        (
            "journal_title",
            |s| config::set_journal_page_title_format(s, "yyyy-MM-dd"),
            |g| g.set_journal_page_title_format("yyyy-MM-dd"),
        ),
        (
            "template",
            |s| config::set_default_journal_template(s, Some("Daily \"A\"")),
            |g| g.set_default_journal_template(Some("Daily \"A\"")),
        ),
        (
            "week",
            |s| config::set_start_of_week(s, 6),
            |g| g.set_start_of_week(6),
        ),
    ];
    let cases = [
        ("typical", Some("{:favorites [\"Old\"] :preferred-workflow :now :feature/enable-timetracking? true :ui/show-brackets? true :shortcut/doc-mode-enter-for-new-block? false :editor/logical-outdenting? false :tine/guide-announced? false :preferred-format \"Markdown\" :journal/page-title-format \"MMM do, yyyy\" :default-templates {:journals \"Old\" :pages \"P\"} :start-of-week 0}\n")),
        ("missing", Some("{:unrelated 42}\n")),
        ("absent", None),
        ("comments", Some("{ ; :favorites [\"comment\"]\n :favorites ; odd spacing\n [\"Old\"]\n :preferred-workflow  ; comment\n :now\n :start-of-week   2\n :default-templates { :pages \"P\" ; keep\n :journals \"Old\"}}\n")),
    ];
    for (op_name, new, old) in operations {
        for (case_name, input) in cases {
            let (new_root, store) = fixture(&format!("config-{op_name}-{case_name}-new"));
            let (old_root, _) = fixture(&format!("config-{op_name}-{case_name}-old"));
            for root in [&new_root, &old_root] {
                fs::create_dir_all(root.join("logseq")).unwrap();
                if let Some(input) = input {
                    fs::write(root.join("logseq/config.edn"), input).unwrap();
                }
            }
            let legacy = Graph::open(&old_root);
            let actual = new(&store);
            let expected = old(&legacy);
            assert_eq!(
                actual
                    .as_ref()
                    .map(|_| ())
                    .map_err(|e| (e.kind(), e.to_string())),
                expected
                    .as_ref()
                    .map(|_| ())
                    .map_err(|e| (e.kind(), e.to_string())),
                "{op_name}/{case_name}"
            );
            assert_eq!(
                fs::read(new_root.join("logseq/config.edn")).unwrap(),
                fs::read(old_root.join("logseq/config.edn")).unwrap(),
                "{op_name}/{case_name}"
            );
        }
    }
}

#[test]
fn custom_css_matches_legacy_present_absent_and_unreadable() {
    for (name, contents) in [
        ("present", Some(b"body { color: red }".as_slice())),
        ("absent", None),
        ("unreadable", Some(b"\xff".as_slice())),
    ] {
        let (root, store) = fixture(&format!("custom-css-{name}"));
        fs::create_dir_all(root.join("logseq")).unwrap();
        if let Some(contents) = contents {
            fs::write(root.join("logseq/custom.css"), contents).unwrap();
        }
        assert_eq!(
            config::custom_css(&store),
            Graph::open(&root).custom_css(),
            "{name}"
        );
    }
}

#[test]
fn config_retry_reapplies_edit_over_external_write() {
    let (root, store) = fixture("config-retry");
    fs::create_dir_all(root.join("logseq")).unwrap();
    fs::write(root.join("logseq/config.edn"), "{:start-of-week 0}\n").unwrap();
    store.inject_fault(FaultPoint::Stage2ConfigExternal);
    config::set_start_of_week(&store, 6).unwrap();
    let actual = fs::read_to_string(root.join("logseq/config.edn")).unwrap();
    assert!(actual.contains(":external true"), "{actual}");
    assert!(actual.contains(":start-of-week 6"), "{actual}");
}

#[test]
fn config_invalid_utf8_error_matches_legacy() {
    let (new_root, store) = fixture("config-invalid-utf8-new");
    let (old_root, _) = fixture("config-invalid-utf8-old");
    for root in [&new_root, &old_root] {
        fs::create_dir_all(root.join("logseq")).unwrap();
        fs::write(root.join("logseq/config.edn"), b"\xff").unwrap();
    }
    let old = Graph::open(&old_root)
        .set_start_of_week(1)
        .unwrap_err()
        .to_string();
    let new = config::set_start_of_week(&store, 1)
        .unwrap_err()
        .to_string();
    assert_eq!(new, old);
    assert_eq!(
        fs::read(new_root.join("logseq/config.edn")).unwrap(),
        b"\xff"
    );
}

#[test]
fn conflict_clients_match_legacy_values_and_disk_bytes() {
    use std::collections::HashMap;
    let (new_root, store) = fixture("conflict-matrix-new");
    let (old_root, _) = fixture("conflict-matrix-old");
    let conflict_name = "Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    for root in [&new_root, &old_root] {
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join("pages/Foo.md"), "- mine\n").unwrap();
        fs::write(root.join("pages").join(conflict_name), "- theirs\n").unwrap();
    }
    let old = Graph::open(&old_root);
    assert_eq!(
        serde_json::to_value(conflicts::list_sync_conflicts(&store)).unwrap(),
        serde_json::to_value(old.list_sync_conflicts()).unwrap(),
    );
    let conflict = format!("pages/{conflict_name}");
    let new_diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", &conflict)
        .unwrap()
        .unwrap();
    let old_diff = old
        .sync_conflict_diff("pages/Foo.md", &conflict)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&new_diff).unwrap(),
        serde_json::to_value(&old_diff).unwrap()
    );
    conflicts::resolve_sync_conflict(
        &store,
        "pages/Foo.md",
        &conflict,
        &HashMap::new(),
        &new_diff.base_rev,
        &new_diff.conflict_rev,
        "union",
    )
    .unwrap();
    old.resolve_sync_conflict(
        "pages/Foo.md",
        &conflict,
        &HashMap::new(),
        &old_diff.base_rev,
        &old_diff.conflict_rev,
        "union",
    )
    .unwrap();
    assert_eq!(disk_tree(&new_root), disk_tree(&old_root));

    // The separate discard operation preserves the same bytes too.
    for root in [&new_root, &old_root] {
        fs::write(root.join("pages").join(conflict_name), "- next\n").unwrap();
    }
    conflicts::trash_sync_conflict(&store, &conflict).unwrap();
    old.trash_sync_conflict(&conflict).unwrap();
    assert_eq!(disk_tree(&new_root), disk_tree(&old_root));
}

#[test]
fn journal_clients_match_legacy_feed_conflicts_read_trash_and_migration() {
    let (new_root, store) = fixture("journal-matrix-new");
    let (old_root, _) = fixture("journal-matrix-old");
    for root in [&new_root, &old_root] {
        fs::create_dir_all(root.join("journals")).unwrap();
        for (name, body) in [
            ("2026_06_18.md", "- canonical\n"),
            ("Jun 18th, 2026.org", "- duplicate\n"),
            ("Jun 19th, 2026.md", "- migrate\n"),
            ("Jun 20th, 2026.md", "- occupied\n"),
            ("2026_06_20.md", "- keeper\n"),
        ] {
            fs::write(root.join("journals").join(name), body).unwrap();
        }
    }
    let old = Graph::open(&old_root);
    let new_feed = journals::feed_journals_desc_through(&store, Day(20260620));
    let old_feed =
        old.feed_journals_desc_through(tine_core::date::JournalDate::from_ordinal(20260620));
    assert_eq!(
        new_feed.iter().map(|(day, _)| day.0).collect::<Vec<_>>(),
        old_feed
            .iter()
            .map(|entry| entry.date_key.unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        serde_json::to_value(journals::journal_conflicts(&store)).unwrap(),
        serde_json::to_value(old.journal_conflicts()).unwrap()
    );
    assert_eq!(
        journals::read_journal_file(&store, "Jun 18th, 2026.org").unwrap(),
        old.read_journal_file("Jun 18th, 2026.org").unwrap()
    );
    assert_eq!(
        journals::has_journal_filename_migrations(&store),
        old.has_journal_filename_migrations()
    );
    // One deliberate difference: v0.6.5 renamed `Jun 18th, 2026.org` to
    // `2026_06_18.org` beside `2026_06_18.md`, creating an md/org twin. The
    // twin rule refuses that move, so the title-named duplicate stays as it was
    // (still listed by `journal_conflicts`). Every other file matches.
    assert_eq!(
        journals::migrate_journal_filenames(&store) + 1,
        old.migrate_journal_filenames()
    );
    assert!(new_root.join("journals/Jun 20th, 2026.md").exists());
    assert!(new_root.join("journals/Jun 18th, 2026.org").exists());
    assert!(!new_root.join("journals/2026_06_18.org").exists());
    fs::rename(
        old_root.join("journals/2026_06_18.org"),
        old_root.join("journals/Jun 18th, 2026.org"),
    )
    .unwrap();
    assert_eq!(disk_tree(&new_root), disk_tree(&old_root));
    journals::trash_journal_file(&store, "Jun 20th, 2026.md").unwrap();
    old.trash_journal_file("Jun 20th, 2026.md").unwrap();
    assert_eq!(disk_tree(&new_root), disk_tree(&old_root));
}

#[test]
fn external_write_during_resolve_rolls_back_winner_and_keeps_external_copy() {
    use std::collections::HashMap;
    let (root, store) = fixture("resolve-external");
    fs::write(root.join("pages/Foo.md"), "- mine\n").unwrap();
    let conflict = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
    fs::write(root.join(conflict), "- theirs\n").unwrap();
    let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", conflict)
        .unwrap()
        .unwrap();
    store.inject_fault(FaultPoint::Stage2MismatchAt(1));
    assert!(conflicts::resolve_sync_conflict(
        &store,
        "pages/Foo.md",
        conflict,
        &HashMap::new(),
        &diff.base_rev,
        &diff.conflict_rev,
        "union"
    )
    .is_err());
    assert_eq!(fs::read(root.join("pages/Foo.md")).unwrap(), b"- mine\n");
    assert_eq!(fs::read(root.join(conflict)).unwrap(), b"external stage-2");
}

#[test]
fn resolve_preblock_keep_choices_match_legacy_bytes() {
    use std::collections::HashMap;
    for choice in ["mine", "theirs"] {
        let (new_root, store) = fixture(&format!("choice-{choice}-new"));
        let (old_root, _) = fixture(&format!("choice-{choice}-old"));
        let conflict = "pages/Foo.sync-conflict-20260705-120000-ABCDEFG.md";
        for root in [&new_root, &old_root] {
            fs::write(root.join("pages/Foo.md"), "alias:: mine\n- shared\n").unwrap();
            fs::write(root.join(conflict), "alias:: theirs\n- shared\n").unwrap();
        }
        let old = Graph::open(&old_root);
        let diff = conflicts::sync_conflict_diff(&store, "pages/Foo.md", conflict)
            .unwrap()
            .unwrap();
        conflicts::resolve_sync_conflict(
            &store,
            "pages/Foo.md",
            conflict,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            choice,
        )
        .unwrap();
        old.resolve_sync_conflict(
            "pages/Foo.md",
            conflict,
            &HashMap::new(),
            &diff.base_rev,
            &diff.conflict_rev,
            choice,
        )
        .unwrap();
        assert_eq!(disk_tree(&new_root), disk_tree(&old_root), "{choice}");
    }
}

#[test]
fn failed_journal_repair_restores_legacy_filename() {
    let (root, store) = fixture("journal-repair-rollback");
    fs::create_dir_all(root.join("journals")).unwrap();
    fs::write(root.join("journals/Jun 18th, 2026.md"), "- preserve\n").unwrap();
    store.inject_fault(FaultPoint::MidStepIoAt(0));
    assert_eq!(journals::migrate_journal_filenames(&store), 0);
    assert_eq!(
        fs::read(root.join("journals/Jun 18th, 2026.md")).unwrap(),
        b"- preserve\n"
    );
    assert!(!root.join("journals/2026_06_18.md").exists());
}

#[test]
fn save_and_stream_import_match_legacy_collision_names_and_bytes() {
    let (a, store) = fixture("asset-new");
    assert_eq!(assets::save_asset(&store, "X", b"one").unwrap(), "X");
    assert_eq!(assets::save_asset(&store, "X", b"two").unwrap(), "X_1");
    assert_eq!(fs::read(a.join("assets/X_1")).unwrap(), b"two");
    let source = a.join("source.drawio.svg");
    fs::write(&source, b"diagram").unwrap();
    assert_eq!(
        assets::import_asset(
            &store,
            "source.drawio.svg",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: u64::MAX
            }
        )
        .unwrap(),
        "source.drawio.svg"
    );
    assert_eq!(
        assets::import_asset(
            &store,
            "source.drawio.svg",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: u64::MAX
            }
        )
        .unwrap(),
        "source_1.drawio.svg"
    );
    assert_eq!(
        fs::read(a.join("assets/source_1.drawio.svg")).unwrap(),
        b"diagram"
    );
}

#[test]
fn stream_cap_and_missing_trash_leave_no_asset() {
    let (root, store) = fixture("cap");
    let source = root.join("oversize");
    fs::write(&source, b"123456").unwrap();
    assert!(assets::import_asset_file(
        &store,
        "capture",
        Content::Stream {
            source: fs::File::open(&source).unwrap(),
            max_bytes: 4
        }
    )
    .is_err());
    assert!(!root.join("assets/capture").exists());
    assert_eq!(
        assets::trash_asset(&store, "missing")
            .unwrap_err()
            .to_string(),
        "no such asset"
    );
}

#[test]
fn asset_import_creates_missing_assets_directory() {
    let root = std::env::temp_dir().join(format!("tine-missing-assets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    let store = Store::from_legacy(Arc::new(Graph::open(&root)));
    assert_eq!(
        assets::save_asset(&store, "new.png", b"new").unwrap(),
        "new.png"
    );
    assert_eq!(fs::read(root.join("assets/new.png")).unwrap(), b"new");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn pdf_image_and_view_state_create_then_replace() {
    let (root, store) = fixture("pdf");
    let name = pdf::write_pdf_area_image(&store, "paper.pdf", 2, "id", 42, b"first").unwrap();
    assert_eq!(name, "paper/2_id_42.png");
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "id", 42, b"second").unwrap();
    assert_eq!(
        fs::read(root.join("assets").join(&name)).unwrap(),
        b"second"
    );
    pdf::write_pdf_view_state(&store, "paper.pdf", 2, 1.5).unwrap();
    pdf::write_pdf_view_state(&store, "paper.pdf", 3, 2.0).unwrap();
    let state = pdf::open_pdf(&store, "paper.pdf", "Paper").unwrap();
    assert_eq!(state.page, Some(3));
}

fn highlight(id: &str) -> Highlight {
    Highlight {
        id: id.into(),
        page: 1,
        position: Position {
            page: 1,
            bounding: Rect {
                top: 0.0,
                left: 0.0,
                width: 1.0,
                height: 1.0,
                source_width: None,
                source_height: None,
            },
            rects: vec![],
        },
        color: "yellow".into(),
        text: Some(id.into()),
        image: None,
    }
}

#[test]
fn highlights_first_write_and_update_persist_both_artifacts() {
    let (a, store) = fixture("hl-new");
    for (items, base) in [
        (vec![highlight("a")], vec![]),
        (vec![highlight("a"), highlight("b")], vec!["a".into()]),
    ] {
        pdf::write_highlights(&store, "paper.pdf", "Paper", &items, &base).unwrap();
        for rel in ["assets/paper.edn", "pages/hls__paper.md"] {
            assert!(!fs::read(a.join(rel)).unwrap().is_empty(), "{rel}");
        }
    }
    assert_eq!(pdf::read_highlights(&store, "paper.pdf").len(), 2);
}

#[test]
fn old_vs_new_matrix_on_identical_fixtures() {
    let (a, store) = fixture("matrix-new");
    let (b, _) = fixture("matrix-old");
    for root in [&a, &b] {
        fs::write(
            root.join("pages/Refs.md"),
            "- ![](../assets/referenced.png)\n",
        )
        .unwrap();
        fs::write(root.join("assets/referenced.png"), b"kept").unwrap();
    }
    let old = Graph::open(&b);
    let same = |rel: &str| {
        assert_eq!(
            fs::read(a.join(rel)).unwrap(),
            fs::read(b.join(rel)).unwrap(),
            "{rel}"
        )
    };
    for bytes in [b"one".as_slice(), b"two".as_slice()] {
        assert_eq!(
            assets::save_asset(&store, "photo.png", bytes).unwrap(),
            old.save_asset("photo.png", bytes).unwrap()
        );
    }
    same("assets/photo.png");
    same("assets/photo_1.png");
    let source = a.join("X");
    fs::write(&source, b"stream").unwrap();
    for expected in ["X", "X_1"] {
        assert_eq!(
            assets::import_asset(
                &store,
                "X",
                Content::Stream {
                    source: fs::File::open(&source).unwrap(),
                    max_bytes: u64::MAX
                }
            )
            .unwrap(),
            expected
        );
        assert_eq!(old.import_asset(&source, Some("X")).unwrap(), expected);
        same(&format!("assets/{expected}"));
    }
    let mut old_source = fs::File::open(&source).unwrap();
    assert_eq!(
        assets::import_asset_file(
            &store,
            "capture",
            Content::Stream {
                source: fs::File::open(&source).unwrap(),
                max_bytes: 100
            }
        )
        .unwrap(),
        old.import_asset_file(&mut old_source, "capture", 100)
            .unwrap()
    );
    same("assets/capture");
    let same_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for root in [&a, &b] {
        for name in ["photo.png", "photo_1.png", "X", "X_1", "capture"] {
            fs::File::open(root.join("assets").join(name))
                .unwrap()
                .set_modified(same_time)
                .unwrap();
        }
    }
    let new_orphans = assets::orphan_assets(&store)
        .into_iter()
        .map(|a| (a.name, a.size, a.modified))
        .collect::<Vec<_>>();
    assert!(!new_orphans
        .iter()
        .any(|(name, _, _)| name == "referenced.png"));
    assert_eq!(
        new_orphans,
        old.orphan_assets()
            .into_iter()
            .map(|a| (a.name, a.size, a.modified))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        assets::trash_asset(&store, "missing")
            .unwrap_err()
            .to_string(),
        old.trash_asset("missing").unwrap_err().to_string()
    );
    assets::trash_asset(&store, "photo.png").unwrap();
    old.trash_asset("photo.png").unwrap();
    assert!(!a.join("assets/photo.png").exists() && !b.join("assets/photo.png").exists());
    let trash_payloads = |root: &PathBuf| {
        let mut files = fs::read_dir(root.join("logseq/.tine-trash/assets"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                (
                    name.split_once("__").unwrap().1.to_owned(),
                    fs::read(entry.path()).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        files.sort();
        files
    };
    assert_eq!(trash_payloads(&a), trash_payloads(&b));

    for bytes in [b"first".as_slice(), b"second".as_slice()] {
        assert_eq!(
            pdf::write_pdf_area_image(&store, "paper.pdf", 2, "area", 42, bytes).unwrap(),
            old.write_pdf_area_image("paper.pdf", 2, "area", 42, bytes)
                .unwrap()
        );
        same("assets/paper/2_area_42.png");
    }
    assert_eq!(
        pdf::write_pdf_area_image(&store, "paper.pdf", 2, "../escape", 42, b"x")
            .unwrap_err()
            .to_string(),
        old.write_pdf_area_image("paper.pdf", 2, "../escape", 42, b"x")
            .unwrap_err()
            .to_string()
    );
    assert_eq!(
        pdf::open_pdf(&store, "paper.pdf", "Paper").unwrap(),
        old.open_pdf("paper.pdf", "Paper").unwrap()
    );
    same("assets/paper.edn");
    same("pages/hls__paper.md");
    pdf::write_pdf_view_state(&store, "paper.pdf", 3, 1.5).unwrap();
    old.write_pdf_view_state("paper.pdf", 3, 1.5).unwrap();
    same("assets/paper.edn");
    assert_eq!(
        pdf::write_pdf_view_state(&store, "paper.pdf", 0, 1.0)
            .unwrap_err()
            .to_string(),
        old.write_pdf_view_state("paper.pdf", 0, 1.0)
            .unwrap_err()
            .to_string()
    );
    for (items, base) in [
        (vec![highlight("a")], vec![]),
        (vec![highlight("a"), highlight("b")], vec!["a".into()]),
    ] {
        pdf::write_highlights(&store, "paper.pdf", "Paper", &items, &base).unwrap();
        old.write_highlights("paper.pdf", "Paper", &items, &base)
            .unwrap();
        same("assets/paper.edn");
        same("pages/hls__paper.md");
    }
    assert_eq!(
        pdf::read_highlights(&store, "paper.pdf"),
        old.read_highlights("paper.pdf")
    );
}

#[test]
fn trash_retries_external_write_without_losing_its_bytes() {
    let (root, store) = fixture("retry-trash");
    fs::write(root.join("assets/throwaway"), b"old").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    assets::trash_asset(&store, "throwaway").unwrap();
    assert!(!root.join("assets/throwaway").exists());
    let trash = root.join("logseq/.tine-trash/assets");
    assert!(fs::read_dir(trash)
        .unwrap()
        .flatten()
        .any(|entry| fs::read(entry.path()).unwrap() == b"external stage-2"));
}

#[test]
fn view_state_retries_external_sidecar_write_and_preserves_foreign_data() {
    let (root, store) = fixture("retry-view");
    pdf::write_pdf_view_state(&store, "paper.pdf", 1, 1.0).unwrap();
    store.inject_fault(FaultPoint::Stage2ValidSidecar);
    pdf::write_pdf_view_state(&store, "paper.pdf", 2, 1.5).unwrap();
    let edn = fs::read_to_string(root.join("assets/paper.edn")).unwrap();
    assert!(edn.contains("external") && edn.contains(":page 2"));
}

#[test]
fn area_image_retries_external_write_in_place() {
    let (root, store) = fixture("retry-image");
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "crop", 12, b"old").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    pdf::write_pdf_area_image(&store, "paper.pdf", 2, "crop", 12, b"new").unwrap();
    assert_eq!(
        fs::read(root.join("assets/paper/2_crop_12.png")).unwrap(),
        b"new"
    );
}

#[test]
fn highlights_retry_external_sidecar_write_and_preserve_foreign_data() {
    let (root, store) = fixture("retry-highlights");
    pdf::write_pdf_view_state(&store, "other.pdf", 1, 1.0).unwrap();
    store.inject_fault(FaultPoint::Stage2ValidSidecar);
    pdf::write_highlights(&store, "other.pdf", "Other", &[highlight("h")], &[]).unwrap();
    let edn = fs::read_to_string(root.join("assets/other.edn")).unwrap();
    assert!(edn.contains("external") && edn.contains("h"));
    assert!(root.join("pages/hls__other.md").exists());
}

#[test]
fn legacy_pdf_artifacts_stay_on_open_and_match_after_write_migration() {
    let (a, store) = fixture("migration-new");
    let (b, _) = fixture("migration-old");
    let pdf_name = "My Paper.pdf";
    let key = tine_core::pdf::asset_key(pdf_name);
    let legacy = tine_core::pdf::legacy_asset_key(pdf_name);
    let h = highlight("one");
    for root in [&a, &b] {
        fs::write(
            root.join("assets").join(format!("{legacy}.edn")),
            tine_core::pdf::write_highlights(&[h.clone()], ""),
        )
        .unwrap();
        let page = tine_core::pdf::hls_page_document(pdf_name, "My Paper", &[h.clone()]);
        fs::write(
            root.join("pages").join(format!("hls__{legacy}.md")),
            tine_core::doc::serialize(&page),
        )
        .unwrap();
    }
    let old = Graph::open(&b);
    pdf::open_pdf(&store, pdf_name, "My Paper").unwrap();
    old.open_pdf(pdf_name, "My Paper").unwrap();
    assert!(!a.join("assets").join(format!("{key}.edn")).exists());
    assert_eq!(
        fs::read(a.join("assets").join(format!("{legacy}.edn"))).unwrap(),
        fs::read(b.join("assets").join(format!("{legacy}.edn"))).unwrap()
    );
    pdf::write_highlights(&store, pdf_name, "My Paper", &[h.clone()], &[h.id.clone()]).unwrap();
    old.write_highlights(pdf_name, "My Paper", &[h.clone()], &[h.id.clone()])
        .unwrap();
    for rel in [format!("assets/{key}.edn"), format!("pages/hls__{key}.md")] {
        assert_eq!(
            fs::read(a.join(&rel)).unwrap(),
            fs::read(b.join(&rel)).unwrap(),
            "{rel}"
        );
    }
    assert!(!a.join("assets").join(format!("{legacy}.edn")).exists());
    assert!(!a.join("pages").join(format!("hls__{legacy}.md")).exists());
}

#[test]
fn blocked_trash_keeps_asset_and_legacy_error_text() {
    let (a, store) = fixture("blocked-new");
    let (b, _) = fixture("blocked-old");
    for root in [&a, &b] {
        fs::create_dir_all(root.join("logseq")).unwrap();
        fs::write(root.join("logseq/.tine-trash"), b"blocked").unwrap();
        fs::write(root.join("assets/photo.png"), b"safe").unwrap();
    }
    let old = Graph::open(&b);
    let old_error = old.trash_asset("photo.png").unwrap_err().to_string();
    let new_error = assets::trash_asset(&store, "photo.png")
        .unwrap_err()
        .to_string();
    assert_eq!(
        new_error,
        old_error.replace(&format!("{}/", b.display()), "")
    );
    assert_eq!(fs::read(a.join("assets/photo.png")).unwrap(), b"safe");
}

#[test]
fn malformed_sidecar_refusal_matches_legacy_text_and_keeps_bytes() {
    let (a, store) = fixture("malformed-new");
    let (b, _) = fixture("malformed-old");
    let malformed = b"{:highlights []} trailing";
    for root in [&a, &b] {
        fs::write(root.join("assets/paper.edn"), malformed).unwrap();
    }
    let old = Graph::open(&b);
    assert_eq!(
        pdf::open_pdf(&store, "paper.pdf", "Paper")
            .unwrap_err()
            .to_string(),
        old.open_pdf("paper.pdf", "Paper").unwrap_err().to_string()
    );
    assert_eq!(
        pdf::write_highlights(&store, "paper.pdf", "Paper", &[highlight("h")], &[])
            .unwrap_err()
            .to_string(),
        old.write_highlights("paper.pdf", "Paper", &[highlight("h")], &[])
            .unwrap_err()
            .to_string()
    );
    assert_eq!(fs::read(a.join("assets/paper.edn")).unwrap(), malformed);
}

#[test]
fn annotation_notes_survive_update_with_legacy_bytes() {
    let h = highlight("one");
    let mut doc = tine_core::pdf::hls_page_document("paper.pdf", "Paper", &[h.clone()]);
    doc.roots[0]
        .children
        .push(tine_core::doc::DocBlock::new("private note"));
    let page = tine_core::doc::serialize(&doc);
    let (a, _) = fixture("notes-new");
    let (b, _) = fixture("notes-old");
    for root in [&a, &b] {
        fs::write(
            root.join("assets/paper.edn"),
            tine_core::pdf::write_highlights(&[h.clone()], ""),
        )
        .unwrap();
        fs::write(root.join("pages/hls__paper.md"), &page).unwrap();
    }
    let store = Store::from_legacy(Arc::new(Graph::open(&a)));
    let old = Graph::open(&b);
    pdf::write_highlights(&store, "paper.pdf", "Paper", &[h.clone()], &[h.id.clone()]).unwrap();
    old.write_highlights("paper.pdf", "Paper", &[h.clone()], &[h.id.clone()])
        .unwrap();
    let new_page = fs::read(a.join("pages/hls__paper.md")).unwrap();
    assert!(String::from_utf8_lossy(&new_page).contains("private note"));
    assert_eq!(new_page, fs::read(b.join("pages/hls__paper.md")).unwrap());
}

#[test]
fn empty_sanitized_pdf_key_keeps_legacy_crop_location() {
    let (a, store) = fixture("empty-key-new");
    let (b, _) = fixture("empty-key-old");
    let pdf_name = "??.pdf";
    assert_eq!(tine_core::pdf::asset_key(pdf_name), "");
    let old = Graph::open(&b);
    assert_eq!(
        pdf::write_pdf_area_image(&store, pdf_name, 1, "crop", 5, b"png").unwrap(),
        old.write_pdf_area_image(pdf_name, 1, "crop", 5, b"png")
            .unwrap()
    );
    assert_eq!(
        fs::read(a.join("assets/1_crop_5.png")).unwrap(),
        fs::read(b.join("assets/1_crop_5.png")).unwrap()
    );
}

fn put(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

fn operation_result(result: &std::io::Result<()>, root: &std::path::Path) -> String {
    match result {
        Ok(()) => "ok".to_owned(),
        Err(error) => format!("{:?}: {}", error.kind(), error)
            .replace(&root.to_string_lossy().to_string(), "<root>"),
    }
}

#[test]
fn page_rename_matches_legacy_for_refs_namespace_alias_and_title() {
    for (label, old_name, new_name, files, config) in [
        (
            "plain",
            "x",
            "Next Name",
            vec![
                ("pages/x.md", "- own [[x]]\n"),
                ("pages/one.md", "- [[x]] #x #[[x]]\n"),
                ("pages/two.md", "tags:: x, [[x]], #x\n- [[x]]\n"),
                ("pages/three.org", "* [[x]] #x #[[x]]\n"),
                ("journals/2026_06_18.md", "- [[x]]\n"),
            ],
            None,
        ),
        (
            "namespace",
            "a",
            "b",
            vec![
                ("pages/a.md", "- [[a/child]]\n"),
                ("pages/a___child.md", "- [[a]]\n"),
                ("pages/ref.md", "- [[a/child]] [[a]]\n"),
            ],
            Some("{:file/name-format :triple-lowbar}"),
        ),
        (
            "alias-title",
            "Target",
            "Changed",
            vec![
                (
                    "pages/Target.md",
                    "title:: Display Target\nalias:: Alternate\n- [[Target]]\n",
                ),
                ("pages/ref.md", "- [[Target]] [[Alternate]]\n"),
            ],
            None,
        ),
        (
            "case-only",
            "target",
            "TARGET",
            vec![("pages/target.md", "- [[target]]\n")],
            None,
        ),
        (
            "reference-only",
            "Missing",
            "Found",
            vec![("pages/ref.md", "- [[Missing]] #Missing\n")],
            None,
        ),
    ] {
        let (a, _) = fixture(&format!("rename-{label}-new"));
        let (b, _) = fixture(&format!("rename-{label}-old"));
        for root in [&a, &b] {
            for (rel, body) in &files {
                put(root, rel, body);
            }
            if let Some(edn) = config {
                put(root, "logseq/config.edn", edn);
            }
        }
        let store = Store::from_legacy(Arc::new(Graph::open(&a)));
        let old = Graph::open(&b);
        let client = pages::rename_page_expected(&store, old_name, new_name, None);
        let legacy = old.rename_page_expected(old_name, new_name, None);
        assert_eq!(
            operation_result(&client, &a),
            operation_result(&legacy, &b),
            "{label} return"
        );
        assert_eq!(disk_tree(&a), disk_tree(&b), "{label} disk");
        if label == "plain" {
            let moved = store.file_id(Area::Pages, "Next Name.md").unwrap();
            let disk_rev = store.read(&moved, None).unwrap().1;
            assert_eq!(
                store.page(&store.as_page(&moved).unwrap()).unwrap().rev,
                disk_rev
            );
        }
        if label == "case-only" {
            assert!(a.join("pages/target.md").exists());
            assert!(!a.join("pages/TARGET.md").exists());
        }
    }
}

#[test]
fn page_rename_refusals_match_legacy_and_keep_disk() {
    for (label, files) in [
        (
            "target-exists",
            vec![("pages/x.md", "- x\n"), ("pages/y.md", "- y\n")],
        ),
        (
            "org-h1",
            vec![
                ("pages/x.md", "- x\n"),
                ("pages/ref.org", "* Parent\n*** [[x]]\n"),
            ],
        ),
    ] {
        let (a, _) = fixture(&format!("rename-refusal-{label}-new"));
        let (b, _) = fixture(&format!("rename-refusal-{label}-old"));
        for root in [&a, &b] {
            for (rel, body) in &files {
                put(root, rel, body);
            }
        }
        let store = Store::from_legacy(Arc::new(Graph::open(&a)));
        let old = Graph::open(&b);
        let before = disk_tree(&a);
        let to = if label == "target-exists" { "y" } else { "z" };
        let client = pages::rename_page_expected(&store, "x", to, None);
        let legacy = old.rename_page_expected("x", to, None);
        assert_eq!(
            operation_result(&client, &a),
            operation_result(&legacy, &b),
            "{label} result"
        );
        assert_eq!(disk_tree(&a), before, "{label} client changed disk");
        assert_eq!(disk_tree(&a), disk_tree(&b), "{label} legacy disk");
    }
}

#[test]
fn page_merge_delete_and_rescue_match_legacy_bytes() {
    use tine_core::model::PageKind;
    let (a, _) = fixture("page-operations-new");
    let (b, _) = fixture("page-operations-old");
    for root in [&a, &b] {
        put(
            root,
            "pages/src.md",
            "alias:: Alias\ntags:: shared\n- moved\n",
        );
        put(root, "pages/dst.md", "tags:: keep\n- kept\n");
        put(root, "journals/Loose.md", "- rescued\n");
        put(root, "pages/delete.md", "- gone\n");
        put(root, "pages/ref.md", "- [[src]] and [[dst]]\n");
    }
    let store = Store::from_legacy(Arc::new(Graph::open(&a)));
    let old = Graph::open(&b);
    pages::merge_pages(&store, "pages/src.md", "pages/dst.md").unwrap();
    old.merge_pages("pages/src.md", "pages/dst.md").unwrap();
    assert_eq!(disk_tree(&a), disk_tree(&b), "merge bytes");
    pages::rename_file_to_page(&store, "journals/Loose.md", "Rescued").unwrap();
    old.rename_file_to_page("journals/Loose.md", "Rescued")
        .unwrap();
    assert_eq!(disk_tree(&a), disk_tree(&b), "rescue bytes");
    let id = store.file_id(Area::Pages, "delete.md").unwrap();
    let stale = store.read(&id, None).unwrap().1;
    put(&a, "pages/delete.md", "- later\n");
    put(&b, "pages/delete.md", "- later\n");
    let before_delete = disk_tree(&a);
    assert!(
        pages::delete_page_expected(&store, "delete", PageKind::Page, None, Some(&stale)).is_err()
    );
    assert_eq!(disk_tree(&a), before_delete, "stale delete changed disk");
    pages::delete_page_expected(&store, "delete", PageKind::Page, None, None).unwrap();
    old.delete_page_expected("delete", PageKind::Page, None)
        .unwrap();
    assert_eq!(disk_tree(&a), disk_tree(&b), "delete bytes");
}

#[test]
fn org_merge_and_binary_rescue_match_legacy() {
    let (a, _) = fixture("org-merge-new");
    let (b, _) = fixture("org-merge-old");
    for root in [&a, &b] {
        put(root, "pages/src.org", "* moved\n");
        put(root, "pages/dst.org", "* kept\n");
        put(root, "pages/ref.md", "- [[src]] [[dst]]\n");
        fs::create_dir_all(root.join("journals")).unwrap();
        fs::write(root.join("journals/Loose.md"), [0xff, 0xfe, 0x00]).unwrap();
    }
    let store = Store::from_legacy(Arc::new(Graph::open(&a)));
    let old = Graph::open(&b);
    let client = pages::merge_pages(&store, "pages/src.org", "pages/dst.org");
    let legacy = old.merge_pages("pages/src.org", "pages/dst.org");
    assert_eq!(operation_result(&client, &a), operation_result(&legacy, &b));
    assert_eq!(disk_tree(&a), disk_tree(&b));
    pages::rename_file_to_page(&store, "journals/Loose.md", "Rescued").unwrap();
    old.rename_file_to_page("journals/Loose.md", "Rescued")
        .unwrap();
    assert_eq!(disk_tree(&a), disk_tree(&b));
}

#[test]
fn page_rename_retries_external_change_and_rolls_back_third_step_failure() {
    let (a, _) = fixture("rename-fault-retry");
    put(&a, "pages/x.md", "- [[x]]\n");
    put(&a, "pages/one.md", "- [[x]]\n");
    put(&a, "pages/two.md", "- [[x]]\n");
    let store = Store::from_legacy(Arc::new(Graph::open(&a)));
    store.inject_fault(FaultPoint::Stage2MismatchAt(1));
    pages::rename_page_expected(&store, "x", "y", None).unwrap();
    assert!(a.join("pages/y.md").exists());
    assert_eq!(
        fs::read(a.join("pages/two.md")).unwrap(),
        b"external stage-2"
    );
    assert_eq!(fs::read(a.join("pages/one.md")).unwrap(), b"- [[y]]\n");
    let (b, _) = fixture("rename-fault-rollback");
    put(&b, "pages/x.md", "- [[x]]\n");
    put(&b, "pages/one.md", "- [[x]]\n");
    put(&b, "pages/two.md", "- [[x]]\n");
    let store = Store::from_legacy(Arc::new(Graph::open(&b)));
    let before = disk_tree(&b);
    store.inject_fault(FaultPoint::MidStepIoAt(2));
    assert!(pages::rename_page_expected(&store, "x", "y", None).is_err());
    assert_eq!(disk_tree(&b), before, "failed commit changed disk");
}

/// OG `:block/refs` excludes `{{query}}` arguments, so a page that mentions the
/// renamed page only inside a query is not a referrer and keeps its bytes. v0.6.5
/// did this only with a warm reference index; its full-scan fallback rewrote them.
#[test]
fn page_rename_leaves_query_only_mentions_alone() {
    let (a, _) = fixture("rename-query-only");
    put(&a, "pages/x.md", "- body\n");
    put(
        &a,
        "pages/query.md",
        "- {{query (and (task TODO) [[x]])}}\n",
    );
    put(&a, "pages/ref.md", "- [[x]] and #x\n");
    let store = Store::from_legacy(Arc::new(Graph::open(&a)));
    pages::rename_page_expected(&store, "x", "y", None).unwrap();
    assert_eq!(
        fs::read(a.join("pages/query.md")).unwrap(),
        b"- {{query (and (task TODO) [[x]])}}\n"
    );
    assert_eq!(
        fs::read(a.join("pages/ref.md")).unwrap(),
        b"- [[y]] and #y\n"
    );
    assert!(a.join("pages/y.md").exists() && !a.join("pages/x.md").exists());
}

/// Corpus acceptance for the pages client: on two copies of a real-shaped graph
/// (`TINE_CORPUS`, e.g. the anonymized graph), rename a spread of pages with
/// v0.6.5 and with the client and require identical return kinds and disk
/// trees after each rename. Run with `--ignored`; never point it at a live graph.
#[test]
#[ignore]
fn corpus_renames_match_legacy() {
    let Ok(corpus) = std::env::var("TINE_CORPUS") else {
        eprintln!("TINE_CORPUS unset; skipping");
        return;
    };
    let copy = |label: &str| {
        let (root, _) = fixture(label);
        fs::remove_dir_all(&root).unwrap();
        let status = std::process::Command::new("cp")
            .args(["-a", &corpus])
            .arg(&root)
            .status()
            .unwrap();
        assert!(status.success());
        root
    };
    let new_root = copy("corpus-new");
    let old_root = copy("corpus-old");
    let store = Store::from_legacy(Arc::new(Graph::open(&new_root)));
    let old = Graph::open(&old_root);
    let names: Vec<String> = {
        let graph = store.whole_graph().unwrap();
        let mut names: Vec<String> = graph
            .inventory()
            .0
            .iter()
            .filter(|entry| !entry.is_journal)
            .map(|entry| entry.name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    };
    let step = (names.len() / 25).max(1);
    let mut renamed = 0;
    for name in names.iter().step_by(step) {
        let to = format!("{name} og-renamed");
        let client = pages::rename_page_expected(&store, name, &to, None);
        let legacy = old.rename_page_expected(name, &to, None);
        assert_eq!(
            client.as_ref().map_err(|e| e.kind()),
            legacy.as_ref().map_err(|e| e.kind()),
            "{name}: client {client:?} legacy {legacy:?}"
        );
        renamed += client.is_ok() as usize;
        let (a, b) = (disk_tree(&new_root), disk_tree(&old_root));
        if a != b {
            let a: std::collections::BTreeMap<_, _> = a.into_iter().collect();
            let b: std::collections::BTreeMap<_, _> = b.into_iter().collect();
            let differing: Vec<&String> = a
                .keys()
                .chain(b.keys())
                .filter(|rel| a.get(*rel) != b.get(*rel))
                .collect();
            // Deliberate difference: a content-unchanged move is a rename, so
            // the client leaves no trash copy of the source. Accept exactly a
            // legacy-only trash entry whose bytes are live in the client tree.
            let only_redundant_trash = differing.iter().all(|rel| {
                rel.starts_with("logseq/.tine-trash/")
                    && a.get(*rel).is_none()
                    && a.iter().any(|(live, bytes)| {
                        !live.starts_with("logseq/") && Some(bytes) == b.get(*rel)
                    })
            });
            assert!(
                only_redundant_trash,
                "disk differs after renaming {name}: {differing:?}"
            );
            // Align the trees so the next rename compares from equal states.
            // (`disk_tree` drops the trash stamp: `dir/__name` is `dir/<stamp>__name`.)
            for rel in differing {
                let (dir, name) = rel.rsplit_once("/__").unwrap();
                for entry in fs::read_dir(old_root.join(dir)).unwrap() {
                    let path = entry.unwrap().path();
                    let file = path.file_name().unwrap().to_string_lossy().into_owned();
                    if file.ends_with(&format!("__{name}")) {
                        fs::remove_file(&path).unwrap();
                    }
                }
            }
        }
    }
    eprintln!(
        "renamed {renamed} of {} sampled pages",
        names.len().div_ceil(step)
    );
    fs::remove_dir_all(&new_root).unwrap();
    fs::remove_dir_all(&old_root).unwrap();
}
