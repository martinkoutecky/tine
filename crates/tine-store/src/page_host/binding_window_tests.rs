//! Test-only stdio peer for the real TS window. No production feature or IPC hook.
//! Run by nativeConformance.test.ts using this lib-test executable, never by the app.
use super::*;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const TARGET: &str = "12345678-1234-4234-8234-123456789abc";

fn image(store: &Store) -> Value {
    let view = store.whole_graph().unwrap();
    let mut pages: Vec<Value> = view
        .list
        .iter()
        .map(|entry| {
            let rel = entry.path.strip_prefix(&store.graph.root).unwrap();
            let dto = store
                .graph
                .page_dto_for_bytes(&entry.path, &fs::read(&entry.path).unwrap())
                .unwrap()
                .unwrap();
            json!({"path": rel.to_string_lossy(), "dto": dto})
        })
        .collect();
    pages.sort_by_key(|p| p["path"].as_str().unwrap().to_owned());
    // Full query DTO, including multiline raws and nested children, not a fingerprint.
    let (query, settings) = tine_core::query::parse_query_input(
        "[[Target]]",
        tine_core::query::QueryInput::Og,
        tine_core::date::JournalDate::today(),
        tine_core::query::registry::Registry::none(),
    );
    let query = match view
        .query_ir(
            crate::IrRequest::Run {
                query: &query,
                view: &settings,
                context: &tine_core::query::ir::ExecutionContext::default(),
            },
            &crate::Cancel::default(),
        )
        .unwrap()
        .expect("not cancelled")
    {
        crate::IrAnswer::Result(result) => json!(result),
        _ => panic!("query result"),
    };
    json!({"rev": String::from(view.rev()), "counts": view.block_ref_counts(),
        "query": query, "pages": pages})
}

fn fresh(live: &Live) -> Value {
    // Physically separate files and parsed graph: no live cache crosses this boundary.
    let copy = tempfile::tempdir().unwrap();
    for area in ["pages", "journals"] {
        fs::create_dir_all(copy.path().join(area)).unwrap();
        for entry in fs::read_dir(live.root.join(area)).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), copy.path().join(area).join(entry.file_name())).unwrap();
        }
    }
    let cold = Store::open(copy.path(), Default::default()).unwrap().0;
    cold.whole_graph_reconciled().unwrap();
    let value = image(&cold);
    cold.close();
    value
}

fn paused_submit(live: &Live, arg: &Value) -> Value {
    let key = arg["key"].as_str().unwrap();
    let dto: PageDto = serde_json::from_value(arg["dto"].clone()).unwrap();
    let version = arg["version"].as_u64().unwrap();
    let session = arg["session"].as_u64().unwrap();
    let id = arg["id"].as_u64().unwrap();
    let kinds: Vec<EditKind> = serde_json::from_value(arg["kinds"].clone()).unwrap();
    let resolve: Option<DiskToken> = serde_json::from_value(arg["resolve"].clone()).unwrap();
    live.pause(true);
    // Independently capture the exact comparison source/serialized request before admit.
    let (spelling, source, _, _) = live.host.source(key, version, resolve.as_ref());
    let serialized = live
        .host
        .serialize(&spelling, &dto, source.as_ref(), &kinds);
    let reply = live
        .host
        .submit(session, id, key, &dto, version, resolve.as_ref(), &kinds)
        .err();
    let expected = serialized.ok().and_then(|(bytes, _)| bytes);
    let _writer = live.store.writer.lock().unwrap();
    let mut queued = live.host.driver.shared.state_for_tests();
    let request = queued
        .progress
        .host
        .queue
        .iter()
        .chain(queued.progress.host.applying.iter())
        .find(|request| request.id == id);
    if reply.is_none() {
        let request = request.expect("admitted input is in native custody before the reply");
        let RequestKind::Submit { bytes, .. } = &request.kind else {
            panic!("submit")
        };
        assert_eq!(
            bytes, &expected,
            "wire generation serialized differently at admission"
        );
    } else {
        assert!(request.is_none(), "refused request acquired custody");
    }
    let value = json!({"reply": reply, "custody": request.is_some(),
        "bytes": expected.map(|b| String::from_utf8(b.to_vec()).unwrap())});
    if let Some(external) = arg["coalesceBytes"].as_str() {
        assert!(reply.is_none());
        // Hold the existing test guard across native apply, native save and
        // observation. Book::collect then sees a real answer plus newer content
        // in ONE delivery, just as when the driver collects after those events.
        queued.progress.with_host(|host| {
            host.test.paused = false;
            let held = host.held.take(); // existing test mode takes real path locks per step
            if host.applying.is_none() {
                assert_eq!(host.dequeue(), Disposition::Pending);
            }
            assert_eq!(host.apply_request(), Disposition::Applied);
            assert_eq!(host.start_save(key), Disposition::Pending);
            for _ in 0..20 {
                if host.job.is_none() {
                    break;
                }
                host.advance_save();
            }
            assert!(host.pages[key].clean(), "native save completed");
            fs::write(live.root.join(key), external).unwrap();
            assert_eq!(host.observe(key), Disposition::Applied);
            host.held = held;
        });
    }
    drop(queued);
    value
}

fn command(
    live: &mut Option<Live>,
    subscription: &mut Option<crate::Subscription>,
    oracle: &mut crate::page_state::conformance::Oracle,
    arg: Value,
) -> Value {
    let op = arg["op"].as_str().unwrap();
    if op == "reset" {
        *oracle = crate::page_state::conformance::Oracle::new("base", 1);
        if let Some(old) = live.take() {
            old.host.stop();
        }
        let initial = format!("- target\n  id:: {TARGET}\n");
        *live = Some(Live::watching(
            &[
                ("pages/Target.md", &initial),
                ("pages/Alpha.md", "- initial [[Target]]\n"),
                ("pages/Beta.md", "- bee\n"),
            ],
            crate::WatchMode::Poll,
        ));
        let run = live.as_ref().unwrap();
        run.submits_wait();
        *subscription = Some(run.store.subscribe());
        return image(&run.store);
    }
    if op == "oracle" {
        *oracle = oracle
            .next(
                arg["action"].as_str().unwrap(),
                arg["args"].as_array().unwrap(),
            )
            .unwrap_or_else(|| panic!("disabled oracle action: {arg}"));
        return oracle.state();
    }
    let run = live.as_mut().unwrap();
    let session = arg["session"].as_u64().unwrap_or(run.host.session());
    let id = arg["id"].as_u64().unwrap_or(0);
    let key = arg["key"].as_str().unwrap_or("");
    match op {
        "reload" => json!(run.host.window_reloaded()),
        "open" => json!(run
            .host
            .open(
                session,
                id,
                &PageId::from(arg["path"].as_str().unwrap()),
                arg["name"].as_str().unwrap()
            )
            .map_or_else(|r| json!(r), |o| json!(o))),
        "submit" => paused_submit(run, &arg),
        "resume" => {
            run.pause(false);
            Value::Null
        }
        "mail" => {
            let mut mail = Vec::new();
            if let Some(id) = arg["answer"].as_u64() {
                loop {
                    let next = run
                        .mail
                        .recv_timeout(Duration::from_secs(10))
                        .expect("native answer");
                    let done = next.answer.as_ref().is_some_and(|a| a.id == id);
                    mail.push(next);
                    if done {
                        break;
                    }
                }
            }
            mail.extend(run.mail.try_iter());
            json!(mail)
        }
        "close" => json!(run.host.close(session, id, key).err()),
        "discard" => json!(run.host.discard(session, id, key).err()),
        "save" => {
            run.host.save_now(
                session,
                &serde_json::from_value::<Vec<String>>(arg["keys"].clone()).unwrap(),
            );
            Value::Null
        }
        "wait" => {
            let needs: Vec<(String, u64, Option<String>)> = arg["needs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| {
                    (
                        n["key"].as_str().unwrap().into(),
                        n["version"].as_u64().unwrap(),
                        n["witness"].as_str().map(str::to_owned),
                    )
                })
                .collect();
            match run
                .host
                .wait_published(session, &needs, Duration::from_millis(50))
            {
                PageOperation::Applied => json!(true),
                PageOperation::Refused => json!(false),
                _ => Value::Null,
            }
        }
        "owed" => json!(run.host.owed(session, None).map(|owed| owed
            .into_iter()
            .map(|(key, version)| json!({"key":key,"version":version}))
            .collect::<Vec<_>>())),
        "inspect" => {
            let state = run.host.driver.shared.state_for_tests();
            json!({"lastAdmitted":state.progress.host.last_admitted,
                "buffers":state.progress.host.pages.iter().map(|(key, page)|
                    (key.clone(), json!({"version":page.version,"bytes":page.buf.as_ref()
                        .map(|b| String::from_utf8(b.to_vec()).unwrap())}))).collect::<BTreeMap<_,_>>()})
        }
        "external" => {
            fs::write(
                run.root.join(arg["path"].as_str().unwrap()),
                arg["bytes"].as_str().unwrap(),
            )
            .unwrap();
            assert!(run.store.watcher_drained_for_tests(Duration::from_secs(10)));
            Value::Null
        }
        "remove" => {
            fs::remove_file(run.root.join(arg["path"].as_str().unwrap())).unwrap();
            assert!(run.store.watcher_drained_for_tests(Duration::from_secs(10)));
            Value::Null
        }
        "image" => image(&run.store),
        "fresh" => fresh(run),
        "changes" => {
            let mut changes = Vec::new();
            while let Some(change) = subscription.as_ref().unwrap().try_recv().unwrap() {
                changes.push(serde_json::to_value(change).unwrap());
            }
            json!(changes)
        }
        "restore" => {
            // Exact saved-stop/relaunch lifecycle used around Store::restore; the restore
            // multi-file protocol is a separate model and is intentionally not injected.
            let old = live.take().unwrap();
            let Live {
                _dir,
                root,
                store,
                host,
                mail,
                kinds,
                id,
            } = old;
            let stopped = host
                .stop_saved(arg["consumed"].as_u64().unwrap(), StopMode::Restore)
                .unwrap_or_else(|_| panic!("saved restore stop refused"));
            let host = stopped.relaunch().unwrap();
            *live = Some(Live {
                _dir,
                root,
                store,
                host,
                mail,
                kinds,
                id,
            });
            Value::Null
        }
        _ => panic!("unknown op {op}"),
    }
}

#[test]
#[ignore = "JSON-lines peer driven by src/document/host/nativeConformance.test.ts"]
fn native_window_peer() {
    let (mut live, mut subscription) = (None, None);
    let mut oracle = crate::page_state::conformance::Oracle::new("base", 1);
    for line in std::io::stdin().lock().lines() {
        let arg: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let value = command(&mut live, &mut subscription, &mut oracle, arg);
        println!("TINE_WINDOW_JSON {value}");
        std::io::stdout().flush().unwrap();
    }
    if let Some(run) = live {
        run.host.stop();
    }
}
