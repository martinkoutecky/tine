//! Which pages a publication selects, and the site's own file names (og c3v).
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::publish;
use tine_graph_features::publish_query::publish_static;
use tine_store::Store;

fn graph(label: &str, config: &str, pages: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-c3v-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    for sub in ["pages", "journals", "logseq"] {
        fs::create_dir_all(dir.join(sub)).unwrap();
    }
    fs::write(dir.join("logseq/config.edn"), config).unwrap();
    for (file, text) in pages {
        fs::write(dir.join("pages").join(file), text).unwrap();
    }
    dir
}

fn site(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            out.insert(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(&path).unwrap_or_default(),
            );
        }
    }
    out
}

const SECRET_ID: &str = "77777777-7777-4777-8777-777777777777";

fn opted_out_pages() -> Vec<(&'static str, String)> {
    vec![
        (
            "Open.md",
            format!(
                "- {{{{query (task TODO)}}}}\n- {{{{embed [[Secret]]}}}}\n- {{{{embed (({SECRET_ID}))}}}}\n- see (({SECRET_ID}))\n- {{{{namespace Secret}}}}\n"
            ),
        ),
        (
            "Secret.md",
            format!("public:: false\n\n- TODO OPTED_OUT_TOKEN\n  id:: {SECRET_ID}\n"),
        ),
        ("Secret___Child.md", "public:: false\n\n- CHILD_TOKEN\n".into()),
        ("Diary.org", "#+PUBLIC: false\n* ORG_OPTED_OUT_TOKEN\n".into()),
    ]
}

fn assert_no_opted_out(files: &BTreeMap<String, String>) {
    for (name, text) in files {
        for token in ["OPTED_OUT_TOKEN", "CHILD_TOKEN", "ORG_OPTED_OUT_TOKEN"] {
            assert!(!text.contains(token), "{token} leaked into {name}");
        }
    }
    for name in ["secret.html", "secret-child.html", "diary.html"] {
        assert!(!files.contains_key(name), "{name} was published");
    }
    assert!(files.contains_key("open.html"));
    assert!(!files["search-index.js"].contains("Secret"));
    assert!(!files["pages.html"].contains("Secret"));
}

/// OG `publishing/db.cljs` `clean-export!`: with `:publishing/all-pages-public?`
/// every page is public except one marked `public:: false`, whose blocks are
/// removed too, so no embed, query, ref or namespace list can show them.
#[test]
fn all_pages_public_config_still_excludes_public_false_pages() {
    let pages = opted_out_pages();
    let pages: Vec<_> = pages.iter().map(|(f, t)| (*f, t.as_str())).collect();
    let dir = graph("optout", "{:publishing/all-pages-public? true}\n", &pages);
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let (out, count) = publish::publish_html(&store).unwrap();
    let files = site(Path::new(&out));
    assert_no_opted_out(&files);
    assert_eq!(count, 1, "only Open is published: {:?}", files.keys());
    store.close();
    let _ = fs::remove_dir_all(&dir);
}

/// The explicit "include every page" export is the same OG selection.
#[test]
fn every_page_static_export_excludes_public_false_pages() {
    let pages = opted_out_pages();
    let pages: Vec<_> = pages.iter().map(|(f, t)| (*f, t.as_str())).collect();
    let dir = graph("optout-static", "{}\n", &pages);
    let out = dir.with_extension("out");
    fs::create_dir_all(&out).unwrap();
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let receipt = publish_static(&store, &out, "Site", true).unwrap();
    assert_eq!(receipt.pages, 1);
    assert_no_opted_out(&site(Path::new(&receipt.path)));
    store.close();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&out);
}
