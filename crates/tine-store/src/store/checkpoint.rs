//! The launch checkpoint (storage spec §7.6; ADR 0070): one app-data file per
//! graph holding one whole published generation, written by one background
//! publisher and loaded whole at launch. It is deliberately dumb: a checkpoint
//! that is missing, damaged, of another format, parser, root or config is not
//! used and the launch runs today's initial build; it is never refused,
//! repaired or migrated.
//!
//! File: `MAGIC`, `FORMAT` (u32 LE), header length (u32 LE), postcard
//! [`Header`], then the zstd frame of the postcard [`Body`]. The header pins
//! the parser, the canonical root, the revision of the config bytes the
//! generation was parsed under, and the payload's length and SHA-256.
//!
//! Writing: after a dirtying publication, once the graph has been idle for
//! [`IDLE`] and the last write is [`MIN_INTERVAL`] old, or once the change has
//! waited [`MAX_AGE`] (continuous editing cannot starve it); a store that
//! launched cold writes its first checkpoint [`FIRST_IDLE`] after the last
//! publication instead, so the next launch is warm. Capture takes the store writer only to clone `Arc`s and the
//! revision table; serialization, compression and the atomic replace
//! (`atomic_write_with_check`: temp + fsync + rename + directory fsync) run on
//! the `tine-checkpoint` thread. A write killed at any point leaves the
//! previous checkpoint usable.
//!
//! Unit cost: no per-edit write. Per checkpoint, one file of the whole
//! generation (measured in ADR 0070), at most one per `MIN_INTERVAL` (12 an
//! hour) plus the first one after a cold build; zero transport bytes.
use super::*;
use crate::model::{GraphState, NotCaptured, PagesIn, PagesOut};
use crate::watch::Stamp;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"TINECKPT";
/// Bump whenever anything a checkpoint holds changes meaning or shape: a
/// serialized type, the parser's output, an index's semantics.
/// `checkpoint_tests::the_golden_body_is_pinned_to_format` fails on any such change.
pub(crate) const FORMAT: u32 = 2;
/// The lsdoc release tine-core parses with (`crates/tine-core/Cargo.toml`;
/// `checkpoint_tests::the_parser_tag_matches_the_lsdoc_pin` keeps them equal).
pub(crate) const PARSER: &str = "lsdoc v0.5.7";
/// Quiet time after the last dirtying publication before a checkpoint.
pub(crate) const IDLE: Duration = Duration::from_secs(60);
/// Least time between two checkpoint writes (Martin, 2026-10-02): at most 12
/// whole-generation writes an hour, however the edits are spaced.
pub(crate) const MIN_INTERVAL: Duration = Duration::from_secs(300);
/// Longest a published change waits for a checkpoint under continuous editing.
pub(crate) const MAX_AGE: Duration = Duration::from_secs(600);
/// Quiet time before the first checkpoint of a store that launched without
/// loading one (a cold initial build): that write is what makes the next
/// launch warm, so it is not held to `IDLE` or `MIN_INTERVAL`.
pub(crate) const FIRST_IDLE: Duration = Duration::from_secs(5);
/// A checkpoint whose raw body claims more is not loaded (damaged header).
const MAX_RAW: u64 = 4 << 30;

#[derive(Serialize, Deserialize)]
struct Header {
    parser: String,
    root: PathBuf,
    config_rev: Option<FileRev>,
    raw_len: u64,
    payload_len: u64,
    payload_sha256: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct Body<P> {
    graph: GraphState<P>,
    claimants: Arc<SharedMap<(bool, String), Vec<PageEntry>>>,
    name_by_path: Arc<SharedMap<PathBuf, (PageKind, String)>>,
    /// Each path's stamp as recorded when its bytes were read (§5.4).
    stamps: Vec<(PathBuf, Stamp)>,
    racy: Vec<PathBuf>,
}

/// Why a checkpoint was not loaded; every one falls back to the initial build.
/// Closed tokens for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fallback {
    Missing,
    Unreadable,
    Format,
    Parser,
    Root,
    Config,
    Length,
    Checksum,
    Decode,
    Raced,
}

impl Fallback {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Unreadable => "unreadable",
            Self::Format => "format",
            Self::Parser => "parser",
            Self::Root => "root",
            Self::Config => "config",
            Self::Length => "length",
            Self::Checksum => "checksum",
            Self::Decode => "decode",
            Self::Raced => "raced",
        }
    }
}

/// Outcome of one checkpoint attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointWrite {
    /// Written.
    Written {
        /// Uncompressed body bytes.
        raw_bytes: u64,
        /// Bytes of the file written (header and compressed body).
        file_bytes: u64,
    },
    /// Not written this time; the reason is a closed token. Not an error:
    /// the graph is still loading, the generation is mid-mutation, has
    /// unreadable files, or the store is closing.
    Skipped(&'static str),
    /// The write failed (disk full, permission): the previous checkpoint,
    /// if any, is untouched (atomic replace).
    Failed(String),
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Encode a captured body. Postcard streams into the zstd encoder, so the raw
/// body is never held in memory; its length is counted as it passes.
fn encode(
    root: &Path,
    config_rev: Option<FileRev>,
    body: &Body<PagesOut>,
) -> Result<(Vec<u8>, u64), String> {
    struct Counting<W>(W, u64);
    impl<W: std::io::Write> std::io::Write for Counting<W> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let written = self.0.write(bytes)?;
            self.1 += written as u64;
            Ok(written)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    let encoder = zstd::stream::Encoder::new(Vec::new(), 3).map_err(|error| error.to_string())?;
    let mut counting = Counting(encoder, 0);
    postcard::to_io(body, &mut counting).map_err(|error| error.to_string())?;
    let raw_len = counting.1;
    let payload = counting.0.finish().map_err(|error| error.to_string())?;
    let header = Header {
        parser: PARSER.to_owned(),
        root: root.to_path_buf(),
        config_rev,
        raw_len,
        payload_len: payload.len() as u64,
        payload_sha256: sha256(&payload),
    };
    let header = postcard::to_stdvec(&header).map_err(|error| error.to_string())?;
    let mut file = Vec::with_capacity(16 + header.len() + payload.len());
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&FORMAT.to_le_bytes());
    file.extend_from_slice(&(header.len() as u32).to_le_bytes());
    file.extend_from_slice(&header);
    file.extend_from_slice(&payload);
    Ok((file, raw_len))
}

/// Validate and decode a checkpoint file's bytes for `root` under the config
/// revision `config_rev`. Every check names what it defends: a torn or
/// truncated write (crash, power loss: length, checksum), a disk error (a
/// flipped byte: checksum, decode), another Tine build (format, parser), a
/// moved or different graph (root), or a config edited while Tine was closed
/// (config: the generation was parsed under other settings).
fn decode(
    bytes: &[u8],
    root: &Path,
    config_rev: Option<&FileRev>,
) -> Result<Body<PagesIn>, Fallback> {
    // Probed with `get`: a truncated file (torn write) is a fallback, never
    // a panic.
    let word = |at: usize| {
        bytes
            .get(at..at + 4)
            .and_then(|word| <[u8; 4]>::try_from(word).ok())
            .map(u32::from_le_bytes)
    };
    if bytes.get(..MAGIC.len()) != Some(MAGIC.as_slice()) || word(8) != Some(FORMAT) {
        return Err(Fallback::Format);
    }
    let header_len = word(12).ok_or(Fallback::Format)? as usize;
    let rest = bytes.get(16..).ok_or(Fallback::Format)?;
    let (Some(header), Some(payload)) = (rest.get(..header_len), rest.get(header_len..)) else {
        return Err(Fallback::Length);
    };
    let header: Header = postcard::from_bytes(header).map_err(|_| Fallback::Decode)?;
    if header.parser != PARSER {
        return Err(Fallback::Parser);
    }
    if header.root != root {
        return Err(Fallback::Root);
    }
    if header.config_rev.as_ref() != config_rev {
        return Err(Fallback::Config);
    }
    if payload.len() as u64 != header.payload_len || header.raw_len > MAX_RAW {
        return Err(Fallback::Length);
    }
    if sha256(payload) != header.payload_sha256 {
        return Err(Fallback::Checksum);
    }
    let raw =
        zstd::bulk::decompress(payload, header.raw_len as usize).map_err(|_| Fallback::Decode)?;
    if raw.len() as u64 != header.raw_len {
        return Err(Fallback::Length);
    }
    postcard::from_bytes(&raw).map_err(|_| Fallback::Decode)
}

/// What the load worker installs from a checkpoint.
pub(crate) struct Loaded {
    body: Body<PagesIn>,
    pub(crate) file_bytes: u64,
}

/// Read and validate the checkpoint at `path` (one read of one file). No lock
/// is held; nothing is installed.
pub(crate) fn load(
    path: &Path,
    root: &Path,
    config_rev: Option<&FileRev>,
) -> Result<Loaded, Fallback> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Fallback::Missing)
        }
        Err(_) => return Err(Fallback::Unreadable),
    };
    let body = decode(&bytes, root, config_rev)?;
    Ok(Loaded {
        body,
        file_bytes: bytes.len() as u64,
    })
}

impl ChangeFeed {
    /// Install a loaded checkpoint as this feed's first generation (rev 1,
    /// an `Origin::External` publication with no file tuples), and its page
    /// cache in the graph. Returns the watcher baseline it carries, or
    /// `None` when another path filled the cache or published first (an
    /// on-demand build during launch): the launch then completes cold.
    /// Caller holds the store writer.
    pub(crate) fn install_checkpoint(
        &self,
        loaded: Loaded,
    ) -> Option<crate::model::LaunchObservations> {
        if self.snapshot.read().unwrap().is_some() {
            return None;
        }
        let Body {
            graph,
            claimants,
            name_by_path,
            stamps,
            racy,
        } = loaded.body;
        let config = self.config.read().unwrap().clone();
        let (evaluator, list) = self
            .graph
            .checkpoint_install(graph, (*config.config).clone())?;
        *self.journal_ids.lock().unwrap() =
            journal_ids_from_entries(&self.graph, self.graph.list_pages_shared().as_ref());
        let mut state = self.state.lock().unwrap();
        state.rev += 1;
        let mut snapshot = Snapshot {
            cache_generation: self.graph.cache_generation(),
            graph: evaluator,
            rev: GraphRev(state.rev),
            config,
            journal_format: self.graph.current_journal_format(),
            list,
            claimants,
            name_by_path,
            unreadable: Arc::new(Vec::new()),
            answers: Default::default(),
        };
        // The first publication carries every answer, as a cold one does.
        snapshot.answers = snapshot.answer_changes(None, &[], true);
        let change = Change {
            graph_rev: snapshot.rev,
            origin: Origin::External,
            files: Vec::new(),
            pages: Vec::new(),
            answers: snapshot.answers.clone(),
            watch: None,
        };
        *self.snapshot.write().unwrap() = Some(Arc::new(snapshot));
        if !state.closed {
            state.queue.push_back(change);
            self.ready.notify_all();
        }
        Some(crate::model::LaunchObservations {
            stamps: stamps.into_iter().collect(),
            racy: racy.into_iter().collect(),
            announce: Vec::new(),
        })
    }
}

/// The publisher's shared state: publications mark it dirty, and the
/// checkpoint thread waits on it.
struct SignalState {
    dirty_since: Option<Instant>,
    last_publication: Option<Instant>,
    /// When the last attempt that wrote (or failed to write) began.
    last_write: Option<Instant>,
    /// When the attempt in flight began.
    attempt: Option<Instant>,
    /// No checkpoint was loaded at launch and none has been written since:
    /// the next one is due [`FIRST_IDLE`] after the last publication.
    first: bool,
    requested: u64,
    done: u64,
    stop: bool,
    last: Option<CheckpointWrite>,
}

impl Default for SignalState {
    fn default() -> Self {
        SignalState {
            dirty_since: None,
            last_publication: None,
            last_write: None,
            attempt: None,
            first: true,
            requested: 0,
            done: 0,
            stop: false,
            last: None,
        }
    }
}

impl SignalState {
    /// How long until a dirty generation is due at `now` (`ZERO`: due), or
    /// `None` when nothing is dirty. Due once the graph has been quiet for
    /// `IDLE` and the last write is `MIN_INTERVAL` old, or once the change
    /// has waited `MAX_AGE`; the first write after a cold build only waits
    /// `FIRST_IDLE`. Since `dirty_since` is set no earlier than the start of
    /// the last attempt, `MAX_AGE` (> `MIN_INTERVAL`) never shortens the
    /// spacing.
    fn wait(&self, now: Instant) -> Option<Duration> {
        let (since, last) = (self.dirty_since?, self.last_publication?);
        let (quiet, spacing) = if self.first {
            (FIRST_IDLE, Duration::ZERO)
        } else {
            let spacing = self.last_write.map_or(Duration::ZERO, |at| {
                MIN_INTERVAL.saturating_sub(now.saturating_duration_since(at))
            });
            (IDLE, spacing)
        };
        let idle = quiet.saturating_sub(now.saturating_duration_since(last));
        let age = MAX_AGE.saturating_sub(now.saturating_duration_since(since));
        Some(idle.max(spacing).min(age))
    }
}

#[derive(Default)]
pub(crate) struct Signal {
    state: Mutex<SignalState>,
    wake: Condvar,
}

impl Signal {
    /// The launch served a loaded checkpoint: the next one follows the
    /// ordinary cadence.
    pub(crate) fn loaded(&self) {
        self.state.lock().unwrap().first = false;
    }

    /// A publication changed the graph's derived state.
    pub(crate) fn published(&self) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        state.dirty_since.get_or_insert(now);
        state.last_publication = Some(now);
        self.wake.notify_all();
    }

    /// Ask for a checkpoint now (after a Rescan), without waiting.
    pub(crate) fn request(&self) {
        self.ticket();
    }

    fn ticket(&self) -> u64 {
        let mut state = self.state.lock().unwrap();
        state.requested += 1;
        self.wake.notify_all();
        state.requested
    }

    /// Ask for a checkpoint now and wait for its outcome.
    pub(crate) fn request_and_wait(&self) -> CheckpointWrite {
        let ticket = self.ticket();
        let mut state = self.state.lock().unwrap();
        while state.done < ticket && !state.stop {
            state = self.wake.wait(state).unwrap();
        }
        if state.done < ticket {
            return CheckpointWrite::Skipped("closed");
        }
        state
            .last
            .clone()
            .unwrap_or(CheckpointWrite::Skipped("closed"))
    }

    pub(crate) fn stop(&self) {
        self.state.lock().unwrap().stop = true;
        self.wake.notify_all();
    }

    /// Wait until a checkpoint is due; `None` once stopped. Returns the
    /// highest request ticket the coming attempt answers.
    fn due(&self) -> Option<u64> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.stop {
                return None;
            }
            let now = Instant::now();
            let wait = state.wait(now);
            if state.requested > state.done || wait.is_some_and(|wait| wait.is_zero()) {
                state.dirty_since = None;
                state.attempt = Some(now);
                return Some(state.requested);
            }
            state = match wait {
                Some(wait) => self.wake.wait_timeout(state, wait).unwrap().0,
                None => self.wake.wait(state).unwrap(),
            };
        }
    }

    fn finished(&self, ticket: u64, outcome: CheckpointWrite) {
        let mut state = self.state.lock().unwrap();
        // A generation that could not be captured yet is retried after the
        // next idle period rather than dropped.
        if matches!(outcome, CheckpointWrite::Skipped("unpublished" | "loading")) {
            let now = Instant::now();
            state.dirty_since.get_or_insert(now);
            state.last_publication = Some(now);
        } else if !matches!(outcome, CheckpointWrite::Skipped(_)) {
            state.last_write = state.attempt;
            state.first = false;
        }
        state.done = state.done.max(ticket);
        state.last = Some(outcome);
        self.wake.notify_all();
    }
}

/// Launch from the checkpoint at `path` (load worker, before any parse).
/// Returns `true` when the graph was served from it and Ready published;
/// `false` sends the launch down the ordinary initial build, which is then
/// the whole cost of an unusable checkpoint.
pub(crate) fn launch_from(
    path: &Path,
    (graph, load, writer, changes): (&Arc<Graph>, &LoadState, &Mutex<()>, &ChangeFeed),
    watch: &crate::watch::Core,
    wake: &std::sync::mpsc::Sender<()>,
) -> bool {
    let began = Instant::now();
    let loaded = match self::load(path, &graph.root, watch.config_rev().as_ref()) {
        Ok(loaded) => loaded,
        Err(reason) => {
            graph
                .diag
                .checkpoint_load(reason.token(), began.elapsed(), 0);
            return false;
        }
    };
    let bytes = loaded.file_bytes;
    let _writer = writer.lock().unwrap();
    if !matches!(*load.status.lock().unwrap(), LoadStatus::Loading) {
        return false;
    }
    let Some(observed) = changes.install_checkpoint(loaded) else {
        graph
            .diag
            .checkpoint_load(Fallback::Raced.token(), began.elapsed(), bytes);
        return false;
    };
    graph.diag.checkpoint_load("loaded", began.elapsed(), bytes);
    if let Some(signal) = changes.checkpoint.get() {
        signal.loaded();
    }
    load.serving.store(true, Ordering::Release);
    load.ready.notify_all();
    graph.diag.serving();
    #[cfg(any(test, feature = "test-faults"))]
    while graph.root.join(".tine-test-pause-launch-diff").exists() {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // Storage spec §5.1 step 2 over the checkpoint's stamps: every file
    // changed, created or removed while Tine was closed, and every racy path,
    // is reread before Ready. A failure (root gone, store closed) is left to
    // the next watcher cycle, as on the cold path.
    watch.install_launch_baseline(observed);
    let found = watch.launch_diff().unwrap_or_default();
    let publish_began = Instant::now();
    changes.publish_with(
        Origin::External,
        found.files,
        found.config_changed,
        found.pages,
        || {
            let mut status = load.status.lock().unwrap();
            if matches!(*status, LoadStatus::Loading) {
                *status = LoadStatus::Ready;
            }
        },
    );
    graph.diag.ready(publish_began);
    load.ready.notify_all();
    let _ = wake.send(());
    true
}

impl Store {
    /// Write the launch checkpoint now and wait for the outcome (tests and
    /// diagnostics; the publisher otherwise writes on its cadence). `None`
    /// when this store keeps no checkpoint.
    #[cfg(any(test, feature = "test-faults"))]
    pub fn write_checkpoint_now(&self) -> Option<CheckpointWrite> {
        Some(self.changes.checkpoint.get()?.request_and_wait())
    }
}

/// Everything the checkpoint thread reads; all shared with the store.
pub(crate) struct Publisher {
    path: PathBuf,
    graph: Arc<Graph>,
    writer: Arc<Mutex<()>>,
    load: Arc<LoadState>,
    changes: Arc<ChangeFeed>,
    watch: Arc<crate::watch::Core>,
    signal: Arc<Signal>,
}

impl Publisher {
    /// Register the signal with the feed and start the publisher.
    pub(crate) fn start(
        path: &Path,
        graph: &Arc<Graph>,
        writer: &Arc<Mutex<()>>,
        load: &Arc<LoadState>,
        changes: &Arc<ChangeFeed>,
        watch: &crate::watch::WatchHandle,
    ) {
        let signal = Arc::new(Signal::default());
        if changes.checkpoint.set(Arc::clone(&signal)).is_err() {
            return;
        }
        Self {
            path: path.to_path_buf(),
            graph: Arc::clone(graph),
            writer: Arc::clone(writer),
            load: Arc::clone(load),
            changes: Arc::clone(changes),
            watch: watch.core_for_load(),
            signal,
        }
        .spawn();
    }

    /// Start the single checkpoint publisher. It stops when `Signal::stop`
    /// is called (`Store::close`); a write in flight then finishes or is
    /// killed with the process, and the atomic replace keeps the previous
    /// checkpoint either way.
    pub(crate) fn spawn(self) {
        let spawned = std::thread::Builder::new()
            .name("tine-checkpoint".into())
            .spawn(move || {
                while let Some(ticket) = self.signal.due() {
                    let began = Instant::now();
                    let outcome = self.write_once();
                    let (token, raw, file) = match &outcome {
                        CheckpointWrite::Written {
                            raw_bytes,
                            file_bytes,
                        } => ("written", *raw_bytes, *file_bytes),
                        CheckpointWrite::Skipped(reason) => (*reason, 0, 0),
                        CheckpointWrite::Failed(_) => ("failed", 0, 0),
                    };
                    self.graph
                        .diag
                        .checkpoint_write(token, began.elapsed(), raw, file);
                    self.signal.finished(ticket, outcome);
                }
            });
        // No thread (resource exhaustion): launches stay cold. Not a refusal.
        let _ = spawned;
    }

    fn capture(&self) -> Result<(Body<PagesOut>, Option<FileRev>), &'static str> {
        let _writer = self.writer.lock().unwrap();
        if !matches!(*self.load.status.lock().unwrap(), LoadStatus::Ready) {
            return Err("loading");
        }
        let snapshot = self
            .changes
            .snapshot
            .read()
            .unwrap()
            .clone()
            .ok_or("loading")?;
        if !snapshot.unreadable.is_empty() {
            return Err("unreadable");
        }
        let graph =
            self.graph
                .checkpoint_capture(&snapshot.graph)
                .map_err(|reason| match reason {
                    NotCaptured::Unpublished => "unpublished",
                    NotCaptured::Unreadable => "unreadable",
                })?;
        let (stamps, mut racy, config_rev) = self.watch.checkpoint_observations();
        // A page whose recorded stamp does not vouch for the bytes its cached
        // document was parsed from (an own write raced its watcher echo) is
        // stored racy, so the launch diff rereads it.
        for (path, stamp) in &stamps {
            if let Some(parsed) = graph.disk_rev(path) {
                if stamp.rev().map(|rev| rev.0.as_str()) != Some(parsed) && !racy.contains(path) {
                    racy.push(path.clone());
                }
            }
        }
        racy.sort();
        Ok((
            Body {
                graph,
                claimants: Arc::clone(&snapshot.claimants),
                name_by_path: Arc::clone(&snapshot.name_by_path),
                stamps,
                racy,
            },
            config_rev,
        ))
    }

    fn write_once(&self) -> CheckpointWrite {
        let (body, config_rev) = match self.capture() {
            Ok(captured) => captured,
            Err(reason) => return CheckpointWrite::Skipped(reason),
        };
        let (bytes, raw_bytes) = match encode(&self.graph.root, config_rev, &body) {
            Ok(encoded) => encoded,
            Err(error) => return CheckpointWrite::Failed(error),
        };
        drop(body);
        if let Some(dir) = self.path.parent() {
            if let Err(error) = fs::create_dir_all(dir) {
                return CheckpointWrite::Failed(error.to_string());
            }
        }
        match crate::atomic_file::atomic_write_with_check(
            &self.path,
            &bytes,
            || Ok(()),
            || {},
            || {},
            || {},
        ) {
            Ok(()) => CheckpointWrite::Written {
                raw_bytes,
                file_bytes: bytes.len() as u64,
            },
            Err(error) => CheckpointWrite::Failed(error.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod checkpoint_tests;
