use std::fs;
use std::path::Path;
use std::sync::Arc;
use tine_graph_features::print::PrintOpts;
use tine_graph_features::{print, publish};
use tine_store::model::Graph;
use tine_store::Store;

fn publish_graph(store: &Store) -> std::io::Result<(String, usize)> {
    publish::publish_html(store)
}

const PRINT_ASSET_MAX_BYTES: u64 = 12 * 1024 * 1024;

#[test]
fn publication_snapshot_uses_live_file_runtime_identity() {
    let dir = std::env::temp_dir().join(format!(
        "tine-publish-runtime-identity-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(dir.join("pages/Target.md"), "- target\n").unwrap();
    fs::write(dir.join("pages/Source.md"), "- [[Target]] from source\n").unwrap();

    let store = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let whole = store.whole_graph().unwrap();
    let live_id = whole.backlinks("Target").unwrap()[0].blocks[0].id.clone();
    let corpus = whole.corpus();
    let source = corpus
        .pages
        .iter()
        .find(|page| page.name == "Source")
        .unwrap();
    assert_eq!(source.document.roots[0].uuid, live_id);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_emits_sidebar_search_and_block_anchors() {
    let dir = std::env::temp_dir().join(format!("tine-publish-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:favorites [\"Alpha\"]}\n",
    )
    .unwrap();
    fs::write(
            dir.join("pages").join("Alpha.md"),
            "public:: true\n- # Intro to [[Beta]] and **bold** text\n  id:: 11111111-1111-1111-1111-111111111111\n- a unique searchwidget term\n",
        )
        .unwrap();
    fs::write(
        dir.join("pages").join("Beta.md"),
        "public:: true\n- linking back to [[Alpha]]\n",
    )
    .unwrap();
    // A non-public page must NOT be exported.
    fs::write(dir.join("pages").join("Secret.md"), "- private stuff\n").unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&g).unwrap();
    assert_eq!(count, 2, "only the two public pages");
    let out = std::path::Path::new(&outdir);

    // assets emitted alongside the pages
    assert!(out.join("fuse.min.js").exists(), "vendored Fuse shipped");
    assert!(out.join("app.js").exists(), "app.js shipped");
    assert!(
        out.join("enhance.js").exists(),
        "script-free enhancement shipped"
    );

    // embedded search data: pages + blocks globals, favorite flag, stripped text
    let sidx = fs::read_to_string(out.join("search-index.js")).unwrap();
    assert!(
        sidx.starts_with("window.__tinePages="),
        "{}",
        &sidx[..60.min(sidx.len())]
    );
    let alpha = fs::read_to_string(out.join("alpha.html")).unwrap();
    assert!(alpha.contains("Content-Security-Policy"), "{alpha}");
    assert!(!alpha.contains(" onload="), "{alpha}");
    assert!(sidx.contains("window.__tineBlocks="));
    assert!(
        sidx.contains("\"favorite\":true"),
        "Alpha is a favorite: {sidx}"
    );
    assert!(sidx.contains("searchwidget"), "block content indexed");
    assert!(
        sidx.contains("Intro to Beta and bold text"),
        "markup stripped in index: {sidx}"
    );
    assert!(!sidx.contains("[[Beta]]"), "no raw wiki brackets in index");

    // page html: EVERY block carries an anchor (id:: uuid or generated b{n}); the
    // sidebar + scripts are present; no anchorless <li>.
    let alpha = fs::read_to_string(out.join("alpha.html")).unwrap();
    assert!(
        alpha.contains("id=\"11111111-1111-1111-1111-111111111111\""),
        "id:: anchor kept"
    );
    assert!(
        alpha.contains("id=\"b0\""),
        "id-less block got a generated anchor: {alpha}"
    );
    assert!(!alpha.contains("<li>"), "no anchorless <li>");
    assert!(
        alpha.contains("<aside class=\"sidebar\">"),
        "sidebar present"
    );
    assert!(alpha.contains("id=\"tine-search\""), "search box present");
    assert!(alpha.contains("src=\"app.js\""), "app.js linked");

    // index lists public pages, excludes the private one, and uses the shell.
    let index = fs::read_to_string(out.join("index.html")).unwrap();
    assert!(index.contains("alpha.html") && index.contains("beta.html"));
    assert!(!index.contains("secret.html"), "private page excluded");
    assert!(
        index.contains("<aside class=\"sidebar\">"),
        "index uses the sidebar shell"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_fails_closed_when_public_and_private_files_claim_one_page_identity() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("tine-publish-ambiguous-{unique}"));
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("pages/Twin.md"),
        "public:: true\n- public sentinel\n- {{query (page Twin)}}\n",
    )
    .unwrap();
    fs::write(dir.join("journals/Twin.md"), "- PRIVATE SENTINEL\n").unwrap();
    fs::write(dir.join("pages/Visible.md"), "public:: true\n- visible\n").unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    let out = Path::new(&outdir);
    assert_eq!(count, 1);
    assert!(!out.join("twin.html").exists());
    let all = fs::read_to_string(out.join("search-index.js")).unwrap();
    assert!(!all.contains("PRIVATE SENTINEL"), "{all}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_macros_never_expand_private_graph_content() {
    let dir = std::env::temp_dir().join(format!(
        "tine-publish-private-macros-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    let private_id = "99999999-9999-4999-8999-999999999999";
    fs::write(
            dir.join("pages/Dashboard.md"),
            format!(
                "public:: true\n- {{{{query (task TODO)}}}}\n- {{{{embed [[Secret]]}}}}\n- {{{{embed (({private_id}))}}}}\n- {{{{namespace PrivateNS}}}}\n"
            ),
        )
        .unwrap();
    fs::write(
        dir.join("pages/Secret.md"),
        format!("- TODO PRIVATE_QUERY_AND_EMBED_TOKEN\n  id:: {private_id}\n"),
    )
    .unwrap();
    fs::write(
        dir.join("pages/PrivateNS___Child.md"),
        "- PRIVATE_NAMESPACE_TOKEN\n",
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 1);
    let dashboard =
        fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
    assert!(!dashboard.contains("PRIVATE_QUERY_AND_EMBED_TOKEN"));
    assert!(!dashboard.contains("PRIVATE_NAMESPACE_TOKEN"));
    assert!(!dashboard.contains("PrivateNS/Child"));
    assert!(
        dashboard.contains("No matching blocks")
            || dashboard.contains("Embedded content is not public"),
        "private macro targets should fail closed: {dashboard}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_uses_one_fresh_snapshot_after_external_visibility_rewrite() {
    let dir = std::env::temp_dir().join(format!(
        "tine-publish-external-visibility-snapshot-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    let source_id = "99999999-9999-4999-8999-999999999998";
    fs::write(
            dir.join("pages/Dashboard.md"),
            format!(
                "public:: true\n- {{{{query (task TODO)}}}}\n- {{{{query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}}}}\n- {{{{embed (({source_id}))}}}}\n"
            ),
        )
        .unwrap();
    fs::write(
        dir.join("pages/Source.md"),
        format!("- TODO PRIVATE_STALE_TOKEN\n  id:: {source_id}\n"),
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    graph.whole_graph().unwrap().corpus();

    // Simulate a sync/editor outside Tine changing both visibility and body
    // after the live graph cache was populated. Publication must not combine
    // the fresh public classification with stale cached query/embed DTOs.
    fs::write(
        dir.join("pages/Source.md"),
        format!("public:: true\n- harmless current body\n  id:: {source_id}\n"),
    )
    .unwrap();

    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 2);
    let out = Path::new(&outdir);
    let dashboard = fs::read_to_string(out.join("dashboard.html")).unwrap();
    let source = fs::read_to_string(out.join("source.html")).unwrap();
    let search = fs::read_to_string(out.join("search-index.js")).unwrap();
    let published = format!("{dashboard}\n{source}\n{search}");
    assert!(published.contains("harmless current body"), "{published}");
    assert!(
        !published.contains("PRIVATE_STALE_TOKEN"),
        "a stale live-graph query/embed result crossed the publication snapshot: {published}"
    );
    assert!(
        dashboard.contains("harmless current body"),
        "the block embed must resolve from the same fresh snapshot: {dashboard}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn republish_retires_pages_that_are_no_longer_public() {
    let dir =
        std::env::temp_dir().join(format!("tine-publish-retire-stale-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("pages/Visible.md"),
        "public:: true\n- visible body\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages/Secret.md"),
        "public:: true\n- stale private token\n",
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 2);
    let out = std::path::Path::new(&outdir);
    assert!(out.join("secret.html").exists());

    fs::write(dir.join("pages/Secret.md"), "- stale private token\n").unwrap();
    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 1);
    let out = std::path::Path::new(&outdir);
    assert!(out.join("visible.html").exists());
    assert!(
        !out.join("secret.html").exists(),
        "a formerly public page must not remain deployable after republish"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_uses_welcome_home_and_public_reverse_block_refs() {
    let dir = std::env::temp_dir().join(format!("tine-publish-welcome-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    let id = "11111111-1111-1111-1111-111111111111";
    fs::write(
        dir.join("pages/Welcome to Tine.md"),
        format!("public:: true\n- Welcome target\n  id:: {id}\n- same page (({id}))\n"),
    )
    .unwrap();
    fs::write(
            dir.join("pages/Other.md"),
            format!("public:: true\n- cross page (({id})) and (({id}))\n- missing ((22222222-2222-2222-2222-222222222222))\n"),
        )
        .unwrap();
    fs::write(
        dir.join("pages/Private.md"),
        format!("- private ref (({id}))\n"),
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 2);
    let out = std::path::Path::new(&outdir);
    let entry = fs::read_to_string(out.join("index.html")).unwrap();
    let welcome = fs::read_to_string(out.join("welcome-to-tine.html")).unwrap();
    let pages = fs::read_to_string(out.join("pages.html")).unwrap();

    assert!(entry.contains("<h1 class=\"page\">Welcome to Tine</h1>"));
    assert!(entry.contains("href=\"welcome-to-tine.html\">⌂ Home</a>"));
    assert!(welcome.contains("aria-label=\"2 block references\">2</summary>"));
    assert!(welcome.contains("href=\"welcome-to-tine.html#b0\""));
    assert!(welcome.contains("href=\"other.html#b0\""));
    assert!(
        !welcome.contains("Private"),
        "private referrer must not leak"
    );
    assert!(pages.contains("welcome-to-tine.html") && pages.contains("other.html"));
    assert!(pages.contains("href=\"welcome-to-tine.html\">⌂ Home</a>"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_gives_distinct_nonempty_files_on_slug_collision() {
    // DS#4: two titles that collapse to the same ASCII slug ("foo"), plus a
    // title with NO ASCII-alnum chars (empty slug pre-fix). Pre-fix, `Foo!`
    // and `Foo#` both write `foo.html` (the second silently overwrites the
    // first) and `日本語` writes a degenerate `.html`. Post-fix every page must
    // get a distinct, nonempty file, and every cross-page link must point at
    // the file its target was actually written to.
    let dir = std::env::temp_dir().join(format!("tine-publish-collide-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    // Each page links to the next so we can check the link map matches files.
    fs::write(
        dir.join("pages").join("Foo!.md"),
        "- alpha body linking [[Foo#]]\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Foo#.md"),
        "- bravo body linking [[日本語]]\n",
    )
    .unwrap();
    fs::write(dir.join("pages").join("日本語.md"), "- charlie body\n").unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&g).unwrap();
    assert_eq!(count, 3, "all three public pages exported");
    let out = std::path::Path::new(&outdir);

    // The embedded page index (`__tinePages`) is the single source of truth
    // name -> slug; parse it back out.
    let sidx = fs::read_to_string(out.join("search-index.js")).unwrap();
    let after = sidx.strip_prefix("window.__tinePages=").unwrap();
    let json = &after[..after.find(";\n").unwrap()];
    let pages: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
    assert_eq!(pages.len(), 3, "three pages in the index");

    // Every slug is nonempty + distinct, and names an existing, nonempty file.
    let mut seen = std::collections::HashSet::new();
    for p in &pages {
        let s = p["slug"].as_str().unwrap();
        assert!(!s.is_empty(), "slug must be nonempty: {p}");
        assert!(seen.insert(s.to_string()), "slugs must be distinct: {s}");
        let f = out.join(format!("{s}.html"));
        assert!(f.exists(), "file for slug {s} exists");
        assert!(
            fs::metadata(&f).unwrap().len() > 0,
            "file {s}.html nonempty"
        );
    }
    // No page landed in a degenerate empty-slug file.
    assert!(!out.join(".html").exists(), "no empty-slug .html file");

    // Cross-page links point at the file each target was actually written to.
    let name_slug = |name: &str| -> String {
        pages
            .iter()
            .find(|p| p["title"] == name)
            .unwrap_or_else(|| panic!("page {name} in index"))["slug"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let foo_bang = name_slug("Foo!");
    let foo_hash = name_slug("Foo#");
    let cjk = name_slug("日本語");
    let bang_html = fs::read_to_string(out.join(format!("{foo_bang}.html"))).unwrap();
    assert!(
        bang_html.contains(&format!("href=\"{foo_hash}.html\"")),
        "Foo! links to Foo#'s real file ({foo_hash}.html): {bang_html}"
    );
    let hash_html = fs::read_to_string(out.join(format!("{foo_hash}.html"))).unwrap();
    assert!(
        hash_html.contains(&format!("href=\"{cjk}.html\"")),
        "Foo# links to 日本語's real file ({cjk}.html): {hash_html}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn page_print_html_is_self_contained_with_inlined_image() {
    let dir = std::env::temp_dir().join(format!("tine-print-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    // A 1x1 PNG (real bytes) so the inliner produces a valid data: URI.
    let png: [u8; 67] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    fs::write(dir.join("assets").join("pic.png"), png).unwrap();
    let oversized = fs::File::create(dir.join("assets").join("oversized.png")).unwrap();
    oversized.set_len(PRINT_ASSET_MAX_BYTES + 1).unwrap();
    fs::write(
            dir.join("pages").join("Report.md"),
            "- # Report\n- Some **bold** text and a [[Welcome]] link.\n- ![shot](../assets/pic.png)\n- ![large](../assets/oversized.png)\n",
        )
        .unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let html = print::page_print_html(&g, "Report", PrintOpts::default())
        .unwrap()
        .expect("page exists");

    // Self-contained: inlined stylesheet + inlined image, no sidebar / app scripts /
    // external style.css.
    assert!(html.contains("<style>"), "stylesheet inlined");
    assert!(
        html.contains("data:image/png;base64,"),
        "image inlined as data URI: {}",
        &html[..0]
    );
    assert!(
        !html.contains("../assets/pic.png"),
        "no relative asset link left"
    );
    assert!(
        html.contains("class=\"print-asset-omitted\""),
        "an oversized image becomes an explicit inert marker"
    );
    assert!(
        !html.contains("oversized.png"),
        "the rejected source path is not retained in the print document"
    );
    assert!(
        !html.contains("<aside class=\"sidebar\">"),
        "no sidebar in print doc"
    );
    assert!(!html.contains("src=\"app.js\""), "no app.js in print doc");
    assert!(!html.contains("<script"), "print doc executes no scripts");
    assert!(
        !html.contains("cdn.jsdelivr.net"),
        "print doc has no CDN resources"
    );
    assert!(
        html.contains("script-src 'none'"),
        "print doc denies script execution even if markup regresses"
    );
    assert!(
        !html.contains("href=\"style.css\""),
        "no external stylesheet link"
    );
    // Content actually rendered.
    assert!(
        html.contains("<h1 class=\"page\">Report</h1>"),
        "page heading"
    );
    assert!(
        html.contains("<strong>bold</strong>"),
        "inline markup rendered"
    );
    assert!(html.contains("@media print"), "print CSS present");

    // Missing page → None (not an error).
    assert!(
        print::page_print_html(&g, "No Such Page", PrintOpts::default())
            .unwrap()
            .is_none()
    );

    // Collapsed handling: a collapsed parent's children are expanded by default,
    // and hidden when expand_collapsed is off.
    fs::write(
        dir.join("pages").join("Folded.md"),
        "- Parent\n  collapsed:: true\n\t- hidden child text\n",
    )
    .unwrap();
    let g2 = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let expanded = print::page_print_html(&g2, "Folded", PrintOpts::default())
        .unwrap()
        .unwrap();
    assert!(
        expanded.contains("hidden child text"),
        "default expands collapsed"
    );
    let folded = print::page_print_html(
        &g2,
        "Folded",
        PrintOpts {
            expand_collapsed: false,
            ..PrintOpts::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!(
        !folded.contains("hidden child text"),
        "folded hides collapsed children"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_begin_query_renders_authored_title_and_results() {
    let dir = std::env::temp_dir().join(format!("tine-publish-begin-query-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq/config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages/Tasks.md"),
        "- TODO BEGIN_QUERY_PUBLIC_RESULT\n",
    )
    .unwrap();
    fs::write(
            dir.join("pages/Dashboard.md"),
            "- #+BEGIN_QUERY\n  {:title \"Open work\"\n   :query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}\n  #+END_QUERY\n",
        )
        .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&graph).unwrap();
    let dashboard =
        fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

    assert!(
        dashboard.contains("class=\"query-head\">Open work"),
        "{dashboard}"
    );
    assert!(
        dashboard.contains("BEGIN_QUERY_PUBLIC_RESULT"),
        "{dashboard}"
    );
    assert!(
        !dashboard.contains("class=\"query-omitted\""),
        "{dashboard}"
    );
    assert!(!dashboard.contains("#+BEGIN_QUERY"), "{dashboard}");
    assert!(!dashboard.contains("#+END_QUERY"), "{dashboard}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_begin_query_reports_private_rows_without_leaking_them() {
    let dir = std::env::temp_dir().join(format!(
        "tine-publish-begin-query-private-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
            dir.join("pages/Dashboard.md"),
            "public:: true\n- #+BEGIN_QUERY\n  {:title \"Private-aware work\"\n   :query [:find (pull ?b [*]) :where (task ?b \"TODO\")]}\n  #+END_QUERY\n",
        )
        .unwrap();
    fs::write(
        dir.join("pages/Secret.md"),
        "- TODO PRIVATE_BEGIN_QUERY_RESULT_MUST_NOT_LEAK\n",
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&graph).unwrap();
    assert_eq!(count, 1);
    let dashboard =
        fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

    assert!(dashboard.contains("class=\"query-omitted\""), "{dashboard}");
    assert!(
        dashboard.contains("1 result on non-public pages omitted."),
        "{dashboard}"
    );
    assert!(
        !dashboard.contains("PRIVATE_BEGIN_QUERY_RESULT_MUST_NOT_LEAK"),
        "{dashboard}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_malformed_begin_query_is_inert_and_hides_its_payload() {
    let dir = std::env::temp_dir().join(format!(
        "tine-publish-begin-query-malformed-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
            dir.join("pages/Dashboard.md"),
            "public:: true\n- #+BEGIN_QUERY\n  {:title \"MALFORMED_BEGIN_QUERY_PAYLOAD\" :query (task TODO)}\n  #+END_QUERY\n",
        )
        .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&graph).unwrap();
    let dashboard =
        fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

    assert!(
        dashboard.contains("class=\"query-unsupported begin-query-unsupported\""),
        "{dashboard}"
    );
    assert!(
        !dashboard.contains("MALFORMED_BEGIN_QUERY_PAYLOAD"),
        "{dashboard}"
    );
    assert!(!dashboard.contains("#+BEGIN_QUERY"), "{dashboard}");
    assert!(!dashboard.contains("#+END_QUERY"), "{dashboard}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_renders_facets_queries_and_embeds() {
    let dir = std::env::temp_dir().join(format!("tine-publish-facets-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Target.md"),
        "- an embeddable target\n  id:: 22222222-2222-2222-2222-222222222222\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Main.md"),
        "- TODO [#A] do the thing\n  SCHEDULED: <2026-07-10 Fri>\n  notes after the schedule\n\
             - DONE finished it\n\
             - a note\n  status:: open\n\
             - {{query (task TODO)}}\n\
             - {{embed ((22222222-2222-2222-2222-222222222222))}}\n\
             - {{video https://www.youtube.com/watch?v=abc123}}\n",
    )
    .unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&g).unwrap();
    let main = fs::read_to_string(std::path::Path::new(&outdir).join("main.html")).unwrap();

    // task facets: checkbox + marker badge, priority, planning date
    assert!(
        main.contains("class=\"task-checkbox\""),
        "open task checkbox: {main}"
    );
    assert!(
        main.contains("class=\"task-marker m-todo\">TODO"),
        "TODO badge"
    );
    assert!(
        main.contains("class=\"priority p-a\">[#A]"),
        "priority badge"
    );
    assert!(main.contains("SCHEDULED:"), "scheduled line");
    assert!(
        main.contains("notes after the schedule"),
        "body after schedule"
    );
    let scheduled_trailers = main.matches("class=\"planning scheduled\"").count();
    assert!(scheduled_trailers > 0, "scheduled trailer rendered");
    assert_eq!(
        main.matches("2026-07-10 Fri").count(),
        scheduled_trailers,
        "each planning date renders only in trailer chrome, not again in the body"
    );
    assert!(
        main.contains("class=\"task-checkbox checked\"") && main.contains("class=\"b done\""),
        "DONE checked + muted"
    );
    // block property shown
    assert!(
        main.contains("status") && main.contains("open"),
        "block property rendered"
    );
    // query ran and rendered the TODO result (not empty, not the literal macro)
    assert!(main.contains("class=\"query\""), "query block rendered");
    assert!(
        !main.contains("{{query"),
        "query macro expanded, not literal"
    );
    // embed inlined the target block's text
    assert!(main.contains("class=\"embed"), "embed rendered");
    assert!(
        main.contains("class=\"embed block-embed single-root\"><ul class=\"embed-outline\">"),
        "block embed exposes one CSS-scoped root without generic outline connectors: {main}"
    );
    assert!(
        main.contains("an embeddable target"),
        "embed inlined target content: {main}"
    );
    // video → youtube iframe
    assert!(main.contains("youtube.com/embed/abc123"), "video iframe");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_memoizes_repeated_query_macros() {
    let dir = std::env::temp_dir().join(format!("tine-publish-query-memo-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Tasks.md"),
        "- TODO repeated query memo target\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Dashboard.md"),
        "- {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n\
             - {{query (task TODO)}}\n",
    )
    .unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&g).unwrap();

    let dash = fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
    assert_eq!(
        dash.matches("class=\"query\"").count(),
        5,
        "each macro occurrence still renders independently"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_query_keeps_and_hydrates_a_match_below_a_nonmatching_gap() {
    let dir = std::env::temp_dir().join(format!("tine-publish-query-gap-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    fs::write(
            dir.join("pages").join("Tasks.md"),
            "- TODO parity ancestor\n  - DONE parity gap\n    - TODO parity grandchild\n      - live child below retained grandchild\n",
        )
        .unwrap();
    fs::write(
        dir.join("pages").join("Dashboard.md"),
        "- {{query (task TODO)}}\n",
    )
    .unwrap();

    let graph = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&graph).unwrap();
    let dashboard =
        fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();

    assert_eq!(
        dashboard.matches("parity grandchild").count(),
        2,
        "{dashboard}"
    );
    assert_eq!(
        dashboard
            .matches("live child below retained grandchild")
            .count(),
        2,
        "{dashboard}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn publish_reuses_pass1_docs_for_repeated_page_embeds() {
    let dir = std::env::temp_dir().join(format!("tine-publish-embed-reuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true}\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Target.md"),
        "- shared embed target\n  id:: 33333333-3333-3333-3333-333333333333\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Org Page.org"),
        "public:: true\n* org fixture page\n",
    )
    .unwrap();
    fs::write(
        dir.join("pages").join("Dashboard.md"),
        "- {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n\
             - {{embed [[Target]]}}\n",
    )
    .unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, _) = publish_graph(&g).unwrap();

    let dash = fs::read_to_string(std::path::Path::new(&outdir).join("dashboard.html")).unwrap();
    assert_eq!(
        dash.matches("shared embed target").count(),
        4,
        "each embed occurrence still renders"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
#[ignore]
fn gen_sample_export() {
    let dir = std::env::temp_dir().join("tine-sample-export");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("journals")).unwrap();
    fs::create_dir_all(dir.join("pages")).unwrap();
    fs::create_dir_all(dir.join("logseq")).unwrap();
    fs::write(
        dir.join("logseq").join("config.edn"),
        "{:publishing/all-pages-public? true\n :favorites [\"Welcome\" \"Parameterized IP\"]}\n",
    )
    .unwrap();
    let write = |name: &str, body: &str| fs::write(dir.join("pages").join(name), body).unwrap();
    write(
            "Welcome.md",
            "- # Welcome to the published graph\n  id:: aaaaaaaa-0000-0000-0000-000000000001\n- This is a static export with a **sidebar** and fuzzy [[search]].\n- See [[Parameterized IP]] and the [[ILP Survey]].\n",
        );
    write(
            "Parameterized IP.md",
            "- # Parameterized IP\n- Fixed-parameter tractability of integer programming; **parameterized complexity** of ILPs.\n  id:: aaaaaaaa-0000-0000-0000-000000000002\n- Related to [[ILP Survey]].\n",
        );
    write(
            "ILP Survey.md",
            "- # ILP Survey\n- A survey of integer linear programming techniques and n-fold IP.\n- Back to [[Welcome]].\n",
        );
    write("Search.md", "- Notes on full-text search and ranking.\n");
    write("Project Ideas.md", "- # Project Ideas\n- A grab-bag of ideas, including continuous bribery and opinion diffusion.\n");
    // Exercises every construct the M3 render_html rewrite added to the export:
    // fenced code (highlight.js), tables (data-align), callouts, inline/display math,
    // lists (incl. checkboxes + def-list term), blockquote, org timestamps.
    write(
            "Rendering Showcase.md",
            "- # Rendering Showcase\n\
             - A paragraph with **bold**, *italic*, `inline code`, a [[Welcome]] link and a #demo tag.\n\
             - ```rust\n  fn solve(n: usize) -> usize {\n      (0..n).filter(|x| x & 1 == 0).count()\n  }\n  ```\n\
             - | Method | Time | Note |\n  | :--- | :---: | ---: |\n  | n-fold IP | fast | linear |\n  | brute force | slow | exp |\n\
             - > [!NOTE] Heads up\n  > Callouts now render straight from the AST.\n\
             - > [!WARNING]\n  > Macros are dropped in a static export (they can't run).\n\
             - Inline math $e^{i\\pi}+1=0$ and a display block:\n\
             - $$\\int_0^1 x^2\\,dx = \\tfrac{1}{3}$$\n\
             - Unordered:\n  * first\n  * second\n      * nested\n\
             - Tasks:\n  * [ ] open item\n  * [x] done item\n\
             - Coffee\n  : A hot drink brewed from beans.\n\
             - > A plain blockquote over\n  > two lines.\n\
             - let's try again *(something)* -> --> -- en --- em \\alpha \\Delta\n\
             - A meeting <2026-06-30 Tue 14:00> and a deadline.\n",
        );
    fs::write(
        dir.join("journals").join("2026_06_28.md"),
        "- Worked on the **published export**: sidebar + search.\n- Linked [[Parameterized IP]].\n",
    )
    .unwrap();

    let g = Store::from_legacy(Arc::new(Graph::open(&dir)));
    let (outdir, count) = publish_graph(&g).unwrap();
    println!("SAMPLE_EXPORT_DIR={outdir} pages={count}");

    // Also emit the single-page print/PDF document for the showcase page, so
    // the print CSS + self-contained render can be eyeballed / screenshotted.
    let print = print::page_print_html(&g, "Rendering Showcase", PrintOpts::default())
        .unwrap()
        .unwrap();
    let pfile = dir.join("print-sample.html");
    fs::write(&pfile, print).unwrap();
    println!("SAMPLE_PRINT_HTML={}", pfile.display());
}
