//! og C3Y (C3W W1 class siblings): every file name Tine derives from a user
//! file name — copy temps, trash and conflict copies, unique-collision names,
//! the alternate-extension twin probe — must fit whenever the user's own name
//! fits (255 bytes). A 250-byte asset or an 84-CJK-char page (255 bytes) is a
//! name OG/Logseq writes in place; before C3Y each of these paths failed with
//! ENAMETOOLONG, so the import, delete, marker resolution or create refused.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::model::{BlockDto, Format, PageDto, PageKind};
use tine_graph_features::{assets, pages};
use tine_store::{Content, EditKind, PageId, SaveBase, SaveOutcome, Store, TxOutcome};

const NAME_MAX: usize = 255;

fn fixture(label: &str) -> (PathBuf, Store) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "tine-c3y-{label}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    for dir in ["pages", "journals", "assets", "logseq"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

/// Every name under `dir` (recursively), for leftover and length checks.
fn names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(names(&path));
        } else {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    out
}

fn dto(name: &str, raw: &str) -> PageDto {
    PageDto {
        name: name.into(),
        kind: PageKind::Page,
        title: name.into(),
        pre_block: None,
        blocks: vec![BlockDto {
            id: "c3y".into(),
            raw: raw.into(),
            ..Default::default()
        }],
        rev: None,
        format: Format::Md,
        read_only: false,
        guide: false,
    }
}

/// Y1: a streamed import (the transaction stages the stream, then copies it
/// beside the target through a `.restore.tmp` temp) of a 250-byte asset name.
#[test]
fn y1_streamed_import_of_a_long_asset_name_lands() {
    let (root, store) = fixture("y1");
    let name = format!("{}.pdf", "a".repeat(246));
    assert_eq!(name.len(), 250);
    let source = root.join("source.bin");
    fs::write(&source, b"pdf bytes").unwrap();
    let landed = assets::import_asset(
        &store,
        &name,
        Content::Stream {
            source: fs::File::open(&source).unwrap(),
            max_bytes: u64::MAX,
        },
    )
    .expect("streamed import of a 250-byte name");
    assert_eq!(landed, name);
    assert_eq!(
        fs::read(root.join("assets").join(&name)).unwrap(),
        b"pdf bytes"
    );
    let leftovers: Vec<_> = names(&root.join("assets"))
        .into_iter()
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    let _ = fs::remove_dir_all(&root);
}

/// Y2 (unique collision): a second import of the same 254-byte name must land
/// under a fitting unique name, keeping the extension.
#[test]
fn y2_unique_collision_name_of_a_long_asset_fits() {
    let (root, store) = fixture("y2u");
    let name = format!("{}.pdf", "b".repeat(250));
    assert_eq!(name.len(), 254);
    assert_eq!(assets::save_asset(&store, &name, b"one").unwrap(), name);
    let second = assets::save_asset(&store, &name, b"two").expect("colliding long import");
    assert_ne!(second, name);
    assert!(
        second.len() <= NAME_MAX && second.ends_with("_1.pdf"),
        "{second}"
    );
    assert_eq!(fs::read(root.join("assets").join(&second)).unwrap(), b"two");
    assert_eq!(fs::read(root.join("assets").join(&name)).unwrap(), b"one");
    let _ = fs::remove_dir_all(&root);
}

/// Y2 (trash): deleting a long asset and a long page moves each into
/// recoverable trash under a fitting name that keeps its extension.
#[test]
fn y2_trashing_a_long_asset_and_page_keeps_a_recoverable_copy() {
    let (root, store) = fixture("y2t");
    let name = format!("{}.png", "c".repeat(250));
    assert_eq!(name.len(), 254);
    fs::write(root.join("assets").join(&name), b"png").unwrap();
    assets::trash_asset(&store, &name).expect("trash a 254-byte asset");
    assert!(!root.join("assets").join(&name).exists());
    let trashed = names(&root.join("logseq/.tine-trash"));
    assert_eq!(trashed.len(), 1, "{trashed:?}");
    assert!(trashed[0].len() <= NAME_MAX && trashed[0].ends_with(".png"));
    assert!(trashed[0].contains("__ccc"), "{trashed:?}");

    let title = "漢".repeat(84);
    let rel = format!("pages/{title}.md");
    fs::write(root.join(&rel), "- long page\n").unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    pages::delete_page_expected(&store, None, &title, PageKind::Page, None, None)
        .expect("delete an 84-char page");
    assert!(!root.join(&rel).exists());
    let pages_trash = names(&root.join("logseq/.tine-trash/pages"));
    assert_eq!(pages_trash.len(), 1, "{pages_trash:?}");
    let copy = root.join("logseq/.tine-trash/pages").join(&pages_trash[0]);
    assert_eq!(fs::read_to_string(copy).unwrap(), "- long page\n");
    assert!(pages_trash[0].ends_with(".md") && pages_trash[0].len() <= NAME_MAX);
    let _ = fs::remove_dir_all(&root);
}

/// Y2 (conflict copy): resolving VCS markers in a long-named page stages the
/// pre-resolution bytes under a fitting conflict-trash name.
#[test]
fn y2_marker_resolution_of_a_long_page_stages_its_recovery_copy() {
    const MARKED: &str = "<<<<<<< HEAD\n- mine\n=======\n- theirs\n>>>>>>> feature\n";
    let (root, _) = fixture("y2m");
    let title = "漢".repeat(84);
    let rel = format!("pages/{title}.md");
    fs::write(root.join(&rel), MARKED).unwrap();
    let store = Store::open(&root, Default::default()).unwrap().0;
    let id = PageId::from(rel.as_str());
    let rev = store.read(&id.file(), None).unwrap().1;
    let mut tx = store.transaction(Some(EditKind::ReplacePage));
    tx.save_page(
        &[EditKind::ReplacePage],
        &id,
        SaveBase::ResolvingMarkers(rev),
        &dto(&title, "mine"),
    );
    let outcome = tx.commit();
    assert!(
        matches!(outcome, TxOutcome::Committed { .. }),
        "{outcome:?}"
    );
    assert_eq!(fs::read_to_string(root.join(&rel)).unwrap(), "- mine\n");
    let copies = names(&root.join("logseq/.tine-trash/conflicts"));
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert!(copies[0].contains("__markers__") && copies[0].ends_with(".md"));
    let copy = root.join("logseq/.tine-trash/conflicts").join(&copies[0]);
    assert_eq!(fs::read_to_string(copy).unwrap(), MARKED);
    let _ = fs::remove_dir_all(&root);
}

/// Y3: creating an 84-CJK-char page (255 bytes with `.md`) probes the `.org`
/// twin, a 256-byte name no filesystem can hold: that means "no twin".
#[test]
fn y3_create_of_a_255_byte_page_name_is_not_refused_by_the_twin_probe() {
    let (root, store) = fixture("y3");
    let title = "字".repeat(84);
    let rel = format!("pages/{title}.md");
    assert_eq!(rel.len() - "pages/".len(), NAME_MAX);
    let created = store.save(
        EditKind::CreatePage,
        &PageId::from(rel.as_str()),
        SaveBase::CreateNew,
        &dto(&title, "fresh"),
    );
    assert!(matches!(created, SaveOutcome::Saved(_)), "{created:?}");
    assert!(fs::read_to_string(root.join(&rel))
        .unwrap()
        .contains("fresh"));
    let _ = fs::remove_dir_all(&root);
}
