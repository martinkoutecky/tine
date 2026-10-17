//! The owner's link to the window bound to its graph (CLI slice 2, design §1).
//!
//! A command-line write is not started by the window, so the window's own
//! barriers never run for it. Before the write reads the page, the owner asks
//! the window to submit and save the text typed on that page (its `flushPage`
//! barrier), and waits for the answer; the host's `flush_input` only sees
//! text the window has already submitted (it debounces typing). After the
//! write, the owner sends the window the `graph-changed` event a watcher
//! would send for an outside edit: a window that only displays the page (not
//! held by the host, so it gets no page mail) reloads it. Unset under `tine
//! serve`, which has no window.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use serde_json::json;
use tauri::Emitter;
use tine_core::model::PageKind;

/// How long a write waits for the window to save the typed text. The barrier
/// sends the input and waits for its publication (one guarded save); a
/// window that does not answer in this time refuses the write, never guesses.
const FLUSH_WAIT: Duration = Duration::from_secs(5);

/// What the window answered to a flush request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Flushed {
    /// The typed text on that page is saved (or there was none).
    Saved,
    /// The window could not save it (a conflict, a save error).
    Unsaved,
    /// No answer within [`FLUSH_WAIT`], or the window is gone.
    NoAnswer,
}

pub(crate) struct WindowLink {
    app: tauri::AppHandle,
    label: String,
    binding_generation: u64,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::SyncSender<bool>>>,
}

impl WindowLink {
    pub(crate) fn new(app: tauri::AppHandle, label: String, binding_generation: u64) -> Self {
        Self {
            app,
            label,
            binding_generation,
            next: AtomicU64::new(1),
            waiting: Mutex::new(HashMap::new()),
        }
    }

    /// Ask the window to save the text typed on page `name`, and wait for
    /// its answer. Called on the owner's request thread, never the main one.
    pub(crate) fn flush(&self, name: &str) -> Flushed {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::sync_channel(1);
        self.waiting.lock().unwrap().insert(id, answer);
        let sent = self.app.emit_to(
            self.label.as_str(),
            "owner-flush-request",
            json!({ "id": id, "page": name, "binding_generation": self.binding_generation }),
        );
        let outcome = match sent {
            Err(_) => Flushed::NoAnswer,
            Ok(()) => match answered.recv_timeout(FLUSH_WAIT) {
                Ok(true) => Flushed::Saved,
                Ok(false) => Flushed::Unsaved,
                Err(_) => Flushed::NoAnswer,
            },
        };
        self.waiting.lock().unwrap().remove(&id);
        outcome
    }

    /// The window's answer to request `id` (`owner_flush_answer`).
    pub(crate) fn answer(&self, id: u64, saved: bool) {
        if let Some(answer) = self.waiting.lock().unwrap().remove(&id) {
            let _ = answer.try_send(saved);
        }
    }

    /// Tell the window that `path` changed, as the watcher tells it of an
    /// outside edit (`watcher::page_event_payloads`' per-page shape).
    pub(crate) fn changed(&self, path: &str, name: &str, kind: PageKind, created: bool) {
        let _ = self.app.emit_to(
            self.label.as_str(),
            "graph-changed",
            json!({
                "path": path,
                "name": name,
                "kind": kind,
                "created": created,
                "removed": false,
                "binding_generation": self.binding_generation,
                "answers": null,
            }),
        );
    }
}
