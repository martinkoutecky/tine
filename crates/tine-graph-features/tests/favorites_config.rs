//! Family 22: `:tine/favorites-page` is written with membership in ONE guarded
//! config write, byte-identical to master's `set_favorites_page` then
//! `set_favorites` (20d986e67), and read back through GraphMeta.
use std::fs;
use tine_graph_features::config;
use tine_store::Store;

fn graph(label: &str, config_edn: Option<&str>) -> (std::path::PathBuf, Store) {
    let root = std::env::temp_dir().join(format!("tine-fav-config-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("logseq")).unwrap();
    if let Some(text) = config_edn {
        fs::write(root.join("logseq/config.edn"), text).unwrap();
    }
    let store = Store::open(&root, Default::default()).unwrap().0;
    (root, store)
}

fn written(root: &std::path::Path) -> String {
    fs::read_to_string(root.join("logseq/config.edn")).unwrap()
}

#[test]
fn favorites_page_key_is_written_with_membership_like_master() {
    // Master's two writes on `{}`: the page key inserted after `{`, then the
    // membership vector inserted after `{` in front of it.
    let (root, store) = graph("empty", Some("{}\n"));
    config::set_favorites(&store, &["A".into()], Some("Favorites")).unwrap();
    assert_eq!(
        written(&root),
        "{\n :favorites [\"A\"]\n\n :tine/favorites-page \"Favorites\"\n}\n"
    );
    let config = store.config().config;
    let format = tine_core::date::JournalFormat::new(None, None);
    let meta = tine_core::model::GraphMeta::from_config(String::new(), &config, &format);
    assert_eq!(meta.favorites, vec!["A".to_string()]);
    assert_eq!(meta.favorites_page.as_deref(), Some("Favorites"));

    // Replacing keeps one key, the rest of the file, and escapes the name.
    config::set_favorites(&store, &["A".into(), "B".into()], Some("od\"d")).unwrap();
    let text = written(&root);
    assert_eq!(
        text,
        "{\n :favorites [\"A\" \"B\"]\n\n :tine/favorites-page \"od\\\"d\"\n}\n"
    );
    assert_eq!(text.matches(":tine/favorites-page").count(), 1);
    assert_eq!(
        tine_core::config::Config::parse(&text)
            .favorites_page
            .as_deref(),
        Some("od\"d")
    );

    // Membership-only writes leave the page key alone.
    config::set_favorites(&store, &[], None).unwrap();
    assert_eq!(
        written(&root),
        "{\n :favorites []\n\n :tine/favorites-page \"od\\\"d\"\n}\n"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn favorites_page_key_preserves_unknown_keys_and_comments() {
    let input = "{ ; keep me\n :favorites [\"Old\"]\n :unrelated 42}\n";
    let (root, store) = graph("typical", Some(input));
    config::set_favorites(&store, &["New".into()], Some("Favorites")).unwrap();
    assert_eq!(
        written(&root),
        "{\n :tine/favorites-page \"Favorites\"\n ; keep me\n :favorites [\"New\"]\n :unrelated 42}\n"
    );
    // A blank value reads as "no arrangement page".
    assert_eq!(
        tine_core::config::Config::parse("{:tine/favorites-page \"  \"}").favorites_page,
        None
    );
    fs::remove_dir_all(root).unwrap();
}

/// A strict EDN reader for the round trip: every collection closes, every map
/// has an even number of forms, and the file is exactly one top-level map.
/// Returns that map's forms as source text.
fn strict_top_map(text: &str) -> Result<Vec<String>, String> {
    fn blank(b: &[u8], mut i: usize) -> usize {
        loop {
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | b',') {
                i += 1;
            }
            if i < b.len() && b[i] == b';' {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            return i;
        }
    }
    // One form at `i`; returns (end, child forms if it is a map).
    fn form(b: &[u8], i: usize) -> Result<(usize, Option<Vec<(usize, usize)>>), String> {
        let (open, close, start) = match (b.get(i), b.get(i + 1)) {
            (Some(b'"'), _) => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                return if j < b.len() {
                    Ok((j + 1, None))
                } else {
                    Err("unterminated string".into())
                };
            }
            (Some(b'#'), Some(b'{')) => (b'{', b'}', i + 2),
            (Some(b'{'), _) => (b'{', b'}', i + 1),
            (Some(b'['), _) => (b'[', b']', i + 1),
            (Some(b'('), _) => (b'(', b')', i + 1),
            (Some(c), _) if matches!(c, b'}' | b']' | b')') => {
                return Err(format!("stray {}", *c as char))
            }
            (None, _) => return Err("missing form".into()),
            _ => {
                let mut j = i;
                while j < b.len()
                    && !matches!(
                        b[j],
                        b' ' | b'\t'
                            | b'\n'
                            | b'\r'
                            | b','
                            | b'{'
                            | b'}'
                            | b'['
                            | b']'
                            | b'('
                            | b')'
                            | b'"'
                            | b';'
                    )
                {
                    j += 1;
                }
                return Ok((j, None));
            }
        };
        let mut kids = Vec::new();
        let mut j = blank(b, start);
        while b.get(j) != Some(&close) {
            let (end, _) = form(b, j)?;
            kids.push((j, end));
            j = blank(b, end);
        }
        if open == b'{' && b[i] == b'{' && kids.len() % 2 != 0 {
            return Err(format!("map with {} forms", kids.len()));
        }
        Ok((j + 1, (b[i] == b'{').then_some(kids)))
    }
    let b = text.as_bytes();
    let i = blank(b, 0);
    let (end, kids) = form(b, i)?;
    if blank(b, end) != b.len() {
        return Err("trailing forms".into());
    }
    let kids = kids.ok_or("top-level form is not a map")?;
    Ok(kids
        .into_iter()
        .map(|(s, e)| text[s..e].to_string())
        .collect())
}

fn value_of(forms: &[String], key: &str) -> Option<String> {
    forms
        .chunks(2)
        .find(|kv| kv[0] == key)
        .map(|kv| kv[1].clone())
}

#[test]
fn any_existing_value_shape_is_replaced_whole_and_the_file_stays_parsable() {
    for (label, input) in [
        (
            "nil-page",
            "{:tine/favorites-page nil :favorites [\"Old\"]}\n",
        ),
        ("nil-favs", "{:favorites nil :x 1}\n"),
        ("kw-favs", "{:favorites :none :x 1}\n"),
        ("set-favs", "{:favorites #{\"A\" \"B\"} :x 1}\n"),
        (
            "map-page",
            "{:tine/favorites-page {:a \"b\"} :favorites []}\n",
        ),
        ("no-value", "{:x 1 :favorites}\n"),
        (
            "comment-first",
            "; config { with a brace \"and a {string\"\n;; another {\n{:x 1}\n",
        ),
        ("comments-only", "; nothing but a comment {"),
    ] {
        let (root, store) = graph(label, Some(input));
        config::set_favorites(&store, &["A".into(), "B \"q\"".into()], Some("Favs")).unwrap();
        let text = written(&root);
        let forms = strict_top_map(&text).unwrap_or_else(|e| panic!("{label}: {e}\n{text}"));
        assert_eq!(
            value_of(&forms, ":favorites").as_deref(),
            Some("[\"A\" \"B \\\"q\\\"\"]"),
            "{label}\n{text}"
        );
        assert_eq!(
            value_of(&forms, ":tine/favorites-page").as_deref(),
            Some("\"Favs\""),
            "{label}\n{text}"
        );
        let parsed = tine_core::config::Config::parse(&text);
        assert_eq!(
            parsed.favorites,
            vec!["A".to_string(), "B \"q\"".to_string()],
            "{label}"
        );
        assert_eq!(parsed.favorites_page.as_deref(), Some("Favs"), "{label}");
        if label.starts_with("comment") {
            assert!(text.starts_with("; "), "{label}: comments kept\n{text}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn a_config_it_cannot_edit_safely_is_refused_unchanged() {
    for (label, input) in [
        ("list-value", "{:favorites (\"A\") :x 1}\n"),
        ("not-a-map", "[:favorites]\n"),
        ("unterminated", "{:favorites [\"A\"\n"),
    ] {
        let (root, store) = graph(label, Some(input));
        let error = config::set_favorites(&store, &["A".into()], Some("Favs")).unwrap_err();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidData,
            "{label}: {error}"
        );
        assert_eq!(written(&root), input, "{label}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn every_config_writer_inserts_into_the_map_not_a_brace_in_a_comment() {
    let input = "; a { in a comment\n{:x 1}\n";
    let (root, store) = graph("sibling-writers", Some(input));
    config::set_preferred_workflow(&store, "todo").unwrap();
    config::set_start_of_week(&store, 2).unwrap();
    config::set_default_journal_template(&store, Some("Daily")).unwrap();
    config::set_journal_page_title_format(&store, "yyyy-MM-dd").unwrap();
    config::set_preferred_format(&store, tine_core::model::Format::Org).unwrap();
    config::set_show_brackets(&store, false).unwrap();
    config::set_guide_announced(&store, true).unwrap();
    let text = written(&root);
    let forms = strict_top_map(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    for key in [
        ":preferred-workflow",
        ":start-of-week",
        ":default-templates",
        ":journal/page-title-format",
        ":preferred-format",
        ":ui/show-brackets?",
        ":tine/guide-announced?",
    ] {
        assert!(value_of(&forms, key).is_some(), "{key} missing\n{text}");
    }
    assert!(text.starts_with("; a { in a comment\n"), "{text}");
    fs::remove_dir_all(root).unwrap();
}
