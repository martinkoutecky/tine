//! The owner endpoint end to end, in process: a real Store and page host,
//! a real socket, the client's handshake and calls (CLI PROTOCOL §2–5),
//! and the app lease through retirement and adoption (§3.1–3.2).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tine_store::{Input, PageHost, PageId, Store};

use super::client::{open, Conn, Failure};
use super::endpoint::{After, Endpoint, Gate, Handler};
use super::lease::{hello, Claimed};
use super::lock::OwnerLock;
use super::namespace::GraphPaths;
use super::wire::{ErrorBody, Request};
use super::OwnerError;
use crate::state::{GraphRegistry, GraphSlot, PageHostSlot};

const UUID: &str = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";

struct Graph {
    _dir: tempfile::TempDir,
    root: PathBuf,
    app_data: PathBuf,
    owners: PathBuf,
    store: Arc<Store>,
}

impl Graph {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("graph");
        std::fs::create_dir_all(graph.join("pages")).unwrap();
        std::fs::create_dir_all(graph.join("logseq")).unwrap();
        std::fs::write(
            graph.join("logseq/config.edn"),
            "{:file/name-format :triple-lowbar}\n",
        )
        .unwrap();
        std::fs::write(
            graph.join("pages/a.md"),
            format!("- one\n  id:: {UUID}\n\t- under one\n- two\n"),
        )
        .unwrap();
        std::fs::write(
            graph.join("pages/b.md"),
            "- see [[a]]\n- and ((6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f))\n",
        )
        .unwrap();
        std::fs::write(graph.join("pages/ns___child.md"), "- in a namespace\n").unwrap();
        let app_data = dir.path().join("app");
        let owners = dir.path().join("owners");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&owners).unwrap();
        let store = Arc::new(Store::open(&graph, Default::default()).unwrap().0);
        store.whole_graph().unwrap();
        let root = Store::canonical_root(&graph).unwrap();
        Self {
            _dir: dir,
            root,
            app_data,
            owners,
            store,
        }
    }

    fn slot(&self) -> GraphSlot {
        let mut slot = GraphSlot::new(self.store.clone(), self.root.clone());
        let host = PageHost::start_for_tests(&self.store, &self.app_data).unwrap();
        *slot.host.get_mut().unwrap() = PageHostSlot::Running(host);
        slot
    }

    fn paths(&self) -> GraphPaths {
        GraphPaths {
            lock: self.owners.join("g.lock"),
            socket: self.owners.join("g.sock"),
            start_log: self.owners.join("g.start.log"),
        }
    }

    fn connect(&self) -> Conn {
        open(&self.paths().socket, &self.root)
            .unwrap()
            .expect("an endpoint")
    }
}

/// The verbs over one slot, as `tine serve` answers them.
struct Owner(Arc<GraphSlot>);

impl Handler for Owner {
    fn answer(&self, request: &Request) -> (Result<Value, ErrorBody>, After) {
        (super::verbs::answer(&self.0, request), After::Continue)
    }

    fn status(&self, gate: &Gate) -> Value {
        json!({ "state": gate.state() })
    }
}

fn endpoint(graph: &Graph, slot: Arc<GraphSlot>) -> (Endpoint, Arc<Gate>) {
    let gate = Gate::new();
    let endpoint = Endpoint::bind(
        &graph.paths().socket,
        hello(&graph.root, "serve"),
        gate.clone(),
        Arc::new(Owner(slot)),
        None,
    )
    .unwrap();
    (endpoint, gate)
}

fn code(result: Result<Value, Failure>) -> String {
    match result {
        Err(Failure::Owner(body)) => body.code,
        other => panic!("expected an owner error, got {other:?}"),
    }
}

#[test]
fn the_read_verbs_answer_over_the_socket() {
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = endpoint(&graph, slot);
    let mut conn = graph.connect();

    let page = conn.call("page.show", json!({ "name": "A" })).unwrap();
    assert_eq!(page["schema"], "tine.cli/1");
    assert_eq!(page["kind"], "page");
    assert_eq!(page["id"], "pages/a.md");
    assert!(page["text"].as_str().unwrap().starts_with("- one\n"));
    assert!(!page["rev"].as_str().unwrap().is_empty());
    assert!(
        page["revision"].is_null(),
        "an unheld page has no host version"
    );
    assert_eq!(page["page"]["blocks"][0]["children"][0]["raw"], "under one");

    let block = conn.call("block.show", json!({ "uuid": UUID })).unwrap();
    assert_eq!(block["kind"], "block");
    assert_eq!(block["block"]["group"]["page"], "a");

    let refs = conn.call("refs", json!({ "target": "a" })).unwrap();
    assert_eq!(refs["groups"][0]["page"], "b");
    let block_refs = conn
        .call("refs", json!({ "target": UUID, "block": true }))
        .unwrap();
    assert_eq!(block_refs["groups"][0]["page"], "b");

    let search = conn.call("search", json!({ "text": "namespace" })).unwrap();
    assert!(search["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit["entity"] == "block"));

    let query = conn.call("query", json!({ "text": "[[a]]" })).unwrap();
    assert_eq!(query["kind"], "query");
    // OG's simple query matches by path-refs: page a's own blocks too.
    let matched: Vec<&Value> = query["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| &group["page"])
        .collect();
    assert!(matched.contains(&&json!("b")), "{matched:?}");

    let pages = conn.call("pages", Value::Null).unwrap();
    let names: Vec<&str> = pages["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"a") && names.contains(&"b") && names.contains(&"ns/child"),
        "{names:?}"
    );
    let under = conn.call("pages", json!({ "namespace": "NS" })).unwrap();
    assert_eq!(under["pages"].as_array().unwrap().len(), 1);

    assert_eq!(
        code(conn.call("page.show", json!({ "name": "nope" }))),
        "not-found"
    );
    assert_eq!(
        code(conn.call("journal", json!({ "date": "2026-10-12" }))),
        "not-found"
    );
    assert_eq!(
        code(conn.call("journal", json!({ "date": "soon" }))),
        "bad-args"
    );
    assert_eq!(
        code(conn.call("block.show", json!({ "uuid": "3" }))),
        "bad-args"
    );
    assert_eq!(
        code(conn.call("page.show", json!({ "page": "a" }))),
        "bad-args"
    );
    assert_eq!(code(conn.call("page.edit", Value::Null)), "unknown-verb");
}

#[test]
fn a_paused_owner_answers_busy_and_still_reports_status() {
    let graph = Graph::new();
    let (_endpoint, gate) = endpoint(&graph, Arc::new(graph.slot()));
    let mut conn = graph.connect();
    gate.pause();
    let busy = conn.call("pages", Value::Null);
    match busy {
        Err(Failure::Owner(body)) => {
            assert_eq!(body.code, "busy");
            assert_eq!(
                body.detail.unwrap()["admitted"],
                false,
                "busy is never admitted"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        conn.call("owner.status", Value::Null).unwrap()["state"],
        "busy"
    );
    gate.resume();
    assert!(conn.call("pages", Value::Null).is_ok());
}

#[test]
fn a_handshake_for_another_graph_or_protocol_is_refused() {
    let graph = Graph::new();
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let other = graph.root.join("pages");
    assert!(matches!(
        open(&graph.paths().socket, &other),
        Err(OwnerError::Mismatch(_))
    ));
    let reply = raw(
        &graph.paths().socket,
        b"{\"tine\":\"cli\",\"protocol\":99,\"version\":\"x\",\"graph\":\"g\"}\n",
    );
    assert!(reply.contains("\"code\":\"protocol\""), "{reply}");
}

#[test]
fn an_oversized_or_malformed_line_is_answered_and_closed() {
    let graph = Graph::new();
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let mut big = vec![b'x'; super::wire::REQUEST_CAP + 10];
    big.push(b'\n');
    assert!(raw(&graph.paths().socket, &big).contains("frame-too-large"));
    assert!(raw(&graph.paths().socket, b"[1,2]\n").contains("malformed"));
}

/// Write `bytes`, half-close, and read whatever comes back.
fn raw(socket: &Path, bytes: &[u8]) -> String {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let _ = stream.write_all(bytes);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    answer
}

#[test]
fn closing_the_endpoint_removes_its_path_and_drops_its_connections() {
    let graph = Graph::new();
    let (endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let mut conn = graph.connect();
    assert!(conn.call("pages", Value::Null).is_ok());
    drop(endpoint);
    assert!(!graph.paths().socket.exists());
    assert!(matches!(
        conn.call("pages", Value::Null),
        Err(Failure::Lost(_))
    ));
    assert!(open(&graph.paths().socket, &graph.root).unwrap().is_none());
}

#[test]
fn a_dead_owners_socket_path_is_replaced_by_the_next_holder() {
    let graph = Graph::new();
    // A crashed owner leaves its path; the lock went with the process.
    std::fs::write(graph.paths().socket, b"").unwrap();
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    assert!(graph.connect().call("pages", Value::Null).is_ok());
}

/// The app's lease follows the host (§3.1): retirement pauses it, an
/// adoption moves it to the new slot and admits again, and it is released
/// (endpoint gone, lock free) only once the host has stopped.
#[test]
fn the_app_lease_follows_the_host_through_retirement_and_adoption() {
    let graph = Graph::new();
    let paths = graph.paths();
    let lock = OwnerLock::try_take(&paths.lock, "app").unwrap().unwrap();
    let mut registry = GraphRegistry::default();
    let old = Arc::new(graph.slot());
    *old.owner.lock().unwrap() = Some(Claimed::from_lock(lock, paths.clone()).serve_app(&old));
    registry.bind("graph-1".into(), old.clone()).unwrap();
    let mut conn = graph.connect();
    assert!(conn.call("pages", Value::Null).is_ok());

    // A reservation keeps the host from stopping until it is adopted.
    let reservation = match &*old.host.read().unwrap() {
        PageHostSlot::Running(host) => host
            .reserve(|| vec![PageId::from("pages/a.md")], Input::Refuse)
            .unwrap(),
        _ => panic!("no host"),
    };
    assert!(registry.remove("graph-1").is_none(), "retiring");
    assert_eq!(code(conn.call("pages", Value::Null)), "busy");
    assert!(
        OwnerLock::try_take(&paths.lock, "other").unwrap().is_none(),
        "a retiring host keeps its lease"
    );

    let (store, host, lease) = registry.retirement.adopt(&graph.root).expect("retiring");
    let lease = lease.expect("the lease moves with the host");
    let mut fresh = GraphSlot::new(store, graph.root.clone());
    *fresh.host.get_mut().unwrap() = host;
    let fresh = Arc::new(fresh);
    lease.adopted_by(&fresh);
    *fresh.owner.lock().unwrap() = Some(lease);
    drop(old);
    assert!(
        conn.call("pages", Value::Null).is_ok(),
        "admitted again, at the new slot"
    );
    drop(reservation);

    registry.bind("graph-2".into(), fresh.clone()).unwrap();
    drop(fresh);
    assert!(registry.remove("graph-2").is_none(), "retiring again");
    registry
        .retirement
        .wait_idle(Duration::from_secs(20))
        .expect("the host stops");
    // The lease drops just after the root leaves the retirement, outside
    // the host lock; a reopen meanwhile polls the lock (`client::take_over`).
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let next = loop {
        if let Some(next) = OwnerLock::try_take(&paths.lock, "next").unwrap() {
            break next;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the lease is never released"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        !paths.socket.exists(),
        "the endpoint closed before the lock"
    );
    drop(next);
}

/// `owner.yield` (§3.5): custody first. An opener whose drafts live
/// elsewhere is refused and the owner keeps serving; a matching one gets
/// the host stopped, then the reply, then the owner exits.
#[test]
fn serve_yields_only_into_its_own_draft_namespace() {
    use super::serve::ServeOwner;
    use super::wire::Drafts;
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let drafts = Drafts {
        app_data: graph.app_data.display().to_string(),
        graph_id: "test-graph".into(),
    };
    let owner = ServeOwner {
        slot: slot.clone(),
        gate: Gate::new(),
        root: graph.root.clone(),
        drafts: drafts.clone(),
    };
    let ask = |args: Value| {
        owner.answer(&Request {
            id: 1,
            verb: "owner.yield".into(),
            args,
        })
    };
    let elsewhere = json!({ "drafts": { "app_data": "/elsewhere", "graph_id": "test-graph" } });
    let (refused, after) = ask(elsewhere);
    let body = refused.unwrap_err();
    assert_eq!(body.code, "refused");
    assert_eq!(body.detail.unwrap()["reason"], "draft-namespace");
    assert!(matches!(after, After::Continue));
    assert_eq!(
        owner.gate.state(),
        "serving",
        "a refused yield keeps serving"
    );
    assert!(matches!(
        &*slot.host.read().unwrap(),
        PageHostSlot::Running(_)
    ));
    let (bad, _) = ask(json!({ "drafts": drafts, "force": true }));
    assert_eq!(bad.unwrap_err().code, "bad-args");

    let (yielded, after) = ask(json!({ "drafts": drafts }));
    assert_eq!(yielded.unwrap()["yielded"], true);
    assert!(matches!(after, After::Exit));
    assert!(
        !matches!(&*slot.host.read().unwrap(), PageHostSlot::Running(_)),
        "the host stopped before the reply"
    );
    assert_eq!(owner.gate.state(), "stopping", "admits nothing more");
}

/// §1: the namespace is a private directory of this account, and a path
/// the socket cannot bind is refused before anything binds it.
#[test]
fn the_namespace_is_private_and_fits_a_socket_path() {
    use super::namespace::Namespace;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let owners = dir.path().join("state/owners");
    let namespace = Namespace::at(owners.clone()).unwrap();
    let mode = std::fs::metadata(&owners).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    let paths = namespace.graph(Path::new("/some/graph")).unwrap();
    let name = paths.lock.file_name().unwrap().to_str().unwrap();
    assert!(name.len() == 21 && name.ends_with(".lock"), "{name}");
    assert_eq!(paths.socket.with_extension("lock"), paths.lock);

    std::fs::set_permissions(&owners, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        Namespace::at(owners.clone()),
        Err(OwnerError::Unsafe(_))
    ));
    std::fs::set_permissions(&owners, std::fs::Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&owners, &link).unwrap();
    assert!(matches!(Namespace::at(link), Err(OwnerError::Unsafe(_))));

    let deep = dir.path().join("d".repeat(120));
    let namespace = Namespace::at(deep).unwrap();
    assert!(matches!(
        namespace.graph(Path::new("/g")),
        Err(OwnerError::Unsafe(_))
    ));
}

/// AGENTS.md §2: a platform `cfg` family names every shipped target. The
/// namespace's arms cover Linux and macOS (an endpoint) and refuse on
/// Windows, iOS and Android; the catch-all means "not a Tine platform".
#[test]
fn the_namespace_names_every_shipped_target() {
    let source = include_str!("namespace.rs");
    assert_eq!(
        source.matches("fn account_dir()").count(),
        4,
        "one account_dir per platform family"
    );
    let cfgs: Vec<&str> = source
        .match_indices("fn account_dir()")
        .map(|(at, _)| {
            let head = &source[..at];
            &head[head.rfind("#[cfg(").unwrap()..]
        })
        .collect();
    let named = |os: &str| format!("target_os = \"{os}\"");
    for os in ["linux", "macos", "windows", "ios", "android"] {
        let positive = cfgs
            .iter()
            .filter(|cfg| !cfg.starts_with("#[cfg(not(") && cfg.contains(&named(os)))
            .count();
        assert_eq!(positive, 1, "{os}: exactly one positive account_dir arm");
        assert!(
            cfgs.iter()
                .any(|cfg| cfg.starts_with("#[cfg(not(") && cfg.contains(&named(os))),
            "{os}: the catch-all must exclude it"
        );
    }
}

/// PROTOCOL §6: the endpoint is the command line's alone. No Tauri command
/// in the owner module, and the app's invoke handler names nothing of it,
/// so neither the webview nor a plugin (whose capabilities are semantic
/// verbs, ADR 0052) can reach the endpoint.
#[test]
fn the_endpoint_has_no_webview_or_plugin_door() {
    for (name, source) in [
        ("client", include_str!("client.rs")),
        ("endpoint", include_str!("endpoint.rs")),
        ("lease", include_str!("lease.rs")),
        ("lock", include_str!("lock.rs")),
        ("mod", include_str!("mod.rs")),
        ("namespace", include_str!("namespace.rs")),
        ("serve", include_str!("serve.rs")),
        ("terminal", include_str!("terminal.rs")),
        ("verbs", include_str!("verbs.rs")),
        ("wire", include_str!("wire.rs")),
    ] {
        assert!(
            !source.contains(concat!("tauri::", "command")),
            "owner/{name}.rs: no Tauri command may reach the owner endpoint (PROTOCOL §6)"
        );
    }
    // Every owner file is scanned: a new one joins the list above.
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/owner");
    let files = std::fs::read_dir(dir)
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.ends_with(".rs") && name != "tests.rs"
        })
        .count();
    assert_eq!(files, 10, "scan every owner/*.rs file");
    let lib = include_str!("../lib.rs");
    let start = lib.find("generate_handler![").expect("the invoke handler");
    let handlers = &lib[start..start + lib[start..].find("])").unwrap()];
    assert!(
        !handlers.contains("owner::"),
        "the invoke handler must not name the owner module (PROTOCOL §6)"
    );
}
