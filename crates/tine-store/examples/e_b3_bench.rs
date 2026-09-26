//! One-shot release probe used for the E-B3 before/after graph measurements.
use std::path::Path;
use std::time::Instant;
use tine_store::{PageId, SaveBase, SaveOutcome, Store};

fn main() {
    let mut args = std::env::args().skip(1);
    let operation = args.next().expect("operation");
    let root = args.next().expect("fresh graph root");
    let start = Instant::now();
    let store = Store::open(Path::new(&root), Default::default()).unwrap().0;
    store.whole_graph().unwrap();
    if operation == "open" {
        println!("{:.6}", start.elapsed().as_secs_f64() * 1000.0);
        store.close();
        return;
    }
    let file = match operation.as_str() {
        "small" | "print" => "pages/Exlgb.md",
        "large" => "pages/VHI 5569.md",
        _ => panic!("unknown operation"),
    };
    let id = PageId::from(file);
    let read = store.page(&id).unwrap();
    let start = Instant::now();
    match operation.as_str() {
        "print" => {
            assert!(tine_graph_features::print::page_print_html(
                &store,
                &read.doc.name,
                Default::default()
            )
            .unwrap()
            .is_some());
        }
        _ => {
            let mut doc = read.doc;
            doc.blocks[0].raw.push_str(" e-b3-bench");
            assert!(matches!(
                store.save(&id, SaveBase::Existing(read.rev), &doc),
                SaveOutcome::Saved(_)
            ));
        }
    }
    println!("{:.6}", start.elapsed().as_secs_f64() * 1000.0);
    store.close();
}
