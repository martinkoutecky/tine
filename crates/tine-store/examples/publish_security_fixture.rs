use std::env;
use std::path::PathBuf;
use tine_graph_features::publish_query::publish_static;
use tine_store::Store;

fn main() {
    let mut args = env::args_os().skip(1).map(PathBuf::from);
    let (Some(root), Some(parent)) = (args.next(), args.next()) else {
        panic!("usage: publish_security_fixture <graph-root> <existing-output-parent>");
    };
    let (store, _, _) = Store::open(&root, Default::default()).expect("open graph");
    // The site `tine publish` writes (the CLI's export), outside the graph.
    let receipt = publish_static(
        &store,
        &parent,
        "site",
        store.config().all_pages_public,
        true,
    )
    .expect("publish security fixture");
    println!("{}\n{}", receipt.pages, receipt.path);
}
