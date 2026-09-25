use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::pdf::{Highlight, Position, Rect};
use tine_graph_features::{assets, conflicts, journals, pdf};
use tine_store::{model::Graph, Content, Day, FaultPoint, Store};

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
        ("journals", include_str!("../src/journals.rs")),
        ("pdf", include_str!("../src/pdf.rs")),
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
            assert!(
                !source.contains(forbidden),
                "Clients touch no path: {name} contains {forbidden}"
            );
        }
    }
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
