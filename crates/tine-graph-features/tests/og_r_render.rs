//! Checkpoint-5 packet R: the static render/export answers agree with the app
//! and with the parser (I-12), print stays single-page work (I-13) and an Org
//! page is rendered as Org. Each test drives the real entry point
//! (`publish::publish_html`, `print::page_print_html`).
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tine_graph_features::{print, publish};
use tine_store::Store;

fn graph(label: &str, config: &str, pages: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tine-ogr-{label}-{}-{}",
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

fn published(label: &str, config: &str, pages: &[(&str, &str)]) -> (BTreeMap<String, String>, usize) {
    let dir = graph(label, config, pages);
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let (out, count) = publish::publish_html(&store).unwrap();
    let files = site(Path::new(&out));
    store.close();
    let _ = fs::remove_dir_all(&dir);
    (files, count)
}

fn printed(label: &str, config: &str, pages: &[(&str, &str)], page: &str) -> String {
    let dir = graph(label, config, pages);
    let store = Store::open(&dir, Default::default()).unwrap().0;
    let html = print::page_print_html(&store, page, Default::default())
        .unwrap()
        .expect("page exists");
    store.close();
    let _ = fs::remove_dir_all(&dir);
    html
}

// ---- render.rs:2056 page_public_flag: parser-owned property entries ----

/// A fenced `public:: true` example in the preamble is code, not the page's flag.
#[test]
fn fenced_public_example_does_not_publish_a_page() {
    let (files, count) = published(
        "fence-public",
        "{}\n",
        &[
            (
                "Fenced.md",
                "```\npublic:: true\n```\n\n- FENCED_EXAMPLE_TOKEN\n",
            ),
            ("Real.md", "public:: true\n\n- REAL_TOKEN\n"),
        ],
    );
    assert_eq!(count, 1, "{:?}", files.keys().collect::<Vec<_>>());
    assert!(!files.values().any(|t| t.contains("FENCED_EXAMPLE_TOKEN")));
    assert!(files.values().any(|t| t.contains("REAL_TOKEN")));
}

/// A fenced `public:: false` example cannot veto a real `public:: true`.
#[test]
fn fenced_public_false_example_does_not_veto_a_public_page() {
    let (files, count) = published(
        "fence-veto",
        "{}\n",
        &[(
            "Real.md",
            "public:: true\n```\npublic:: false\n```\n\n- REAL_TOKEN\n",
        )],
    );
    assert_eq!(count, 1, "{:?}", files.keys().collect::<Vec<_>>());
    assert!(files.values().any(|t| t.contains("REAL_TOKEN")));
}

// ---- render_facets.rs:38 vs render/block.ts:35 isRenderHiddenProp ----

/// The export hides exactly the property chips the app hides (one answerer):
/// the built-in internal keys, `tine.*`, `logseq.table.*`, and the graph's
/// `:block-hidden-properties`; an arbitrary `logseq.*` key is a visible chip.
#[test]
fn export_hides_the_property_chips_the_app_hides() {
    let page = "public:: true\n\n- body\n  title:: SHOWN_TITLE_TOKEN\n  hl-color:: HLCOLOR_TOKEN\n  \
                tine.view:: TINEVIEW_TOKEN\n  logseq.table.version:: TABLEVER_TOKEN\n  \
                Created_At:: CREATED_TOKEN\n  mine:: USERHIDDEN_TOKEN\n  logseq.custom:: CUSTOM_VISIBLE_TOKEN\n  \
                visible:: VISIBLE_TOKEN\n";
    let (files, _) = published(
        "hidden-props",
        "{:block-hidden-properties #{:mine}}\n",
        &[("Props.md", page)],
    );
    let html = &files["props.html"];
    for hidden in [
        "SHOWN_TITLE_TOKEN",
        "HLCOLOR_TOKEN",
        "TINEVIEW_TOKEN",
        "TABLEVER_TOKEN",
        "CREATED_TOKEN",
        "USERHIDDEN_TOKEN",
    ] {
        assert!(!html.contains(hidden), "{hidden} is hidden in the app: {html}");
    }
    for shown in ["CUSTOM_VISIBLE_TOKEN", "VISIBLE_TOKEN"] {
        assert!(html.contains(shown), "{shown} is a visible chip in the app: {html}");
    }
}

// ---- render_lookups.rs:100,104 + render.rs:170,179: page identity ----

/// `{{embed [[É]]}}` finds the page `é` (Logseq page identity folds Unicode case).
#[test]
fn print_embed_finds_a_page_by_unicode_identity() {
    let html = printed(
        "embed-identity",
        "{}\n",
        &[
            ("Host.md", "- {{embed [[É]]}}\n"),
            ("é.md", "- EMBEDDED_ACCENT_TOKEN\n"),
        ],
        "Host",
    );
    assert!(html.contains("EMBEDDED_ACCENT_TOKEN"), "{html}");
}

/// An NFD spelling of a page link reaches the NFC page's selected file.
#[test]
fn published_link_slug_is_found_by_page_identity() {
    // "Caf" takes the plain slug, so the NFC page gets a hashed one; the link
    // is spelled NFD.
    let (files, _) = published(
        "slug-identity",
        "{:publishing/all-pages-public? true}\n",
        &[
            ("Source.md", "- [[Cafe\u{301}]] link\n"),
            ("Caf\u{e9}.md", "- CAFE_TOKEN\n"),
            ("Caf.md", "- plain caf page\n"),
        ],
    );
    let source = &files["source.html"];
    let target = files
        .iter()
        .find(|(_, text)| text.contains("CAFE_TOKEN"))
        .map(|(name, _)| name.clone())
        .unwrap();
    assert!(
        source.contains(&format!("href=\"{target}\"")),
        "the link reaches {target}: {source}"
    );
}

// ---- print.rs:58 + render.rs:815,989,1458: Org is rendered as Org ----

const ORG_PAGE: &str = "* ORGHEAD *bold-org* and ~code-org~\n:PROPERTIES:\n:ID: 11111111-1111-4111-8111-111111111111\n:END:\n** child with /italic-org/\n";

#[test]
fn org_print_renders_org_markup() {
    let html = printed("org-print", "{}\n", &[("Notes.org", ORG_PAGE)], "Notes");
    assert!(html.contains("<strong>bold-org</strong>"), "{html}");
    assert!(html.contains(">code-org</code>"), "{html}");
    assert!(html.contains("<em>italic-org</em>"), "{html}");
    assert!(!html.contains(":PROPERTIES:"), "drawer is metadata: {html}");
}

#[test]
fn org_publish_renders_org_markup() {
    let (files, _) = published(
        "org-publish",
        "{:publishing/all-pages-public? true}\n",
        &[("Notes.org", ORG_PAGE)],
    );
    let html = &files["notes.html"];
    assert!(html.contains("<strong>bold-org</strong>"), "{html}");
    assert!(html.contains(">code-org</code>"), "{html}");
    assert!(html.contains("<em>italic-org</em>"), "{html}");
    assert!(!html.contains(":PROPERTIES:"), "drawer is metadata: {html}");
}
