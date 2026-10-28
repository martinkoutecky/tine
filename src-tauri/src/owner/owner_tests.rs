//! The owner endpoint end to end, in process: a real Store and page host,
//! a real socket, the client's handshake and calls (CLI PROTOCOL §2–5),
//! and the app lease through retirement and adoption (§3.1–3.2).
// Shared protocol and lifecycle tests run on all three desktop transports.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tine_store::{PageHost, PageId, Store};

use super::client::{dial, Conn, Failure};
use super::endpoint::{After, Endpoint, Gate, Handler, YieldRequest};
use super::lease::{hello, Claimed};
use super::lock::OwnerLock;
use super::namespace::GraphPaths;
#[cfg(windows)]
use super::windows::Stream as RawStream;
use super::wire::{ErrorBody, Request};
use super::OwnerError;
use crate::state::{GraphRegistry, GraphSlot, PageHostSlot};
#[cfg(unix)]
use std::os::unix::net::UnixStream as RawStream;

const UUID: &str = "6512bd43-d9ca-4e3b-9f7a-1a2b3c4d5e6f";

#[path = "owner_tests/lifecycle_tests.rs"]
mod lifecycle;

#[path = "owner_tests/proof.rs"]
mod proof;

#[path = "owner_tests/page_tests.rs"]
mod page_tests;

#[cfg(desktop)]
#[path = "owner_tests/git_run_tests.rs"]
mod git_run;

#[cfg(desktop)]
#[path = "owner_tests/git_pull_tests.rs"]
mod git_pull;

#[cfg(desktop)]
#[path = "owner_tests/git_sync_cycle_tests.rs"]
mod git_sync_cycle;

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
        std::fs::write(graph.join("pages/dup.md"), "- same\n- same\n").unwrap();
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
            #[cfg(unix)]
            socket: self.owners.join("g.sock"),
            #[cfg(windows)]
            socket: super::windows_security::pipe_name(&format!(
                "{:016x}",
                crate::settings::graph_hash(&self.root)
            ))
            .unwrap(),
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
#[cfg(unix)]
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
    let mut stream = RawStream::connect(socket).unwrap();
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
fn a_successor_can_listen_while_a_closed_client_handle_still_exists() {
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (first, _gate) = endpoint(&graph, slot.clone());
    let mut old = graph.connect();
    drop(first);
    assert!(matches!(
        old.call("pages", Value::Null),
        Err(Failure::Lost(_))
    ));
    let (_next, _gate) = endpoint(&graph, slot);
    assert!(graph.connect().call("pages", Value::Null).is_ok());
    drop(old);
}

#[test]
#[cfg(unix)]
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
        PageHostSlot::Running(host) => {
            Box::new(host.pause_for_tests(&[PageId::from("pages/a.md")]))
        }
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

#[test]
fn app_handover_takes_the_lock_only_after_serve_stops_and_closes() {
    let graph = Graph::new();
    let serving = lifecycle::Serving::new(&graph);
    let slot = serving.slot.clone();
    let finished = serving.run();
    let claimed = super::client::take_over(
        &graph.root,
        &graph.paths(),
        super::wire::Drafts {
            app_data: graph.app_data.display().to_string(),
            graph_id: "test-graph".into(),
        },
        Duration::from_secs(5),
        |_| {},
        |_| {},
    )
    .unwrap()
    .expect("app took ownership");
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!matches!(
        &*slot.host.read().unwrap(),
        PageHostSlot::Running(_)
    ));
    assert!(OwnerLock::try_take(&graph.paths().lock, "other")
        .unwrap()
        .is_none());
    assert!(dial(&graph.paths().socket, &graph.root).unwrap().is_none());
    drop(claimed);
    assert!(OwnerLock::try_take(&graph.paths().lock, "next")
        .unwrap()
        .is_some());
}

#[test]
#[cfg(unix)]
fn long_namespace_retains_the_owner_lock_without_a_command_endpoint() {
    let graph = Graph::new();
    let data = tempfile::tempdir().unwrap();
    let owners = data.path().join("d".repeat(120));
    let paths = super::namespace::Namespace::at(owners)
        .unwrap()
        .graph(Path::new("/g"))
        .unwrap();
    let lock = OwnerLock::try_take(&paths.lock, "app").unwrap().unwrap();
    let slot = Arc::new(graph.slot());
    let lease = super::lease::Claimed::from_lock(lock, paths.clone()).serve_app(&slot);
    assert!(!lease.has_endpoint());
    assert!(matches!(
        lease.endpoint_error(),
        Some(OwnerError::Unsafe(_))
    ));
    assert!(OwnerLock::try_take(&paths.lock, "serve").unwrap().is_none());
    drop(lease);
    assert!(OwnerLock::try_take(&paths.lock, "serve").unwrap().is_some());
}

/// §1: the namespace is a private directory of this account, and a path
/// the socket cannot bind is refused before anything binds it.
#[test]
#[cfg(unix)]
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
    let paths = namespace.graph(Path::new("/g")).unwrap();
    assert!(super::namespace::check_socket_path(&paths.socket).is_err());
}

/// AGENTS.md §2: a platform `cfg` family names every shipped target. The
/// namespace's arms cover desktop locks and mobile sandbox exclusion;
/// the catch-all means "not a Tine platform".
#[test]
fn the_namespace_names_every_shipped_target() {
    let source = include_str!("namespace.rs");
    assert_eq!(
        source.matches("fn account_dir()").count(),
        5,
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
        ("add", include_str!("add.rs")),
        ("arena", include_str!("arena.rs")),
        ("client", include_str!("client.rs")),
        ("endpoint", include_str!("endpoint.rs")),
        ("git_autocommit", include_str!("git_autocommit.rs")),
        (
            "git_autocommit_tests",
            include_str!("git_autocommit_tests.rs"),
        ),
        ("git_pull", include_str!("git_pull.rs")),
        ("git_run", include_str!("git_run.rs")),
        ("git_sync", include_str!("git_sync.rs")),
        (
            "git_sync_credentials",
            include_str!("git_sync/credentials.rs"),
        ),
        ("git_sync_cycle", include_str!("git_sync_cycle.rs")),
        ("git_sync_http", include_str!("git_sync/http.rs")),
        ("git_sync_mobile", include_str!("git_sync_mobile.rs")),
        ("lease", include_str!("lease.rs")),
        ("lock", include_str!("lock.rs")),
        ("mod", include_str!("mod.rs")),
        ("namespace", include_str!("namespace.rs")),
        ("pages", include_str!("pages.rs")),
        ("serve", include_str!("serve.rs")),
        ("terminal", include_str!("terminal.rs")),
        ("verbs", include_str!("verbs.rs")),
        ("window", include_str!("window.rs")),
        ("wire", include_str!("wire.rs")),
        ("windows", include_str!("windows.rs")),
        ("windows_security", include_str!("windows_security.rs")),
        ("write", include_str!("write.rs")),
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
    assert_eq!(files, 24, "scan every owner/*.rs file");
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
    #[cfg(unix)]
    let listener = std::os::unix::net::UnixListener::bind(socket).unwrap();
    #[cfg(windows)]
    let listener = super::windows::Listener::bind(socket).unwrap();
    std::thread::spawn(move || loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => return,
        };
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
    // Use separate graph endpoints: Windows pipe names stay occupied while
    // the fake owner's accept thread lives, rather than an unlinkable path.
    let second = Graph::new();
    let socket = second.paths().socket;
    fake_owner(&socket, owner_hello(&root, None), None);
    assert!(matches!(
        dial(&socket, &graph.root),
        Err(OwnerError::Protocol(_))
    ));
    let third = Graph::new();
    let socket = third.paths().socket;
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
        let result = super::client::take_over(
            &root,
            &paths,
            drafts,
            Duration::from_secs(2),
            |_| {},
            |_| {},
        );
        let _ = done.send(result.is_err());
    });
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(15)),
        Ok(true),
        "the handover outlived its bound"
    );
}

#[test]
fn timed_out_owner_retries_wait_for_outstanding_io_workers() {
    use std::io::{BufRead, BufReader, Write};
    let graph = Graph::new();
    let paths = graph.paths();
    let held = OwnerLock::try_take(&paths.lock, "serve").unwrap().unwrap();
    #[cfg(unix)]
    let listener = std::os::unix::net::UnixListener::bind(&paths.socket).unwrap();
    #[cfg(windows)]
    let listener = super::windows::Listener::bind(&paths.socket).unwrap();
    let hello = owner_hello(&graph.root.display().to_string(), Some("serve"));
    let (requested, requests) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept: {error}"),
            }
        };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        writeln!(stream, "{hello}").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("owner.yield"));
        requested.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    let attempt = || {
        super::client::take_over(
            &graph.root,
            &paths,
            super::wire::Drafts {
                app_data: "a".into(),
                graph_id: "g".into(),
            },
            Duration::ZERO,
            |_| {},
            |_| {},
        )
    };
    assert!(attempt().is_err());
    requests.recv_timeout(Duration::from_secs(5)).unwrap();
    let blocked = (0..4)
        .all(|_| matches!(attempt(), Err(message) if message.contains("previous owner handover")));
    release.send(()).unwrap();
    server.join().unwrap();
    drop(held);
    assert!(
        blocked,
        "a timed-out handover must retain its worker reservation until I/O finishes"
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

/// One verb against the slot, as the endpoint would answer it.
fn verb(slot: &GraphSlot, name: &str, args: Value) -> Result<Value, ErrorBody> {
    super::verbs::answer_verb(
        slot,
        &Request {
            id: 1,
            verb: name.into(),
            args,
        },
    )
}

fn error_code(result: Result<Value, ErrorBody>) -> (String, Value) {
    let body = result.expect_err("an error");
    (body.code, body.detail.unwrap_or(Value::Null))
}

fn block_handle(page: &Value, index: usize) -> String {
    page["page"]["blocks"][index]["handle"]
        .as_str()
        .expect("a handle")
        .to_string()
}

#[test]
fn block_edit_identity_ignores_a_nonprimary_id_lookalike() {
    let graph = Graph::new();
    let file = graph.root.join("pages/lookalike.org");
    let raw = format!("one\nbody\n:PROPERTIES:\n:id: {UUID}\n:END:");
    let regions = tine_core::block_regions::parse(&raw, true);
    assert!(regions.property("id").is_some());
    assert!(regions.id.is_none());
    std::fs::write(&file, format!("* {raw}\n")).unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name": "lookalike"})).unwrap();
    let supplied = "650e8400-e29b-41d4-a716-446655440000";
    let text = format!(
        "edited\n:PROPERTIES:\n:id: {supplied}\n:END:\nbody\n:PROPERTIES:\n:id: {UUID}\n:END:"
    );
    verb(
        &slot,
        "block.edit",
        json!({"handle": format!("lookalike/{}", block_handle(&read, 0)), "text": text}),
    )
    .unwrap();
    let doc = graph
        .store
        .page(&PageId::from("pages/lookalike.org"))
        .unwrap()
        .doc;
    let regions = tine_core::block_regions::parse(&doc.blocks[0].raw, true);
    assert_eq!(regions.id.unwrap().value, supplied);
    assert!(doc.blocks[0]
        .raw
        .contains(&format!("body\n:PROPERTIES:\n:id: {UUID}\n:END:")));
}

#[test]
fn block_edit_identity_accepts_case_equivalent_uuid_values() {
    const ID: &str = "650e8400-e29b-41d4-a716-446655440000";
    for org in [false, true] {
        for uppercase_current in [false, true] {
            let graph = Graph::new();
            let file = graph.root.join(if org {
                "pages/identity.org"
            } else {
                "pages/identity.md"
            });
            let (current, supplied) = if uppercase_current {
                (ID.to_ascii_uppercase(), ID.to_string())
            } else {
                (ID.to_string(), ID.to_ascii_uppercase())
            };
            let initial = if org {
                format!("* one\n:PROPERTIES:\n:id: {current}\n:END:\n")
            } else {
                format!("- one\n  id:: {current}\n")
            };
            std::fs::write(&file, initial).unwrap();
            graph.store.refresh(tine_store::Depth::Bytes).unwrap();
            let slot = graph.slot();
            let read = verb(&slot, "page.show", json!({"name": "identity"})).unwrap();
            let text = if org {
                format!("edited\n:PROPERTIES:\n:ID: {supplied}  \n:END:")
            } else {
                format!("edited\nID:: {supplied}  ")
            };
            verb(
                &slot,
                "block.edit",
                json!({"handle": format!("identity/{}", block_handle(&read, 0)), "text": text}),
            )
            .unwrap();
            let doc = graph
                .store
                .page(&PageId::from(if org {
                    "pages/identity.org"
                } else {
                    "pages/identity.md"
                }))
                .unwrap()
                .doc;
            assert_eq!(
                tine_core::block_regions::parse(&doc.blocks[0].raw, org)
                    .id
                    .unwrap()
                    .value
                    .trim(),
                supplied
            );
            // The old spelling still resolves to this block by the existing UUID rule.
            verb(
                &slot,
                "block.edit",
                json!({"handle": current, "text": "resolved"}),
            )
            .unwrap();
            assert!(std::fs::read_to_string(&file).unwrap().contains("resolved"));
        }
    }
}

#[test]
fn block_edit_refuses_a_different_identity_without_writing() {
    const OTHER: &str = "650e8400-e29b-41d4-a716-446655440000";
    for org in [false, true] {
        let graph = Graph::new();
        if org {
            std::fs::write(
                graph.root.join("pages/identity.org"),
                format!("* one\n:PROPERTIES:\n:id: {UUID}\n:END:\n"),
            )
            .unwrap();
            graph.store.refresh(tine_store::Depth::Bytes).unwrap();
        }
        let slot = graph.slot();
        let target = if org { "identity" } else { "a" };
        let read = verb(&slot, "page.show", json!({"name": target})).unwrap();
        let handle = format!("{target}/{}", block_handle(&read, 0));
        let file = graph.root.join(if org {
            "pages/identity.org"
        } else {
            "pages/a.md"
        });
        let before = std::fs::read(&file).unwrap();
        let text = if org {
            format!("edited\n:PROPERTIES:\n:ID: {OTHER}\n:END:")
        } else {
            format!("edited\nID:: {OTHER}")
        };
        let error = verb(&slot, "block.edit", json!({"handle": handle, "text": text}))
            .expect_err("identity change must be refused");
        assert_eq!(error.code, "refused");
        assert_eq!(error.detail.as_ref().unwrap()["reason"], "identity-change");
        let error_text = serde_json::to_string(&error).unwrap();
        assert!(
            error_text.contains(UUID) && error_text.contains(OTHER),
            "{error_text}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), before);
        for text in [
            if org {
                format!("same\n:PROPERTIES:\n:id: {UUID}\n:END:")
            } else {
                format!("same\nid:: {UUID}")
            },
            "omitted".to_string(),
            if org {
                format!("literal\n#+BEGIN_SRC\n:id: {OTHER}\n#+END_SRC")
            } else {
                format!("literal\n```\nid:: {OTHER}\n```")
            },
        ] {
            let read = verb(&slot, "page.show", json!({"name": target})).unwrap();
            let handle = format!("{target}/{}", block_handle(&read, 0));
            verb(&slot, "block.edit", json!({"handle": handle, "text": text})).unwrap();
            let doc = graph
                .store
                .page(&PageId::from(if org {
                    "pages/identity.org"
                } else {
                    "pages/a.md"
                }))
                .unwrap()
                .doc;
            assert_eq!(
                tine_core::block_regions::parse(&doc.blocks[0].raw, org)
                    .property("id")
                    .unwrap()
                    .value,
                UUID
            );
        }
    }
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name": "a"})).unwrap();
    verb(&slot, "block.edit", json!({"handle": format!("a/{}", block_handle(&read, 1)), "text": format!("new identity\nid:: {OTHER}")})).unwrap();
    let doc = graph.store.page(&PageId::from("pages/a.md")).unwrap().doc;
    assert_eq!(
        tine_core::block_regions::parse(&doc.blocks[1].raw, false)
            .property("id")
            .unwrap()
            .value,
        OTHER
    );
}

#[test]
fn edit_and_prop_write_through_the_host_and_keep_hidden_properties() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let one = block_handle(&read, 0);
    let two = block_handle(&read, 1);
    assert!(
        one.starts_with("1~") && two.starts_with("2~"),
        "{one} {two}"
    );
    assert!(read["page"]["blocks"][0]["children"][0]["handle"]
        .as_str()
        .unwrap()
        .starts_with("1.1~"));

    let written = verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("a/{one}"), "text": "ONE, edited" }),
    )
    .unwrap();
    assert_eq!(written["kind"], "written");
    let text = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    assert_eq!(
        text,
        format!("- ONE, edited\n  id:: {UUID}\n\t- under one\n- two\n"),
        "the hidden id:: and the child stay; other blocks keep their bytes"
    );
    let new_one = written["blocks"][0].as_str().unwrap().to_string();
    assert_ne!(
        new_one, one,
        "the reply hands back the edited block's new handle"
    );

    // The old handle no longer names a block: stale, not a guess.
    let (stale, _) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("a/{one}"), "text": "again" }),
    ));
    assert_eq!(stale, "stale");

    // By id:: and by the returned handle.
    verb(
        &slot,
        "block.prop",
        json!({ "handle": UUID, "key": "status", "value": "done" }),
    )
    .unwrap();
    verb(
        &slot,
        "block.prop",
        json!({ "handle": format!("a/{two}"), "key": "owner", "value": "agent" }),
    )
    .unwrap();
    let text = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    assert!(text.contains("status:: done"), "{text}");
    assert!(text.contains("- two\n  owner:: agent"), "{text}");
    verb(
        &slot,
        "block.prop",
        json!({ "handle": UUID, "key": "status" }),
    )
    .unwrap();
    let text = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    assert!(!text.contains("status::"), "{text}");
    assert!(text.contains(&format!("id:: {UUID}")), "{text}");
}

#[test]
fn add_places_forests_and_attributes_every_created_block_only() {
    for placement in ["under", "after", "before", "page"] {
        let graph = Graph::new();
        let slot = graph.slot();
        let read = verb(&slot, "page.show", json!({"name": "a"})).unwrap();
        let mut args = json!({"content": "- added\n\t- nested\n- last", "author": "Ada"});
        args[placement] = if placement == "page" {
            json!("a")
        } else {
            json!(format!("a/{}", block_handle(&read, 1)))
        };
        let request_bytes =
            serde_json::to_vec(&json!({"id": 1, "verb": "block.add", "args": args}))
                .unwrap()
                .len()
                + 1;
        let before_bytes = std::fs::metadata(graph.root.join("pages/a.md"))
            .unwrap()
            .len();
        let reply = verb(&slot, "block.add", args).unwrap();
        eprintln!("add {placement}: page {before_bytes} -> {} bytes; one NDJSON request {request_bytes} bytes, reply {} bytes; 3 created blocks", std::fs::metadata(graph.root.join("pages/a.md")).unwrap().len(), serde_json::to_vec(&json!({"id": 1, "ok": reply})).unwrap().len() + 1);
        assert_eq!(reply["kind"], "written");
        assert_eq!(reply["blocks"].as_array().unwrap().len(), 3);
        assert_eq!(reply["more"], 0);
        let doc = graph.store.page(&PageId::from("pages/a.md")).unwrap().doc;
        let added = if placement == "under" {
            &doc.blocks[1].children
        } else {
            &doc.blocks[if placement == "before" { 1 } else { 2 }..]
        };
        assert!(added[0].raw.starts_with("added\n"));
        assert!(added[1].raw.starts_with("last\n"));
        for block in [&added[0], &added[0].children[0], &added[1]] {
            assert_eq!(
                tine_core::block_regions::parse(&block.raw, false)
                    .property("author")
                    .map(|p| p.value.as_str()),
                Some("Ada")
            );
        }
        let anchor = &doc.blocks[if placement == "before" { 3 } else { 1 }];
        let regions = tine_core::block_regions::parse(&anchor.raw, false);
        assert!(regions.property("author").is_none());
        assert_eq!(
            regions.id.is_some(),
            placement != "page",
            "lazy anchor id for {placement}"
        );
        assert_eq!(doc.blocks[0].raw, format!("one\nid:: {UUID}"));
        assert_eq!(doc.blocks[0].children[0].raw, "under one");
        // Returned handles really chain into a subsequent write, with no author added.
        verb(
            &slot,
            "block.edit",
            json!({"handle": reply["blocks"][0], "text": "edited"}),
        )
        .unwrap();
        let bytes = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
        // An ordinary block's author is editor text, as on OG; replacing that
        // buffer removes it. A comment's hidden author is covered separately.
        assert_eq!(bytes.matches("author::").count(), 2);
        assert!(!bytes.contains("author:: agent"));
    }
}

#[test]
fn block_edit_preserves_comment_metadata_but_replaces_ordinary_author_text() {
    for (content, expected) in [
        ("ordinary\nauthor:: Ada", "edited"),
        (
            "comment\nquote:: one\nquote-prefix:: before\nquote-suffix:: after\nauthor:: Ada",
            "edited\nquote:: one\nquote-prefix:: before\nquote-suffix:: after\nauthor:: Ada",
        ),
    ] {
        let graph = Graph::new();
        let slot = graph.slot();
        let added = verb(
            &slot,
            "block.add",
            json!({"page": "a", "content": content, "author": "Ada"}),
        )
        .unwrap();
        verb(
            &slot,
            "block.edit",
            json!({"handle": added["blocks"][0], "text": "edited"}),
        )
        .unwrap();
        let doc = graph.store.page(&PageId::from("pages/a.md")).unwrap().doc;
        assert_eq!(doc.blocks.last().unwrap().raw, expected);
    }
}

#[test]
fn add_creates_page_and_journal_through_create_new_and_bounds_handles() {
    let graph = Graph::new();
    let slot = graph.slot();
    for (target, value, path) in [
        ("page", "new page", "pages/new page.md"),
        ("journal", "2026-10-18", "journals/2026_10_18.md"),
    ] {
        let mut args = json!({"content": "plain first block"});
        args[target] = json!(value);
        let reply = verb(&slot, "block.add", args).unwrap();
        assert_eq!(reply["page"], path);
        let text = std::fs::read_to_string(graph.root.join(path)).unwrap();
        assert!(
            text.contains("- plain first block\n  author:: agent"),
            "{text}"
        );
        // Existing pages append rather than overwrite.
        let mut args = json!({"content": "- second"});
        args[target] = json!(value);
        verb(&slot, "block.add", args).unwrap();
        let text = std::fs::read_to_string(graph.root.join(path)).unwrap();
        assert!(text.contains("plain first block") && text.contains("second"));
    }
    let content = (0..205)
        .map(|n| format!("- block {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let reply = verb(
        &slot,
        "block.add",
        json!({"page": "bounded", "content": content}),
    )
    .unwrap();
    assert_eq!(
        reply["blocks"].as_array().unwrap().len(),
        super::write::REPLY_HANDLES
    );
    assert_eq!(reply["more"], 5);
}

#[test]
fn add_refusals_leave_page_bytes_and_new_targets_absent() {
    let graph = Graph::new();
    let slot = graph.slot();
    let before = std::fs::read(graph.root.join("pages/a.md")).unwrap();
    let read = verb(&slot, "page.show", json!({"name": "a"})).unwrap();
    let anchor = format!("a/{}", block_handle(&read, 1));
    let missing = "00000000-0000-4000-8000-000000000001";
    for target in [
        json!({"under": anchor}),
        json!({"page": "missing"}),
        json!({"journal": "2026-10-19"}),
    ] {
        let mut args = target;
        args["content"] = json!(format!("- broken (({missing}))"));
        let (code, detail) = error_code(verb(&slot, "block.add", args));
        assert_eq!(code, "refused");
        assert_eq!(detail["reason"], "broken-ref");
        assert_eq!(detail["uuid"], missing);
        assert_eq!(detail["admitted"], false);
    }
    for args in [
        json!({"under": anchor, "content": "- x", "rev": "wrong"}),
        json!({"under": "a/99~aaaaaa", "content": "- x"}),
        json!({"under": anchor, "content": ""}),
        json!({"under": anchor, "page": "missing", "content": "- x"}),
        json!({"page": "missing", "content": "- x", "author": "Ada\n- injected"}),
    ] {
        assert!(verb(&slot, "block.add", args).is_err());
    }
    assert_eq!(
        std::fs::read(graph.root.join("pages/a.md")).unwrap(),
        before
    );
    assert!(!graph.root.join("pages/missing.md").exists());
    assert!(!graph.root.join("journals/2026_10_19.md").exists());
    let readonly = "* a\n*** c\n";
    std::fs::write(graph.root.join("pages/readonly.org"), readonly).unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let (code, detail) = error_code(verb(
        &slot,
        "block.add",
        json!({"page": "readonly", "content": "* x"}),
    ));
    assert_eq!(code, "refused");
    assert_eq!(detail["reason"], "read-only");
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/readonly.org")).unwrap(),
        readonly
    );
    verb(
        &slot,
        "block.add",
        json!({"under": anchor, "content": format!("- valid (({UUID}))\n  id:: {UUID}")}),
    )
    .unwrap();
    let text = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    assert_eq!(
        text.matches(&format!("id:: {UUID}")).count(),
        1,
        "live id collision dropped"
    );
}

#[test]
fn add_to_org_uses_page_grammar_and_property_drawers() {
    let graph = Graph::new();
    std::fs::write(graph.root.join("pages/org.org"), "* anchor\n").unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name": "org"})).unwrap();
    let reply = verb(&slot, "block.add", json!({"under": format!("org/{}", block_handle(&read, 0)), "content": "* root\n** child", "author": "Ada"})).unwrap();
    assert_eq!(reply["blocks"].as_array().unwrap().len(), 2);
    let text = std::fs::read_to_string(graph.root.join("pages/org.org")).unwrap();
    assert!(
        text.contains("** root") && text.contains("*** child"),
        "{text}"
    );
    assert_eq!(text.matches(":author: Ada").count(), 2, "{text}");
    assert!(text.contains(":id:"), "{text}");
    let doc = graph
        .store
        .page(&PageId::from("pages/org.org"))
        .unwrap()
        .doc;
    assert!(tine_core::block_regions::parse(&doc.blocks[0].raw, true)
        .property("author")
        .is_none());
}

#[test]
fn add_resolves_author_environment_and_explicit_override() {
    // A child process owns its environment, so parallel owner tests never see
    // a process-global env mutation.
    if std::env::var_os("TINE_ADD_AUTHOR_TEST_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "owner::owner_tests::add_resolves_author_environment_and_explicit_override"
            ])
            .env("TINE_ADD_AUTHOR_TEST_CHILD", "1")
            .env("TINE_AUTHOR", "Environment Author")
            .status()
            .unwrap()
            .success());
        return;
    }
    let graph = Graph::new();
    let slot = graph.slot();
    verb(
        &slot,
        "block.add",
        json!({"page": "env", "content": "- from env"}),
    )
    .unwrap();
    verb(
        &slot,
        "block.add",
        json!({"page": "env", "content": "- explicit", "author": "Ada"}),
    )
    .unwrap();
    let text = std::fs::read_to_string(graph.root.join("pages/env.md")).unwrap();
    assert!(text.contains("author:: Environment Author"), "{text}");
    assert!(text.contains("author:: Ada"), "{text}");
}

#[test]
fn add_depth_refusal_writes_neither_the_anchor_id_nor_the_forest() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({"name": "a"})).unwrap();
    let before = std::fs::read(graph.root.join("pages/a.md")).unwrap();
    let content = (0..128)
        .map(|depth| format!("{}- node {depth}", "\t".repeat(depth)))
        .collect::<Vec<_>>()
        .join("\n");
    let (code, detail) = error_code(verb(
        &slot,
        "block.add",
        json!({"under": format!("a/{}", block_handle(&read, 1)), "content": content}),
    ));
    assert_eq!(code, "refused");
    assert_eq!(detail["reason"], "kernel:depth_cap");
    assert_eq!(
        std::fs::read(graph.root.join("pages/a.md")).unwrap(),
        before
    );
}

#[test]
fn a_write_refuses_what_would_not_read_back_and_a_moved_rev() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let two = block_handle(&read, 1);
    let before = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();

    let (code, detail) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("a/{two}"), "text": "two\n- a new block?" }),
    ));
    assert_eq!(
        (code.as_str(), detail["reason"].as_str()),
        ("refused", Some("does-not-round-trip"))
    );
    assert_eq!(detail["admitted"], false);

    let (code, _) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": "a/2", "text": "x", "rev": "0000000000000000" }),
    ));
    assert_eq!(code, "stale");
    let (code, _) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": "a/2", "text": "x" }),
    ));
    assert_eq!(code, "bad-args", "a bare position needs --rev");
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap(),
        before
    );

    let rev = read["rev"].as_str().unwrap();
    verb(
        &slot,
        "block.edit",
        json!({ "handle": "a/2", "text": "TWO", "rev": rev }),
    )
    .unwrap();
    assert!(std::fs::read_to_string(graph.root.join("pages/a.md"))
        .unwrap()
        .ends_with("- TWO\n"));
}

#[test]
fn identical_blocks_are_stale_with_candidates_never_position_first() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({ "name": "dup" })).unwrap();
    let first = block_handle(&read, 0);
    let (code, detail) = error_code(verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("dup/{first}"), "text": "changed" }),
    ));
    assert_eq!(code, "stale");
    assert_eq!(detail["candidates"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/dup.md")).unwrap(),
        "- same\n- same\n"
    );
}

#[test]
fn move_carries_the_subtree_and_refuses_a_cycle() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let one = block_handle(&read, 0);
    let two = block_handle(&read, 1);
    let moved = verb(
        &slot,
        "block.move",
        json!({ "handle": format!("a/{one}"), "to": format!("a/{two}"), "at": "after" }),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap(),
        format!("- two\n- one\n  id:: {UUID}\n\t- under one\n"),
        "the block moves with its child and keeps its bytes"
    );
    assert!(
        moved["blocks"][0].as_str().unwrap().starts_with("a/2~"),
        "{moved}"
    );

    // Under its own child: the kernel's cycle refusal, nothing written.
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let one = block_handle(&read, 1);
    let child = read["page"]["blocks"][1]["children"][0]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let before = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    let (code, detail) = error_code(verb(
        &slot,
        "block.move",
        json!({ "handle": format!("a/{one}"), "to": format!("a/{child}"), "at": "under" }),
    ));
    assert_eq!(
        (code.as_str(), detail["reason"].as_str()),
        ("refused", Some("kernel:cycle"))
    );
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap(),
        before
    );

    let (code, detail) = error_code(verb(
        &slot,
        "block.move",
        json!({ "handle": format!("a/{one}"), "to": "b/1", "at": "after", "rev": "x" }),
    ));
    assert_eq!(
        (code.as_str(), detail["reason"].as_str()),
        ("refused", Some("cross-page"))
    );
}

#[test]
fn delete_removes_the_subtree_and_counts_only_the_references_it_breaks() {
    let graph = Graph::new();
    let slot = graph.slot();
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let child = read["page"]["blocks"][0]["children"][0]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    // A reference inside the deleted subtree goes with it: not broken.
    verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("a/{child}"), "text": format!("back (({UUID}))") }),
    )
    .unwrap();
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let one = block_handle(&read, 0);
    let deleted = verb(
        &slot,
        "block.delete",
        json!({ "handle": format!("a/{one}") }),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap(),
        "- two\n",
        "the block goes with its child; the rest keeps its bytes"
    );
    assert_eq!(deleted["kind"], "deleted");
    assert_eq!(deleted["deleted"], format!("a/{one}"));
    assert_eq!(
        deleted["broken_refs"], 1,
        "page b's reference breaks; the child's went with it: {deleted}"
    );

    // The page's last block: the page stays, empty, and nothing is broken.
    let read = verb(&slot, "page.show", json!({ "name": "a" })).unwrap();
    let two = block_handle(&read, 0);
    let deleted = verb(
        &slot,
        "block.delete",
        json!({ "handle": format!("a/{two}") }),
    )
    .unwrap();
    assert_eq!(deleted["broken_refs"], 0);
    assert!(graph.root.join("pages/a.md").exists());
}

/// Review 5 B1: an `id::` on two pages names no single block. A write by that
/// id refuses `ambiguous` (case-insensitively) and writes neither page.
#[test]
fn a_write_by_an_id_on_two_pages_is_ambiguous_and_writes_nothing() {
    let graph = Graph::new();
    let twin = format!("- twin\n  id:: {}\n", UUID.to_ascii_uppercase());
    std::fs::write(graph.root.join("pages/c.md"), &twin).unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    let a = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    for (verb_name, args) in [
        ("block.edit", json!({ "handle": UUID, "text": "which?" })),
        ("block.delete", json!({ "handle": UUID })),
    ] {
        let (code, detail) = error_code(verb(&slot, verb_name, args));
        assert_eq!(code, "ambiguous", "{verb_name}: {detail}");
        assert_eq!(detail["admitted"], false);
        assert_eq!(detail["candidates"].as_array().map(Vec::len), Some(2));
    }
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap(),
        a
    );
    assert_eq!(
        std::fs::read_to_string(graph.root.join("pages/c.md")).unwrap(),
        twin
    );
}

/// Review 5 R4: a write's request carries page text past the hello's 1 MiB
/// cap (PROTOCOL §2.1).
#[test]
fn a_write_request_larger_than_one_mebibyte_is_answered() {
    let graph = Graph::new();
    let slot = Arc::new(graph.slot());
    let (_endpoint, _gate) = endpoint(&graph, slot);
    let mut conn = graph.connect();
    let text = "x".repeat(2 << 20);
    let written = conn
        .call("block.edit", json!({ "handle": UUID, "text": text }))
        .unwrap();
    assert_eq!(written["kind"], "written");
    let page = std::fs::read_to_string(graph.root.join("pages/a.md")).unwrap();
    assert!(page.contains(&text), "the 2 MiB text was written");
}

/// Review 5 R5: `block show` prints the handles a page read prints, with
/// page-absolute positions, so a write can name the block it shows.
#[test]
fn block_show_prints_page_absolute_handles_a_write_accepts() {
    let graph = Graph::new();
    let deep = "7d1e0f2a-3b4c-4d5e-8f60-718293a4b5c6";
    std::fs::write(
        graph.root.join("pages/deep.md"),
        format!("- first\n- second\n\t- nested\n\t  id:: {deep}\n\t\t- leaf\n"),
    )
    .unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    let shown = verb(&slot, "block.show", json!({ "uuid": deep })).unwrap();
    let root = &shown["block"]["group"]["blocks"][0];
    let handle = root["handle"].as_str().unwrap();
    assert!(handle.starts_with("2.1~"), "{handle}");
    let leaf = root["children"][0]["handle"].as_str().unwrap();
    assert!(leaf.starts_with("2.1.1~"), "{leaf}");
    verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("deep/{leaf}"), "text": "leaf, edited" }),
    )
    .unwrap();
    let text = std::fs::read_to_string(graph.root.join("pages/deep.md")).unwrap();
    assert!(text.contains("\t\t- leaf, edited\n"), "{text}");
}

/// Review 6: `block show` takes the text it shows and fingerprints from the
/// same host read. With the store behind the file (`U` reads "old" there,
/// while on disk `U` says "new" and another block says "old"), its handle
/// must name `U`, never the other block.
#[test]
fn block_show_fingerprints_the_text_the_host_holds() {
    let graph = Graph::new();
    let u = "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c8d";
    let path = graph.root.join("pages/lag.md");
    std::fs::write(&path, format!("- old\n  id:: {u}\n")).unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    // The file changes; the store's whole-graph view is not refreshed.
    std::fs::write(&path, format!("- new\n  id:: {u}\n- old\n")).unwrap();
    let shown = verb(&slot, "block.show", json!({ "uuid": u })).unwrap();
    let root = &shown["block"]["group"]["blocks"][0];
    assert!(root["raw"].as_str().unwrap().starts_with("new"), "{root}");
    let handle = root["handle"].as_str().unwrap().to_string();
    verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("lag/{handle}"), "text": "U, edited" }),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("- U, edited\n  id:: {u}\n- old\n")
    );
}

/// Review 7: `block show` names the page the index resolved, not the name its
/// bytes now declare. `a.md` takes `title:: b` after indexing while `b.md`
/// holds a block with the same text; a handle shown for `U` must not edit
/// `b.md`.
#[test]
fn block_show_names_the_page_it_read_not_its_new_title() {
    let graph = Graph::new();
    let u = "6b7c8d9e-0f1a-4b2c-8d3e-4f5a6b7c8d9e";
    let a = graph.root.join("pages/a.md");
    let b = graph.root.join("pages/b.md");
    std::fs::write(&a, format!("- new\n  id:: {u}\n")).unwrap();
    std::fs::write(&b, "- new\n").unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    std::fs::write(&a, format!("title:: b\n\n- new\n  id:: {u}\n")).unwrap();
    let shown = verb(&slot, "block.show", json!({ "uuid": u })).unwrap();
    let group = &shown["block"]["group"];
    assert_eq!(group["page"], "a", "{group}");
    let handle = group["blocks"][0]["handle"].as_str().unwrap().to_string();
    verb(
        &slot,
        "block.edit",
        json!({ "handle": format!("a/{handle}"), "text": "U, edited" }),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "- new\n");
    assert!(
        std::fs::read_to_string(&a)
            .unwrap()
            .contains("- U, edited\n"),
        "{}",
        std::fs::read_to_string(&a).unwrap()
    );
}

/// Review 7: the host preview charges each node as the store's projector
/// does. A root whose text fits the byte bound but whose projected properties
/// do not is omitted (`truncated` 1), as the store preview omits it.
#[test]
fn block_show_charges_projected_properties_against_the_byte_bound() {
    let graph = Graph::new();
    let u = "7c8d9e0f-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
    let big = "x".repeat(17 << 20);
    std::fs::write(
        graph.root.join("pages/big.md"),
        format!("- root\n  id:: {u}\n  p:: {big}\n"),
    )
    .unwrap();
    graph.store.refresh(tine_store::Depth::Bytes).unwrap();
    let slot = graph.slot();
    let shown = verb(&slot, "block.show", json!({ "uuid": u })).unwrap();
    let block = &shown["block"];
    assert_eq!(block["truncated"], 1, "{}", block["group"]["page"]);
    assert_eq!(block["group"]["blocks"].as_array().unwrap().len(), 0);
}

/// The zips `tine serve`'s launch backup published for `graph`.
fn launch_zips(graph: &Graph) -> Vec<String> {
    let mut zips = graph
        .app_data
        .join("backups")
        .join(crate::backup::root_backup_id(&graph.root));
    zips.as_mut_os_string().push(".zips");
    let mut names: Vec<String> = std::fs::read_dir(zips)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".zip"))
        .collect();
    names.sort();
    names
}

#[test]
fn serve_takes_the_apps_launch_backup_once_it_has_gone_quiet() {
    // Journal D-20261019-QC item 3: a graph only `tine serve` opens is backed
    // up like one the app opens (one zip, after the warm and a quiet turn).
    let graph = Graph::new();
    let finished = lifecycle::Serving::new(&graph).run();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while launch_zips(&graph).is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "serve took no launch backup"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut yielding = graph.connect();
    assert!(yielding.call("owner.yield", Value::Null).is_ok());
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(launch_zips(&graph).len(), 1);
}

#[test]
fn a_stop_during_the_quiet_wait_ends_serves_backup_before_it_returns() {
    let graph = Graph::new();
    let finished = lifecycle::Serving::new(&graph).run();
    let mut yielding = graph.connect();
    assert!(yielding.call("owner.yield", Value::Null).is_ok());
    // Inside the 5 s quiet turn: returning means the worker was joined.
    finished.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        launch_zips(&graph).is_empty(),
        "a stopped serve published a backup"
    );
}
