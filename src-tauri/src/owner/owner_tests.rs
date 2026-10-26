//! The owner endpoint end to end, in process: a real Store and page host,
//! a real socket, the client's handshake and calls (CLI PROTOCOL §2–5),
//! and the app lease through retirement and adoption (§3.1–3.2).
// The endpoint is Unix-only in slice 1a (PROTOCOL §10: Windows has none yet).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tine_store::{Input, PageHost, PageId, Store};

use super::client::{dial, Conn, Failure};
use super::endpoint::{After, Endpoint, Gate, Handler, YieldRequest};
use super::lease::{hello, Claimed};
use super::lock::OwnerLock;
use super::namespace::GraphPaths;
use super::wire::{ErrorBody, Request};
use super::OwnerError;
use crate::state::{GraphRegistry, GraphSlot, PageHostSlot};

const UUID: &str = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";

#[path = "owner_tests/lifecycle_tests.rs"]
mod lifecycle;

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
        dial(&self.paths().socket, &self.root)
            .unwrap()
            .expect("an endpoint")
    }
}

/// The verbs over one slot, as `tine serve` answers them.
struct Owner(Arc<GraphSlot>);

impl Handler for Owner {
    fn answer_request(
        &self,
        request: &Request,
        _yielder: &mut YieldRequest,
    ) -> (Result<Value, ErrorBody>, After) {
        (super::verbs::answer_verb(&self.0, request), After::Continue)
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

/// GH #771: on macOS an accepted socket inherits the listener's O_NONBLOCK.
#[test]
fn an_accepted_connection_is_made_blocking() {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    let (stream, _peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    super::endpoint::blocking_connection(&stream).unwrap();
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
    assert_eq!(flags & libc::O_NONBLOCK, 0);
}

/// GH #771: a client that pauses between the hello and its first request is
/// still served (the macOS owner closed it at once and the CLI saw EINVAL).
#[test]
fn a_client_that_waits_before_its_first_request_is_served() {
    let graph = Graph::new();
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let mut conn = graph.connect();
    std::thread::sleep(Duration::from_millis(300));
    assert!(conn.call("owner.status", Value::Null).is_ok());
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
        dial(&graph.paths().socket, &other),
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
    assert!(dial(&graph.paths().socket, &graph.root).unwrap().is_none());
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
    assert_eq!(code(conn.call("owner.yield", Value::Null)), "refused");
    assert!(
        conn.call("pages", Value::Null).is_ok(),
        "the app never claims a yield stop"
    );

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
        std::thread::yield_now();
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
    use super::wire::Drafts;
    let graph = Graph::new();
    let drafts = Drafts {
        app_data: graph.app_data.display().to_string(),
        graph_id: "test-graph".into(),
    };
    let serving = lifecycle::Serving::new(&graph);
    let slot = serving.slot.clone();
    let gate = serving.gate.clone();
    let finished = serving.run();
    let mut conn = graph.connect();
    let elsewhere = json!({ "drafts": { "app_data": "/elsewhere", "graph_id": "test-graph" } });
    let Err(Failure::Owner(body)) = conn.call("owner.yield", elsewhere) else {
        panic!("expected refusal")
    };
    assert_eq!(body.code, "refused");
    assert_eq!(body.detail.unwrap()["reason"], "draft-namespace");
    assert_eq!(gate.state(), "serving", "a refused yield keeps serving");
    assert!(matches!(
        &*slot.host.read().unwrap(),
        PageHostSlot::Running(_)
    ));
    // Args are an object; fields this owner does not know are ignored (§2.4).
    assert_eq!(code(conn.call("owner.yield", json!([drafts]))), "bad-args");

    let yielded = conn
        .call("owner.yield", json!({ "drafts": drafts }))
        .unwrap();
    assert_eq!(yielded["yielded"], true);
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        !matches!(&*slot.host.read().unwrap(), PageHostSlot::Running(_)),
        "the host stopped before the reply"
    );
    assert_eq!(gate.state(), "stopping", "admits nothing more");
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
            name.ends_with(".rs") && name != "owner_tests.rs"
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

/// An owner whose `slow` request holds until released.
struct Holding {
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Handler for Holding {
    fn answer_request(
        &self,
        request: &Request,
        _yielder: &mut YieldRequest,
    ) -> (Result<Value, ErrorBody>, After) {
        match request.verb.as_str() {
            "slow" => {
                let _ = self.release.lock().unwrap().recv();
                (Ok(json!({})), After::Continue)
            }
            _ => (Ok(json!({})), After::Continue),
        }
    }

    fn status(&self, gate: &Gate) -> Value {
        json!({ "state": gate.state() })
    }
}

fn holding(graph: &Graph) -> (Endpoint, Arc<Gate>, std::sync::mpsc::Sender<()>) {
    let gate = Gate::new();
    let (release, released) = std::sync::mpsc::channel();
    let endpoint = Endpoint::bind(
        &graph.paths().socket,
        hello(&graph.root, "serve"),
        gate.clone(),
        Arc::new(Holding {
            release: std::sync::Mutex::new(released),
        }),
    )
    .unwrap();
    (endpoint, gate, release)
}

#[test]
fn a_stop_settles_admitted_work_and_only_resumed_admission_cuts_it_short() {
    // Review CODE-1 F1: a stop waits for admitted work, not a socket deadline.
    let graph = Graph::new();
    let (_endpoint, gate, release) = holding(&graph);
    let mut conn = graph.connect();
    let slow = std::thread::spawn(move || conn.call("slow", Value::Null).is_ok());
    lifecycle::wait_for(|| gate.in_flight() == 1);
    gate.pause();
    let settling = {
        let gate = gate.clone();
        std::thread::spawn(move || gate.settle(0))
    };
    lifecycle::wait_for(|| gate.is_settling());
    assert!(!settling.is_finished(), "settled with work in flight");
    release.send(()).unwrap();
    lifecycle::wait_for(|| settling.is_finished());
    settling.join().unwrap();
    assert!(slow.join().unwrap());
    // An adoption resumes admission: a settle in progress returns.
    gate.resume();
    let mut conn = graph.connect();
    let slow = std::thread::spawn(move || conn.call("slow", Value::Null).is_ok());
    lifecycle::wait_for(|| gate.in_flight() == 1);
    gate.pause();
    let settling = {
        let gate = gate.clone();
        std::thread::spawn(move || gate.settle(0))
    };
    lifecycle::wait_for(|| gate.is_settling());
    gate.resume();
    lifecycle::wait_for(|| settling.is_finished());
    settling.join().unwrap();
    release.send(()).unwrap();
    assert!(slow.join().unwrap());
}

#[test]
fn a_yield_closes_the_other_connections_before_it_replies() {
    // Review CODE-1 F1 / PROTOCOL §4.3.
    let graph = Graph::new();
    let finished = lifecycle::Serving::new(&graph).run();
    let mut other = graph.connect();
    assert!(other.call("pages", Value::Null).is_ok());
    let mut yielding = graph.connect();
    assert!(yielding.call("owner.yield", Value::Null).is_ok());
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        other.call("pages", Value::Null),
        Err(Failure::Lost(_))
    ));
}

/// A peer on the graph's socket that answers each handshake with `hello`
/// and each request with `reply` (or never, when `None`).
fn fake_owner(socket: &Path, hello: Value, reply: Option<Value>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let hello = hello.clone();
            let reply = reply.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let _ = writeln!(writer, "{hello}");
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if let Some(reply) = &reply {
                        let _ = writeln!(writer, "{reply}");
                    }
                }
            });
        }
    });
}

fn owner_hello(graph: &str, owner: Option<&str>) -> Value {
    let mut hello = json!({ "tine": "owner", "protocol": 1, "version": "x", "graph": graph });
    if let Some(owner) = owner {
        hello["owner"] = json!(owner);
    }
    hello
}

#[test]
fn the_client_checks_the_owners_hello_and_each_replys_id() {
    // Review CODE-1 F4.
    let graph = Graph::new();
    let root = graph.root.display().to_string();
    let socket = graph.paths().socket;
    fake_owner(&socket, owner_hello("/elsewhere", Some("serve")), None);
    assert!(matches!(
        dial(&socket, &graph.root),
        Err(OwnerError::Mismatch(_))
    ));
    std::fs::remove_file(&socket).unwrap();
    fake_owner(&socket, owner_hello(&root, None), None);
    assert!(matches!(
        dial(&socket, &graph.root),
        Err(OwnerError::Protocol(_))
    ));
    std::fs::remove_file(&socket).unwrap();
    fake_owner(
        &socket,
        owner_hello(&root, Some("serve")),
        Some(json!({ "id": 99, "ok": {} })),
    );
    let mut conn = dial(&socket, &graph.root).unwrap().unwrap();
    assert!(matches!(
        conn.call("pages", Value::Null),
        Err(Failure::Lost(_))
    ));
}

#[test]
fn the_owner_takes_only_a_clients_hello() {
    // Review CODE-1 F4: the owner checks `tine == "cli"`.
    let graph = Graph::new();
    let (_endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let line = format!(
        "{}\n",
        owner_hello(&graph.root.display().to_string(), Some("serve"))
    );
    assert!(raw(&graph.paths().socket, line.as_bytes()).contains("malformed"));
}

#[test]
fn a_wedged_owner_cannot_hold_the_handover_past_its_bound() {
    // Review CODE-1 F5: the status probe obeys the remaining deadline.
    let graph = Graph::new();
    let paths = graph.paths();
    let _held = OwnerLock::try_take(&paths.lock, "serve").unwrap().unwrap();
    fake_owner(
        &paths.socket,
        owner_hello(&graph.root.display().to_string(), Some("serve")),
        None,
    );
    let (done, finished) = std::sync::mpsc::channel();
    let root = graph.root.clone();
    std::thread::spawn(move || {
        let drafts = super::wire::Drafts {
            app_data: "a".into(),
            graph_id: "g".into(),
        };
        let result =
            super::client::take_over(&root, &paths, drafts, Duration::from_secs(2), |_| {});
        let _ = done.send(result.is_err());
    });
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(15)),
        Ok(true),
        "the handover outlived its bound"
    );
}

#[test]
fn the_start_log_is_emptied_past_its_cap() {
    // Review CODE-1 F9.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("start.log");
    std::fs::write(&path, vec![b'x'; 2048]).unwrap();
    let log = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    super::serve::bound_log(&log, 4096);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 2048);
    super::serve::bound_log(&log, 1024);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
}

#[test]
fn a_second_yield_is_refused_so_the_first_settles() {
    // Review CODE-2 #1: two concurrent yields never wait on each other.
    use super::serve::ServeOwner;
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (stops, stopped) = std::sync::mpsc::channel();
    let owner = Arc::new(ServeOwner {
        slot: slot.clone(),
        gate: Gate::new(),
        root: graph.root.clone(),
        drafts: super::wire::Drafts {
            app_data: graph.app_data.display().to_string(),
            graph_id: "test-graph".into(),
        },
        stops,
    });
    let yield_request = || Request {
        id: 1,
        verb: "owner.yield".into(),
        args: Value::Null,
    };
    // Both admitted, as the endpoint admits them, before either runs.
    let first = owner.gate.admit().unwrap();
    let second = owner.gate.admit().unwrap();
    let (done, finished) = std::sync::mpsc::channel();
    {
        let (owner, request) = (owner.clone(), yield_request());
        std::thread::spawn(move || {
            let mut yielder = YieldRequest::new(0);
            let (result, after) = owner.answer_request(&request, &mut yielder);
            drop(first);
            yielder.acknowledge_closed();
            let _ = done.send((result.is_ok(), after == After::Close));
        });
    }
    let stop = stopped.recv_timeout(Duration::from_secs(5)).unwrap();
    let (refused, after) = owner.answer_request(&yield_request(), &mut YieldRequest::new(1));
    assert_eq!(refused.unwrap_err().code, "busy");
    assert_eq!(after, After::Continue);
    drop(second);
    owner.gate.settle(1);
    let yielder = stop.yielder.unwrap();
    yielder
        .verdict
        .send(Ok(json!({ "yielded": true })))
        .unwrap();
    yielder.closed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        matches!(
            stopped.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "a competing yield scheduled another stop sequence"
    );
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(30)),
        Ok((true, true)),
        "the first yield never settled"
    );
}

#[test]
fn a_stop_is_claimed_once_and_never_reopened() {
    // Review CODE-2 #2: the idle exit claims only an idle gate, in one
    // step, and nothing resumes admission after a stop.
    let gate = Gate::new();
    let admitted = gate.admit().unwrap();
    assert!(
        !gate.claim_idle_stop(Duration::ZERO),
        "claimed with work in flight"
    );
    drop(admitted);
    assert!(gate.claim_stop());
    assert!(!gate.claim_idle_stop(Duration::ZERO));
    assert!(!gate.claim_stop());
    gate.resume();
    assert!(gate.admit().is_none(), "a resume reopened a stopped owner");
}

#[test]
fn a_connection_accepted_after_a_yields_sweep_is_closed() {
    // Review CODE-2 #3: the sweep seals registration.
    let graph = Graph::new();
    let finished = lifecycle::Serving::new(&graph).run();
    let mut yielding = graph.connect();
    assert!(yielding.call("owner.yield", Value::Null).is_ok());
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    let served = match dial(&graph.paths().socket, &graph.root) {
        Ok(Some(mut late)) => late.call("ping", Value::Null).is_ok(),
        _ => false,
    };
    assert!(!served, "a connection accepted after the sweep was served");
}
