//! Native release measurement through the same Store doors used by commands.
//! Run only on graph copies containing QF2 Small/Sixty/Big fixtures.
//! Usage: graph_scale_bench <graph> <checkpoint|cold> <launch|actions|memory>.
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};
use tine_core::model::PageKind;
use tine_core::query::{ir::ExecutionContext, parse_query_text, QueryDialect as Dialect};
use tine_store::{
    Cancel, IrAnswer, IrRequest, OpenOptions, PageId, QueryDialect, QueryResult, SearchRequest,
    Store,
};

fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with("VmRSS:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}
fn timed<T: Serialize>(f: impl FnOnce() -> T) -> Value {
    let started = Instant::now();
    let result = f();
    let execution = started.elapsed().as_secs_f64() * 1e3;
    let encoded = serde_json::to_vec(&result).unwrap();
    json!({"ms": started.elapsed().as_secs_f64()*1e3,
        "executionMs": execution, "jsonBytes": encoded.len()})
}
fn search_sections(store: &Store, text: &str, pages: usize, blocks: usize) -> Value {
    timed(|| {
        store
            .whole_graph()
            .unwrap()
            .search(
                &SearchRequest {
                    text: text.into(),
                    within: None,
                    page_limit: pages,
                    block_limit: blocks,
                    explain: false,
                    page_match_scope: None,
                    page_view: None,
                    block_view: None,
                },
                &Cancel(Arc::new(AtomicBool::new(false))),
            )
            .unwrap()
    })
}
fn search(store: &Store, text: &str) -> Value {
    search_sections(store, text, 100, 100)
}
fn query(store: &Store, tql: bool) -> Value {
    if !tql {
        return timed(|| {
            match store
                .whole_graph()
                .unwrap()
                .query("(page [[QF2 Big]])", QueryDialect::Simple)
                .unwrap()
            {
                QueryResult::Simple(groups) => groups,
                _ => panic!("wrong dialect"),
            }
        });
    }
    let (query, view) = parse_query_text(
        "@block and page.name = 'QF2 Big'",
        Dialect::Tql,
        tine_core::date::JournalDate::today(),
    );
    assert!(query.diagnostics.is_empty(), "{:#?}", query.diagnostics);
    let context = ExecutionContext::none();
    timed(|| {
        match store
            .whole_graph()
            .unwrap()
            .query_ir(IrRequest::Run {
                query: &query,
                view: &view,
                context: &context,
            })
            .unwrap()
        {
            IrAnswer::Result(answer) => answer,
            _ => panic!("wrong answer variant"),
        }
    })
}
fn ready(store: &Store) {
    while !store.is_graph_ready().unwrap() {
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let [_, root, checkpoint, mode] = args.as_slice() else {
        panic!("graph checkpoint|cold launch|actions|memory");
    };
    let started = Instant::now();
    let (store, _, _) = Store::open(
        Path::new(root),
        OpenOptions {
            launch_checkpoint: (checkpoint != "cold").then(|| checkpoint.into()),
            ..Default::default()
        },
    )
    .unwrap();
    let open = started.elapsed().as_secs_f64() * 1e3;
    let first_name = if mode == "sixty" {
        "QF2 Sixty"
    } else {
        "QF2 Small"
    };
    let first = timed(|| {
        tine_graph_features::pages::get_page(&store, first_name, PageKind::Page)
            .unwrap()
            .unwrap()
            .doc
    });
    ready(&store);
    let ready_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut out = json!({"openMs":open, "firstPage":first, "readyMs":ready_ms,
        "rssKiB":rss(), "diagnostics": store.diagnostics()});
    if mode == "stages" {
        for text in ["the", "qf2rareprobe"] {
            out[format!("pages:{text}")] = json!([
                search_sections(&store, text, 100, 0),
                search_sections(&store, text, 100, 0)
            ]);
            out[format!("blocks:{text}")] = json!([
                search_sections(&store, text, 0, 100),
                search_sections(&store, text, 0, 100)
            ]);
        }
    }
    if mode == "launch" && checkpoint != "cold" {
        out["checkpointWrite"] = json!(format!("{:?}", store.write_checkpoint_now()));
    }
    if mode == "actions" {
        // First open after readiness, then identical repeat, through get_page's door.
        for name in ["QF2 Small", "QF2 Sixty"] {
            out[name] = json!([
                timed(
                    || tine_graph_features::pages::get_page(&store, name, PageKind::Page)
                        .unwrap()
                        .unwrap()
                        .doc
                ),
                timed(
                    || tine_graph_features::pages::get_page(&store, name, PageKind::Page)
                        .unwrap()
                        .unwrap()
                        .doc
                ),
            ]);
            out[format!("{name}ById")] = timed(|| {
                store
                    .page(&PageId::from(format!("pages/{name}.md")))
                    .unwrap()
                    .doc
            });
        }
        for text in ["the", "qf2rareprobe"] {
            out[format!("search:{text}")] = json!([search(&store, text), search(&store, text)]);
        }
        out["simple"] = json!([query(&store, false), query(&store, false)]);
        out["tql"] = json!([query(&store, true), query(&store, true)]);
        let scan = Instant::now();
        store.scan_refresh().unwrap();
        out["focusMs"] = json!(scan.elapsed().as_secs_f64() * 1e3);
        let store = Arc::new(store);
        let copy = Arc::clone(&store);
        let scan = std::thread::spawn(move || {
            copy.scan_refresh().unwrap();
        });
        std::thread::sleep(Duration::from_millis(2));
        out["pageDuringFocus"] = timed(|| {
            tine_graph_features::pages::get_page(&store, "QF2 Sixty", PageKind::Page)
                .unwrap()
                .unwrap()
                .doc
        });
        scan.join().unwrap();
        out["afterDiagnostics"] = store.diagnostics();
        store.close();
    } else if mode == "memory" {
        let mut phases = Vec::new();
        for _ in 0..2 {
            let begin = Instant::now();
            for i in 0..600 {
                let name = ["QF2 Small", "QF2 Sixty", "QF2 Big"][i % 3];
                let _ = tine_graph_features::pages::get_page(&store, name, PageKind::Page)
                    .unwrap()
                    .unwrap();
                if i % 6 == 0 {
                    search(&store, ["the", "qf2rareprobe"][i / 6 % 2]);
                }
                if i % 6 == 1 {
                    query(&store, false);
                }
                if i % 6 == 2 {
                    query(&store, true);
                }
                if i % 6 == 3 {
                    let _ = store.whole_graph().unwrap().backlinks(name).unwrap();
                }
            }
            phases.push(json!({"rssKiB":rss(),"seconds":begin.elapsed().as_secs_f64()}));
        }
        out["phases"] = json!(phases);
        store.close();
    } else {
        store.close();
    }
    println!("{out}");
}
