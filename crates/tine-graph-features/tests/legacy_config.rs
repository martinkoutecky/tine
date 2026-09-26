use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::config::Config;
use tine_core::model::Format;
use tine_graph_features::config;
use tine_store::Store;

fn fixture(label: &str, input: &str) -> (PathBuf, Store) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-config-port-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(dir.join("logseq/config.edn"), input).unwrap();
    let store = Store::open(&dir, Default::default()).unwrap().0;
    (dir, store)
}

fn content(dir: &PathBuf) -> String {
    fs::read_to_string(dir.join("logseq/config.edn")).unwrap()
}

#[test]
fn set_timetracking_enabled_round_trips() {
    let (dir, store) = fixture(
        "ttrack",
        "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
    );
    config::set_timetracking_enabled(&store, false).unwrap();
    let after = content(&dir);
    assert!(
        after.contains(":feature/enable-timetracking? false"),
        "not written: {after}"
    );
    assert!(after.contains(":start-of-week 0"), "other keys preserved");
    assert!(!Config::parse(&after).enable_timetracking);
    config::set_timetracking_enabled(&store, true).unwrap();
    assert!(Config::parse(&content(&dir)).enable_timetracking);
}

#[test]
fn show_brackets_defaults_parses_and_round_trips() {
    assert!(Config::parse("{}").show_brackets);
    assert!(!Config::parse("{:ui/show-brackets? false}").show_brackets);
    let (dir, store) = fixture(
        "brackets",
        "{:preferred-format \"Markdown\"\n ;; preserve this comment\n :start-of-week 0}\n",
    );
    config::set_show_brackets(&store, false).unwrap();
    let after = content(&dir);
    assert!(
        after.contains(":ui/show-brackets? false"),
        "not written: {after}"
    );
    assert!(after.contains(":start-of-week 0"), "other keys preserved");
    assert!(
        after.contains(";; preserve this comment"),
        "comments preserved"
    );
    assert!(!Config::parse(&after).show_brackets);
}

#[test]
fn set_preferred_format_replaces_keyword_value() {
    let (dir, store) = fixture("kw", "{:preferred-format :markdown\n :start-of-week 0}\n");
    config::set_preferred_format(&store, Format::Org).unwrap();
    let after = content(&dir);
    assert!(
        after.contains(":preferred-format \"Org\""),
        "keyword not replaced: {after}"
    );
    assert!(
        !after.contains(":markdown"),
        "stale keyword left behind: {after}"
    );
    assert!(after.contains(":start-of-week 0"), "other keys preserved");
    assert_eq!(Config::parse(&after).preferred_format, Format::Org);
}

#[test]
fn set_preferred_format_round_trips() {
    let (dir, store) = fixture(
        "fmt",
        "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
    );
    config::set_preferred_format(&store, Format::Org).unwrap();
    let after = content(&dir);
    assert!(
        after.contains(":preferred-format \"Org\""),
        "value flipped: {after}"
    );
    assert!(after.contains(":start-of-week 0"), "other keys preserved");
    assert_eq!(Config::parse(&after).preferred_format, Format::Org);
    let id = store.file_id(tine_store::Area::Meta, "config.edn").unwrap();
    let rev = store.read(&id, None).unwrap().1;
    let mut tx = store.transaction();
    tx.replace(&id, rev, b"{}\n".to_vec());
    assert!(matches!(
        tx.commit(),
        tine_store::TxOutcome::Committed { .. }
    ));
    config::set_preferred_format(&store, Format::Org).unwrap();
    assert_eq!(Config::parse(&content(&dir)).preferred_format, Format::Org);
}

#[test]
fn set_journal_page_title_format_round_trips() {
    let (dir, store) = fixture(
        "date",
        "{:preferred-format \"Markdown\"\n :start-of-week 0}\n",
    );
    config::set_journal_page_title_format(&store, "yyyy-MM-dd").unwrap();
    let after = content(&dir);
    assert!(
        after.contains(":journal/page-title-format \"yyyy-MM-dd\""),
        "not written: {after}"
    );
    assert!(
        after.contains(":preferred-format \"Markdown\""),
        "other keys clobbered: {after}"
    );
    assert_eq!(
        Config::parse(&after).journal_page_title_format.as_deref(),
        Some("yyyy-MM-dd")
    );
    config::set_journal_page_title_format(&store, "MMMM do, yyyy").unwrap();
    let after2 = content(&dir);
    assert!(
        after2.contains(":journal/page-title-format \"MMMM do, yyyy\""),
        "value not replaced: {after2}"
    );
    assert!(
        !after2.contains("\"yyyy-MM-dd\""),
        "stale value left behind: {after2}"
    );
}
