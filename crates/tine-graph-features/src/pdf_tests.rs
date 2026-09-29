use super::*;
use tine_store::FaultPoint;

#[test]
fn conflict_read_refuses_malformed_sidecar_instead_of_reporting_empty_disk() {
    let root = std::env::temp_dir().join(format!("tine-pdf-conflict-read-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("assets/paper.edn"), "not edn").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    assert_eq!(
        read_highlights_checked(&store, "paper.pdf")
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_sidecar_crop_can_be_rolled_back_to_recoverable_trash() {
    let root = std::env::temp_dir().join(format!(
        "tine-pdf-crop-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    let sidecar = b"{:highlights [] :extra {}}";
    std::fs::write(root.join("assets/paper.edn"), sidecar).unwrap();
    assert!(root.join("assets").join(&rel).is_file());
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "../crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).unwrap();
    assert_eq!(
        std::fs::read(root.join("assets/paper.edn")).unwrap(),
        sidecar
    );
    assert!(!root.join("assets").join(&rel).exists());
    assert_eq!(
        std::fs::read_dir(root.join("logseq/.tine-trash/assets"))
            .unwrap()
            .count(),
        1
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn crop_rollback_refuses_a_current_sidecar_reference_or_malformed_sidecar() {
    let root = std::env::temp_dir().join(format!("tine-pdf-crop-guard-{}", std::process::id()));
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    let sidecar = root.join("assets/paper.edn");
    std::fs::write(&sidecar, r#"{:highlights [{:id "crop-id" :page 1 :position {:page 1 :bounding {:top 1 :left 1 :width 2 :height 2} :rects []} :content {:image 42} :properties {:color "yellow"}}] :extra {}}"#).unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    std::fs::write(&sidecar, "not edn").unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    std::fs::write(
        &sidecar,
        "{:highlights [{:id \"crop-id\" :content {:image 42}}]}",
    )
    .unwrap();
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn crop_rollback_keeps_crop_when_sidecar_changes_at_transaction_guard() {
    let root = std::env::temp_dir().join(format!("tine-pdf-crop-race-{}", std::process::id()));
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    std::fs::write(root.join("assets/paper.edn"), "{:highlights [] :extra {}}").unwrap();
    store.inject_fault(FaultPoint::Stage2Mismatch);
    assert!(rollback_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42).is_err());
    assert!(root.join("assets").join(&rel).is_file());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn highlight_deletion_retires_its_crop_through_the_guarded_sidecar_path() {
    let root = std::env::temp_dir().join(format!("tine-pdf-delete-crop-{}", std::process::id()));
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let rect = pdf::Rect {
        top: 1.0,
        left: 1.0,
        width: 2.0,
        height: 2.0,
        source_width: None,
        source_height: None,
    };
    let area = Highlight {
        id: "crop-id".into(),
        page: 1,
        position: pdf::Position {
            page: 1,
            bounding: rect.clone(),
            rects: vec![rect],
        },
        color: "yellow".into(),
        text: None,
        image: Some(42),
    };
    let rel = write_pdf_area_image(&store, "paper.pdf", 1, "crop-id", 42, b"png").unwrap();
    write_highlights(&store, "paper.pdf", "Paper", &[area.clone()], &[]).unwrap();
    assert!(root.join("assets").join(&rel).is_file());
    write_highlights(&store, "paper.pdf", "Paper", &[], &[area]).unwrap();
    assert!(!root.join("assets").join(&rel).exists());
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn optional_page_checks_org_headline_depth_and_preserves_sidecar_reads() {
    let dir = std::env::temp_dir().join(format!("tine-pdf-org-depth-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("pages/notes.org"),
        format!("{} deep\n", "*".repeat(513)),
    )
    .unwrap();
    std::fs::write(dir.join("assets/notes.edn"), "{:ok true}").unwrap();
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let page = store.file_id(Area::Pages, "notes.org").unwrap();
    let sidecar = store.file_id(Area::Assets, "notes.edn").unwrap();
    assert_eq!(
        optional(&store, &page).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(optional(&store, &sidecar).unwrap().unwrap().0, "{:ok true}");
    std::fs::remove_dir_all(dir).unwrap();
}

// ---- C3U: highlight writes never drop sidecar entries or page blocks they do
// not own (I-1/I-2; in-scope: malformed imported content, sync-service delivery).

const A: &str = "6a5604f8-a337-4336-a711-2ba6bc14fb0a";
const B: &str = "6a5604f8-a337-4336-a711-2ba6bc14fb0b";
const C: &str = "6a5604f8-a337-4336-a711-2ba6bc14fb0c";

fn c3u_graph(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "tine-pdf-c3u-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::create_dir_all(root.join("assets")).unwrap();
    root
}

fn c3u_highlight(id: &str, text: &str) -> Highlight {
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
        text: Some(text.into()),
        image: None,
    }
}

fn c3u_entry(id: &str, text: &str) -> String {
    format!(
        r#"{{:id #uuid "{id}" :page 1 :position {{:page 1 :bounding {{:top 1 :left 1 :width 2 :height 2}} :rects ({{:top 1 :left 1 :width 2 :height 2}})}} :content {{:text "{text}"}} :properties {{:color "yellow"}}}}"#
    )
}

fn c3u_page(ids: &[(&str, &str)]) -> String {
    let mut page =
        "file:: [Paper](../assets/paper.pdf)\nfile-path:: ../assets/paper.pdf\n\n".to_string();
    for (id, text) in ids {
        page.push_str(&format!(
            "- {text}\n  hl-page:: 1\n  hl-color:: yellow\n  ls-type:: annotation\n  id:: {id}\n\t- my note on {text}\n"
        ));
    }
    page
}

fn c3u_entries(raw: &str) -> Vec<tine_core::edn::Edn> {
    tine_core::edn::parse_strict(raw)
        .unwrap()
        .get("highlights")
        .and_then(tine_core::edn::Edn::as_vec)
        .unwrap()
        .to_vec()
}

#[test]
fn unreadable_sidecar_entries_and_their_page_blocks_survive_a_highlight_add() {
    // Each shape is one `highlight_from` rejects today.
    let bad_shapes = [
        // reversed bbox (x2 < x1)
        format!(
            r#"{{:id #uuid "{B}" :page 1 :position {{:page 1 :bounding {{:x1 10 :y1 1 :x2 5 :y2 4 :width 600 :height 800}} :rects ()}} :content {{:text "bee"}} :properties {{:color "red"}}}}"#
        ),
        // missing :position
        format!(
            r#"{{:id #uuid "{B}" :page 1 :content {{:text "bee"}} :properties {{:color "red"}}}}"#
        ),
        // non-int page
        format!(
            r#"{{:id #uuid "{B}" :page "one" :position {{:page 1 :bounding {{:top 1 :left 1 :width 2 :height 2}} :rects ()}} :content {{:text "bee"}}}}"#
        ),
        // source width 0
        format!(
            r#"{{:id #uuid "{B}" :page 1 :position {{:page 1 :bounding {{:x1 1 :y1 1 :x2 5 :y2 4 :width 0 :height 800}} :rects ()}} :content {{:text "bee"}}}}"#
        ),
    ];
    for (n, bad) in bad_shapes.iter().enumerate() {
        let root = c3u_graph(&format!("bad{n}"));
        let sidecar = format!(
            "{{:highlights [{} {bad}] :extra {{:page 3}}}}\n",
            c3u_entry(A, "aye")
        );
        std::fs::write(root.join("assets/paper.edn"), &sidecar).unwrap();
        std::fs::write(
            root.join("pages/hls__paper.md"),
            c3u_page(&[(A, "aye"), (B, "bee")]),
        )
        .unwrap();
        let store = Store::open(&root, Default::default()).unwrap().0;
        let loaded = read_highlights_checked(&store, "paper.pdf").unwrap();
        assert_eq!(loaded.len(), 1, "shape {n}: fixture must be unreadable");
        let mut next = loaded.clone();
        next.push(c3u_highlight(C, "sea"));
        write_highlights(&store, "paper.pdf", "Paper", &next, &loaded).unwrap();

        let written = std::fs::read_to_string(root.join("assets/paper.edn")).unwrap();
        let bad_value = tine_core::edn::parse_strict(bad).unwrap();
        assert!(
            c3u_entries(&written).contains(&bad_value),
            "shape {n}: unreadable entry dropped from sidecar:\n{written}"
        );
        assert_eq!(c3u_entries(&written).len(), 3, "shape {n}: {written}");
        let page = std::fs::read_to_string(root.join("pages/hls__paper.md")).unwrap();
        for needle in [
            format!("id:: {B}"),
            "my note on bee".to_string(),
            format!("id:: {A}"),
            "my note on aye".to_string(),
            format!("id:: {C}"),
        ] {
            assert!(page.contains(&needle), "shape {n}: {needle} lost:\n{page}");
        }
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn highlight_add_before_the_sidecar_syncs_keeps_existing_annotations() {
    // Syncthing/Dropbox delivered the hls page but not (yet) its sidecar.
    let root = c3u_graph("nosidecar");
    std::fs::write(
        root.join("pages/hls__paper.md"),
        c3u_page(&[(A, "aye"), (B, "bee")]),
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let loaded = read_highlights_checked(&store, "paper.pdf").unwrap();
    assert!(loaded.is_empty());
    write_highlights(
        &store,
        "paper.pdf",
        "Paper",
        &[c3u_highlight(C, "sea")],
        &loaded,
    )
    .unwrap();
    let page = std::fs::read_to_string(root.join("pages/hls__paper.md")).unwrap();
    for needle in [
        format!("id:: {A}"),
        "my note on aye".to_string(),
        format!("id:: {B}"),
        "my note on bee".to_string(),
        format!("id:: {C}"),
    ] {
        assert!(page.contains(&needle), "{needle} lost:\n{page}");
    }
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deleting_a_known_highlight_still_drops_its_block_and_entry() {
    let root = c3u_graph("delete");
    std::fs::write(
        root.join("assets/paper.edn"),
        format!(
            "{{:highlights [{} {}] :extra {{}}}}\n",
            c3u_entry(A, "aye"),
            c3u_entry(B, "bee")
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("pages/hls__paper.md"),
        c3u_page(&[(A, "aye"), (B, "bee")]),
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let loaded = read_highlights_checked(&store, "paper.pdf").unwrap();
    assert_eq!(loaded.len(), 2);
    let keep: Vec<Highlight> = loaded.iter().filter(|h| h.id == A).cloned().collect();
    write_highlights(&store, "paper.pdf", "Paper", &keep, &loaded).unwrap();
    let page = std::fs::read_to_string(root.join("pages/hls__paper.md")).unwrap();
    assert!(page.contains(&format!("id:: {A}")), "{page}");
    assert!(!page.contains(&format!("id:: {B}")), "{page}");
    let written = std::fs::read_to_string(root.join("assets/paper.edn")).unwrap();
    assert_eq!(c3u_entries(&written).len(), 1, "{written}");
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn unchanged_entries_keep_fields_og_cannot_read_across_an_unrelated_edit() {
    // A readable entry carrying one rect og rejects (reversed) and a symbol value.
    let root = c3u_graph("partial");
    let partial = format!(
        r#"{{:id #uuid "{B}" :page 1 :position {{:page 1 :bounding {{:top 1 :left 1 :width 2 :height 2}} :rects ({{:top 1 :left 1 :width 2 :height 2}} {{:x1 9 :y1 1 :x2 3 :y2 4 :width 600 :height 800}})}} :content {{:text "bee"}} :properties {{:color "red"}} :plugin/kind foo.bar/baz}}"#
    );
    std::fs::write(
        root.join("assets/paper.edn"),
        format!(
            "{{:highlights [{} {partial}] :extra {{}}}}\n",
            c3u_entry(A, "aye")
        ),
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let loaded = read_highlights_checked(&store, "paper.pdf").unwrap();
    assert_eq!(loaded.len(), 2);
    // Recolour B (geometry untouched) and add C.
    let mut next = loaded.clone();
    next[1].color = "green".into();
    next.push(c3u_highlight(C, "sea"));
    write_highlights(&store, "paper.pdf", "Paper", &next, &loaded).unwrap();
    let written = std::fs::read_to_string(root.join("assets/paper.edn")).unwrap();
    let entries = c3u_entries(&written);
    let b = entries
        .iter()
        .find(|e| {
            e.get("content")
                .and_then(|c| c.get("text"))
                .and_then(tine_core::edn::Edn::as_str)
                == Some("bee")
        })
        .unwrap();
    assert_eq!(
        b.get("position")
            .and_then(|p| p.get("rects"))
            .and_then(tine_core::edn::Edn::as_vec)
            .map(<[_]>::len),
        Some(2),
        "unreadable rect dropped by a recolour:\n{written}"
    );
    assert!(
        written.contains(":plugin/kind foo.bar/baz"),
        "symbol changed type:\n{written}"
    );
    assert!(written.contains(r#":color "green""#), "{written}");
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn escape_before_a_multibyte_char_is_refused_without_panicking() {
    let root = c3u_graph("escape");
    std::fs::write(
        root.join("assets/paper.edn"),
        "{:highlights [{:id \"x\" :content {:text \"caf\\é\"}}] :extra {}}",
    )
    .unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        read_highlights_checked(&store, "paper.pdf")
    }));
    assert!(
        read.is_ok(),
        "read_highlights panicked on a \\<multibyte> escape"
    );
    let open = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_pdf(&store, "paper.pdf", "Paper")
    }));
    assert!(open.is_ok(), "open_pdf panicked on a \\<multibyte> escape");
    let write = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        write_highlights(
            &store,
            "paper.pdf",
            "Paper",
            &[c3u_highlight(C, "sea")],
            &[],
        )
    }));
    assert!(
        matches!(write, Ok(Err(_))),
        "write must refuse, not panic or replace"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("assets/paper.edn")).unwrap(),
        "{:highlights [{:id \"x\" :content {:text \"caf\\é\"}}] :extra {}}"
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}
