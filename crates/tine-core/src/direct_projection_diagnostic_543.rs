//! Temporary CI diagnostic. Synthetic data only; no product behavior changes.
use super::*;

#[test]
fn windows_indexing_progress_probe_543() {
    if std::env::var_os("TINE_DIAGNOSE_543").is_none() {
        return;
    }
    let _serial = serialize_projection_tests();
    let sizes = std::env::var("TINE_DIAGNOSE_543_SIZES").unwrap_or("1000,10000".into());
    for count in sizes.split(',').map(|n| n.parse::<usize>().unwrap()) {
        let root = scratch("543-图谱");
        std::fs::create_dir_all(root.join("pages")).unwrap();
        for page in 0..count {
            let mut content = format!("title:: Topic {page} 你好\n\n");
            for block in 0..60 {
                content.push_str(&format!(
                    "- outline sentinel543 你好世界 page {page} block {block} [[Topic {} 你好]] #tag{}\n",
                    (page + 1) % count,
                    block % 10
                ));
            }
            std::fs::write(root.join(format!("pages/主题-{page:05}.md")), content).unwrap();
        }
        let db_root = scratch("543-db");
        let database = db_root.join("projection.sqlite");
        for phase in ["cold-warm-first", "reopen-query-first", "cold-startup-race"] {
            let path = if phase == "cold-startup-race" {
                db_root.join("race.sqlite")
            } else {
                database.clone()
            };
            reset_lowerings(&root);
            let graph = Arc::new(Graph::open(&root));
            graph.attach_direct_projection(path.clone()).unwrap();
            let projection = graph.direct_projection_test().unwrap();
            let start = Instant::now();
            println!(
                "DIAG543 BEGIN os={} pages={count} blocks={} phase={phase}",
                std::env::consts::OS,
                count * 60
            );
            let warm_done = Arc::new(AtomicBool::new(false));
            let stop = Arc::new(AtomicBool::new(false));
            let successes = Arc::new(AtomicUsize::new(0));
            let last_query = Arc::new(Mutex::new(String::from("not started")));
            let warm = {
                let graph = Arc::clone(&graph);
                let done = Arc::clone(&warm_done);
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    if phase != "cold-warm-first" {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    let result = graph.warm_cache_cancellable(|| stop.load(Ordering::Acquire));
                    println!(
                        "DIAG543 WARM phase={phase} elapsed={:?} completed={result}",
                        start.elapsed()
                    );
                    done.store(true, Ordering::Release);
                })
            };
            let query = {
                let graph = Arc::clone(&graph);
                let stop = Arc::clone(&stop);
                let done = Arc::clone(&warm_done);
                let successes = Arc::clone(&successes);
                let last_query = Arc::clone(&last_query);
                std::thread::spawn(move || {
                    let tokens = ["sentinel543", "你", "Topic 42"];
                    let mut attempts = 0;
                    while !stop.load(Ordering::Acquire) {
                        if phase == "cold-warm-first" && !done.load(Ordering::Acquire) {
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        let token = tokens[successes.load(Ordering::Acquire).min(2)];
                        let began = Instant::now();
                        *last_query.lock().unwrap() =
                            format!("in-flight {token} attempt={attempts}");
                        let outcome = match graph.search(token, 50) {
                            Ok(groups) => {
                                assert!(
                                    !groups.is_empty(),
                                    "synthetic matching search returned no groups: {token}"
                                );
                                successes.fetch_add(1, Ordering::AcqRel);
                                format!("OK groups={}", groups.len())
                            }
                            Err(error) => format!("{error:?}"),
                        };
                        let record = format!("token={token} took={:?} {outcome}", began.elapsed());
                        *last_query.lock().unwrap() = record.clone();
                        if attempts < 5
                            || outcome.starts_with("OK")
                            || began.elapsed() > Duration::from_secs(2)
                        {
                            println!(
                                "DIAG543 QUERY phase={phase} elapsed={:?} {record}",
                                start.elapsed()
                            );
                        }
                        attempts += 1;
                        if successes.load(Ordering::Acquire) >= 3 {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(800));
                    }
                })
            };
            let budget = Duration::from_secs(
                std::env::var("TINE_DIAGNOSE_543_BUDGET")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(if count <= 1000 { 180 } else { 600 }),
            );
            while start.elapsed() < budget {
                let bytes = |suffix: &str| {
                    std::fs::metadata(format!("{}{suffix}", path.display()))
                        .map(|m| m.len())
                        .unwrap_or(0)
                };
                println!("DIAG543 PROGRESS phase={phase} t={:?} generation={} lowered={} parsed={} warm_done={} answers={} db_bytes={} wal_bytes={} query={:?} {}",
                    start.elapsed(), graph.cache_generation(), lowerings(), graph.warm_stream_parses_test(),
                    warm_done.load(Ordering::Acquire), successes.load(Ordering::Acquire), bytes(""), bytes("-wal"),
                    last_query.lock().unwrap().clone(), projection.debug_state_test());
                if warm_done.load(Ordering::Acquire) && successes.load(Ordering::Acquire) >= 3 {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            stop.store(true, Ordering::Release);
            assert!(
                warm_done.load(Ordering::Acquire) && successes.load(Ordering::Acquire) >= 3,
                "DIAG543 TIMEOUT pages={count} phase={phase} last_query={:?} {}",
                last_query.lock().unwrap(),
                projection.debug_state_test()
            );
            warm.join().unwrap();
            query.join().unwrap();
            println!(
                "DIAG543 RESULT pages={count} phase={phase} elapsed={:?} lowered={}",
                start.elapsed(),
                lowerings()
            );
            drop(projection);
            release_projection(&*graph);
        }
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(db_root).unwrap();
    }
}
