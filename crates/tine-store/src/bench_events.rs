//! Bench-only NDJSON events for the storage A/B harness (STEP3 §13, gate 2).
//! The contract is "Candidate bench events" in `docs/bench-storage-ab.md` on
//! the `og-storage-perf-ab` branch (`scripts/bench-storage-ab.mjs`).
//!
//! `TINE_BENCH_EVENTS=<absolute path>` is read once, at the first question
//! (`lib.rs` `run` asks at startup). Unset or empty: no file is opened, no
//! writer thread exists, and every emitter is one static load and a branch;
//! no event is built. Set: an emitter stamps the event (`t`, epoch ms) and
//! hands it to a channel; one writer thread formats it and appends it with a
//! single `write` on an `O_APPEND` descriptor, never synced. The save path
//! never waits on the file. Best effort: a file that cannot be opened
//! disables the events, and a lost line is lost.
//!
//! These lines carry page paths and text. They exist only on a machine
//! where someone set the variable to a file of their choosing; nothing else
//! reads or ships them.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The page text a line carries at most: a lossless UTF-8 prefix.
const TEXT_LIMIT: usize = 65536;

/// One event; the writer thread formats it.
pub(crate) enum Ev {
    /// The host starts persisting `version` of the page (its save job).
    SaveBegin {
        key: u64,
        version: u64,
        path: String,
    },
    /// The host recorded Published for `version`: renamed and synced.
    Published {
        key: u64,
        version: u64,
        path: String,
        text: Option<Arc<[u8]>>,
    },
    /// A draft record for the page is durable (the vehicle's directory sync).
    DraftDurable {
        path: String,
        text: Option<Arc<[u8]>>,
    },
    /// A deletion's custody completed: source removed, trash entry durable.
    CustodyComplete { path: String },
    /// One page-mail DTO parse.
    MailParse { path: String, parse_us: u64 },
    /// The host's event vector: length and approximate bytes.
    HostStats {
        events_len: usize,
        events_bytes: usize,
    },
    /// Launch recovery finished with `drafts` recovered pages.
    LaunchRecovered { drafts: usize },
    /// The window unfroze after a planned multi-page sequence.
    Unfreeze,
    /// A named point in an operation, with a count or a duration (µs) where
    /// the site has one: the A/B's breakdown of a rename or delete.
    Mark { name: &'static str, n: u64 },
}

impl Ev {
    fn json(self, t: f64) -> serde_json::Value {
        use serde_json::json;
        let text = |text: Option<Arc<[u8]>>| {
            let bytes = text.as_deref().unwrap_or_default();
            (bytes.len(), prefix(bytes).to_owned())
        };
        match self {
            Ev::SaveBegin { key, version, path } => {
                json!({"ev": "save_begin", "t": t, "key": save_key(key, version), "path": path})
            }
            Ev::Published {
                key,
                version,
                path,
                text: bytes,
            } => {
                let (bytes_len, text) = text(bytes);
                json!({"ev": "published", "t": t, "key": save_key(key, version), "path": path,
                    "version": version, "bytes_len": bytes_len, "text": text})
            }
            Ev::DraftDurable { path, text: bytes } => {
                json!({"ev": "draft_durable", "t": t, "path": path, "text": text(bytes).1})
            }
            Ev::CustodyComplete { path } => json!({"ev": "custody_complete", "t": t, "path": path}),
            Ev::MailParse { path, parse_us } => {
                json!({"ev": "mail_parse", "t": t, "path": path, "parse_us": parse_us})
            }
            Ev::HostStats {
                events_len,
                events_bytes,
            } => json!({"ev": "host_stats", "t": t, "events_len": events_len,
                "events_bytes": events_bytes}),
            Ev::LaunchRecovered { drafts } => {
                json!({"ev": "launch_recovered", "t": t, "drafts": drafts})
            }
            Ev::Unfreeze => json!({"ev": "unfreeze", "t": t}),
            Ev::Mark { name, n } => json!({"ev": "mark", "t": t, "name": name, "n": n}),
        }
    }
}

/// The opaque key a save's `save_begin` and `published` share.
fn save_key(key: u64, version: u64) -> String {
    format!("b{key}:v{version}")
}

/// The longest valid UTF-8 prefix of `bytes` within [`TEXT_LIMIT`].
fn prefix(bytes: &[u8]) -> &str {
    let cut = &bytes[..bytes.len().min(TEXT_LIMIT)];
    match std::str::from_utf8(cut) {
        Ok(text) => text,
        Err(error) => std::str::from_utf8(&cut[..error.valid_up_to()]).unwrap_or_default(),
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64() * 1000.0)
}

enum Msg {
    Line(f64, Ev),
    /// Answered once every earlier line is written (tests).
    #[cfg(test)]
    Sync(Sender<()>),
}

/// An events file's writer thread and the channel to it.
pub(crate) struct Writer {
    tx: Sender<Msg>,
}

impl Writer {
    /// Open `path` for appending and start its writer thread; None when it
    /// cannot be opened or the thread cannot start.
    pub(crate) fn start(path: &Path) -> Option<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("tine-bench-evts".into())
            .spawn(move || write_lines(file, rx))
            .ok()?;
        Some(Self { tx })
    }

    fn send(&self, ev: Ev) {
        let _ = self.tx.send(Msg::Line(now_ms(), ev));
    }

    /// Wait until every line sent so far is written (tests).
    #[cfg(test)]
    pub(crate) fn sync(&self) {
        let (tx, rx) = channel();
        if self.tx.send(Msg::Sync(tx)).is_ok() {
            let _ = rx.recv();
        }
    }
}

/// One `write` per line: the whole line is built first, and the descriptor
/// appends, so concurrent writers of one file never interleave inside a line.
fn write_lines(mut file: File, rx: Receiver<Msg>) {
    let mut line = Vec::new();
    for msg in rx {
        match msg {
            Msg::Line(t, ev) => {
                line.clear();
                if serde_json::to_writer(&mut line, &ev.json(t)).is_ok() {
                    line.push(b'\n');
                    let _ = file.write_all(&line);
                }
            }
            #[cfg(test)]
            Msg::Sync(done) => {
                let _ = done.send(());
            }
        }
    }
}

static GLOBAL: OnceLock<Option<Writer>> = OnceLock::new();

fn global() -> Option<&'static Writer> {
    GLOBAL
        .get_or_init(|| {
            let path = std::env::var_os("TINE_BENCH_EVENTS").filter(|p| !p.is_empty())?;
            Writer::start(Path::new(&path))
        })
        .as_ref()
}

#[cfg(test)]
thread_local! {
    /// A test's writer for events emitted on its own thread.
    static TEST_WRITER: std::cell::RefCell<Option<Arc<Writer>>> =
        const { std::cell::RefCell::new(None) };
}

/// Route this thread's events to `writer` (tests); None stops it.
#[cfg(test)]
pub(crate) fn capture_on_this_thread(writer: Option<Arc<Writer>>) {
    TEST_WRITER.with(|w| *w.borrow_mut() = writer);
}

fn with_writer(send: impl FnOnce(&Writer)) {
    #[cfg(test)]
    if let Some(writer) = TEST_WRITER.with(|w| w.borrow().clone()) {
        return send(&writer);
    }
    if let Some(writer) = global() {
        send(writer);
    }
}

/// Whether events are written. The first call reads `TINE_BENCH_EVENTS`.
pub fn enabled() -> bool {
    #[cfg(test)]
    if TEST_WRITER.with(|w| w.borrow().is_some()) {
        return true;
    }
    global().is_some()
}

/// Emit the event `make` builds; it runs only when events are written.
pub(crate) fn emit(make: impl FnOnce() -> Ev) {
    with_writer(|writer| writer.send(make()));
}

/// Run `parse`, one page-mail DTO parse of the page at `path`; with events
/// on, its wall time is a `mail_parse` event.
pub(crate) fn mail_parse<T>(path: &str, parse: impl FnOnce() -> T) -> T {
    let began = enabled().then(std::time::Instant::now);
    let parsed = parse();
    if let Some(began) = began {
        let parse_us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);
        emit(|| Ev::MailParse {
            path: path.into(),
            parse_us,
        });
    }
    parsed
}

static NEXT_SAVE: AtomicU64 = AtomicU64::new(1);

/// `save_begin` for `version` of the page at `path`: the save's key for its
/// `published`, or 0 when events are off (nothing is built).
pub(crate) fn save_begin(version: u64, path: impl FnOnce() -> String) -> u64 {
    let mut key = 0;
    with_writer(|writer| {
        key = NEXT_SAVE.fetch_add(1, Ordering::Relaxed);
        writer.send(Ev::SaveBegin {
            key,
            version,
            path: path(),
        });
    });
    key
}

/// The window unfroze after a planned multi-page sequence, at `t` (its
/// epoch ms); `bench_event` in the app.
pub fn unfreeze(t: f64) {
    with_writer(|writer| {
        let _ = writer.tx.send(Msg::Line(t, Ev::Unfreeze));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(path: &Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("a whole JSON line"))
            .collect()
    }

    #[test]
    fn unset_starts_no_writer_and_builds_nothing() {
        // The global follows the variable; the test binary never sets it.
        assert!(std::env::var_os("TINE_BENCH_EVENTS").is_none());
        assert!(!enabled());
        emit(|| unreachable!("an event was built with events off"));
        assert_eq!(save_begin(1, || unreachable!("a path was built")), 0);
        // A writer starts only from a path that opens.
        let dir = tempfile::tempdir().unwrap();
        assert!(Writer::start(&dir.path().join("missing/events.ndjson")).is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn text_is_a_lossless_utf8_prefix_with_the_full_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.ndjson");
        let writer = Writer::start(&path).unwrap();
        // 'é' is two bytes: the limit falls inside the last one.
        let text = format!("{}é tail", "a".repeat(TEXT_LIMIT - 1));
        writer.send(Ev::Published {
            key: 7,
            version: 3,
            path: "pages/a.md".into(),
            text: Some(Arc::from(text.as_bytes())),
        });
        writer.sync();
        let line = &lines(&path)[0];
        assert_eq!(line["ev"], "published");
        assert_eq!(line["key"], "b7:v3");
        assert_eq!(line["bytes_len"], text.len());
        assert_eq!(line["text"], "a".repeat(TEXT_LIMIT - 1));
        assert!(line["t"].as_f64().unwrap() > 1.7e12);
    }

    /// Two writers (two processes' descriptors) and many emitting threads on
    /// one file: every line is whole.
    #[test]
    fn lines_are_never_torn_under_concurrent_emitters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.ndjson");
        let writers = [
            Arc::new(Writer::start(&path).unwrap()),
            Arc::new(Writer::start(&path).unwrap()),
        ];
        let big = Arc::<[u8]>::from("x".repeat(40_000).into_bytes());
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let writer = writers[n % 2].clone();
                let big = big.clone();
                std::thread::spawn(move || {
                    for i in 0..50 {
                        let key = n as u64 * 1000 + i;
                        writer.send(Ev::Published {
                            key,
                            version: i,
                            path: format!("pages/p{n}.md"),
                            text: Some(big.clone()),
                        });
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        for writer in &writers {
            writer.sync();
        }
        let lines = lines(&path);
        assert_eq!(lines.len(), 8 * 50);
        assert!(lines
            .iter()
            .all(|l| l["text"].as_str().unwrap().len() == 40_000));
    }
}

/// Emit a [`Ev::Mark`]; nothing is built when events are off.
pub(crate) fn mark(name: &'static str, n: u64) {
    emit(|| Ev::Mark { name, n });
}
