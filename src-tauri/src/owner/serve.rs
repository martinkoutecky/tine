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
use tine_store::{OpenOptions, PageHost, Store};

use super::endpoint::{After, Gate, Handler, IO_TIMEOUT};
use super::lease::{self, Claim};
use super::wire::{ok, Drafts, ErrorBody, Request};
use crate::state::{GraphSlot, PageHostSlot};

/// An owner with no request for this long exits (§4).
const IDLE_EXIT: Duration = Duration::from_secs(30 * 60);

/// Run the owner of `graph` until it yields or idles out.
pub(crate) fn run(graph: &Path) -> Result<(), String> {
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
        Claim::Failed(error) => return Err(format!("{}: {error}", error.code())),
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
    let host = PageHost::start(
        &slot.store,
        &app_data,
        &graph_id,
        crate::page_commands::log_edit_kinds,
        // No window: nothing to tell.
        |_mail| {},
    )?;
    *slot.host.write().unwrap() = PageHostSlot::Running(host);
    let gate = Gate::new();
    let (exit, exited) = mpsc::channel();
    let owner = Arc::new(ServeOwner {
        slot: slot.clone(),
        gate: gate.clone(),
        root: root.clone(),
        drafts: Drafts {
            app_data: app_data.display().to_string(),
            graph_id,
        },
    });
    let lease = claimed.serve(
        lease::hello(&root, "serve"),
        gate.clone(),
        owner,
        Some(exit),
    );
    if !lease.has_endpoint() {
        stop_host(&slot, &gate);
        slot.store.close();
        return Err("cannot open the command endpoint".into());
    }
    eprintln!("tine serve: owning {}", root.display());
    loop {
        match exited.recv_timeout(Duration::from_secs(5)) {
            // Yielded: the host has stopped and the reply went out.
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if gate.idle_for().is_some_and(|idle| idle >= IDLE_EXIT) {
                    gate.pause();
                    // A request admitted just before the pause counts as
                    // work: answer it first.
                    if gate.drain(0, IO_TIMEOUT) {
                        stop_host(&slot, &gate);
                        break;
                    }
                    gate.resume();
                }
            }
        }
    }
    // The host stopped; close the Store, then release the endpoint and,
    // last, the lock (§3.2).
    slot.store.close();
    drop(lease);
    eprintln!("tine serve: released {}", root.display());
    Ok(())
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
        std::thread::sleep(pause);
    }
}

pub(super) struct ServeOwner {
    pub(super) slot: Arc<GraphSlot>,
    pub(super) gate: Arc<Gate>,
    pub(super) root: PathBuf,
    pub(super) drafts: Drafts,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct YieldArgs {
    #[serde(default)]
    drafts: Option<Drafts>,
}

impl Handler for ServeOwner {
    fn answer(&self, request: &Request) -> (Result<Value, ErrorBody>, After) {
        if request.verb != "owner.yield" {
            return (super::verbs::answer(&self.slot, request), After::Continue);
        }
        let args = if request.args.is_null() {
            json!({})
        } else {
            request.args.clone()
        };
        let YieldArgs { drafts } = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(e) => {
                return (
                    Err(ErrorBody::new("bad-args", e.to_string(), false)),
                    After::Continue,
                )
            }
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
        self.gate.pause();
        // Every admitted request but this one settles first.
        if !self.gate.drain(1, IO_TIMEOUT) {
            crate::debug::diag("owner-yield-with-requests-in-flight");
        }
        stop_host(&self.slot, &self.gate);
        (Ok(ok("yield", json!({ "yielded": true }))), After::Exit)
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
