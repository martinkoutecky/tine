//! Core Spotlight index of the current graph (iOS; Martin 2026-10-10).
//!
//! Each page becomes one searchable item: its title plus a short excerpt,
//! addressed by its page key and opening `tine://page/<name>` (the S3 route
//! for the current graph, `src/deepLinks.ts`). Only the main window's graph is
//! indexed. The source is the published whole-graph view, so a page the
//! app's own visibility rules keep from every other surface (`:hidden`
//! directories, trash, version files, conflict copies) is never indexed:
//! discovery (`graph_text_relative_eligible`) already left it out.
//!
//! - launch: clear (`launched`): the index persists across processes, and a
//!   launch that binds no graph must not leave an old graph's pages;
//! - graph bound in the window: clear (`bound`); once warm, replace the
//!   whole index (`reindex`), O(P) once per graph load;
//! - every publication, own or external (watcher.rs `dispatch`): upsert or
//!   delete the changed pages only (`observe`), O(changed pages);
//! - graph released from the window, or forgotten: clear (`released`,
//!   `forget`).
//!
//! Ordering (review round 1, finding 7): every update goes through one FIFO
//! queue and one worker thread, which waits for Core Spotlight's completion
//! (the native call resolves in its completion handler) before the next.
//! Each bind/release/reindex starts a new generation; work queued under an
//! older generation is dropped, before it runs and again before it
//! publishes. Since every generation change queues its own clear or
//! replace, the index ends as the newest generation's state, and a change
//! to one graph is applied in publication order.
//!
//! Spotlight is a cache: every failure is logged and otherwise ignored.
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use tine_core::model::PageKind;

/// The longest excerpt shown under a result, in characters.
const EXCERPT_CHARS: usize = 120;
/// Only this window's graph is "the current graph" (iOS has no other).
const INDEXED_WINDOW: &str = "main";

/// One unit of index work: builds the updates to publish, off the caller's
/// thread (page reads take the store writer).
type Work = Box<dyn FnOnce() -> Vec<Update> + Send>;

/// The window binding whose pages the index holds: graph root and binding
/// generation (state.rs `GraphSlot::binding_generation`).
type Binding = (String, u64);

#[derive(Default)]
struct Queue {
    /// Bumped by every bind, release and reindex.
    generation: u64,
    bound: Option<Binding>,
    /// A replace for `bound` is queued or done: incremental updates apply.
    indexed: bool,
    jobs: VecDeque<(u64, Work)>,
}

/// The serialized index writer: one FIFO, one worker, one sink.
pub(crate) struct Index {
    queue: Mutex<Queue>,
    wake: Condvar,
}

impl Index {
    /// Start the worker; `sink` applies one update and returns when the
    /// index applied it.
    fn start(sink: impl Fn(&Update) + Send + 'static) -> Arc<Index> {
        let index = Arc::new(Index {
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
        });
        let worker = index.clone();
        std::thread::Builder::new()
            .name("spotlight".into())
            .spawn(move || worker.run(sink))
            .expect("spawn the Spotlight worker");
        index
    }

    fn run(&self, sink: impl Fn(&Update)) {
        loop {
            let (generation, work) = {
                let mut queue = self.queue.lock().unwrap();
                loop {
                    if let Some(job) = queue.jobs.pop_front() {
                        break job;
                    }
                    queue = self.wake.wait(queue).unwrap();
                }
            };
            if !self.current(generation) {
                continue; // superseded before it ran
            }
            let updates = work();
            if !self.current(generation) {
                continue; // superseded while it ran: a newer clear/replace follows
            }
            for update in &updates {
                sink(update);
            }
        }
    }

    fn current(&self, generation: u64) -> bool {
        self.queue.lock().unwrap().generation == generation
    }

    fn push(queue: &mut Queue, generation: u64, work: Work) {
        queue.jobs.push_back((generation, work));
    }

    /// Start a new generation whose first update is `work`.
    fn restart(&self, bound: Option<Binding>, indexed: bool, work: Work) {
        let mut queue = self.queue.lock().unwrap();
        queue.generation += 1;
        queue.bound = bound;
        queue.indexed = indexed;
        let generation = queue.generation;
        Self::push(&mut queue, generation, work);
        self.wake.notify_one();
    }

    fn clear_all(&self, bound: Option<Binding>) {
        self.restart(bound, false, Box::new(|| vec![Update::Clear]));
    }

    /// A graph binding became the window's: drop the old pages now.
    fn bound(&self, binding: Binding) {
        self.clear_all(Some(binding));
    }

    /// Replace the index with `binding`'s pages, if it is still the bound one.
    fn reindex(
        &self,
        binding: &Binding,
        entries: impl FnOnce() -> Option<Vec<Entry>> + Send + 'static,
    ) {
        if self.queue.lock().unwrap().bound.as_ref() != Some(binding) {
            return;
        }
        self.restart(
            Some(binding.clone()),
            true,
            Box::new(move || {
                entries()
                    .map(|entries| vec![Update::Replace { entries }])
                    .unwrap_or_default()
            }),
        );
    }

    /// Queue an incremental update for `binding` if its pages are indexed.
    fn observe(&self, binding: &Binding, work: Work) {
        let mut queue = self.queue.lock().unwrap();
        if !queue.indexed || queue.bound.as_ref() != Some(binding) {
            return; // not indexed yet: the coming reindex covers this change
        }
        let generation = queue.generation;
        Self::push(&mut queue, generation, work);
        self.wake.notify_one();
    }

    /// The binding left the window (`None`: whatever is bound).
    fn released(&self, binding_generation: Option<u64>) {
        let releases = {
            let queue = self.queue.lock().unwrap();
            binding_generation.is_none_or(|generation| {
                queue
                    .bound
                    .as_ref()
                    .is_some_and(|bound| bound.1 == generation)
            })
        };
        if releases {
            self.clear_all(None);
        }
    }

    fn forget(&self, root: &str) {
        let forgets = self
            .queue
            .lock()
            .unwrap()
            .bound
            .as_ref()
            .is_some_and(|bound| bound.0 == root);
        if forgets {
            self.clear_all(None);
        }
    }
}

static INDEX: OnceLock<Arc<Index>> = OnceLock::new();

fn index(app: &tauri::AppHandle) -> &'static Index {
    INDEX.get_or_init(|| {
        let app = app.clone();
        Index::start(move |update| publish(&app, update))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Entry {
    /// Stable within a graph: the page key, so renames and deletes address it.
    pub id: String,
    pub title: String,
    pub excerpt: String,
    /// The S3 route a tap opens.
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub(crate) enum Update {
    /// Drop everything, then index `entries`.
    Replace {
        entries: Vec<Entry>,
    },
    Upsert {
        entries: Vec<Entry>,
    },
    Delete {
        ids: Vec<String>,
    },
    Clear,
}

/// RFC 3986 percent-encoding of everything but unreserved characters, the
/// inverse of the frontend's `decodeURIComponent`.
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn first_text(blocks: &[tine_core::doc::DocBlock]) -> Option<String> {
    blocks.iter().find_map(|block| {
        let text = collapsed(block.visible_text());
        if text.is_empty() {
            first_text(&block.children)
        } else {
            Some(text)
        }
    })
}

/// The same walk over a loaded page's blocks (watcher updates): each body is
/// projected exactly as [`tine_core::doc::DocBlock::visible_text`] does.
fn first_dto_text(blocks: &[tine_core::model::BlockDto], is_org: bool) -> Option<String> {
    blocks.iter().find_map(|block| {
        let mut doc = tine_core::doc::DocBlock::new(block.raw.clone());
        doc.set_org(is_org);
        let text = collapsed(doc.visible_text());
        if text.is_empty() {
            first_dto_text(&block.children, is_org)
        } else {
            Some(text)
        }
    })
}

/// Whitespace-collapsed text cut to [`EXCERPT_CHARS`].
fn cut(text: String) -> String {
    if text.chars().count() <= EXCERPT_CHARS {
        return text;
    }
    let head: String = text.chars().take(EXCERPT_CHARS - 1).collect();
    format!("{}…", head.trim_end())
}

/// The first non-empty block's visible text, whitespace collapsed, cut to
/// [`EXCERPT_CHARS`].
pub(crate) fn excerpt(document: &tine_core::doc::Document) -> String {
    cut(first_text(&document.roots).unwrap_or_default())
}

fn entry_with(name: &str, excerpt: String) -> Entry {
    Entry {
        id: tine_core::refs::page_key(name),
        title: name.to_owned(),
        excerpt,
        url: format!("tine://page/{}", encode_component(name)),
    }
}

pub(crate) fn entry(name: &str, document: &tine_core::doc::Document) -> Entry {
    entry_with(name, excerpt(document))
}

/// [`entry`] for a page the store just loaded.
pub(crate) fn entry_for_page(page: &tine_core::model::PageDto) -> Entry {
    let is_org = page.format == tine_core::model::Format::Org;
    entry_with(
        &page.name,
        cut(first_dto_text(&page.blocks, is_org).unwrap_or_default()),
    )
}

fn binding_of(slot: &crate::state::GraphSlot) -> Binding {
    (slot.root_key.display().to_string(), slot.binding_generation)
}

/// The app started (lib.rs `setup`): the persisted index may hold a graph
/// this launch never binds.
pub(crate) fn launched(app: &tauri::AppHandle) {
    if INDEXES {
        index(app).clear_all(None);
    }
}

/// `slot` became `label`'s graph (graph.rs `load_graph_for_label`).
pub(crate) fn bound(app: &tauri::AppHandle, label: &str, slot: &crate::state::GraphSlot) {
    if INDEXES && label == INDEXED_WINDOW {
        index(app).bound(binding_of(slot));
    }
}

/// Replace the index with `slot`'s pages. Called once the window's graph is
/// warm (graph.rs `warm_cache_async`); a no-op unless `slot` is still bound.
pub(crate) fn reindex(app: &tauri::AppHandle, label: &str, slot: &Arc<crate::state::GraphSlot>) {
    if !INDEXES || label != INDEXED_WINDOW {
        return;
    }
    let source = slot.clone();
    index(app).reindex(&binding_of(slot), move || {
        match source.store.whole_graph() {
            Ok(view) => Some(
                view.corpus()
                    .pages
                    .iter()
                    .map(|page| entry(&page.name, &page.document))
                    .collect(),
            ),
            Err(error) => {
                crate::debug::diag_private("spotlight-reindex-failed", &format!("{error:?}"));
                None
            }
        }
    });
}

/// Pages one publication changed: `(name, kind, removed)`.
pub(crate) type Changed = Vec<(String, PageKind, bool)>;

/// Bring the changed pages' items up to date (watcher.rs `dispatch`).
pub(crate) fn observe(
    app: &tauri::AppHandle,
    label: &str,
    slot: &Arc<crate::state::GraphSlot>,
    changed: Changed,
) {
    if !INDEXES || label != INDEXED_WINDOW || changed.is_empty() {
        return;
    }
    let source = slot.clone();
    index(app).observe(
        &binding_of(slot),
        Box::new(move || {
            let mut entries = Vec::new();
            let mut ids = Vec::new();
            for (name, kind, removed) in changed {
                match (!removed)
                    .then(|| source.store.page_named(&name, kind))
                    .transpose()
                {
                    Ok(Some(Some(read))) => entries.push(entry_for_page(&read.doc)),
                    Ok(_) => ids.push(tine_core::refs::page_key(&name)),
                    Err(error) => crate::debug::diag_private(
                        "spotlight-page-read-failed",
                        &format!("{error:?}"),
                    ),
                }
            }
            let mut updates = Vec::new();
            if !ids.is_empty() {
                updates.push(Update::Delete { ids });
            }
            if !entries.is_empty() {
                updates.push(Update::Upsert { entries });
            }
            updates
        }),
    );
}

/// `label`'s graph binding was released (window destroyed, or an open
/// abandoned): its pages leave the index. `binding_generation` names the
/// released binding; `None` releases whatever the window held.
pub(crate) fn released(app: &tauri::AppHandle, label: &str, binding_generation: Option<u64>) {
    if INDEXES && label == INDEXED_WINDOW {
        index(app).released(binding_generation);
    }
}

/// A graph was removed from the known-graph list: clear the index if it was
/// the indexed one.
pub(crate) fn forget(app: &tauri::AppHandle, root: &str) {
    if INDEXES {
        index(app).forget(root);
    }
}

// ---- Platform split: every shipped target is named. ----

/// iOS: Core Spotlight, through the native integrations plugin.
#[cfg(any(target_os = "ios"))]
const INDEXES: bool = true;
#[cfg(any(target_os = "ios"))]
fn publish(app: &tauri::AppHandle, update: &Update) {
    if let Err(error) = crate::native_integrations::spotlight(app, update) {
        crate::debug::diag_private("spotlight-update-failed", &error);
    }
}

/// No system search index Tine feeds on these targets.
#[cfg(any(
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_os = "macos"
))]
const INDEXES: bool = false;
#[cfg(any(
    target_os = "android",
    target_os = "linux",
    target_os = "windows",
    target_os = "macos"
))]
fn publish(_app: &tauri::AppHandle, _update: &Update) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(markdown: &str) -> tine_core::doc::Document {
        tine_core::doc::parse(markdown)
    }

    #[test]
    fn an_entry_is_title_excerpt_and_the_open_page_route() {
        let doc = document("title:: ignored\n\n- \n- First **real** line\n  continues\n- second\n");
        let entry = entry("Über Plan/2026", &doc);
        assert_eq!(entry.id, tine_core::refs::page_key("Über Plan/2026"));
        assert_eq!(entry.title, "Über Plan/2026");
        assert!(entry.excerpt.starts_with("First"), "{:?}", entry.excerpt);
        assert!(!entry.excerpt.contains('\n'));
        assert_eq!(entry.url, "tine://page/%C3%9Cber%20Plan%2F2026");
    }

    #[test]
    fn a_long_excerpt_is_cut_to_the_limit() {
        let doc = document(&format!("- {}\n", "word ".repeat(100)));
        let text = excerpt(&doc);
        assert_eq!(text.chars().count(), EXCERPT_CHARS);
        assert!(text.ends_with('…'));
        assert_eq!(excerpt(&document("")), "");
    }

    /// A watcher update must produce the same item as the warm reindex did,
    /// or an edit would flip a page's excerpt between two spellings.
    #[test]
    fn a_loaded_page_yields_the_same_entry_as_the_corpus() {
        let markdown = "- \n- TODO First [[real]] line\n  id:: 6512b1a4-0000-4000-8000-000000000001\n- second\n";
        let doc = document(markdown);
        let page = tine_core::projection::markdown_page_dto("Plan", "Plan", markdown);
        let from_corpus = entry("Plan", &doc);
        assert!(
            !from_corpus.excerpt.contains("id::"),
            "{:?}",
            from_corpus.excerpt
        );
        assert_eq!(
            serde_json::to_value(entry_for_page(&page)).unwrap(),
            serde_json::to_value(from_corpus).unwrap()
        );
    }

    #[test]
    fn updates_serialize_with_an_op_tag() {
        let json = serde_json::to_value(Update::Delete {
            ids: vec!["a".into()],
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({"op": "delete", "ids": ["a"]}));
        let json = serde_json::to_value(Update::Clear).unwrap();
        assert_eq!(json, serde_json::json!({"op": "clear"}));
    }

    // ---- The serialized index writer (review round 1, finding 7). ----

    use std::sync::mpsc;
    use std::time::Duration;

    /// An index whose sink records each update as a short string.
    fn recorder() -> (Arc<Index>, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        let index = Index::start(move |update| {
            let line = match update {
                Update::Replace { entries } => format!(
                    "replace {}",
                    entries
                        .iter()
                        .map(|e| e.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Update::Upsert { entries } => format!(
                    "upsert {}",
                    entries
                        .iter()
                        .map(|e| e.id.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Update::Delete { ids } => format!("delete {}", ids.join(",")),
                Update::Clear => "clear".into(),
            };
            sink.lock().unwrap().push(line);
        });
        (index, log)
    }

    /// The index contents after applying `log` in order (a superseded clear
    /// may be skipped: the replace that follows drops everything anyway).
    fn contents(log: &[String]) -> Vec<String> {
        let mut ids = std::collections::BTreeSet::new();
        for line in log {
            let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
            let named = rest
                .split(',')
                .filter(|id| !id.is_empty())
                .map(str::to_owned);
            match op {
                "replace" => ids = named.collect(),
                "upsert" => ids.extend(named),
                "delete" => named.for_each(|id| {
                    ids.remove(&id);
                }),
                _ => ids.clear(),
            }
        }
        ids.into_iter().collect()
    }

    /// Wait until the worker finished everything queued so far.
    fn drain(index: &Index) {
        let (done, wait) = mpsc::channel();
        let mut queue = index.queue.lock().unwrap();
        let generation = queue.generation;
        Index::push(
            &mut queue,
            generation,
            Box::new(move || {
                done.send(()).unwrap();
                Vec::new()
            }),
        );
        index.wake.notify_one();
        drop(queue);
        // A newer generation drops the marker: then it never ran, and the
        // queue holds only that generation's work, which a second drain sees.
        if wait.recv_timeout(Duration::from_secs(10)).is_err() {
            drain(index);
        }
    }

    fn page(name: &str) -> Entry {
        entry(name, &document("- text\n"))
    }

    fn a() -> Binding {
        ("/graphs/a".into(), 1)
    }

    fn b() -> Binding {
        ("/graphs/b".into(), 2)
    }

    /// Work that blocks until the test releases it: a worker mid-read.
    fn held(updates: Vec<Update>) -> (Work, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (release, gate) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        let work: Work = Box::new(move || {
            started.send(()).unwrap();
            gate.recv().unwrap();
            updates
        });
        (work, release, running)
    }

    #[test]
    fn a_graph_switch_drops_the_old_graphs_in_flight_reindex() {
        let (index, log) = recorder();
        index.bound(a());
        let (release, gate) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        index.reindex(&a(), move || {
            started.send(()).unwrap();
            gate.recv().unwrap();
            Some(vec![page("A1")])
        });
        running.recv().unwrap();
        index.bound(b()); // switch while A's reindex is reading
        index.reindex(&b(), || Some(vec![page("B1")]));
        release.send(()).unwrap();
        drain(&index);
        let log = log.lock().unwrap();
        assert_eq!(contents(&log), ["b1"], "{log:?}");
        assert!(!log.iter().any(|line| line.contains("a1")), "{log:?}");
    }

    #[test]
    fn an_old_graphs_in_flight_update_never_publishes_after_the_switch() {
        let (index, log) = recorder();
        index.bound(a());
        index.reindex(&a(), || Some(vec![page("A1")]));
        let (work, release, running) = held(vec![Update::Upsert {
            entries: vec![page("A2")],
        }]);
        index.observe(&a(), work);
        running.recv().unwrap();
        index.bound(b());
        index.reindex(&b(), || Some(vec![page("B1")]));
        release.send(()).unwrap();
        drain(&index);
        let log = log.lock().unwrap();
        assert_eq!(contents(&log), ["b1"], "{log:?}");
        assert!(!log.iter().any(|line| line.contains("a2")), "{log:?}");
    }

    #[test]
    fn a_stale_reindex_is_refused_and_an_unindexed_graph_takes_no_updates() {
        let (index, log) = recorder();
        index.bound(b());
        index.reindex(&a(), || Some(vec![page("A1")])); // A's warm finished after the switch
        index.observe(
            &b(),
            Box::new(|| {
                vec![Update::Upsert {
                    entries: vec![page("B0")],
                }]
            }),
        );
        index.observe(
            &a(),
            Box::new(|| {
                vec![Update::Upsert {
                    entries: vec![page("A2")],
                }]
            }),
        );
        drain(&index);
        assert_eq!(*log.lock().unwrap(), ["clear"]);
    }

    #[test]
    fn updates_to_one_graph_publish_in_order() {
        let (index, log) = recorder();
        index.bound(a());
        index.reindex(&a(), || Some(vec![page("P")]));
        // The upsert's read is slow; the delete that follows must wait for it.
        let (work, release, running) = held(vec![Update::Upsert {
            entries: vec![page("P")],
        }]);
        index.observe(&a(), work);
        index.observe(
            &a(),
            Box::new(|| {
                vec![Update::Delete {
                    ids: vec!["p".into()],
                }]
            }),
        );
        running.recv().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(contents(&log.lock().unwrap()), ["p"]);
        assert!(!log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.starts_with("delete")));
        release.send(()).unwrap();
        drain(&index);
        let log = log.lock().unwrap();
        assert_eq!(log[log.len() - 2..], ["upsert p", "delete p"]);
        assert!(contents(&log).is_empty(), "{log:?}");
    }

    #[test]
    fn release_forget_and_launch_clear_the_index() {
        let (index, log) = recorder();
        let now = |log: &Arc<Mutex<Vec<String>>>| contents(&log.lock().unwrap());
        index.clear_all(None); // launched
        drain(&index);
        assert_eq!(*log.lock().unwrap(), ["clear"]);
        index.bound(a());
        index.reindex(&a(), || Some(vec![page("A1")]));
        drain(&index);
        assert_eq!(now(&log), ["a1"]);
        index.released(Some(99)); // an older binding's release: not ours
        index.forget("/graphs/b");
        drain(&index);
        assert_eq!(now(&log), ["a1"]);
        index.released(Some(1));
        drain(&index);
        assert!(now(&log).is_empty());
        index.bound(b());
        index.reindex(&b(), || Some(vec![page("B1")]));
        drain(&index);
        assert_eq!(now(&log), ["b1"]);
        index.forget("/graphs/b");
        drain(&index);
        assert!(now(&log).is_empty());
        // A release overtaking an in-flight reindex drops it.
        index.bound(a());
        let (release, gate) = mpsc::channel::<()>();
        let (started, running) = mpsc::channel::<()>();
        index.reindex(&a(), move || {
            started.send(()).unwrap();
            gate.recv().unwrap();
            Some(vec![page("A1")])
        });
        running.recv().unwrap();
        index.released(None);
        release.send(()).unwrap();
        drain(&index);
        assert!(now(&log).is_empty(), "{:?}", log.lock().unwrap());
    }

    /// AGENTS.md section 2: a platform `cfg` list names every shipped target.
    #[test]
    fn every_shipped_target_is_named_exactly_once() {
        let source = include_str!("spotlight.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let mut named: Vec<&str> = production
            .split("#[cfg(any(")
            .skip(1)
            .step_by(2) // each arm has a const and a fn; count the const's list
            .flat_map(|arm| {
                arm.split("))]")
                    .next()
                    .unwrap()
                    .split('"')
                    .skip(1)
                    .step_by(2)
            })
            .collect();
        named.sort_unstable();
        assert_eq!(
            named,
            ["android", "ios", "linux", "macos", "windows"],
            "the Spotlight platform split must name Linux, Windows, macOS, iOS and Android \
             exactly once (AGENTS.md section 2; exemplar defender.rs)"
        );
    }
}
