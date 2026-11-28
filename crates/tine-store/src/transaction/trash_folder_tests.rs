//! Which kind folder under the graph trash a transaction files a trashed file
//! in: a sync-conflict copy in either text area, then the pages area, the
//! journals area, `assets/`, then `other/`. A file that would land in
//! `other/` is matched again against a configured folder written with a
//! trailing `/`, as if written without it.
use crate::{Area, Store, TrashIf, TxOutcome};
use std::fs;

/// Trash `rel` (inside `area`) with `config` and answer the kind folder the
/// trashed copy landed in, checking it kept the original bytes.
fn trashed_into(config: &str, area: Area, rel: &str, disk: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    fs::write(dir.path().join("logseq/config.edn"), config).unwrap();
    let file = dir.path().join(disk);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"bytes\x00\x01").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let id = store.file_id(area, rel).unwrap();
    assert_eq!(id.as_str(), disk, "the fixture names the file it wrote");
    let rev = store.read(&id, None).unwrap().1;
    let mut tx = store.transaction(None);
    tx.trash(&id, rev, TrashIf::Any);
    assert!(matches!(tx.commit(), TxOutcome::Committed { .. }));
    store.close();
    assert!(!file.exists(), "the file left its folder");
    let name = disk.rsplit('/').next().unwrap();
    let mut found = Vec::new();
    for kind in fs::read_dir(dir.path().join("logseq/.tine-trash")).unwrap() {
        let kind = kind.unwrap();
        for entry in fs::read_dir(kind.path()).unwrap() {
            let entry = entry.unwrap();
            if entry
                .file_name()
                .to_string_lossy()
                .ends_with(&format!("__{name}"))
            {
                assert_eq!(fs::read(entry.path()).unwrap(), b"bytes\x00\x01");
                found.push(kind.file_name().to_string_lossy().into_owned());
            }
        }
    }
    assert_eq!(found.len(), 1, "one trashed copy: {found:?}");
    found.remove(0)
}

#[test]
fn a_journals_folder_with_a_trailing_slash_files_its_files_under_journals() {
    let folder = trashed_into(
        r#"{:journals-directory "logseq/daily/"}"#,
        Area::Meta,
        "daily/attachment.bin",
        "logseq/daily/attachment.bin",
    );
    assert_eq!(folder, "journals");
}

#[test]
fn a_pages_folder_with_a_trailing_slash_files_its_files_under_pages() {
    let folder = trashed_into(
        r#"{:pages-directory "logseq/notes/"}"#,
        Area::Meta,
        "notes/attachment.bin",
        "logseq/notes/attachment.bin",
    );
    assert_eq!(folder, "pages");
}

#[test]
fn the_pages_area_wins_where_the_journals_folder_lies_inside_it() {
    let folder = trashed_into(
        r#"{:pages-directory "pages" :journals-directory "pages/daily"}"#,
        Area::Pages,
        "daily/attachment.bin",
        "pages/daily/attachment.bin",
    );
    assert_eq!(folder, "pages");
}

/// A journals folder spelled with `./` is matched as written, as before.
#[test]
fn a_dotted_journals_folder_keeps_its_answer() {
    let folder = trashed_into(
        r#"{:journals-directory "logseq/./daily"}"#,
        Area::Meta,
        "daily/attachment.bin",
        "logseq/daily/attachment.bin",
    );
    assert_eq!(folder, "other");
}

#[test]
fn each_area_keeps_its_folder() {
    let config = "{}";
    let cases = [
        (Area::Pages, "a.bin", "pages/a.bin", "pages"),
        (Area::Journals, "b.bin", "journals/b.bin", "journals"),
        (Area::Assets, "c.png", "assets/c.png", "assets"),
        (Area::Meta, "d.bin", "logseq/d.bin", "other"),
        (
            Area::Pages,
            "Foo.sync-conflict-20240101-000000-ABCDEFG.bin",
            "pages/Foo.sync-conflict-20240101-000000-ABCDEFG.bin",
            "conflicts",
        ),
    ];
    for (area, rel, disk, expected) in cases {
        assert_eq!(trashed_into(config, area, rel, disk), expected, "{disk}");
    }
}

/// `<digits>-<digits><rest>`: a trash stamp followed by `rest`.
fn stamped(name: &str, rest: &str) -> bool {
    let Some((ms, tail)) = name.split_once('-') else {
        return false;
    };
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    !ms.is_empty()
        && ms.bytes().all(|b| b.is_ascii_digit())
        && digits > 0
        && &tail[digits..] == rest
}

/// The conflict-trash copies keep their folder and their own names: a
/// marker copy, a pre-transaction copy and a rollback withdrawal.
#[test]
fn conflict_copies_keep_their_folder_and_names() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("pages")).unwrap();
    fs::write(dir.path().join("pages/A.md"), "- a\n").unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    let file = store.file_id(Area::Pages, "A.md").unwrap();
    let plan = store.page_plan(None);
    let marker = plan.stage_marker_file(&file, b"- old\n").unwrap();
    let old = plan.write_old_copy(&file, b"- older\n").unwrap();
    let path = dir.path().join("pages/A.md");
    let withdrawn = match store
        .graph
        .transaction_withdraw_exact(&path, b"- a\n", "tx-undo")
        .unwrap()
    {
        crate::model::Withdrawal::Exact(staged) => staged,
        _ => panic!("the bytes match"),
    };
    let conflicts = "logseq/.tine-trash/conflicts/";
    let marker = marker.as_str().strip_prefix(conflicts).unwrap();
    assert!(stamped(marker, "__markers__A.md"), "{marker}");
    let old = old.as_str().strip_prefix(conflicts).unwrap();
    assert!(stamped(old, "__tx-old__A.md"), "{old}");
    assert_eq!(
        withdrawn.parent().unwrap(),
        store.graph.root.join(conflicts.trim_end_matches('/'))
    );
    let withdrawn = withdrawn.file_name().unwrap().to_str().unwrap();
    assert!(stamped(withdrawn, "__tx-undo__A.md"), "{withdrawn}");
    let trashed = |name: &str| fs::read(store.graph.root.join(conflicts).join(name)).unwrap();
    assert_eq!(trashed(marker), b"- old\n");
    assert_eq!(trashed(old), b"- older\n");
    assert_eq!(trashed(withdrawn), b"- a\n");
    store.close();
}

/// The trailing-slash match only rescues a file from `other/`: where the
/// folder as written already gave a kind, that kind stands.
#[test]
fn a_trailing_slash_moves_only_what_went_to_other() {
    let cases = [
        (
            r#"{:pages-directory "assets/"}"#,
            Area::Assets,
            "crop.png",
            "assets/crop.png",
            "assets",
        ),
        (
            r#"{:journals-directory "assets/"}"#,
            Area::Assets,
            "photo.sync-conflict-20240101-000000-ABCDEFG.png",
            "assets/photo.sync-conflict-20240101-000000-ABCDEFG.png",
            "assets",
        ),
        (
            r#"{:pages-directory "notes/" :journals-directory "notes/daily"}"#,
            Area::Journals,
            "attachment.bin",
            "notes/daily/attachment.bin",
            "journals",
        ),
    ];
    let mut wrong = Vec::new();
    for (config, area, rel, disk, expected) in cases {
        let folder = trashed_into(config, area, rel, disk);
        if folder != expected {
            wrong.push(format!("{config} {disk}: {folder}, not {expected}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
