use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tine_core::model::{PageDto, PageKind};
use tine_store::{
    Area, ChangeKind, Content, OpenOptions, PageId, Refusal, Resolved, Store, StoreError,
    TxOutcome, Why,
};

struct Fixture(std::path::PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "tine-store-read-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("pages")).unwrap();
        std::fs::create_dir_all(root.join("journals")).unwrap();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("pages/Note.md"), "- before\n").unwrap();
        std::fs::write(root.join("pages/Bad.md"), [0xff]).unwrap();
        std::fs::write(root.join("pages/Org.org"), "* a\n*** c\n").unwrap();
        std::fs::write(root.join("assets/pic.bin"), b"abcdef").unwrap();
        Self(root)
    }

    fn store(&self) -> Store {
        Store::open(&self.0, OpenOptions::default()).unwrap().0
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = self.0.join(format!(
            ".read-fixture-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&temp, bytes).unwrap();
        std::fs::rename(temp, self.0.join(rel)).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn configured_hidden_paths_are_not_page_claimants_or_path_targets() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.0.join("logseq")).unwrap();
    std::fs::create_dir_all(f.0.join("archive/private")).unwrap();
    std::fs::create_dir_all(f.0.join("archive/public")).unwrap();
    std::fs::write(
        f.0.join("logseq/config.edn"),
        r#"{:hidden ["archive/private" "pages/Private"]}"#,
    )
    .unwrap();
    std::fs::write(f.0.join("archive/private/Secret.md"), "- hidden\n").unwrap();
    std::fs::write(f.0.join("archive/public/Visible.Markdown"), "- visible\n").unwrap();
    std::fs::write(f.0.join("pages/Private.md"), "- hidden\n").unwrap();
    let store = f.store();
    assert!(store
        .page_named("Secret", PageKind::Page)
        .unwrap()
        .is_none());
    assert!(store
        .page_named("Private", PageKind::Page)
        .unwrap()
        .is_none());
    assert!(store
        .page_named("Visible", PageKind::Page)
        .unwrap()
        .is_some());
    assert!(store
        .as_page(&PageId::from("archive/private/Secret.md").file())
        .is_none());
    assert!(store.file_id(Area::Pages, "Private.md").is_err());
    assert!(store.page(&PageId::from("pages/Private.md")).is_err());
}

/// `:hidden` has one reader in og (`configured_hidden`), shared by discovery,
/// the watcher, snapshot capture (`Area::Graph`) and scoped restore. Its
/// answers must match master's `GraphTextScope` (graph_text_scope.rs tests
/// `one_trailing_hidden_separator_matches_the_unseparated_literal_prefix` and
/// `empty_hidden_pattern_matches_og_hide_all_behavior`): a byte-exact prefix,
/// so case differences do not hide; one trailing `/` is ignored; malformed
/// aliases are inert; an empty entry hides everything.
#[test]
fn hidden_prefix_answers_match_master_for_listing_and_discovery() {
    let paths = [
        "archive/Page.md",
        "archive-old/Old.md",
        "elsewhere/archive/Else.md",
        "pages/Kept.md",
    ];
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["archive"],
            &["elsewhere/archive/Else.md", "pages/Kept.md"],
        ),
        (
            &["archive/"],
            &["elsewhere/archive/Else.md", "pages/Kept.md"],
        ),
        (&["Archive"], &paths),
        (&["ARCHIVE/"], &paths),
        (
            &[
                "/",
                "/archive",
                "../archive",
                "archive/../private",
                "archive/./private",
                "archive//",
                "archive//nested",
                "archive\\nested",
                " archive",
                "archive ",
                // Master `lexical_components` trims Unicode whitespace (og-T2).
                "\u{a0}archive",
                "archive\u{a0}",
                "archive\u{3000}",
            ],
            &paths,
        ),
        (&[""], &[]),
    ];
    for (hidden, visible) in cases {
        let f = Fixture::new();
        std::fs::remove_file(f.0.join("pages/Note.md")).unwrap();
        std::fs::remove_file(f.0.join("pages/Bad.md")).unwrap();
        std::fs::remove_file(f.0.join("pages/Org.org")).unwrap();
        std::fs::create_dir_all(f.0.join("logseq")).unwrap();
        for rel in paths {
            std::fs::create_dir_all(f.0.join(rel).parent().unwrap()).unwrap();
            std::fs::write(f.0.join(rel), "- x\n").unwrap();
        }
        std::fs::write(
            f.0.join("logseq/config.edn"),
            format!(
                "{{:hidden {}}}",
                serde_json::to_string(hidden).unwrap().replace(',', " ")
            ),
        )
        .unwrap();
        let store = f.store();
        let mut listed: Vec<String> = store
            .scan_area(Area::Graph, None)
            .unwrap()
            .files
            .into_iter()
            .map(|file| file.rel.to_string())
            .filter(|rel| rel.ends_with(".md"))
            .collect();
        listed.sort();
        let mut expected = visible.to_vec();
        expected.sort();
        assert_eq!(listed, expected, "snapshot listing for {hidden:?}");
        for rel in paths {
            let stem = rel.rsplit('/').next().unwrap().trim_end_matches(".md");
            assert_eq!(
                store.page_named(stem, PageKind::Page).unwrap().is_some(),
                visible.contains(&rel),
                "discovery of {rel} for {hidden:?}"
            );
        }
    }
}

/// og-T2: an entry with leading or trailing Unicode whitespace is inert, as
/// master's `lexical_components` (`relative != relative.trim()`) reads it,
/// even where a path starts with those exact bytes (ADR 0062 remaining edge).
#[test]
fn hidden_entry_with_unicode_edge_whitespace_is_inert() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.0.join("logseq")).unwrap();
    for dir in ["archive\u{a0}old", "\u{3000}notes"] {
        std::fs::create_dir_all(f.0.join(dir)).unwrap();
    }
    std::fs::write(f.0.join("archive\u{a0}old/Nbsp.md"), "- x\n").unwrap();
    std::fs::write(f.0.join("\u{3000}notes/Ideo.md"), "- x\n").unwrap();
    std::fs::write(
        f.0.join("logseq/config.edn"),
        "{:hidden [\"archive\u{a0}\" \"\u{3000}notes\"]}",
    )
    .unwrap();
    let store = f.store();
    for name in ["Nbsp", "Ideo"] {
        assert!(
            store.page_named(name, PageKind::Page).unwrap().is_some(),
            "{name} stays visible"
        );
    }
}

/// og-T2 (master `hidden_parse_failed_closed`): a malformed or over-limit
/// `:hidden` value (a torn or hand-broken config.edn from sync or an external
/// editor) hides all graph text instead of reading as "nothing hidden",
/// which would admit the text the owner excluded.
#[test]
fn malformed_hidden_value_hides_all_graph_text() {
    let oversized = format!("{{:hidden [\"{}\"]}}", "x".repeat(64 * 1024));
    for config in [
        r#"{:hidden ["archive" "unterminated"#,
        r#"{:hidden ["bad\q"]}"#,
        oversized.as_str(),
    ] {
        let f = Fixture::new();
        std::fs::create_dir_all(f.0.join("logseq")).unwrap();
        std::fs::create_dir_all(f.0.join("archive")).unwrap();
        std::fs::write(f.0.join("archive/Secret.md"), "- hidden\n").unwrap();
        std::fs::write(f.0.join("logseq/config.edn"), config).unwrap();
        let store = f.store();
        let listed: Vec<String> = store
            .scan_area(Area::Graph, None)
            .unwrap()
            .files
            .into_iter()
            .map(|file| file.rel.to_string())
            .filter(|rel| rel.ends_with(".md"))
            .collect();
        assert!(listed.is_empty(), "{listed:?}");
        for name in ["Secret", "Note"] {
            assert!(
                store.page_named(name, PageKind::Page).unwrap().is_none(),
                "{name} under {config:.40}"
            );
        }
        assert!(store.page(&PageId::from("archive/Secret.md")).is_err());
    }
}

/// og-T2 recovery: fixing a broken `:hidden` is taken in by the watcher's
/// config reload, and the graph text is back without reopening.
#[test]
fn a_repaired_hidden_value_restores_the_graph_text() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.0.join("logseq")).unwrap();
    f.put("logseq/config.edn", br#"{:hidden ["archive" "unterminated"#);
    let store = Store::open(
        &f.0,
        OpenOptions {
            approved_external_assets: None,
            watch: tine_store::WatchMode::Poll,
        },
    )
    .unwrap()
    .0;
    assert!(store.page_named("Note", PageKind::Page).unwrap().is_none());
    f.put("logseq/config.edn", br#"{:hidden ["archive"]}"#);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while store.page_named("Note", PageKind::Page).unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "repaired config not taken in"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    store.close();
}

#[test]
fn page_reads_and_publishes_external_edit() {
    let f = Fixture::new();
    let store = f.store();
    let id = PageId::from("pages/Note.md");
    let first = store.page(&id).unwrap();
    assert_eq!(first.id, id);
    assert_eq!(first.doc.blocks[0].raw, "before");
    assert_eq!(first.doc.kind, PageKind::Page);
    assert!(first.read_only.is_none());
    assert_eq!(
        serde_json::to_string(&first.rev).unwrap(),
        serde_json::to_string(&store.read(&id.file(), None).unwrap().1).unwrap()
    );
    let _ = store.whole_graph().unwrap().complete_page_names("Note", 10);
    f.put("pages/Note.md", b"- after edit\n");
    store.scan_refresh().unwrap();
    let second = store.page(&id).unwrap();
    assert_eq!(second.doc.blocks[0].raw, "after edit");
    assert_ne!(first.rev, second.rev);
    assert!(
        store
            .whole_graph()
            .unwrap()
            .search(
                &tine_store::SearchRequest {
                    text: "after edit".into(),
                    within: None,
                    page_limit: 10,
                    block_limit: 10,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &tine_store::Cancel(Arc::new(false.into()))
            )
            .unwrap()
            .hits
            .len()
            > 0
    );
}

#[test]
fn many_unmatched_openers_on_separate_lines_round_trip() {
    let f = Fixture::new();
    let source = format!(
        "{}- {} unpaired openers\n",
        "- ordinary ( note\n".repeat(2_000),
        "(".repeat(2_000)
    );
    std::fs::write(f.0.join("pages/Note.md"), &source).unwrap();
    let store = f.store();
    assert!(!store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == "pages/Note.md"));
    let id = PageId::from("pages/Note.md");
    let read = store.page(&id).unwrap();
    assert!(matches!(
        store.save(
            tine_store::EditKind::ReplacePage,
            &id,
            tine_store::SaveBase::Existing(read.rev),
            &read.doc
        ),
        tine_store::SaveOutcome::Unchanged(_)
    ));
    assert_eq!(
        std::fs::read_to_string(f.0.join("pages/Note.md")).unwrap(),
        source
    );
}

#[test]
fn direct_first_read_publishes_creation_and_updates_name_claimants() {
    let f = Fixture::new();
    let store = f.store();
    store.whole_graph().unwrap();
    let subscription = store.subscribe();
    f.put("pages/Arrived.md", b"- newly arrived\n");
    let id = PageId::from("pages/Arrived.md");
    assert_eq!(store.page(&id).unwrap().doc.blocks[0].raw, "newly arrived");
    let change = subscription
        .try_recv()
        .unwrap()
        .expect("direct read publication");
    assert_eq!(change.files[0].0, id.file());
    assert_eq!(change.files[0].1, ChangeKind::Created);
    let view = store.whole_graph().unwrap();
    assert!(
        matches!(view.resolve("Arrived", false), Resolved::Existing { id: found, .. } if found == id)
    );
    assert!(view
        .inventory()
        .0
        .iter()
        .any(|entry| entry.name == "Arrived"));
    assert!(view
        .complete_page_names("Arr", 10)
        .iter()
        .any(|entry| entry.name == "Arrived"));
    let mut tx = store.transaction(Some(tine_store::EditKind::ReplacePage));
    tx.create(
        &store.file_id(Area::Pages, "arrived.org").unwrap(),
        Content::Bytes(b"* duplicate\n".to_vec()),
    );
    assert!(matches!(
        tx.commit(),
        TxOutcome::NotCommitted {
            why: Why::Refused(Refusal::Twin { .. }),
            ..
        }
    ));
}

#[test]
fn direct_first_read_updates_journal_day_and_derived_answers() {
    let f = Fixture::new();
    let store = f.store();
    store.whole_graph().unwrap();
    f.put("journals/2026_09_25.org", b"* arrived day\n");
    let id = PageId::from("journals/2026_09_25.org");
    store.page(&id).unwrap();
    let day = tine_store::Day(
        tine_core::date::JournalDate::from_file_stem("2026_09_25")
            .unwrap()
            .ordinal_key(),
    );
    assert_eq!(store.journal_id(day), id);
    let view = store.whole_graph().unwrap();
    assert!(
        matches!(view.resolve("Sep 25th, 2026", true), Resolved::Existing { id: found, .. } if found == id)
    );
    assert!(view
        .inventory()
        .0
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
    assert!(view
        .complete_page_names("Sep 25", 10)
        .iter()
        .any(|entry| entry.name == "Sep 25th, 2026"));
}

#[test]
fn page_reports_invalid_missing_and_undecodable() {
    let f = Fixture::new();
    let store = f.store();
    assert!(matches!(
        store.page(&PageId::from("pages/missing.md")),
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        store.page(&PageId::from("pages/../Bad.md")),
        Err(StoreError::InvalidTarget(_))
    ));
    assert!(matches!(
        store.page(&PageId::from("pages/Bad.md")),
        Err(StoreError::Undecodable)
    ));
    let org = store.page(&PageId::from("pages/Org.org")).unwrap();
    assert_eq!(org.doc.format, tine_core::model::Format::Org);
    assert!(org.read_only.is_some());
    assert!(org.doc.read_only);
}

#[cfg(unix)]
#[test]
fn page_symlinks_are_not_pages() {
    // v0.6.5's walker skips symlinked page files; neither route reads one.
    let f = Fixture::new();
    let outside = f.0.with_extension("outside.md");
    std::fs::write(&outside, "- outside graph\n").unwrap();
    std::os::unix::fs::symlink(&outside, f.0.join("pages/Outside.md")).unwrap();
    std::os::unix::fs::symlink(f.0.join("pages/Note.md"), f.0.join("pages/Linked.md")).unwrap();
    let store = f.store();
    for name in ["Outside", "Linked"] {
        let id = PageId::from(format!("pages/{name}.md"));
        assert!(matches!(store.page(&id), Err(StoreError::InvalidTarget(_))));
        assert!(matches!(
            store.whole_graph().unwrap().resolve(name, false),
            Resolved::Absent { .. }
        ));
    }
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn byte_reads_streaming_and_os_handoff() {
    let f = Fixture::new();
    let store = f.store();
    let id = store.file_id(Area::Assets, "pic.bin").unwrap();
    assert_eq!(store.read(&id, None).unwrap().0, b"abcdef");
    assert_eq!(store.read(&id, Some(6)).unwrap().0, b"abcdef");
    assert!(matches!(
        store.read(&id, Some(5)),
        Err(StoreError::TooLarge { limit: 5, len: 6 })
    ));
    assert_eq!(store.open_read(&id).unwrap().1, 6);
    // Hand-off paths live under the canonical root `Store::open` binds (on
    // Windows a `\\?\` long-name path; on macOS `/private/var`).
    let root = std::fs::canonicalize(&f.0).unwrap();
    assert_eq!(
        store.path_for_os_handoff(&id, false).unwrap(),
        root.join("assets").join("pic.bin")
    );
    let future = store.file_id(Area::Pages, "future.md").unwrap();
    assert_eq!(
        store.path_for_os_handoff(&future, false).unwrap(),
        root.join("pages").join("future.md")
    );
    assert!(matches!(
        store.path_for_os_handoff(&PageId::from("pages/../escape.md").file(), false),
        Err(StoreError::InvalidTarget(_))
    ));
}

#[cfg(unix)]
#[test]
fn streaming_refuses_symlinked_asset() {
    let f = Fixture::new();
    std::os::unix::fs::symlink(f.0.join("assets/pic.bin"), f.0.join("assets/link.bin")).unwrap();
    let store = f.store();
    let id = store.file_id(Area::Assets, "link.bin").unwrap();
    assert!(matches!(
        store.open_read(&id),
        Err(StoreError::StreamSymlink(_))
    ));
    std::os::unix::fs::symlink(f.0.parent().unwrap(), f.0.join("pages/outside")).unwrap();
    let escaping = PageId::from("pages/outside/next.md");
    assert!(matches!(
        store.path_for_os_handoff(&escaping.file(), false),
        Err(StoreError::InvalidTarget(_))
    ));
}

#[test]
fn page_id_keeps_string_wire_form_beside_a_pathless_dto() {
    // The file identity travels as `PageRead.id` (a plain string on the wire),
    // not inside the DTO: the DTO no longer carries a `path`.
    let f = Fixture::new();
    let store = f.store();
    let read = store.page(&PageId::from("pages/Note.md")).unwrap();
    assert_eq!(
        serde_json::to_string(&read.id).unwrap(),
        "\"pages/Note.md\""
    );
    assert_eq!(
        serde_json::from_str::<PageId>("\"pages/Note.md\"").unwrap(),
        read.id
    );
    let real = serde_json::to_string(&read.doc).unwrap();
    assert!(!real.contains("\"path\""), "{real}");
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<PageDto>(&real).unwrap()).unwrap(),
        real
    );
}

#[test]
fn graph_meta_omits_layout_directories() {
    let f = Fixture::new();
    let (_, meta, _) = Store::open(&f.0, OpenOptions::default()).unwrap();
    let value = serde_json::to_value(meta).unwrap();
    assert!(value.get("pages_dir").is_none(), "{value}");
    assert!(value.get("journals_dir").is_none(), "{value}");
}

#[test]
fn unreadable_files_reports_skipped_non_utf8_page() {
    let f = Fixture::new();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, reason)| id.as_str() == "pages/Bad.md" && !reason.is_empty()));
    assert!(view
        .corpus()
        .pages
        .iter()
        .all(|page| page.id != "pages/Bad.md"));
    f.put("pages/Bad.md", b"- now readable\n");
    store.scan_refresh().unwrap();
    assert!(store
        .whole_graph()
        .unwrap()
        .unreadable_files()
        .iter()
        .all(|(id, _)| id.as_str() != "pages/Bad.md"));
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, _)| id.as_str() == "pages/Bad.md"));
}

#[cfg(unix)]
#[test]
fn unreadable_files_reports_unlistable_page_directory() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let nested = f.0.join("pages/nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0)).unwrap();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    assert!(view
        .unreadable_files()
        .iter()
        .any(|(id, reason)| { id.as_str() == "pages/nested" && !reason.is_empty() }));
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    store.close();
}

#[test]
fn resolve_then_page_covers_titles_aliases_namespaces_and_journals() {
    let f = Fixture::new();
    std::fs::write(
        f.0.join("pages/Title.md"),
        "title:: Display Title\n\n- body\n",
    )
    .unwrap();
    std::fs::write(f.0.join("pages/Owner.md"), "alias:: Also Owner\n\n- body\n").unwrap();
    std::fs::write(f.0.join("pages/Parent%2FChild.md"), "- nested name\n").unwrap();
    std::fs::write(f.0.join("journals/2026_09_25.md"), "- journal body\n").unwrap();
    let store = f.store();
    let view = store.whole_graph().unwrap();
    for (name, journal, expected) in [
        ("Display Title", false, "pages/Title.md"),
        ("Also Owner", false, "pages/Owner.md"),
        ("Parent/Child", false, "pages/Parent%2FChild.md"),
        ("Sep 25th, 2026", true, "journals/2026_09_25.md"),
    ] {
        let id = match view.resolve(name, journal) {
            Resolved::Existing { id, .. } => id,
            Resolved::Alias { owners } => owners.into_iter().next().unwrap(),
            Resolved::Absent { .. } => panic!("{name} did not resolve"),
        };
        assert_eq!(id.as_str(), expected);
        assert_eq!(store.page(&id).unwrap().id, id);
    }
}

#[test]
fn page_named_accepts_journal_file_stem() {
    let f = Fixture::new();
    std::fs::write(f.0.join("journals/2026_09_25.md"), "- journal\n").unwrap();
    let store = f.store();
    assert_eq!(
        store
            .page_named("2026_09_25", PageKind::Journal)
            .unwrap()
            .unwrap()
            .id
            .as_str(),
        "journals/2026_09_25.md"
    );
}

#[test]
fn configured_nested_page_directory_keeps_page_identity() {
    let f = Fixture::new();
    std::fs::create_dir_all(f.0.join("logseq")).unwrap();
    std::fs::create_dir_all(f.0.join("archive/pages")).unwrap();
    std::fs::create_dir_all(f.0.join("diary")).unwrap();
    std::fs::write(
        f.0.join("logseq/config.edn"),
        "{:pages-directory \"archive/pages\" :journals-directory \"diary\"}\n",
    )
    .unwrap();
    std::fs::write(f.0.join("archive/pages/Nested.md"), "- nested\n").unwrap();
    let store = f.store();
    let id = store.file_id(Area::Pages, "Nested.md").unwrap();
    let page = store.page(&store.as_page(&id).unwrap()).unwrap();
    assert_eq!(page.id.as_str(), "archive/pages/Nested.md");
    assert_eq!(page.doc.blocks[0].raw, "nested");
}

#[cfg(unix)]
#[test]
fn approved_external_assets_reject_retarget() {
    let f = Fixture::new();
    let approved = f.0.with_extension("approved-assets");
    let other = f.0.with_extension("other-assets");
    std::fs::create_dir_all(&approved).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(approved.join("asset.bin"), b"approved").unwrap();
    std::fs::write(other.join("asset.bin"), b"unapproved").unwrap();
    std::fs::remove_file(f.0.join("assets/pic.bin")).unwrap();
    std::fs::remove_dir(f.0.join("assets")).unwrap();
    std::os::unix::fs::symlink(&approved, f.0.join("assets")).unwrap();
    let store = Store::open(
        &f.0,
        OpenOptions {
            approved_external_assets: Some(approved.clone()),
            ..Default::default()
        },
    )
    .unwrap()
    .0;
    let id = store.file_id(Area::Assets, "asset.bin").unwrap();
    assert_eq!(store.read(&id, None).unwrap().0, b"approved");
    std::fs::remove_file(f.0.join("assets")).unwrap();
    std::os::unix::fs::symlink(&other, f.0.join("assets")).unwrap();
    assert!(matches!(
        store.read(&id, None),
        Err(StoreError::InvalidTarget(_))
    ));
    std::fs::remove_dir_all(approved).unwrap();
    std::fs::remove_dir_all(other).unwrap();
}
