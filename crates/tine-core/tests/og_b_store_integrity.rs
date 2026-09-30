use tine_core::config::{Config, FileNameFormat};
use tine_core::edn::{self, Edn};

#[test]
fn config_reads_only_direct_entries_at_each_map_level() {
    let config = Config::parse(
        r#"{:extension {:favorites ["Shadow"]
        :pages-directory "shadow-pages" :preferred-format "Org"
        :shortcuts {:cmd "shadow"} :macros {"x" "shadow"}
        :hidden ["shadow"] :start-of-week 1 :property-pages/enabled? false
        :default-templates {:journals "Shadow"}
        :logbook/settings {:enabled-in-all-blocks true}}
      :favorites ["Real"] :pages-directory "real-pages"
      :shortcuts {:cmd "real"} :macros {"x" "real"} :hidden ["real"]
      :start-of-week 3 :property-pages/enabled? true
      :default-templates {:extension {:journals "Shadow"} :journals "Real"}
      :logbook/settings {:extension {:enabled-in-all-blocks true}
                         :enabled-in-all-blocks false}}"#,
    );
    assert_eq!(config.favorites, ["Real"]);
    assert_eq!(config.pages_dir, "real-pages");
    assert_eq!(config.preferred_format, tine_core::model::Format::Md);
    assert_eq!(config.shortcuts["cmd"], "real");
    assert_eq!(config.macros["x"], "real");
    assert_eq!(config.hidden, ["real"]);
    assert_eq!(config.start_of_week, 3);
    assert!(config.property_pages_enabled);
    assert_eq!(config.default_journal_template.as_deref(), Some("Real"));
    assert!(!config.logbook.enabled_in_all_blocks);
}

#[test]
fn config_strings_share_the_edn_decoder() {
    let config = Config::parse(
        r#"{:favorites ["\u0041" "\101" "\uD83D\uDE00"]
        :pages-directory "\u0070ages" :hidden ["\u0041" "\101" "\uD83D\uDE00"]
        :default-home {:page "\u0041"}
        :default-templates {:journals "\u0041"}
        :shortcuts {:cmd "\u0041"} :macros {"\u0041" "\t\n\b\f\r"}}"#,
    );
    assert_eq!(config.favorites, ["A", "A", "😀"]);
    assert_eq!(config.hidden, config.favorites);
    assert_eq!(config.pages_dir, "pages");
    assert_eq!(config.default_home.as_deref(), Some("A"));
    assert_eq!(config.default_journal_template.as_deref(), Some("A"));
    assert_eq!(config.shortcuts["cmd"], "A");
    assert_eq!(config.macros["A"], "\t\n\u{8}\u{c}\r");
}

#[test]
fn exact_l03_unicode_rename_input_is_already_fixed() {
    let raw = "* Example [[Old]]\n#+aaaaaé\n";
    let renames = std::collections::HashMap::from([("old".into(), "New".into())]);
    assert_eq!(
        tine_core::refs::rename_refs_multi(raw, &renames, true, FileNameFormat::TripleLowbar),
        "* Example [[New]]\n#+aaaaaé\n"
    );
}

#[test]
fn config_readers_have_one_root_selector_and_one_string_decoder() {
    let source = include_str!("../src/config.rs");
    let readers = source
        .split("fn read_string_at")
        .nth(1)
        .unwrap()
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert!(!readers.contains("find_keyword("),
        "I-12: config readers must use the setters' root selector; exemplar config::find_top_level_keyword");
    for second_decoder in ["fn unescape", "fn decode_hidden_string"] {
        assert!(!source.contains(second_decoder),
            "I-12: all config strings use edn::parse_strict through config::read_string_at; remove {second_decoder}");
    }
    assert!(
        readers.contains("crate::edn::parse_strict"),
        "I-12: config::read_string_at is the EDN decoder's client"
    );
}

#[test]
fn a_torn_later_form_does_not_relocate_an_already_read_directory() {
    let config = Config::parse(r#"{:pages-directory "archive" :favorites ["Real"] :unfinished ["#);
    assert_eq!(config.pages_dir, "archive");
    assert_eq!(config.favorites, ["Real"]);
    let nested = Config::parse(
        r#"{:extension {:pages-directory "shadow"} :pages-directory "archive" :unfinished ["#,
    );
    assert_eq!(nested.pages_dir, "archive");
}
