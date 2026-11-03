//! Requested Rust heap, including transient peaks; no RSS estimates.
//! Run in a fresh process on a generated graph, with `--features test-faults`.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
struct Counted;
fn add(bytes: usize) {
    let live = LIVE.fetch_add(bytes, SeqCst) + bytes;
    PEAK.fetch_max(live, SeqCst);
}
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            add(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            add(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        LIVE.fetch_sub(layout.size(), SeqCst);
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, new: usize) -> *mut u8 {
        // System::realloc may hold both buffers during the call. Conservatively
        // charge both at that boundary; remove the old charge after success.
        let new_ptr = System.realloc(ptr, old, new);
        if !new_ptr.is_null() {
            add(new);
            LIVE.fetch_sub(old.size(), SeqCst);
        }
        new_ptr
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;

#[derive(serde::Serialize)]
struct Stage {
    name: &'static str,
    live_bytes: usize,
    peak_bytes: usize,
    elapsed_ms: f64,
}
fn stage(name: &'static str, start: Instant) -> Stage {
    let live = LIVE.load(SeqCst);
    Stage {
        name,
        live_bytes: live,
        peak_bytes: PEAK.swap(live, SeqCst),
        elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
    }
}
fn walk(blocks: &mut [tine_core::DocBlock], f: &impl Fn(&mut tine_core::DocBlock)) {
    for block in blocks {
        f(block);
        walk(&mut block.children, f);
    }
}
fn count(blocks: &[tine_core::DocBlock]) -> usize {
    blocks.iter().map(|b| 1 + count(&b.children)).sum()
}

fn host_probe(root: &std::path::Path, store: tine_store::Store, view: tine_store::WholeGraph) {
    let corpus = view.corpus(None);
    let files = corpus.pages.len();
    let blocks: usize = corpus.pages.iter().map(|p| count(&p.document.roots)).sum();
    let mut pages: Vec<_> = corpus
        .pages
        .iter()
        .map(|p| (p.name.clone(), p.id.clone()))
        .collect();
    pages.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    pages.truncate(5);
    pages.shrink_to_fit();
    drop(corpus);
    drop(view);
    let app_data = tempfile::tempdir_in(root.parent().unwrap()).unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let mut stages = Vec::with_capacity(3);
    let before = LIVE.load(SeqCst);
    PEAK.store(before, SeqCst);
    let start = Instant::now();
    let store = std::sync::Arc::new(store);
    let host = tine_store::PageHost::start(
        &store,
        app_data.path(),
        "heap-probe",
        |_, _| {},
        move |mail| {
            let _ = send.send(serde_json::to_vec(&mail).unwrap());
        },
    )
    .unwrap();
    stages.push(stage("host_started", start));
    let session = serde_json::to_value(host.window_reloaded()).unwrap();
    let mut id = session["nextId"].as_u64().unwrap();
    let session = session["session"].as_u64().unwrap();
    let mut payloads = Vec::with_capacity(5);
    let start = Instant::now();
    for (name, path) in &pages {
        host.open(session, id, &tine_store::PageId::from(path.as_str()), name)
            .unwrap();
        loop {
            let bytes = receive.recv_timeout(Duration::from_secs(30)).unwrap();
            let mail: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if mail["answer"]["id"].as_u64() == Some(id) {
                payloads.push(bytes);
                break;
            }
        }
        id += 1;
    }
    stages.push(stage("host_five_pages_with_mail_json", start));
    let json_length: usize = payloads.iter().map(Vec::len).sum();
    let json_capacity: usize = payloads.iter().map(Vec::capacity).sum();
    drop(payloads);
    std::thread::sleep(Duration::from_millis(100));
    while receive.try_recv().is_ok() {}
    stages.push(stage("host_five_pages_settled_without_mail_json", start));
    let host_extra_bytes = LIVE.load(SeqCst) - before;
    drop(host);
    store.close();
    println!(
        "{}",
        serde_json::json!({ "files": files, "blocks": blocks,
            "store_baseline_bytes": before, "host_extra_bytes": host_extra_bytes,
            "five_open_mail_json_length": json_length,
            "five_open_mail_json_capacity": json_capacity, "stages": stages })
    );
}

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("generated graph root"));
    let pause = root.join(".tine-test-pause-load");
    std::fs::write(&pause, b"").unwrap();
    let mut stages = Vec::with_capacity(8);
    let start = Instant::now();
    let (store, meta, config) = tine_store::Store::open(&root, Default::default()).unwrap();
    drop((meta, config));
    stages.push(stage("launch_before_load", start));
    std::fs::remove_file(pause).unwrap();
    let start = Instant::now();
    let view = store.whole_graph_reconciled().unwrap();
    // Capture the default observer state before stopping it for attribution.
    stages.push(stage("whole_graph_ready", start));
    store.stop_watcher_for_tests();
    // The load worker publishes Ready just before returning. Allow its local
    // temporaries to leave before the settled reading (no timings use this).
    std::thread::sleep(Duration::from_millis(100));
    stages.push(stage("whole_graph_settled_observer_stopped", start));
    if std::env::args().any(|arg| arg == "--host") {
        host_probe(&root, store, view);
        return;
    }
    if std::env::args().any(|arg| arg == "--indexes") {
        let indexes = view.rebuilt_index_heap_for_tests(|| LIVE.load(SeqCst));
        println!("{}", serde_json::json!({ "rebuilt_index_bytes": indexes }));
        store.close();
        return;
    }
    let corpus = view.corpus(None);
    let files = corpus.pages.len();
    let blocks: usize = corpus.pages.iter().map(|p| count(&p.document.roots)).sum();
    let mut names: Vec<_> = corpus
        .pages
        .iter()
        .map(|p| (p.name.clone(), p.kind))
        .collect();
    names.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    names.truncate(5);
    names.shrink_to_fit();
    drop(corpus);
    let start = Instant::now();
    let opened: Vec<_> = names
        .iter()
        .map(|(name, kind)| store.page_named(name, *kind).unwrap().unwrap())
        .collect();
    stages.push(stage("five_pages_with_dtos", start));
    let dto_before = LIVE.load(SeqCst);
    let ipc_json = serde_json::to_vec(&opened.iter().map(|p| &p.doc).collect::<Vec<_>>()).unwrap();
    let ipc_bytes = ipc_json.len();
    let ipc_heap_bytes = LIVE.load(SeqCst) - dto_before;
    drop(ipc_json);
    let dto_before = LIVE.load(SeqCst);
    drop(opened);
    let dto_bytes = dto_before - LIVE.load(SeqCst);
    drop(names);
    let start = Instant::now();
    // A common first-screen query exercises the lazy query index and memo.
    let result = view
        .query(
            "(and (task TODO DOING) #tag1)",
            tine_store::QueryDialect::Simple,
        )
        .unwrap();
    stages.push(stage("task_query_with_result", start));
    drop(result);
    // Only the attribution pass retains an owned corpus table. Its entries
    // share documents; the measured opening/query stages retain no such table.
    let mut corpus = view.corpus(None);
    let store_before = LIVE.load(SeqCst);
    drop(view);
    store.close();
    drop(store);
    std::thread::sleep(Duration::from_millis(100));
    let documents_only = LIVE.load(SeqCst);
    let stores_indexes_caches = store_before - documents_only;
    let start = Instant::now();
    for page in &mut corpus.pages {
        let doc =
            std::sync::Arc::get_mut(&mut page.document).expect("corpus is the sole document owner");
        walk(&mut doc.roots, &|b| b.clear_projection_for_tests());
    }
    let projections = documents_only - LIVE.load(SeqCst);
    let ids_before = LIVE.load(SeqCst);
    for page in &mut corpus.pages {
        walk(
            &mut std::sync::Arc::get_mut(&mut page.document).unwrap().roots,
            &|b| {
                drop(std::mem::take(&mut b.uuid));
            },
        );
    }
    let runtime_ids = ids_before - LIVE.load(SeqCst);
    let raw_before = LIVE.load(SeqCst);
    let mut raw_text_length = 0;
    for page in &mut corpus.pages {
        let doc = std::sync::Arc::get_mut(&mut page.document).unwrap();
        raw_text_length += doc.pre_block.as_ref().map_or(0, String::len);
        drop(doc.pre_block.take());
        fn clear_raw(blocks: &mut [tine_core::DocBlock], len: &mut usize) {
            for block in blocks {
                *len += block.raw().len();
                block.set_raw("");
                clear_raw(&mut block.children, len);
            }
        }
        clear_raw(&mut doc.roots, &mut raw_text_length);
    }
    let raw_text = raw_before - LIVE.load(SeqCst);
    let trees_before = LIVE.load(SeqCst);
    for page in &mut corpus.pages {
        std::sync::Arc::get_mut(&mut page.document)
            .unwrap()
            .roots
            .clear();
        std::sync::Arc::get_mut(&mut page.document)
            .unwrap()
            .roots
            .shrink_to_fit();
    }
    let document_tree_buffers = trees_before - LIVE.load(SeqCst);
    drop(corpus);
    stages.push(stage("after_drop", start));
    println!(
        "{}",
        serde_json::json!({
            "files": files, "blocks": blocks, "stages": stages,
            "attribution": { "projection_heap": projections, "runtime_ids": runtime_ids,
                "raw_text": raw_text, "raw_text_length": raw_text_length,
                "document_tree_buffers": document_tree_buffers,
                "store_indexes_caches_and_metadata": stores_indexes_caches,
                "five_page_dtos": dto_bytes, "five_page_ipc_json_length": ipc_bytes,
                "five_page_ipc_json_heap_capacity": ipc_heap_bytes },
            "doc_block_inline_bytes": std::mem::size_of::<tine_core::DocBlock>(),
            "projection_inline_bytes": std::mem::size_of::<tine_core::doc::BlockProjection>(),
        })
    );
}
