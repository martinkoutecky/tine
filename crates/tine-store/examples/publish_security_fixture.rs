use std::env;
use std::path::PathBuf;
use tine_graph_features::publish::publish_html;
use tine_store::Store;

fn main() {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: publish_security_fixture <graph-root>");
    let (store, _, _) = Store::open(&root, Default::default()).expect("open graph");
    let (output, count) = publish_html(&store).expect("publish security fixture");
    println!("{count}\n{output}");
}
