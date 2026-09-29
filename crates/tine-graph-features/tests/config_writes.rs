//! Family 11: every settings write to `logseq/config.edn` edits only its own
//! top-level key (DUP-3 on master, 4ae2f6f4f), refuses a config it cannot
//! parse as a map instead of writing over it, and is atomic across a crash.
//! Contract: `docs/contracts/config-live-reload.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use tine_core::config::Config;
use tine_core::model::Format;
use tine_graph_features::config;
use tine_store::{Area, FaultPoint, Store};

fn scratch(label: &str, input: Option<&str>) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-config-writes-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    if let Some(input) = input {
        fs::write(dir.join("logseq/config.edn"), input).unwrap();
    }
    dir
}

fn open(dir: &Path) -> Store {
    Store::open(dir, Default::default()).unwrap().0
}

fn content(dir: &Path) -> String {
    fs::read_to_string(dir.join("logseq/config.edn")).unwrap()
}

type Setter = fn(&Store) -> io::Result<()>;

/// Every public config.edn setter, with the top-level key it owns and a check
/// that the written file serves the new value.
fn setters() -> Vec<(&'static str, Setter, fn(&Config) -> bool)> {
    vec![
        (
            ":favorites",
            |s| config::set_favorites(s, &["New".into()], None),
            |c| c.favorites == ["New"],
        ),
        (
            ":tine/favorites-page",
            |s| config::set_favorites(s, &["New".into()], Some("Arranged")),
            |c| c.favorites_page.as_deref() == Some("Arranged"),
        ),
        (
            ":default-home",
            |s| config::set_default_home_page(s, Some("Home")),
            |c| c.default_home.as_deref() == Some("Home"),
        ),
        (
            ":preferred-workflow",
            |s| config::set_preferred_workflow(s, "todo"),
            |c| format!("{:?}", c.preferred_workflow).eq_ignore_ascii_case("todo"),
        ),
        (
            ":feature/enable-timetracking?",
            |s| config::set_timetracking_enabled(s, false),
            |c| !c.enable_timetracking,
        ),
        (
            ":ui/show-brackets?",
            |s| config::set_show_brackets(s, false),
            |c| !c.show_brackets,
        ),
        (
            ":shortcut/doc-mode-enter-for-new-block?",
            |s| config::set_doc_mode_enter_for_new_block(s, true),
            |c| c.doc_mode_enter_for_new_block,
        ),
        (
            ":editor/logical-outdenting?",
            |s| config::set_logical_outdenting(s, true),
            |c| c.logical_outdenting,
        ),
        (
            ":tine/guide-announced?",
            |s| config::set_guide_announced(s, true),
            |c| c.guide_announced,
        ),
        (
            ":preferred-format",
            |s| config::set_preferred_format(s, Format::Org),
            |c| c.preferred_format == Format::Org,
        ),
        (
            ":journal/page-title-format",
            |s| config::set_journal_page_title_format(s, "yyyy-MM-dd"),
            |c| c.journal_page_title_format.as_deref() == Some("yyyy-MM-dd"),
        ),
        (
            ":default-templates",
            |s| config::set_default_journal_template(s, Some("Daily")),
            |c| c.default_journal_template.as_deref() == Some("Daily"),
        ),
        (
            ":start-of-week",
            |s| config::set_start_of_week(s, 3),
            |c| c.start_of_week == 3,
        ),
    ]
}

/// DUP-3 (master 4ae2f6f4f): the depth-blind keyword search found a key
/// nested inside another map before the real top-level entry, and the setter
/// spliced its value there. Each setter must leave the nested map
/// byte-for-byte and write its own key at the top level.
#[test]
fn every_setter_edits_the_top_level_key_never_a_nested_shadow() {
    for (key, set, reads_new) in setters() {
        let shadow = format!(":shadow {{{key} {{:journals \"Shadow\"}} :other 1}}");
        let input = format!(
            ";; user comment {{:keep me}}\n{{{shadow}\n :unknown/key #{{\"a\" \"b\"}}\n}}\n"
        );
        let dir = scratch("dup3", Some(&input));
        let store = open(&dir);
        set(&store).unwrap_or_else(|error| panic!("{key}: {error}"));
        let written = content(&dir);
        assert!(
            written.contains(&shadow),
            "{key}: the nested map must survive byte-for-byte:\n{written}"
        );
        assert!(
            written.starts_with(";; user comment {:keep me}\n{"),
            "{key}:\n{written}"
        );
        assert!(
            written.contains(":unknown/key #{\"a\" \"b\"}"),
            "{key}:\n{written}"
        );
        assert!(
            tine_core::config::find_top_level_keyword(&written, key).is_some(),
            "{key}: must be written at the top level:\n{written}"
        );
        assert!(reads_new(&Config::parse(&written)), "{key}:\n{written}");
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Master's DUP-3 case in meaning: an existing top-level key after a nested
/// shadow is replaced in place, the shadow untouched.
#[test]
fn an_existing_top_level_key_after_a_nested_shadow_is_replaced_in_place() {
    let input = "{:default-queries {:preferred-format :org}\n :preferred-format \"Markdown\"}\n";
    let dir = scratch("dup3-existing", Some(input));
    let store = open(&dir);
    config::set_preferred_format(&store, Format::Org).unwrap();
    assert_eq!(
        content(&dir),
        "{:default-queries {:preferred-format :org}\n :preferred-format \"Org\"}\n"
    );
    let input =
        "{:default-templates {:journals \"J\" :favorites [\"nested\"]}\n :favorites [\"real\"]}\n";
    fs::write(dir.join("logseq/config.edn"), input).unwrap();
    config::set_favorites(&store, &["Replaced".into()], None).unwrap();
    assert_eq!(
        content(&dir),
        "{:default-templates {:journals \"J\" :favorites [\"nested\"]}\n :favorites [\"Replaced\"]}\n"
    );
}

/// Refusal scenario (external-editor race / sync delivery): a config.edn that
/// is truncated mid-delivery or is not a map is never written over by a
/// settings change; its bytes stay exactly as delivered.
#[test]
fn every_setter_refuses_a_config_that_is_not_a_balanced_map() {
    for malformed in [
        "{:favorites [\"A\"] :default-home {:page \"H\"",
        "{:favorites [\"A\"]\n :shadow {:start-of-week 1\n",
        "[:not :a :map]\n",
        ":preferred-format :md\n",
    ] {
        let dir = scratch("malformed", Some(malformed));
        let store = open(&dir);
        for (key, set, _) in setters() {
            let error = set(&store).expect_err(key);
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{key}: {error}");
            assert_eq!(
                content(&dir),
                malformed,
                "{key} wrote over a malformed config"
            );
        }
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

/// A missing, empty or comment-only config.edn is not malformed: the first
/// setting write creates a map and keeps the comments.
#[test]
fn a_missing_or_comment_only_config_gets_a_map_and_keeps_its_comments() {
    for (input, expected) in [
        (None, "{\n :start-of-week 3\n}\n"),
        (Some(""), "{:start-of-week 3}\n"),
        (
            Some(";; my notes {not a map}\n"),
            ";; my notes {not a map}\n{:start-of-week 3}\n",
        ),
    ] {
        let dir = scratch("blank", input);
        let store = open(&dir);
        config::set_start_of_week(&store, 3).unwrap();
        assert_eq!(content(&dir), expected);
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

/// A malformed config.edn never blocks opening the graph: pages are listed and
/// readable, and the served configuration is the default one.
#[test]
fn a_malformed_config_never_blocks_open() {
    for malformed in ["{:favorites [\"A\"", "\u{0}\u{1}garbage", "}}}{{{"] {
        let dir = scratch("open", Some(malformed));
        fs::write(dir.join("pages/Kept.md"), "- still here\n").unwrap();
        let store = open(&dir);
        store.whole_graph().unwrap();
        let page = store.file_id(Area::Pages, "Kept.md").unwrap();
        let (bytes, _) = store.read(&page, None).unwrap();
        assert_eq!(bytes, b"- still here\n", "config {malformed:?}");
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn crash_config_write_worker() {
    let Ok(root) = std::env::var("TINE_CONFIG_CRASH_ROOT") else {
        return;
    };
    let store = open(Path::new(&root));
    store.inject_fault(FaultPoint::AbortAfterStep(0));
    let _ = config::set_favorites(&store, &["After".into()], Some("Arranged"));
    panic!("I-2: config write fault did not abort; exemplar config::update");
}

/// I-2: a settings write killed right after its one step reached disk leaves a
/// whole, parseable config.edn with the new value and the rest intact, and the
/// reopened store serves it.
#[test]
fn a_config_write_killed_after_its_step_reopens_whole() {
    let before = ";; keep\n{:favorites [\"Before\"]\n :unknown {:a [1 2]}}\n";
    let dir = scratch("crash", Some(before));
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_config_write_worker", "--nocapture"])
        .env("TINE_CONFIG_CRASH_ROOT", &dir)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "I-2: the child must abort at the config step: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let after = content(&dir);
    // The step reached disk before the abort, so the new bytes are whole.
    assert!(
        after.starts_with(";; keep\n{")
            && after.contains(":favorites [\"After\"]")
            && after.contains(":tine/favorites-page \"Arranged\"")
            && after.contains(":unknown {:a [1 2]}")
            && after.ends_with("}\n"),
        "I-2: config.edn is whole new bytes after a crash at its step:\n{after}"
    );
    let reopened = open(&dir);
    assert_eq!(reopened.config().config.favorites, ["After"]);
    drop(reopened);
    fs::remove_dir_all(dir).unwrap();
}

/// I-1 / Rule 8 door: every config.edn setter reaches the store through the
/// one guarded `update` (a base-revision-checked `Transaction::replace` or
/// `create`), never a filesystem call. Exemplar: `config::set_start_of_week`.
#[test]
fn every_config_setter_goes_through_the_one_guarded_update() {
    const SOURCE: &str = include_str!("../src/config.rs");
    assert_eq!(
        SOURCE.matches("store.transaction(").count(),
        1,
        "I-1: config.edn has one write door, `update`; exemplar config::set_start_of_week"
    );
    assert!(!SOURCE.contains("std::fs") && !SOURCE.contains("fs::write"));
    let setters: Vec<&str> = SOURCE.split("\npub fn set_").skip(1).collect();
    assert_eq!(
        setters.len(),
        12,
        "a new config setter must be added to `setters()` above"
    );
    for body in setters {
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(
            body.contains("update(") || body.contains("update_scalar("),
            "I-1: config setter bypasses `update`: {}",
            &body[..body.find('(').unwrap()]
        );
    }
}

/// Living contract: every test docs/contracts/config-live-reload.md cites
/// exists, so the contract cannot outlive its evidence.
#[test]
fn the_config_contract_cites_only_existing_tests() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let contract =
        std::fs::read_to_string(format!("{root}docs/contracts/config-live-reload.md")).unwrap();
    let cited: Vec<&str> = contract
        .lines()
        .filter_map(|line| line.strip_prefix("- crates/"))
        .collect();
    assert!(cited.len() >= 10, "the contract's Tests list went missing");
    for entry in cited {
        let (file, name) = entry.split_once("::").unwrap();
        let source = std::fs::read_to_string(format!("{root}crates/{file}")).unwrap();
        assert!(
            source.contains(&format!("fn {name}(")),
            "docs/contracts/config-live-reload.md cites missing test {file}::{name}"
        );
    }
}
