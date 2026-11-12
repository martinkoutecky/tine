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
//! `page.current` also uses this link: the frontend answers from its existing
//! focused-window/pane route; the owner keeps only the pending question.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Emitter;
use tine_core::model::PageKind;

use crate::watcher::GraphChange;
use tine_store::GraphRev;

type PageDelivery = (Vec<GraphChange>, Option<serde_json::Value>);

/// Join reload metadata to the existing ordered publication feed, whichever
/// arrives first. Coalesce by path and bound a stalled dispatcher's backlog.
/// Overflow uses the existing conservative notification; it never drops a reload.
#[derive(Default)]
struct ReloadPublications {
    latest: Option<(GraphRev, serde_json::Value)>,
    pending: BTreeMap<String, (GraphRev, GraphChange)>,
}

impl ReloadPublications {
    fn reload(
        &mut self,
        pages: Vec<GraphChange>,
        revision: Option<GraphRev>,
    ) -> Option<PageDelivery> {
        let Some(revision) = revision else {
            return Some((pages, None));
        };
        if let Some((published, answers)) = &self.latest {
            if *published >= revision {
                return Some((pages, Some(answers.clone())));
            }
        }
        if pages.len() + self.pending.len() > 32 {
            let mut all: Vec<_> = std::mem::take(&mut self.pending)
                .into_values()
                .map(|(_, page)| page)
                .collect();
            all.extend(pages);
            return Some((all, None));
        }
        for page in pages {
            self.pending.insert(page.path.clone(), (revision, page));
        }
        None
    }

    fn published(
        &mut self,
        revision: GraphRev,
        answers: serde_json::Value,
    ) -> Option<PageDelivery> {
        self.latest = Some((revision, answers.clone()));
        let mut ready = Vec::new();
        self.pending.retain(|_, (expected, page)| {
            if *expected <= revision {
                ready.push(page.clone());
                false
            } else {
                true
            }
        });
        (!ready.is_empty()).then_some((ready, Some(answers)))
    }
}

#[cfg(test)]
mod reload_publication_tests {
    use super::*;

    fn rev(n: u64) -> GraphRev {
        n.to_string().try_into().unwrap()
    }
    fn page(n: usize) -> GraphChange {
        GraphChange {
            path: format!("pages/Writer {n}.md"),
            name: format!("Writer {n}"),
            kind: PageKind::Page,
            created: false,
            removed: false,
        }
    }

    #[test]
    fn cli_reload_and_publication_join_in_either_arrival_order() {
        let answers = json!({"rev":"1","summary":{"full":false,"queryTo":[]}});
        let mut state = ReloadPublications::default();
        assert!(state.reload(vec![page(0)], Some(rev(1))).is_none());
        assert_eq!(
            state.published(rev(1), answers.clone()),
            Some((vec![page(0)], Some(answers.clone())))
        );
        assert_eq!(
            state.reload(vec![page(0)], Some(rev(1))),
            Some((vec![page(0)], Some(answers)))
        );
        assert!(state.pending.is_empty());
    }

    #[test]
    fn a_later_publication_carries_queued_reload_without_reusing_older_evidence() {
        let mut state = ReloadPublications::default();
        assert!(state.published(rev(1), json!({"rev":"1"})).is_none());
        assert!(state.reload(vec![page(0)], Some(rev(2))).is_none());
        assert!(state.published(rev(1), json!({"rev":"1"})).is_none());
        let latest = json!({"rev":"3","summary":{"full":true}});
        assert_eq!(
            state.published(rev(3), latest.clone()),
            Some((vec![page(0)], Some(latest)))
        );
    }

    #[test]
    fn a_stalled_dispatcher_coalesces_paths_and_has_a_bounded_fallback() {
        let mut state = ReloadPublications::default();
        for i in 1..100 {
            assert!(state.reload(vec![page(0)], Some(rev(i))).is_none());
        }
        assert_eq!(state.pending.len(), 1);
        let (pages, answers) = state
            .reload((1..33).map(page).collect(), Some(rev(100)))
            .unwrap();
        assert_eq!(pages.len(), 33);
        assert!(answers.is_none());
        assert!(state.pending.is_empty());
        assert_eq!(
            state.reload(vec![page(0)], None),
            Some((vec![page(0)], None))
        );
    }
}

/// How long a write waits for the window to save the typed text. The barrier
/// sends the input and waits for its publication (one guarded save); a
/// window that does not answer in this time refuses the write, never guesses.
const FLUSH_WAIT: Duration = Duration::from_secs(5);

/// Request ids are unique in the process, not per link: a late answer from a
/// window's previous binding can never match a newer link's request.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// The frontend's focused route, read once without consulting the Store.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum CurrentPage {
    CurrentPage {
        name: String,
        url: String,
        block_url: Option<String>,
    },
    NoCurrentPage {
        reason: NoCurrentPage,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NoCurrentPage {
    NoOwnerEndpoint,
    NoFocusedWindow,
    NotPage,
    Journals,
}

impl CurrentPage {
    pub(crate) fn absent() -> Self {
        Self::NoCurrentPage {
            reason: NoCurrentPage::NoFocusedWindow,
        }
    }

    pub(crate) fn payload(self) -> Value {
        let mut value = serde_json::to_value(self).expect("current page contains only strings");
        value["schema"] = json!(super::wire::SCHEMA);
        value
    }
}

/// Pending frontend questions belong to this graph binding; timeout removes
/// them and late answers cannot satisfy later requests. No route cache or I/O.
#[derive(Default)]
struct CurrentPageRequests(Mutex<HashMap<u64, mpsc::SyncSender<CurrentPage>>>);

impl CurrentPageRequests {
    fn ask_current_page(
        &self,
        send: impl FnOnce(u64) -> Result<(), tauri::Error>,
    ) -> Result<Value, super::wire::ErrorBody> {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::sync_channel(1);
        self.0.lock().unwrap().insert(id, answer);
        let result = send(id)
            .ok()
            .and_then(|()| answered.recv_timeout(FLUSH_WAIT).ok());
        self.0.lock().unwrap().remove(&id);
        result.map(CurrentPage::payload).ok_or_else(|| {
            super::wire::ErrorBody::new(
                "busy",
                "the focused Tine window did not answer the page request",
                false,
            )
        })
    }

    fn deliver_current_page(&self, id: u64, page: CurrentPage) {
        if let Some(answer) = self.0.lock().unwrap().remove(&id) {
            let _ = answer.try_send(page);
        }
    }
}

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
    store: std::sync::Weak<tine_store::Store>,
    publications: Mutex<ReloadPublications>,
    waiting: Mutex<HashMap<u64, mpsc::SyncSender<bool>>>,
    current: CurrentPageRequests,
}

impl WindowLink {
    pub(crate) fn new(
        app: tauri::AppHandle,
        label: String,
        binding_generation: u64,
        store: std::sync::Weak<tine_store::Store>,
    ) -> Self {
        Self {
            app,
            label,
            binding_generation,
            store,
            publications: Mutex::new(ReloadPublications::default()),
            waiting: Mutex::new(HashMap::new()),
            current: CurrentPageRequests::default(),
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

    /// One frontend route question on the admitted owner's request thread.
    /// The frontend owns workspace-window and pane focus. O(answer bytes), no
    /// graph I/O. Unresponsive or retired bindings answer busy within 5 s.
    pub(crate) fn current_page(&self) -> Result<Value, super::wire::ErrorBody> {
        self.current.ask_current_page(|id| {
            self.app.emit_to(
                self.label.as_str(),
                "owner-current-page-request",
                json!({"id": id, "binding_generation": self.binding_generation}),
            )
        })
    }

    /// Accept only the graph binding that received the question.
    pub(crate) fn answer_current_page(&self, id: u64, page: CurrentPage, binding_generation: u64) {
        if binding_generation == self.binding_generation {
            self.current.deliver_current_page(id, page);
        }
    }

    /// Tell the window that these pages changed, as the watcher tells it of
    /// outside edits (`watcher::page_event_payloads`: one `graph-changed` per
    /// page, one `graph-changed-bulk` for a checkout-sized batch).
    pub(crate) fn pages_changed(&self, pages: Vec<GraphChange>) {
        // Owner writes already acquired a whole-graph view before planning.
        // Sampling its published revision is O(1), with no file read or scan.
        let revision = self
            .store
            .upgrade()
            .and_then(|store| store.whole_graph().ok().map(|view| view.rev()));
        let delivery = self.publications.lock().unwrap().reload(pages, revision);
        if let Some(delivery) = delivery {
            self.deliver_pages(delivery);
        }
    }

    /// Called by the existing watcher dispatch owner, with its already
    /// serialized evidence. No extra Store surface, subscription or thread.
    pub(crate) fn published(&self, revision: GraphRev, answers: serde_json::Value) {
        let delivery = self
            .publications
            .lock()
            .unwrap()
            .published(revision, answers);
        if let Some(delivery) = delivery {
            self.deliver_pages(delivery);
        }
    }

    fn deliver_pages(&self, (pages, answers): PageDelivery) {
        for (event, payload) in
            crate::watcher::page_event_payloads(pages, self.binding_generation, answers)
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

#[cfg(test)]
mod current_page_tests {
    use super::*;

    #[test]
    fn current_page_delivery_failure_is_busy_and_releases_the_request() {
        let requests = CurrentPageRequests::default();
        let result =
            requests.ask_current_page(|_| Err(std::io::Error::other("window gone").into()));
        let error = result.unwrap_err();
        assert_eq!(error.code, "busy");
        assert!(error.message.contains("window did not answer"));
        assert!(requests.0.lock().unwrap().is_empty());
    }

    /// PC-3 for the whole successful window request: the owner's question to
    /// the frontend and the frontend's answer are the only work. The frontend
    /// is a second thread answering through `answer`, as the webview answers
    /// through its command. Below that, the store spends nothing: no read,
    /// parse, listing, stamp, write or sync, no Store opened. Its lower bound
    /// is one channel send and one receive, microseconds; the budget catches a
    /// graph scan, a sleep, or a retry loop, not noise. The counters are
    /// process-global: the case runs in a child process of its own.
    #[test]
    fn a_successful_current_page_request_is_one_question_one_answer_and_no_graph_work() {
        const CHILD: &str = "TINE_CURRENT_PAGE_COST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(tine_store::test_binary::own_test_binary())
                .args([
                    "--exact",
                    "owner::window::current_page_tests::a_successful_current_page_request_is_one_question_one_answer_and_no_graph_work",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let (out, err) = (
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            assert!(output.status.success(), "{out}\n{err}");
            assert!(
                out.contains("1 passed"),
                "the child ran no case:\n{out}\n{err}"
            );
            return;
        }
        const REQUESTS: u32 = 200;
        const BUDGET: Duration = Duration::from_millis(50);
        let requests = std::sync::Arc::new(CurrentPageRequests::default());
        let (to_frontend, from_owner) = mpsc::channel::<u64>();
        let answers = std::sync::Arc::new(AtomicU64::new(0));
        let frontend = {
            let (requests, answers) = (requests.clone(), answers.clone());
            std::thread::spawn(move || {
                for id in from_owner {
                    answers.fetch_add(1, Ordering::Relaxed);
                    requests.deliver_current_page(
                        id,
                        CurrentPage::CurrentPage {
                            name: "日本 / Notes".into(),
                            url: "tine://page/%E6%97%A5%E6%9C%AC%20%2F%20Notes".into(),
                            block_url: Some(
                                "tine://block/92500000-0000-4000-8000-000000000001".into(),
                            ),
                        },
                    );
                }
            })
        };
        let questions = AtomicU64::new(0);
        tine_store::cost_counters::reset();
        let mut slowest = Duration::ZERO;
        for _ in 0..REQUESTS {
            let started = std::time::Instant::now();
            let payload = requests
                .ask_current_page(|id| {
                    questions.fetch_add(1, Ordering::Relaxed);
                    to_frontend.send(id).unwrap();
                    Ok(())
                })
                .unwrap();
            slowest = slowest.max(started.elapsed());
            assert_eq!(payload["kind"], "current-page");
            assert_eq!(payload["name"], "日本 / Notes");
        }
        drop(to_frontend);
        frontend.join().unwrap();
        println!("{REQUESTS} requests, slowest {slowest:?} (budget {BUDGET:?})");
        assert_eq!(
            questions.load(Ordering::Relaxed),
            u64::from(REQUESTS),
            "one question each"
        );
        assert_eq!(
            answers.load(Ordering::Relaxed),
            u64::from(REQUESTS),
            "one answer each"
        );
        assert!(
            requests.0.lock().unwrap().is_empty(),
            "every request retires"
        );
        assert!(slowest < BUDGET, "slowest {slowest:?} over {BUDGET:?}");
        assert_eq!(
            format!("{:?}", tine_store::cost_counters::snapshot()),
            format!("{:?}", tine_store::cost_counters::Counts::default()),
            "no graph read, parse, listing, stamp, write or sync"
        );
    }

    #[test]
    fn current_page_question_answers_page_block_and_absence_once() {
        for page in [
            CurrentPage::CurrentPage {
                name: "日本 / Notes".into(),
                url: "tine://page/Notes".into(),
                block_url: None,
            },
            CurrentPage::CurrentPage {
                name: "Notes".into(),
                url: "tine://page/Notes".into(),
                block_url: Some("tine://block/92500000-0000-4000-8000-000000000001".into()),
            },
            CurrentPage::NoCurrentPage {
                reason: NoCurrentPage::NotPage,
            },
            CurrentPage::NoCurrentPage {
                reason: NoCurrentPage::NoFocusedWindow,
            },
        ] {
            let expected = serde_json::to_value(&page).unwrap();
            let requests = CurrentPageRequests::default();
            let questions = std::cell::Cell::new(0);
            let started = std::time::Instant::now();
            let payload = requests
                .ask_current_page(|id| {
                    questions.set(questions.get() + 1);
                    requests.deliver_current_page(id, page);
                    Ok(())
                })
                .unwrap();
            assert_eq!(questions.get(), 1);
            assert!(started.elapsed() < Duration::from_secs(1));
            assert!(requests.0.lock().unwrap().is_empty());
            assert_eq!(payload["schema"], super::super::wire::SCHEMA);
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(&payload[key], value);
            }
        }
    }
}
