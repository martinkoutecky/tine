//! Build the static "live demo" site from Tine's onboarding demo graph, using
//! Tine's OWN HTML export — so the public demo dogfoods the publish feature.
//!
//! Scaffolds the demo graph in a temp dir, publishes ALL its pages, and writes a
//! self-contained site into the given output dir (e.g. `website/demo`). The demo
//! pages carry no `public::` markers, so all-pages-public is forced **in memory
//! for this export only** — the shipped onboarding config stays
//! `all-pages-public=false`, so a real user's new graph never silently publishes.
//!
//! `publish_graph` emits asset embeds as `../assets/<file>` (it assumes the site
//! is served from `<graph>/publish` next to `<graph>/assets`). To keep the hosted
//! demo self-contained under one directory, the emitted HTML is rewritten to
//! `assets/<file>` and the graph's `assets/` is copied in alongside the pages.
//!
//! Usage: cargo run -q -p tine-core --example build-demo-site -- website/demo
//! (Re-run after changing the demo templates in src/templates/.)

use std::fs;
use std::path::{Path, PathBuf};

use tine_core::onboarding::create_demo_graph;
use tine_core::publish::publish_graph;
use tine_core::Graph;

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: build-demo-site <out_dir>   (e.g. website/demo)"),
    );

    let tmp = std::env::temp_dir().join("tine-demo-site-build");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).expect("create temp graph dir");

    create_demo_graph(&tmp).expect("scaffold demo graph");

    // A dense "Feature showcase" page — exercises every page-level feature Tine
    // renders, so the public demo doubles as an honest render reference. It's
    // injected into the DEMO SITE ONLY (not the onboarding graph in
    // `create_demo_graph`), so a brand-new user's graph stays lean.
    const SHOWCASE: &str = include_str!("../src/templates/showcase.md");
    fs::write(tmp.join("pages").join("Feature showcase.md"), SHOWCASE)
        .expect("write showcase page");

    let mut graph = Graph::open(&tmp);
    graph.config.all_pages_public = true;
    let (publish_dir, count) = publish_graph(&graph).expect("publish demo graph");
    let publish_dir = PathBuf::from(publish_dir);

    // Fresh output dir = the published pages, with self-contained asset paths.
    let _ = fs::remove_dir_all(&out);
    copy_dir(&publish_dir, &out).expect("copy publish output");
    for entry in fs::read_dir(&out).expect("read out dir") {
        let p = entry.expect("dir entry").path();
        if p.extension().map(|e| e == "html").unwrap_or(false) {
            let html = fs::read_to_string(&p).expect("read html");
            fs::write(&p, html.replace("\"../assets/", "\"assets/")).expect("write html");
        }
    }

    // Copy the demo graph's assets in next to the pages so `assets/<file>` resolves.
    let assets = tmp.join("assets");
    if assets.is_dir() {
        copy_dir(&assets, &out.join("assets")).expect("copy assets");
    }

    let _ = fs::remove_dir_all(&tmp);
    println!("published {count} pages -> {}", out.display());
}
