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
/// them and late answers cannot satisfy later requests. No route cache;
/// identity comes from the Store's bounded read or first guarded create.
#[derive(Default)]
struct CurrentPageRequests(Mutex<HashMap<u64, mpsc::SyncSender<CurrentPage>>>);

impl CurrentPageRequests {
    fn ask_current_page(
        &self,
        store: &tine_store::Store,
        send: impl FnOnce(u64, String) -> Result<(), tauri::Error>,
    ) -> Result<Value, super::wire::ErrorBody> {
        let graph_id = store
            .ensure_link_identity()
            .map_err(|cause| super::verbs::internal(super::verbs::Diag::WriteFailed, cause))?;
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = mpsc::sync_channel(1);
        self.0.lock().unwrap().insert(id, answer);
        let result = send(id, graph_id)
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
    /// The frontend owns workspace-window and pane focus. O(answer bytes), plus
    /// one bounded identity read, plus a guarded create on first use.
    /// Unresponsive or retired bindings answer busy within 5 s.
    pub(crate) fn current_page(&self) -> Result<Value, super::wire::ErrorBody> {
        let store = self.store.upgrade().ok_or_else(|| {
            super::verbs::internal(super::verbs::Diag::GraphLoad, "the Store closed")
        })?;
        self.current.ask_current_page(&store, |id, graph_id| {
            self.app.emit_to(
                self.label.as_str(),
                "owner-current-page-request",
                json!({"id": id, "binding_generation": self.binding_generation, "graph_id": graph_id}),
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

#[cfg(all(test, desktop))]
mod current_page_tests {
    use super::*;

    #[test]
    fn current_page_delivery_failure_is_busy_and_releases_the_request() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _, _) = tine_store::Store::open(temp.path(), Default::default()).unwrap();
        let requests = CurrentPageRequests::default();
        let result = requests.ask_current_page(&store, |_, _| {
            Err(std::io::Error::other("window gone").into())
        });
        let error = result.unwrap_err();
        assert_eq!(error.code, "busy");
        assert!(error.message.contains("window did not answer"));
        assert!(requests.0.lock().unwrap().is_empty());
    }

    #[test]
    fn current_page_identity_failure_returns_the_cause_without_a_page_link() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _, _) = tine_store::Store::open(temp.path(), Default::default()).unwrap();
        let requests = CurrentPageRequests::default();
        store.inject_fault(tine_store::FaultPoint::MidStepIo);
        let error = requests
            .ask_current_page(&store, |_, _| {
                panic!("failed identity must not publish a link")
            })
            .unwrap_err();
        assert_eq!(error.code, "internal");
        assert_eq!(error.detail.unwrap()["diag"], "cli-write-failed");
        assert!(error.message.contains("Couldn't save graph link identity"));
        assert!(!temp.path().join("logseq/tine-graph-id").exists());
        assert!(requests.0.lock().unwrap().is_empty());
        let identity = store.ensure_link_identity().unwrap();
        let payload = requests
            .ask_current_page(&store, |id, graph_id| {
                assert_eq!(graph_id, identity);
                requests.deliver_current_page(
                    id,
                    CurrentPage::CurrentPage {
                        name: "Notes".into(),
                        url: format!("tine://page/Notes?graph={graph_id}"),
                        block_url: None,
                    },
                );
                Ok(())
            })
            .unwrap();
        assert!(payload["url"].as_str().unwrap().ends_with(&identity));
    }

    #[test]
    fn current_page_read_only_graph_returns_the_storage_refusal_without_a_link() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("logseq")).unwrap();
        let config = temp.path().join("logseq/config.edn");
        let bytes = b"{:a \"\xff\"}\n";
        std::fs::write(&config, bytes).unwrap();
        let (store, _, _) = tine_store::Store::open(temp.path(), Default::default()).unwrap();
        assert!(store.config().problem.is_some(), "graph is read-only");
        let requests = CurrentPageRequests::default();
        let error = requests
            .ask_current_page(&store, |_, _| {
                panic!("read-only graph must not publish an unqualified link")
            })
            .unwrap_err();
        assert_eq!(error.code, "internal");
        assert!(error.message.contains("ConfigUnreadable"));
        assert_eq!(error.detail.unwrap()["diag"], "cli-write-failed");
        assert!(!temp.path().join("logseq/tine-graph-id").exists());
        assert_eq!(std::fs::read(config).unwrap(), bytes);
        assert!(requests.0.lock().unwrap().is_empty());
    }

    /// PC-3 for the whole successful window request: the owner's question to
    /// the frontend and the frontend's answer carry the graph identity. The frontend
    /// is a second thread answering through `answer`, as the webview answers
    /// through its command. The Store reads one tiny identity per request and
    /// creates it once; no page read, parse or graph listing. Its lower bound
    /// is two socket round trips and one frontend question/answer; the budget catches a
    /// graph scan, a sleep, or a retry loop, not noise. The counters are
    /// process-global: the case runs in a child process of its own.
    #[test]
    fn a_successful_current_page_request_reads_one_identity_and_creates_it_once() {
        const CHILD: &str = "TINE_CURRENT_PAGE_COST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(tine_store::test_binary::own_test_binary())
                .args([
                    "--exact",
                    "owner::window::current_page_tests::a_successful_current_page_request_reads_one_identity_and_creates_it_once",
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
            print!("{out}");
            return;
        }
        const REQUESTS: u32 = 200;
        const BUDGET: Duration = Duration::from_millis(50);
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("pages")).unwrap();
        std::fs::create_dir(temp.path().join("logseq")).unwrap();
        let note = temp.path().join("pages/Notes.md");
        let bytes = b"- unchanged\r\n";
        std::fs::write(&note, bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&note)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(86_400)),
            )
            .unwrap();
        let (store, _, _) = tine_store::Store::open(temp.path(), Default::default()).unwrap();
        // An answering graph window already has its loaded view.
        store.whole_graph().unwrap();
        let store = std::sync::Arc::new(store);
        let root = tine_store::Store::canonical_root(temp.path()).unwrap();
        #[cfg(unix)]
        let socket = temp.path().join("owner.sock");
        #[cfg(windows)]
        let socket = super::super::windows_security::pipe_name(&format!(
            "{:016x}",
            crate::settings::graph_hash(&root)
        ))
        .unwrap();
        #[cfg(unix)]
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        #[cfg(windows)]
        let listener = super::super::windows::Listener::bind(&socket).unwrap();
        let requests = std::sync::Arc::new(CurrentPageRequests::default());
        let (to_frontend, from_owner) = mpsc::channel::<(u64, String)>();
        let answers = std::sync::Arc::new(AtomicU64::new(0));
        let frontend = {
            let (requests, answers) = (requests.clone(), answers.clone());
            std::thread::spawn(move || {
                for (id, graph_id) in from_owner {
                    answers.fetch_add(1, Ordering::Relaxed);
                    requests.deliver_current_page(
                        id,
                        CurrentPage::CurrentPage {
                            name: "日本 / Notes".into(),
                            url: format!(
                                "tine://page/%E6%97%A5%E6%9C%AC%20%2F%20Notes?graph={graph_id}"
                            ),
                            block_url: Some(
                                "tine://block/92500000-0000-4000-8000-000000000001".into(),
                            ),
                        },
                    );
                }
            })
        };
        tine_store::cost_counters::reset();
        let owner = {
            let (requests, store, root) = (requests.clone(), store.clone(), root.clone());
            std::thread::spawn(move || {
                use super::super::wire::{self, Hello, Reply, Request, REQUEST_CAP};
                let (mut frames, mut questions) = (0, 0);
                for _ in 0..REQUESTS {
                    let (mut writer, _) = loop {
                        match listener.accept() {
                            Ok(connection) => break connection,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(1));
                            }
                            Err(error) => panic!("owner accept failed: {error}"),
                        }
                    };
                    let mut reader = std::io::BufReader::new(writer.try_clone().unwrap());
                    let line = wire::read_frame(&mut reader, REQUEST_CAP).unwrap().unwrap();
                    let hello: Hello = wire::read_line(&line).unwrap();
                    assert_eq!(hello.tine, "cli");
                    wire::write_frame(&mut writer, &super::super::lease::hello(&root, "app"))
                        .unwrap();
                    frames += 1;
                    let line = wire::read_frame(&mut reader, REQUEST_CAP).unwrap().unwrap();
                    let request: Request = wire::read_line(&line).unwrap();
                    assert_eq!(request.verb, "page.current");
                    let ok = requests
                        .ask_current_page(&store, |id, graph_id| {
                            questions += 1;
                            to_frontend.send((id, graph_id)).unwrap();
                            Ok(())
                        })
                        .unwrap();
                    wire::write_frame(
                        &mut writer,
                        &Reply {
                            id: request.id,
                            ok: Some(ok),
                            error: None,
                        },
                    )
                    .unwrap();
                    frames += 1;
                    assert!(
                        wire::read_frame(&mut reader, REQUEST_CAP)
                            .unwrap()
                            .is_none(),
                        "no extra owner request"
                    );
                }
                (frames, questions)
            })
        };
        let mut slowest = Duration::ZERO;
        let mut identity = None;
        for request in 0..REQUESTS {
            let started = std::time::Instant::now();
            let mut connection = super::super::client::dial(&socket, &root).unwrap().unwrap();
            let payload = connection.call("page.current", Value::Null).unwrap();
            drop(connection);
            slowest = slowest.max(started.elapsed());
            assert_eq!(payload["kind"], "current-page");
            assert_eq!(payload["name"], "日本 / Notes");
            let published =
                std::fs::read_to_string(temp.path().join("logseq/tine-graph-id")).unwrap();
            assert_eq!(published.len(), 37);
            if let Some(identity) = &identity {
                assert_eq!(&published, identity);
            } else {
                identity = Some(published.clone());
            }
            assert!(payload["url"].as_str().unwrap().ends_with(published.trim()));
            let counts = tine_store::cost_counters::snapshot();
            assert_eq!(counts.store_reads, u64::from(request + 1));
            assert_eq!(counts.files_written, 1, "only the first request creates");
            assert_eq!(counts.bytes_written, 37);
        }
        let (frames, questions) = owner.join().unwrap();
        frontend.join().unwrap();
        println!("{REQUESTS} requests, slowest {slowest:?} (budget {BUDGET:?})");
        assert_eq!(questions, REQUESTS, "one question each");
        assert_eq!(
            frames,
            REQUESTS * 2,
            "hello and verb: two socket round trips each"
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
        let counts = tine_store::cost_counters::snapshot();
        println!("identity work: {counts:?}");
        let expected = tine_store::cost_counters::Counts {
            save_path_resolutions: 5,
            transaction_record_probes: 2,
            fsyncs: 2,
            bytes_written: 37,
            files_written: 1,
            asset_stamps_by_path: 1,
            store_reads: u64::from(REQUESTS),
            file_renames: 1,
            ..Default::default()
        };
        let counts = tine_store::cost_counters::Counts {
            snapshot_nanos: 0,
            ..counts
        };
        assert_eq!(
            format!("{counts:?}"),
            format!("{expected:?}"),
            "identity work only"
        );
        assert_eq!(std::fs::read(note).unwrap(), bytes);
    }

    #[test]
    fn current_page_question_answers_page_block_and_absence_once() {
        let temp = tempfile::tempdir().unwrap();
        let (store, _, _) = tine_store::Store::open(temp.path(), Default::default()).unwrap();
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
                .ask_current_page(&store, |id, _| {
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
