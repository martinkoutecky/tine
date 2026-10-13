//! The owner lease (CLI PROTOCOL §3.1): the owner lock and the endpoint,
//! created with a page host and kept with it. In the app it lives on the
//! `GraphSlot` next to `host`: retirement keeps it while the host stops,
//! `HostRetirement::adopt` moves it with the host (an adoption never
//! reacquires, §3.2), and retirement releases it once the host has stopped.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, Weak};

use serde_json::{json, Value};

use super::endpoint::{busy, After, Endpoint, Gate, Handler, YieldRequest};
use super::lock::OwnerLock;
use super::namespace::{GraphPaths, Namespace};
use super::wire::{ok, Drafts, ErrorBody, Hello, Request, PROTOCOL};
use super::OwnerError;
use crate::state::GraphSlot;

/// A held lease. Field order is drop order (§3.2): the endpoint closes
/// (admitted requests settle, connections close, the socket path goes)
/// while the lock is still held; then the lock is released.
pub(crate) struct Lease {
    endpoint: Option<Endpoint>,
    gate: Arc<Gate>,
    route: Option<Arc<Route>>,
    _lock: OwnerLock,
}

/// The graph's lock, taken before its host starts.
pub(crate) struct Claimed {
    lock: OwnerLock,
    pub(crate) paths: GraphPaths,
}

pub(crate) enum Claim {
    Taken(Claimed),
    /// Another process owns the graph.
    Held(GraphPaths),
    /// No arbitration: the namespace or the lock call failed (§3.5).
    Failed(OwnerError),
}

/// Take `root`'s owner lock as `owner` ("app" or "serve").
pub(crate) fn claim(root: &Path, owner: &str) -> Claim {
    let paths = match Namespace::resolve().and_then(|ns| ns.graph(root)) {
        Ok(paths) => paths,
        Err(error) => return Claim::Failed(error),
    };
    match OwnerLock::try_take(&paths.lock, owner) {
        Ok(Some(lock)) => Claim::Taken(Claimed { lock, paths }),
        Ok(None) => Claim::Held(paths),
        Err(error) => Claim::Failed(error),
    }
}

/// The handshake an owner of `root` answers with.
pub(crate) fn hello(root: &Path, owner: &str) -> Hello {
    Hello {
        tine: "owner".into(),
        protocol: PROTOCOL,
        version: env!("CARGO_PKG_VERSION").into(),
        graph: root.display().to_string(),
        owner: Some(owner.into()),
    }
}

impl Claimed {
    /// Open the endpoint, after the host started. A bind failure keeps the
    /// lock (still the owner, so no second host) and opens no endpoint:
    /// no endpoint, no serving (§3.5).
    pub(crate) fn serve(self, hello: Hello, gate: Arc<Gate>, handler: Arc<dyn Handler>) -> Lease {
        let endpoint = match Endpoint::bind(&self.paths.socket, hello, gate.clone(), handler) {
            Ok(endpoint) => Some(endpoint),
            Err(error) => {
                crate::debug::diag_private("owner-endpoint-unavailable", &error.to_string());
                None
            }
        };
        Lease {
            endpoint,
            gate,
            route: None,
            _lock: self.lock,
        }
    }

    /// The app's lease for `slot`, whose host just started.
    pub(crate) fn serve_app(self, slot: &Arc<GraphSlot>) -> Lease {
        let route = Arc::new(Route(RwLock::new(Arc::downgrade(slot))));
        let handler = Arc::new(AppOwner {
            route: route.clone(),
            root: slot.root_key.clone(),
        });
        let mut lease = self.serve(hello(&slot.root_key, "app"), Gate::new(), handler);
        lease.route = Some(route);
        lease
    }
}

impl Lease {
    #[cfg(all(test, unix))]
    pub(crate) fn pause_yielder_close(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        self.endpoint.as_ref().unwrap().pause_yielder_close()
    }
    #[cfg(all(test, unix))]
    pub(crate) fn pause_next_accept(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        self.endpoint.as_ref().unwrap().pause_next_accept()
    }
    pub(crate) fn gate(&self) -> &Arc<Gate> {
        &self.gate
    }

    pub(crate) fn has_endpoint(&self) -> bool {
        self.endpoint.is_some()
    }

    pub(crate) fn seal(&self, except: Option<u64>) {
        if let Some(endpoint) = &self.endpoint {
            endpoint.seal(except);
        }
    }

    /// Requests now go to `slot` (an adoption moved the host there), and
    /// are admitted again after the retirement paused them.
    pub(crate) fn adopted_by(&self, slot: &Arc<GraphSlot>) {
        if let Some(route) = &self.route {
            *route.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(slot);
        }
        self.gate.resume();
    }
}

/// Which slot an app lease's requests reach: the binding holding the host
/// now. Weak, because the slot holds the lease.
struct Route(RwLock<Weak<GraphSlot>>);

/// The app as owner. It never yields: a window is open on the graph.
struct AppOwner {
    route: Arc<Route>,
    root: PathBuf,
}

impl Handler for AppOwner {
    fn answer_request(
        &self,
        request: &Request,
        _yielder: &mut YieldRequest,
    ) -> (Result<Value, ErrorBody>, After) {
        if request.verb == "owner.yield" {
            let mut body = ErrorBody::new(
                "refused",
                "the graph is open in the Tine app, which stays its owner",
                false,
            );
            body.detail = Some(json!({ "admitted": false, "reason": "app-owner" }));
            return (Err(body), After::Continue);
        }
        let slot = self
            .route
            .0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        let Some(slot) = slot else {
            return (Err(busy()), After::Continue);
        };
        (super::verbs::answer_verb(&slot, request), After::Continue)
    }

    fn status(&self, gate: &Gate) -> Value {
        ok(
            "status",
            json!({
                "owner": "app",
                "graph": self.root.display().to_string(),
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "state": gate.state(),
            }),
        )
    }
}

/// The app's claim at a fresh open (§3.5, §4): `Some` when it holds the
/// lock, `None` to open without an endpoint (no arbitration available).
/// When `tine serve` owns the graph, the app asks it to yield and waits up
/// to `bound` for the lock, reporting serve's progress through `waiting`;
/// past the bound the open fails, and serve stays the owner (a timeout
/// never authorises a takeover).
#[cfg(desktop)]
pub(crate) fn claim_for_app(
    root: &Path,
    drafts: Drafts,
    bound: std::time::Duration,
    waiting: impl FnMut(&str),
) -> Result<Option<Claimed>, String> {
    match claim(root, "app") {
        Claim::Taken(claimed) => Ok(Some(claimed)),
        Claim::Failed(OwnerError::Unsupported) => Ok(None),
        Claim::Failed(error) => {
            crate::debug::diag_private("owner-lock-unavailable", &error.to_string());
            Ok(None)
        }
        Claim::Held(paths) => super::client::take_over(root, &paths, drafts, bound, waiting),
    }
}

/// Mobile has no command line, so no owner arbitration.
#[cfg(not(desktop))]
pub(crate) fn claim_for_app(
    _root: &Path,
    _drafts: Drafts,
    _bound: std::time::Duration,
    _waiting: impl FnMut(&str),
) -> Result<Option<Claimed>, String> {
    Ok(None)
}

impl Claimed {
    /// Wrap a lock the handover took (`client::take_over`).
    #[cfg(desktop)]
    pub(crate) fn from_lock(lock: OwnerLock, paths: GraphPaths) -> Self {
        Self { lock, paths }
    }
}
