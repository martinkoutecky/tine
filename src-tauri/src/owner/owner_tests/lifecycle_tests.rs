//! Lifecycle transition boundaries; this is an out-of-line test module.
use super::super::endpoint::LifecycleBoundary;
use super::*;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Instant;

pub(super) fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "condition never reached");
        std::thread::yield_now();
    }
}

#[test]
fn i_a_stop_and_idle_claims_permanently_end_admission() {
    for idle in [false, true] {
        let gate = Gate::new();
        let admitted = gate.admit().unwrap();
        if idle {
            assert!(!gate.claim_idle_stop(Duration::ZERO));
            drop(admitted);
            assert!(gate.claim_idle_stop(Duration::ZERO));
        } else {
            assert!(gate.claim_stop());
            drop(admitted);
        }
        assert!(gate.admit().is_none());
        gate.resume();
        assert!(gate.admit().is_none(), "adoption reopened a stop");
    }
}

#[test]
fn i_e_idle_and_yield_claim_exactly_one_stop_owner() {
    for _ in 0..32 {
        let gate = Gate::new();
        let (ready, waiting) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let other_gate = gate.clone();
        let idle = std::thread::spawn(move || {
            ready.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            other_gate.claim_idle_stop(Duration::ZERO)
        });
        waiting.recv_timeout(Duration::from_secs(5)).unwrap();
        release.send(()).unwrap();
        let yielding = gate.claim_stop();
        wait_for(|| idle.is_finished());
        assert_eq!(usize::from(idle.join().unwrap()) + usize::from(yielding), 1);
        assert!(!gate.claim_stop());
        assert!(!gate.claim_idle_stop(Duration::ZERO));
    }
}

pub(super) struct Serving {
    pub(super) slot: Arc<GraphSlot>,
    pub(super) gate: Arc<Gate>,
    lease: super::super::lease::Lease,
    stopped: mpsc::Receiver<super::super::endpoint::Stop>,
}

impl Serving {
    pub(super) fn new(graph: &Graph) -> Self {
        let slot = Arc::new(graph.slot());
        let gate = Gate::new();
        let (stops, stopped) = mpsc::channel();
        let owner = Arc::new(super::super::serve::ServeOwner {
            slot: slot.clone(),
            gate: gate.clone(),
            root: graph.root.clone(),
            drafts: super::super::wire::Drafts {
                app_data: graph.app_data.display().to_string(),
                graph_id: "test-graph".into(),
            },
            stops,
        });
        let paths = graph.paths();
        let lock = OwnerLock::try_take(&paths.lock, "serve").unwrap().unwrap();
        let lease =
            Claimed::from_lock(lock, paths).serve(hello(&graph.root, "serve"), gate.clone(), owner);
        Self {
            slot,
            gate,
            lease,
            stopped,
        }
    }

    pub(super) fn run(self) -> mpsc::Receiver<()> {
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            super::super::serve::serve_until_stop(
                &self.slot,
                &self.gate,
                self.lease,
                self.stopped,
                Duration::from_secs(1800),
            );
            done.send(()).unwrap();
        });
        finished
    }
}

#[test]
fn i_b_yield_waits_for_accepted_registration_and_closes_all_peers() {
    let graph = Graph::new();
    let serving = Serving::new(&graph);
    let mut yielding = graph.connect();
    let mut status = graph.connect();
    let mut peers = Vec::new();
    for _ in 0..3 {
        let mut peer = UnixStream::connect(graph.paths().socket).unwrap();
        // Idle after hello, using an unbuffered byte reader so no reply
        // bytes can hide the closure we check below.
        let hello = json!({"tine":"cli", "protocol":1, "version":"x",
            "graph":graph.root.display().to_string()});
        writeln!(peer, "{hello}").unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut byte = [0];
        while peer.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {}
        peers.push(peer);
    }
    let (accepted, resume) = serving.lease.pause_next_accept();
    let gate = serving.gate.clone();
    let (progress, progressed) = mpsc::channel();
    let seal_completed = Mutex::new(false);
    gate.observe_lifecycle(move |boundary| match boundary {
        LifecycleBoundary::SealWaiting => {
            progress.send(boundary).unwrap();
        }
        LifecycleBoundary::Sealed => *seal_completed.lock().unwrap() = true,
        LifecycleBoundary::ReplyReady | LifecycleBoundary::ReplyWrite => assert!(
            *seal_completed.lock().unwrap(),
            "yield reply crossed the accept thread's seal barrier"
        ),
        _ => {}
    });
    let finished = serving.run();
    let mut mid_hello = UnixStream::connect(graph.paths().socket).unwrap();
    mid_hello.write_all(b"{\"tine\":").unwrap();
    accepted.recv_timeout(Duration::from_secs(2)).unwrap();
    let (done, reply) = mpsc::channel();
    std::thread::spawn(move || {
        done.send(yielding.call("owner.yield", Value::Null))
            .unwrap()
    });
    wait_for(|| gate.state() == "stopping");
    assert_eq!(
        status.call("owner.status", Value::Null).unwrap()["state"],
        "stopping",
        "status must remain available until the accept thread seals"
    );
    // Accept is still paused, so its seal ack cannot exist. This event
    // establishes coordinator progress, rather than inferring it from time.
    assert_eq!(
        progressed.recv_timeout(Duration::from_secs(5)).unwrap(),
        LifecycleBoundary::SealWaiting
    );
    let early = reply.try_recv();
    resume.send(()).unwrap();
    assert!(
        matches!(early, Err(mpsc::TryRecvError::Empty)),
        "yield replied before accepted registration completed: {early:?}"
    );
    assert!(reply.recv_timeout(Duration::from_secs(3)).unwrap().is_ok());
    peers.push(mid_hello);
    for mut peer in peers {
        peer.set_nonblocking(true).unwrap();
        assert_eq!(peer.read(&mut [0]).unwrap(), 0, "peer still open at reply");
    }
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn i_c_yielding_transport_is_closed_when_serve_returns_and_unlocks() {
    let graph = Graph::new();
    let serving = Serving::new(&graph);
    let (written, resume) = serving.lease.pause_yielder_close();
    let gate = serving.gate.clone();
    let (progress, progressed) = mpsc::channel();
    let written = Mutex::new(written);
    gate.observe_close_wait(move |closed| {
        // Make even an early acknowledgement visible: the worker has
        // reached its close pause before main probes the real ack channel.
        written
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            matches!(closed.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "main received close ack before yielder shutdown"
        );
        progress.send(LifecycleBoundary::CloseWaiting).unwrap();
    });
    let slot = serving.slot.clone();
    let observed_gate = Arc::downgrade(&gate);
    gate.observe_lifecycle(move |boundary| match boundary {
        LifecycleBoundary::ReplyWrite => assert!(
            !matches!(&*slot.host.read().unwrap(), PageHostSlot::Running(_)),
            "host still running at yield reply-write boundary"
        ),
        LifecycleBoundary::BeforeLeaseRelease => assert_eq!(
            observed_gate.upgrade().unwrap().in_flight(),
            0,
            "main reached lease release before yielder closure"
        ),
        _ => {}
    });
    let finished = serving.run();
    let mut peer = UnixStream::connect(graph.paths().socket).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(
        peer,
        "{}",
        json!({"tine":"cli", "protocol":1, "version":"x", "graph":graph.root.display().to_string()})
    )
    .unwrap();
    let read_line = |peer: &mut UnixStream| {
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            assert_eq!(peer.read(&mut byte).unwrap(), 1);
            line.push(byte[0]);
            if byte[0] == b'\n' {
                return line;
            }
        }
    };
    read_line(&mut peer);
    writeln!(peer, "{}", json!({"id":1, "verb":"owner.yield", "args":{}})).unwrap();
    let reply: Value = serde_json::from_slice(&read_line(&mut peer)).unwrap();
    assert_eq!(reply["ok"]["yielded"], true);
    assert_eq!(
        progressed.recv_timeout(Duration::from_secs(5)).unwrap(),
        LifecycleBoundary::CloseWaiting
    );
    assert_eq!(
        gate.in_flight(),
        1,
        "yielder released admission before closure"
    );
    assert!(
        OwnerLock::try_take(&graph.paths().lock, "early")
            .unwrap()
            .is_none(),
        "main released the lease before the worker closed"
    );
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    resume.send(()).unwrap();
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
    let _next = OwnerLock::try_take(&graph.paths().lock, "next")
        .unwrap()
        .expect("serve retained its lease");
    peer.set_nonblocking(true).unwrap();
    assert_eq!(
        peer.read(&mut [0]).unwrap(),
        0,
        "yielder open after lease release"
    );
}

#[test]
fn i_d_idle_endpoint_drop_never_allocates_a_wake_socket() {
    // CODE-3 RN2: EMFILE must not strand accept/join. This guard is paired
    // with the real idle-drop deadline; a wake connect violates the rule
    // even if it succeeds on a developer's machine.
    assert!(
        !include_str!("../endpoint.rs").contains("UnixStream::connect"),
        "CODE-3 RN2: endpoint shutdown must not allocate a wake socket"
    );
    let graph = Graph::new();
    let (endpoint, _gate) = endpoint(&graph, Arc::new(graph.slot()));
    let (done, joined) = mpsc::channel();
    let start = Instant::now();
    std::thread::spawn(move || {
        drop(endpoint);
        done.send(()).unwrap();
    });
    joined
        .recv_timeout(Duration::from_secs(1))
        .expect("idle accept join hung");
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn disconnected_yielder_does_not_strand_coordinator() {
    use super::super::endpoint::{Stop, YieldHandle};
    let graph = Graph::new();
    let mut serving = Serving::new(&graph);
    let slot = serving.slot.clone();
    let (stop, stopped) = mpsc::channel();
    serving.stopped = stopped;
    let (verdict, reply) = mpsc::channel();
    let (closed, ack) = mpsc::channel();
    assert!(serving.gate.claim_stop());
    // Model a worker dying after submitting its handle: its close-ack
    // sender disappears without ever acknowledging closure.
    drop(closed);
    stop.send(Stop {
        yielder: Some(YieldHandle {
            id: 0,
            verdict,
            closed: ack,
        }),
    })
    .unwrap();
    let finished = serving.run();
    assert!(reply.recv_timeout(Duration::from_secs(5)).unwrap().is_ok());
    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("dead yielder stranded main");
    assert!(!matches!(
        &*slot.host.read().unwrap(),
        PageHostSlot::Running(_)
    ));
    let _next = OwnerLock::try_take(&graph.paths().lock, "next")
        .unwrap()
        .expect("dead yielder retained the lease");
}
