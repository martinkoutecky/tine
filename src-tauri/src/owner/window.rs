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

use crate::watcher::GraphChange;

/// How long a write waits for the window to save the typed text. The barrier
/// sends the input and waits for its publication (one guarded save); a
/// window that does not answer in this time refuses the write, never guesses.
const FLUSH_WAIT: Duration = Duration::from_secs(5);

/// Request ids are unique in the process, not per link: a late answer from a
/// window's previous binding can never match a newer link's request.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

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
    waiting: Mutex<HashMap<u64, mpsc::SyncSender<bool>>>,
}

impl WindowLink {
    pub(crate) fn new(app: tauri::AppHandle, label: String, binding_generation: u64) -> Self {
        Self {
            app,
            label,
            binding_generation,
            waiting: Mutex::new(HashMap::new()),
        }
    }

    /// Ask the window to save the text typed on page `name`, and wait for
    /// its answer. Called on the owner's request thread, never the main one.
    pub(crate) fn flush_page(&self, name: &str) -> Flushed {
        self.request_flush(name, false)
    }

    pub(crate) fn flush_all(&self) -> Flushed {
        self.request_flush("", true)
    }

    fn request_flush(&self, name: &str, all: bool) -> Flushed {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::sync_channel(1);
        self.waiting.lock().unwrap().insert(id, answer);
        let sent = self.app.emit_to(
            self.label.as_str(),
            "owner-flush-request",
            json!({ "id": id, "page": name, "all": all, "binding_generation": self.binding_generation }),
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

    /// The window's answer to request `id` (`owner_flush_answer`), made under
    /// `binding_generation`; an answer from another binding is ignored.
    pub(crate) fn answer_flush(&self, id: u64, saved: bool, binding_generation: u64) {
        if binding_generation != self.binding_generation {
            return;
        }
        if let Some(answer) = self.waiting.lock().unwrap().remove(&id) {
            let _ = answer.try_send(saved);
        }
    }

    /// Tell the window that these pages changed, as the watcher tells it of
    /// outside edits (`watcher::page_event_payloads`: one `graph-changed` per
    /// page, one `graph-changed-bulk` for a checkout-sized batch).
    pub(crate) fn pages_changed(&self, pages: Vec<GraphChange>) {
        for (event, payload) in
            crate::watcher::page_event_payloads(pages, self.binding_generation, None)
        {
            let _ = self.app.emit_to(self.label.as_str(), event, payload);
        }
    }

    pub(crate) fn changed(&self, path: &str, name: &str, kind: PageKind, created: bool) {
        self.pages_changed(vec![GraphChange {
            path: path.into(),
            name: name.into(),
            kind,
            created,
            removed: false,
        }]);
    }

    pub(crate) fn removed(&self, path: &str, name: &str, kind: PageKind) {
        self.pages_changed(vec![GraphChange {
            path: path.into(),
            name: name.into(),
            kind,
            created: false,
            removed: true,
        }]);
    }

    pub(crate) fn renamed(
        &self,
        old: &str,
        new: &str,
        path: Option<&str>,
        kind: PageKind,
        report: &tine_graph_features::pages::RenameReport,
    ) {
        let _ = self.app.emit_to(
            self.label.as_str(),
            "owner-page-renamed",
            json!({
                "old":old,"name":new,"path":path,"kind":kind,"report":report,
                "binding_generation":self.binding_generation,
            }),
        );
    }
}
