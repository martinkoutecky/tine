use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::Graph;
use tine_core::pdf::{Highlight, Position, Rect};

fn scratch(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "tine-pdf-area-lifecycle-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(path.join("pages")).unwrap();
    path
}

fn area(id: &str, stamp: i64) -> Highlight {
    Highlight {
        id: id.to_string(),
        page: 2,
        position: Position {
            page: 2,
            bounding: Rect {
                top: 10.0,
                left: 20.0,
                width: 30.0,
                height: 40.0,
                source_width: None,
                source_height: None,
            },
            rects: vec![],
        },
        color: "yellow".to_string(),
        text: None,
        image: Some(stamp),
    }
}

fn seed_sidecar(root: &Path, highlights: &[Highlight]) {
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(
        root.join("assets").join("paper.edn"),
        tine_core::pdf::write_highlights(highlights, ""),
    )
    .unwrap();
}

fn area_path(root: &Path, highlight: &Highlight) -> PathBuf {
    root.join("assets").join("paper").join(format!(
        "{}_{}_{}.png",
        highlight.page,
        highlight.id,
        highlight.image.unwrap()
    ))
}

#[test]
fn deleted_area_cleanup_waits_for_pair_commit_and_keeps_shared_stamps() {
    let failed_root = scratch("failed-pair");
    fs::create_dir_all(failed_root.join("logseq")).unwrap();
    fs::write(
        failed_root.join("logseq").join("config.edn"),
        "{:preferred-format \"Org\"}\n",
    )
    .unwrap();
    let failed = area("11111111-1111-1111-1111-111111111111", 1001);
    seed_sidecar(&failed_root, &[failed.clone()]);
    let failed_graph = Graph::open(&failed_root);
    failed_graph
        .write_pdf_area_image(
            "paper.pdf",
            failed.page,
            &failed.id,
            failed.image.unwrap(),
            b"png",
        )
        .unwrap();
    fs::write(
        failed_root.join("pages").join("hls__paper.org"),
        "* a\n*** c\n",
    )
    .unwrap();
    assert!(failed_graph
        .write_highlights("paper.pdf", "Paper", &[], std::slice::from_ref(&failed.id))
        .is_err());
    assert!(
        area_path(&failed_root, &failed).exists(),
        "failed pair must leave PNG untouched"
    );

    let shared_root = scratch("shared-stamp");
    let removed = area("22222222-2222-2222-2222-222222222222", 2002);
    let keeper = area("33333333-3333-3333-3333-333333333333", 2002);
    seed_sidecar(&shared_root, &[removed.clone(), keeper.clone()]);
    let shared_graph = Graph::open(&shared_root);
    shared_graph
        .write_pdf_area_image(
            "paper.pdf",
            removed.page,
            &removed.id,
            removed.image.unwrap(),
            b"png",
        )
        .unwrap();
    shared_graph
        .write_highlights(
            "paper.pdf",
            "Paper",
            std::slice::from_ref(&keeper),
            &[removed.id.clone(), keeper.id.clone()],
        )
        .unwrap();
    assert!(
        area_path(&shared_root, &removed).exists(),
        "a shared image stamp must keep the PNG"
    );

    let success_root = scratch("successful-pair");
    let deleted = area("44444444-4444-4444-4444-444444444444", 3003);
    seed_sidecar(&success_root, &[deleted.clone()]);
    let success_graph = Graph::open(&success_root);
    success_graph
        .write_pdf_area_image(
            "paper.pdf",
            deleted.page,
            &deleted.id,
            deleted.image.unwrap(),
            b"png",
        )
        .unwrap();
    success_graph
        .write_highlights("paper.pdf", "Paper", &[], std::slice::from_ref(&deleted.id))
        .unwrap();
    assert!(
        !area_path(&success_root, &deleted).exists(),
        "successful pair must move the PNG"
    );
    let asset_trash = success_root
        .join("logseq")
        .join(".tine-trash")
        .join("assets");
    assert!(
        fs::read_dir(asset_trash)
            .unwrap()
            .flatten()
            .any(|entry| fs::read(entry.path()).unwrap() == b"png"),
        "deleted area PNG must remain recoverable"
    );

    for root in [failed_root, shared_root, success_root] {
        let _ = fs::remove_dir_all(root);
    }
}
