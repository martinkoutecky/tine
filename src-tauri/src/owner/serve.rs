//! `tine serve <graph>` (CLI PROTOCOL §3–4): the graph's headless owner
//! while no app has it open. It takes the owner lock, opens the Store and
//! starts the page host exactly as the app does (same app-data home, launch
//! checkpoint, watch mode and draft namespace), and answers the command line
//! until an app asks it to yield, `tine serve --stop`, or 30 idle minutes.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tine_store::{OpenOptions, Store};

use super::endpoint::{After, Gate, Handler, Stop, YieldHandle, YieldRequest};
use super::lease::{self, Claim};
use super::wire::{ok, Drafts, ErrorBody, Request};
use super::OwnerError;
use crate::state::{GraphSlot, PageHostSlot};

/// An owner with no request for this long exits (§4).
const IDLE_EXIT: Duration = Duration::from_secs(30 * 60);
/// The start log's bound while serve runs (§3.4).
const START_LOG_CAP: u64 = 1 << 20;

/// Keep this run's start log (the detached serve's stderr, opened for
/// append by `client::spawn_serve`) under `START_LOG_CAP` by emptying it when
/// it grows past: a long draft-failure retry cannot fill the disk, and serve
/// keeps custody (review CODE-1 F9). A terminal or pipe is left alone.
fn bound_start_log() {
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        let Ok(owned) = std::io::stderr().as_fd().try_clone_to_owned() else {
            return;
        };
        bound_log(&std::fs::File::from(owned), START_LOG_CAP);
    }
}

/// Empty `log` if it is a regular file longer than `cap`.
pub(super) fn bound_log(log: &std::fs::File, cap: u64) {
    if log
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > cap)
    {
        let _ = log.set_len(0);
    }
}

/// Run the owner of `graph` until it yields or idles out.
/// Why `tine serve` stopped with an error: a failed owner claim keeps its
/// typed code (it exits like the other owner-discovery failures, PROTOCOL
/// §2.3, §3.5); anything else is internal.
#[derive(Debug)]
pub(crate) enum ServeFailure {
    Claim(OwnerError),
    Other(String),
}

impl From<String> for ServeFailure {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for ServeFailure {
    fn from(message: &str) -> Self {
        Self::Other(message.to_owned())
    }
}

impl std::fmt::Display for ServeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claim(error) => write!(f, "{}: {error}", error.code()),
            Self::Other(message) => f.write_str(message),
        }
    }
}

pub(crate) fn serve_graph(graph: &Path) -> Result<(), ServeFailure> {
    // Before anything reads settings, as the app's startup does (§3.5).
    crate::data_home::ensure_usable(crate::app_identity::APP_IDENTIFIER);
    let root = crate::state::canonical_graph_root(&graph.display().to_string())?;
    let claimed = match lease::claim(&root, "serve") {
        Claim::Taken(claimed) => claimed,
        // Lost the race: the client's retry reaches the winner (§3.4).
        Claim::Held(_) => {
            eprintln!("tine serve: {} already has an owner", root.display());
            return Ok(());
        }
        Claim::Failed(error) => return Err(ServeFailure::Claim(error)),
    };
    let app_data = crate::app_identity::current_app_data_dir()
        .ok_or("no app data directory for crash drafts")?;
    let (store, _, _) = Store::open(
        &root,
        OpenOptions {
            approved_external_assets: crate::settings::approved_external_assets_in(
                &app_data, &root,
            ),
            watch: crate::watcher::watch_mode_in(&app_data),
            launch_checkpoint: Some(crate::graph::launch_checkpoint_path(&app_data, &root)),
        },
    )
    .map_err(|error| crate::graph::open_error_text(error, true))?;
    let slot = Arc::new(GraphSlot::new(store, root.clone()));
    let graph_id = crate::settings::host_graph_id(&root);
    // No window: no page mail to deliver.
    let host = crate::graph::launch_host(&slot.store, &app_data, &root, |_mail| {})?;
    *slot.host.write().unwrap() = PageHostSlot::Running(host);
    let gate = Gate::new();
    let (stops, stopped) = mpsc::channel();
    let owner = Arc::new(ServeOwner {
        slot: slot.clone(),
        gate: gate.clone(),
        root: root.clone(),
        drafts: Drafts {
            app_data: app_data.display().to_string(),
            graph_id,
        },
        stops,
    });
    let lease = claimed.serve(lease::hello(&root, "serve"), gate.clone(), owner);
    if !lease.has_endpoint() {
        stop_host(&slot, &gate);
        slot.store.close();
        return Err("cannot open the command endpoint".into());
    }
    eprintln!("tine serve: owning {}", root.display());
    serve_until_stop(&slot, &gate, lease, stopped, IDLE_EXIT);
    eprintln!("tine serve: released {}", root.display());
    Ok(())
}

pub(super) fn serve_until_stop(
    slot: &GraphSlot,
    gate: &Gate,
    lease: lease::Lease,
    stopped: mpsc::Receiver<Stop>,
    idle: Duration,
) {
    let yielder = loop {
        match stopped.recv_timeout(Duration::from_secs(5)) {
            Ok(stop) => break stop.yielder,
            Err(RecvTimeoutError::Disconnected) => {
                if gate.claim_stop() {
                    break None;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                bound_start_log();
                // Claimed only with nothing admitted, in one step: no
                // request runs past this, and a yield's stop is never
                // reopened (review CODE-2 #2).
                if gate.claim_idle_stop(idle) {
                    break None;
                }
            }
        }
    };
    stop_sequence(slot, gate, lease, yielder);
}

fn stop_sequence(slot: &GraphSlot, gate: &Gate, lease: lease::Lease, yielder: Option<YieldHandle>) {
    gate.set_progress("finishing requests");
    gate.settle(usize::from(yielder.is_some()));
    stop_host(slot, gate);
    lease.seal(yielder.as_ref().map(|yielder| yielder.id));
    if let Some(yielder) = yielder {
        #[cfg(test)]
        gate.lifecycle_boundary(super::endpoint::LifecycleBoundary::ReplyReady);
        let _ = yielder
            .verdict
            .send(Ok(ok("yield", json!({ "yielded": true }))));
        // The worker's bounded write, explicit shutdown, registry removal
        // and admission release precede this acknowledgement.
        #[cfg(test)]
        gate.yielder_close_wait(&yielder.closed);
        let _ = yielder.closed.recv();
    }
    #[cfg(test)]
    gate.lifecycle_boundary(super::endpoint::LifecycleBoundary::BeforeLeaseRelease);
    slot.store.close();
    drop(lease);
}

/// Stop the host as retirement does, retrying while a draft cannot be
/// written; the stop's progress is what `owner.status` reports. No bound:
/// a stuck stop keeps this process the owner (§4.5).
fn stop_host(slot: &GraphSlot, gate: &Gate) {
    gate.set_progress("stopping");
    let mut backoff = Duration::from_secs(1);
    loop {
        let pause = {
            let mut host = slot.host.write().unwrap_or_else(|e| e.into_inner());
            crate::host_retirement::stop_pass(&mut host, &mut backoff)
        };
        let Some(pause) = pause else {
            return;
        };
        if pause >= Duration::from_secs(1) {
            gate.set_progress("retrying: a crash draft could not be written");
        }
        bound_start_log();
        std::thread::sleep(pause);
    }
}

pub(super) struct ServeOwner {
    pub(super) slot: Arc<GraphSlot>,
    pub(super) gate: Arc<Gate>,
    pub(super) root: PathBuf,
    pub(super) drafts: Drafts,
    pub(super) stops: mpsc::Sender<Stop>,
}

#[derive(serde::Deserialize)]
struct YieldArgs {
    #[serde(default)]
    drafts: Option<Drafts>,
}

impl Handler for ServeOwner {
    fn answer_request(
        &self,
        request: &Request,
        yielder: &mut YieldRequest,
    ) -> (Result<Value, ErrorBody>, After) {
        if request.verb != "owner.yield" {
            return (
                super::verbs::answer_verb(&self.slot, request),
                After::Continue,
            );
        }
        let YieldArgs { drafts } = match super::wire::read_object(request.args.clone()) {
            Ok(args) => args,
            Err(e) => return (Err(ErrorBody::new("bad-args", e, false)), After::Continue),
        };
        // Custody first: never yield into a namespace that cannot recover
        // this host's drafts (§3.5).
        if drafts.as_ref().is_some_and(|drafts| *drafts != self.drafts) {
            let mut body = ErrorBody::new(
                "refused",
                "the opening app keeps its crash drafts elsewhere",
                false,
            );
            body.detail = Some(json!({ "admitted": false, "reason": "draft-namespace" }));
            return (Err(body), After::Continue);
        }
        // One stop at a time: a second yield (or one racing the idle exit)
        // answers busy and ends its own admission, so the stop it would
        // wait for can settle (review CODE-2 #1).
        if !self.gate.claim_stop() {
            return (Err(super::endpoint::busy()), After::Continue);
        }
        (yielder.stop(&self.stops), After::Close)
    }

    fn status(&self, gate: &Gate) -> Value {
        ok(
            "status",
            json!({
                "owner": "serve",
                "graph": self.root.display().to_string(),
                "pid": std::process::id(),
                "version": env!("CARGO_PKG_VERSION"),
                "state": gate.state(),
            }),
        )
    }
}
