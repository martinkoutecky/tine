//! I-22 (og C3 L04): export macro expansion was depth-bounded (4) but not
//! fan-out bounded. A synced or imported block holding N copies of
//! `{{embed ((its-own-id))}}` rendered N^4 subtrees into one String during
//! print/publish: N=100 is 10^8 block renders, a hang or OOM abort. One
//! per-tree expansion budget now cuts the tree with a visible marker; the
//! benign extremes (a chain at the depth cap, a wide shallow page) render in
//! full.
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use tine_store::Store;

const LIMIT: &str = "macro expansion limit";

fn graph(tag: &str, pages: &[(&str, String)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("tine-i22-fanout-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("pages")).unwrap();
    fs::create_dir_all(root.join("journals")).unwrap();
    for (name, text) in pages {
        fs::write(root.join("pages").join(name), text).unwrap();
    }
    root
}

fn print(root: &PathBuf, page: &str) -> String {
    let store = Store::open(root, Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    let html = tine_graph_features::print::page_print_html(&store, page, Default::default())
        .unwrap()
        .unwrap();
    store.close();
    html
}

const ID: &str = "6a1c0c3e-0000-4000-8000-000000000001";

#[test]
fn self_embedding_fan_out_is_cut_with_a_visible_marker() {
    let embeds = format!("{{{{embed (({ID}))}}}} ").repeat(24);
    let root = graph(
        "hostile",
        &[("Hostile.md", format!("- loop {embeds}\n  id:: {ID}\n"))],
    );
    let started = Instant::now();
    let html = print(&root, "Hostile");
    assert!(html.contains(LIMIT), "I-22: the cut must be visible");
    assert!(
        started.elapsed().as_secs() < 60,
        "I-22: 24^4 embeds must not all render ({:?})",
        started.elapsed()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_embed_chain_at_the_depth_cap_renders_in_full() {
    let ids: Vec<String> = (0..4)
        .map(|n| format!("6a1c0c3e-0000-4000-8000-00000000010{n}"))
        .collect();
    let mut text = String::from("- start {{embed ((");
    text.push_str(&ids[0]);
    text.push_str("))}}\n");
    for n in 0..4 {
        let next = if n < 3 {
            format!("{{{{embed (({}))}}}}", ids[n + 1])
        } else {
            "CHAIN-LEAF".to_string()
        };
        text.push_str(&format!("- link{n} {next}\n  id:: {}\n", ids[n]));
    }
    let root = graph("chain", &[("Chain.md", text)]);
    let html = print(&root, "Chain");
    assert!(html.contains("CHAIN-LEAF"));
    assert!(!html.contains(LIMIT));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_wide_shallow_page_renders_every_embed() {
    let mut text = String::new();
    for _ in 0..300 {
        text.push_str(&format!("- {{{{embed (({ID}))}}}}\n"));
    }
    text.push_str(&format!("- WIDE-LEAF\n  id:: {ID}\n"));
    let root = graph("wide", &[("Wide.md", text)]);
    let html = print(&root, "Wide");
    assert_eq!(html.matches("WIDE-LEAF").count(), 301);
    assert!(!html.contains(LIMIT));
    let _ = fs::remove_dir_all(root);
}
