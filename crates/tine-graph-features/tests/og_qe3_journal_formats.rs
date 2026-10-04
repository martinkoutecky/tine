use std::fs;
use tine_core::config::Config;
use tine_core::date::{Format, JournalDate};
use tine_graph_features::{config, journals};
use tine_store::{Day, Store};

#[test]
fn config_resolves_modern_then_legacy_then_default() {
    for (edn, expected) in [
        ("{}", None),
        (
            "{:journal/page-title-format \"\" :date-formatter \"yyyy-MM-dd\"}",
            Some("yyyy-MM-dd"),
        ),
        ("{:date-formatter \"E, dd-MM-yyyy\"}", Some("E, dd-MM-yyyy")),
        (
            "{:date-formatter \"yyyy-MM-dd\" :journal/page-title-format \"dd-MM-yyyy\"}",
            Some("dd-MM-yyyy"),
        ),
        (
            "{:extension {:date-formatter \"shadow\"} :date-formatter \"yyyy-MM-dd\"}",
            Some("yyyy-MM-dd"),
        ),
        (
            "{;; :journal/page-title-format \"shadow\"\n :date-formatter \"yyyy-MM-dd\"}",
            Some("yyyy-MM-dd"),
        ),
    ] {
        assert_eq!(
            Config::parse(edn).journal_page_title_format.as_deref(),
            expected,
            "{edn}"
        );
    }
}

#[test]
fn feed_and_routed_page_use_every_og_format_without_rewriting_files() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/og-journal-formats.json"
    ))
    .unwrap();
    let date = JournalDate::from_ordinal(20240105);
    for row in golden["formats"].as_array().unwrap() {
        let pattern = row["pattern"].as_str().unwrap();
        let title = row["title"].as_str().unwrap();
        assert_eq!(Format::compile(pattern).format(date), title, "{pattern}");
        assert_eq!(
            Format::compile(pattern).parse(title),
            Some(date),
            "{pattern}"
        );
        for (ext, content) in [
            ("md", "- existing bullet\n"),
            ("org", "* existing bullet\n"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("logseq")).unwrap();
            fs::create_dir_all(dir.path().join("your-directory")).unwrap();
            let edn = format!(
                "{{:journals-directory \"your-directory\" :date-formatter \"{pattern}\"}}\n"
            );
            let cfg = dir.path().join("logseq/config.edn");
            let file = dir.path().join(format!("your-directory/2024_01_05.{ext}"));
            fs::write(&cfg, &edn).unwrap();
            fs::write(&file, content).unwrap();
            let store = Store::open(dir.path(), Default::default()).unwrap().0;
            let feed = journals::feed_page(&store, 10, None).unwrap();
            assert_eq!(feed.pages.len(), 1, "{pattern}/{ext}");
            assert_eq!(feed.pages[0].doc.title, title, "{pattern}/{ext}");
            let routed = store.page(&store.journal_id(Day(20240105))).unwrap();
            assert_eq!(routed.doc.title, title);
            assert!(routed
                .doc
                .blocks
                .iter()
                .any(|b| b.raw.contains("existing bullet")));
            store.close();
            assert_eq!(fs::read_to_string(&cfg).unwrap(), edn);
            assert_eq!(fs::read_to_string(&file).unwrap(), content);
        }
    }
}

#[test]
fn settings_modern_write_preserves_legacy_key_and_wins_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("logseq")).unwrap();
    let path = dir.path().join("logseq/config.edn");
    let original = "{:date-formatter \"E, dd-MM-yyyy\"}\n";
    fs::write(&path, original).unwrap();
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(
        store.config().config.journal_page_title_format.as_deref(),
        Some("E, dd-MM-yyyy")
    );
    config::set_journal_page_title_format(&store, "yyyy-MM-dd").unwrap();
    store.close();
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains(":date-formatter \"E, dd-MM-yyyy\""));
    let store = Store::open(dir.path(), Default::default()).unwrap().0;
    assert_eq!(
        store.config().config.journal_page_title_format.as_deref(),
        Some("yyyy-MM-dd")
    );
    store.close();
}

#[test]
fn bundled_guide_explains_legacy_precedence_and_weekday_choices() {
    let pages = tine_core::guide::bundled_guide_pages();
    let page = pages
        .iter()
        .find(|p| p.title == "Reference/Journals, tasks, and scheduling")
        .unwrap();
    for phrase in [
        ":date-formatter",
        "takes precedence",
        "yyyy年MM月dd日",
        "abbreviated weekdays",
        "legacy setting untouched",
    ] {
        assert!(
            page.markdown.contains(phrase),
            "Guide must explain {phrase}"
        );
    }
}
