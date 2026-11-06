//! Whole inventory read and guarded save -> publication -> held inventory.
//! Run only on the generated disposable realistic-10k graph.
use std::sync::Arc;
use std::time::Instant;
use tine_store as store_crate;
#[path = "../src/test_host_writer.rs"]
mod test_host_writer;

fn main() {
    let root = std::env::args().nth(1).expect("generated graph root");
    let year = std::env::args().nth(2).unwrap_or_else(|| "2040".to_owned());
    let store = Arc::new(
        tine_store::Store::open(std::path::Path::new(&root), Default::default())
            .unwrap()
            .0,
    );
    while !store.is_graph_ready().unwrap() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let start = Instant::now();
    let view = store.whole_graph().unwrap();
    let inventory = view.inventory(tine_store::InventoryScope::All);
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    let id = tine_store::PageId::from("pages/CM bench.md");
    let writer = test_host_writer::HostWriter::start(&store);
    let mut samples = Vec::new();
    for n in 0..21 {
        let sub = store.subscribe();
        let read = store.page(&id).unwrap();
        let mut doc = read.doc;
        doc.blocks[0].raw = format!("[[{year}-10-{}]]", if n % 2 == 0 { "26" } else { "27" });
        let start = Instant::now();
        writer
            .save(
                tine_store::EditKind::ReplacePage,
                &id,
                tine_store::SaveBase::Existing(read.rev),
                &doc,
            )
            .unwrap();
        let mut held = inventory.0.clone();
        while let Some(change) = sub.try_recv().unwrap() {
            if serde_json::to_value(&change).unwrap()["inventoryChanged"] == true {
                if let Some((keys, rows)) = change.inventory_delta() {
                    held.retain(|entry| {
                        matches!(entry.target, tine_store::Resolved::Existing { .. })
                            || !keys.contains(&tine_core::refs::page_key(&entry.name))
                    });
                    held.extend(rows.0.iter().cloned());
                } else {
                    held = store
                        .whole_graph()
                        .unwrap()
                        .inventory(tine_store::InventoryScope::All)
                        .0
                        .clone();
                }
            }
        }
        std::hint::black_box(held);
        if n > 0 {
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({"build_ms": build_ms, "update_p50_ms": samples[10], "update_p95_ms": samples[18], "entries": inventory.0.len()})
    );
    store.close();
}
