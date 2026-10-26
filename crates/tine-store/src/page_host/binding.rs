//! One page host per graph binding (STEP3 §1–§3): the driver thread, the
//! command path that serializes a window's page DTO against the right
//! comparison source and admits it, and the page-mail bridge. The app
//! starts one at every graph-window binding (`load_graph_for_label`); it is
//! the only writer of a window's page edits.
use super::driver::{Deadline, Driver, Owner, Sink, State, SystemClock, Unanswered};
use super::production::ProductionIo;
use super::progress::{backoff, Clock, Notice, Progress, Stopping};
use super::*;
use crate::model::{bytes_hold_block_id, content_rev};
use crate::store::PublishedObservations;
use crate::{ChangeKind, EditKind, FileId, FileRev, Origin, PageId, Store, Why};
use std::collections::HashMap;
use std::path::Path;
use tine_core::doc::Document;
use tine_core::model::PageDto;

/// The graph binding's page host. Every command carries the window
/// generation and a strictly increasing request id (§3.1).
pub struct PageHost {
    driver: Driver<ProductionIo, SystemClock>,
    store: Arc<Store>,
    /// What a relaunch after a restore binds again (§7 step 6).
    launch: Launch,
    /// Each page launch recovered from a draft, at the version it installed
    /// (STEP3 §9): recovered while still at that version.
    recovered: BTreeMap<PageKey, u64>,
    /// Index publications to fail before the next success (tests).
    #[cfg(test)]
    index_faults: Arc<std::sync::atomic::AtomicU32>,
}

/// Why a command did not admit its request. Nothing was sent: the window
/// keeps its text and shows the reason (§3.2, B′).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
pub enum PageRefusal {
    /// Not a page path, unsafe nesting, or a virtual Guide page.
    InvalidTarget {
        /// What was refused.
        message: String,
    },
    /// The file cannot be rewritten safely (an Org file that does not
    /// round-trip, unresolved VCS markers, a read-only file).
    ReadOnly {
        /// Why.
        message: String,
    },
    /// Another file already claims the page's name (`.md` ↔ `.org`).
    Twin {
        /// The claiming file.
        existing: String,
    },
    /// The page's bytes are not decodable.
    Undecodable,
    /// An unreadable file owns the name.
    UnreadableOwner {
        /// The owning file.
        file: String,
    },
    /// A content firewall or read failed (the GH #163 page-header check,
    /// parse validation, an I/O error).
    Failed {
        /// What failed.
        message: String,
    },
    /// The host did not admit the request: admission closed (a switch), a
    /// stale session, an id out of order, or a stopped host.
    NotAdmitted,
}

/// A window session (plan v3 §5, R8): process-unique, drawn at every host
/// launch and every window reload. Commands carry it and mail names it, so
/// nothing from an earlier host or window load is ever applied: versions
/// from different sessions are never compared (E101).
static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn next_session() -> u64 {
    NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `page_window_reloaded`'s reply: the window's new session, and the first
/// request id the host can admit (ids never restart below its watermark).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reloaded {
    /// The session every later command carries.
    pub(crate) session: u64,
    /// One past the host's last admitted request id.
    pub(crate) next_id: u64,
}

/// `page_open`'s reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    /// The host's key for the page (its entry identity, A-H2).
    pub(crate) key: String,
    /// The opened path names the key's entry under the shared entry-identity
    /// rule (B1): the window's text, read from that path, is the key's. False
    /// when the path only collides with a registered entry unproved (the
    /// window then sends its text as stale input, D-b, S5).
    pub(crate) baseline_entry: bool,
}

impl From<Why> for PageRefusal {
    fn from(why: Why) -> Self {
        use crate::Refusal as R;
        match why {
            Why::Refused(R::ReadOnly(message)) => Self::ReadOnly { message },
            Why::Refused(R::Twin { existing }) => Self::Twin {
                existing: existing.as_str().into(),
            },
            Why::Refused(R::Undecodable) => Self::Undecodable,
            Why::Refused(R::UnreadableOwner { file }) => Self::UnreadableOwner {
                file: file.as_str().into(),
            },
            Why::Refused(R::InvalidTarget(message)) => Self::InvalidTarget { message },
            Why::Refused(other) => Self::InvalidTarget {
                message: format!("{other:?}"),
            },
            Why::Failed(error) => Self::Failed {
                message: error.to_string(),
            },
            Why::Conflict { file, .. } => Self::Failed {
                message: format!("{} changed on disk", file.as_str()),
            },
        }
    }
}

/// A retained writer's contract for unsaved input on the pages it touches
/// (§7 step 1). Each writer keeps today's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// Flush then write: input found under the reservation is saved first,
    /// with the reservation released meanwhile, and reserving starts over.
    /// A page whose save cannot complete (conflict, persistent save error)
    /// is returned, as today's stuck pages are.
    Flush,
    /// Refuse a touched page with unsaved input (today's message).
    Refuse,
}

/// How the host stops (§6, §7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopMode {
    /// A graph switch, quit or last-window close: a page that cannot be
    /// saved is drafted.
    Switch,
    /// A backup restore: no fallback draft; every drained edit is saved and
    /// published, and every draft and custody debt retired, first.
    Restore,
}

/// Where a stop stands (§6 steps 4–5, §7 step 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopState {
    /// Saves, drafts, cleanup or publications are still running, or no
    /// stop has begun.
    Waiting,
    /// `stop_finish` stops the host now.
    Ready,
    /// The stop cannot complete without losing custody: a draft failure
    /// (both modes), or in a restore a failed save, a conflict, a custody
    /// or index failure. These pages are affected (none: trash custody
    /// cannot be listed). The caller aborts (`stop_abort`), unfreezes and
    /// shows them; nothing closes over missing custody.
    Aborted(BTreeSet<PageKey>),
}

/// The disk state a window was shown, named by revision: a Keep-mine
/// submit's resolve token (§3.2, Q1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "rev", rename_all = "kebab-case")]
pub enum DiskToken {
    /// The page had no file.
    NoFile,
    /// The file held bytes of this revision.
    File(FileRev),
}

impl DiskToken {
    fn of(bytes: &Text) -> Self {
        match bytes {
            None => Self::NoFile,
            Some(bytes) => Self::File(FileRev::from_bytes(bytes)),
        }
    }
}

/// A page's text in a mail.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MailText {
    /// Exactly the state the window submitted (R3), or a notice-only mail.
    /// `rev` names the buffer's bytes (none for no file), so the window's
    /// baseline advances with the text the host took (S5).
    Unchanged {
        /// The buffer's revision.
        rev: Option<FileRev>,
    },
    /// The page has no file.
    NoFile,
    /// The page's bytes, parsed.
    Page {
        /// The parsed page.
        dto: Box<PageDto>,
    },
    /// Bytes the parser cannot take (they stay the host's buffer).
    Unreadable {
        /// The parse error.
        message: String,
    },
}

/// The host's state of one page, as mailed.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailPage {
    /// The host version of the buffer.
    pub version: u64,
    /// The buffer conflicts with the file on disk.
    pub conflict: bool,
    /// The buffer is not known durable on disk.
    pub risk: bool,
    /// The last observed disk state: what Keep mine resolves against.
    pub disk: Option<DiskToken>,
    /// The buffer's text.
    pub text: MailText,
}

/// A request's typed outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AnswerOutcome {
    /// The host applied it (Open and Discard apply without taking input).
    Applied,
    /// The host refused it; the window keeps its text.
    Refused {
        /// Why.
        reason: Refusal,
    },
}

/// The answer to the window's latest request on a page.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailAnswer {
    /// The request id answered.
    pub id: u64,
    /// The page version the answer leaves.
    pub version: u64,
    /// The request's input became the buffer.
    pub took: bool,
    /// Applied or refused, with the reason.
    pub outcome: AnswerOutcome,
}

/// A page's save and draft status for the window's indicators.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailNotice {
    /// Consecutive failed saves.
    pub failures: u32,
    /// The latest save failed.
    pub save_error: bool,
    /// The latest failed save's platform step, a fixed literal such as
    /// `renameat2(RENAME_NOREPLACE)` (GH #538; never a path, I-5), and its
    /// OS error code, when the filesystem named them. The window shows both
    /// in the save-failure toast (Q-P2b-3).
    pub operation: Option<&'static str>,
    pub os_error: Option<i32>,
    /// The latest draft write failed.
    pub draft_error: bool,
    /// A rename or delete reported Uncertain to its caller later ended with
    /// its draft durably absent: it did not happen (Q2).
    pub dropped: bool,
    /// The conflict has been reported to the window.
    pub conflict_reported: bool,
    /// A trash payload's custody could not be settled.
    pub custody_error: bool,
    /// The page's index publication failed a third time; it keeps retrying
    /// (§5). Search and references may lag the file meanwhile.
    pub index_error: bool,
    /// The page's file could not be read three times running after a
    /// watcher change or a retained writer's release; the driver keeps
    /// retrying, and the notice clears once a read succeeds (§5, §7).
    pub observe_error: bool,
    /// Another page file claims this page's name (the alternate-extension
    /// twin, §3.2, Q9): it failed the page's creation, or appeared beside
    /// it. Cleared once a later save publishes.
    pub twin: Option<String>,
}

/// `page-mail` (§3.3). The window checks `session` before applying anything.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMail {
    /// The window session it is for (0: none, never current).
    pub(crate) session: u64,
    /// The page key.
    pub(crate) key: String,
    /// The page's state, None once the host released it.
    pub(crate) page: Option<MailPage>,
    /// The answer to the window's latest request, if any.
    pub(crate) answer: Option<MailAnswer>,
    /// Save and draft status.
    pub(crate) notice: MailNotice,
}

/// A host page operation's disposition (delete, rename).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageOperation {
    /// Done.
    Applied,
    /// Under way; its result arrives as mail.
    Pending,
    /// The page is busy; retry once it is clean.
    Waiting,
    /// Refused (unsaved input, a stopped host; for a delete also a failed
    /// draft write, so nothing changed).
    Refused,
    /// Admitted, but neither its answer nor its publication arrived within
    /// the bound: it may still complete; Tine keeps trying.
    Uncertain,
    /// Applied, but the page was discarded or edited back before it
    /// published: a later edit decides it.
    Superseded,
}

/// Derived data a request carries (R8): its serialized bytes, the Document
/// they were serialized from (runtime block ids) and its edit kinds.
struct Handoff {
    bytes: Text,
    document: Document,
    kinds: Vec<EditKind>,
}

/// What the bridge needs beside a mail, taken under the state mutex.
pub(super) struct MailFacts {
    pub notice: Notice,
    /// False when the mail's page is exactly the state the window submitted
    /// (R3), or for a notice-only mail: the text is not sent again.
    pub content: bool,
    pub refused: Option<Refusal>,
    /// The key's current spelling, to parse its bytes as that file.
    pub spelling: String,
    /// The binding's own notices (§5, Q9).
    pub binding: BindingNotice,
    /// The window session current when the mail was collected.
    pub session: u64,
}

/// The page notices the binding keeps beside the host's (A2, REVIEW-3a).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BindingNotice {
    /// The page's index publication failed a third time (§5).
    pub index_error: bool,
    pub observe_error: bool,
    pub twin: Option<String>,
}

/// An index publication the consumer owes (§5): a held key's disk state,
/// from the host's events in host order.
#[derive(Clone)]
pub(super) struct Publication {
    pub key: PageKey,
    /// The key's spelling when the event was taken.
    pub spelling: String,
    pub bytes: Text,
    /// An own save's host version (the index watermark it reaches, §4.4)
    /// and the Document its bytes were serialized from (R8), if it matched.
    pub own: Option<(u64, Option<Document>)>,
    /// The pre-read stamp of the read these bytes came from, if the host's
    /// I/O took one (`HostIo::take_stamp`, GH #623).
    pub stamp: Option<crate::watch::Stamp>,
}

/// What became of one publication.
pub(super) enum Indexing {
    Indexed,
    Failed,
    /// A reservation holds the key: its retained transaction publishes
    /// meanwhile, and this waits for the release (Q6).
    Reserved,
}

/// A publication awaiting retry. A newer publication of its key, or the
/// observation that ends a reservation, supersedes it.
struct Retry {
    publication: Publication,
    failures: u32,
    due: u64,
}

/// One driver step's results, in host order.
#[derive(Default)]
pub(super) struct Delivery {
    /// Keys the host took on without an Open (launch-recovered drafts, an
    /// operation): their index moves to the consumer before anything is
    /// published (§5). (key, spelling, the bytes the host observed for it
    /// when it has read the file: its owner's observation, (G) as amended).
    pub claimed: Vec<(PageKey, String, Option<Arc<[u8]>>)>,
    /// Index publications, in host order (§5).
    pub publications: Vec<Publication>,
    /// Edit kinds taken since each published page's last publication.
    pub kinds: Vec<(PageKey, Vec<EditKind>)>,
    /// Keys the host evicted with no publication left: their index returns
    /// to the watcher (§5, Q6). (key, spelling).
    pub evicted: Vec<(PageKey, String)>,
    pub mail: Vec<(PageKey, Mail, MailFacts)>,
}

impl Delivery {
    pub fn is_empty(&self) -> bool {
        self.mail.is_empty()
            && self.publications.is_empty()
            && self.claimed.is_empty()
            && self.evicted.is_empty()
    }
}

/// Binding bookkeeping beside the host, under the host state mutex. None of
/// it is page authority: losing it costs a parse, never a byte.
#[derive(Default)]
pub(super) struct Book {
    /// Admitted requests' handoffs, by (page, request id), until answered.
    pending: BTreeMap<(PageKey, u64), Handoff>,
    /// Applied versions' handoffs, kept only while the page's current buffer
    /// or its save job is that version (Q8): a conflict or failing-save loop
    /// holds the live entries, never the edit history.
    versions: BTreeMap<PageKey, BTreeMap<u64, Handoff>>,
    /// Edit kinds taken since the page's last publication (OG-RULES Rule 8).
    kinds: BTreeMap<PageKey, Vec<EditKind>>,
    /// The notice each subscribed page's window last received.
    notices: BTreeMap<PageKey, (Notice, BindingNotice)>,
    /// Keys whose index the publication consumer owns (§5): from Open (or
    /// the host taking the key on) until actual eviction (Q6). Written only
    /// by `own`, `collect` and eviction, which keep `orphans` with it.
    owned: BTreeSet<PageKey>,
    /// Owned keys the host did not hold when last seen (B2): the eviction
    /// candidates. A blocked one stays until it is evicted or held again.
    orphans: BTreeSet<PageKey>,
    /// Each owned key's last indexed bytes and index watermark: the newest
    /// own save version the index has applied (§4.4).
    index: BTreeMap<PageKey, (Text, u64)>,
    retry: BTreeMap<PageKey, Retry>,
    /// Keys whose file read failed a third time, until a read succeeds.
    pub observe_errors: BTreeSet<PageKey>,
    /// Each key's last twin: (existing file, save version, after the rename).
    twins: BTreeMap<PageKey, (String, u64, bool)>,
    /// Released keys whose release observation has not yet succeeded
    /// (V2, REVIEW-3a): the retained transaction's index stands until the
    /// host has read its bytes, so older publications keep waiting.
    pub handover: BTreeSet<PageKey>,
    /// A collected delivery's publications are with the sink and their
    /// results not yet recorded (V5, REVIEW-3a).
    delivering: bool,
    /// The current window session: the host's generation under a
    /// process-unique name (E97). 0 until launch draws one.
    pub session: u64,
    /// No window owns the session any more (`orphan_stop`): the window
    /// crash ran once for that owner. A window reload clears it.
    orphaned: bool,
}

impl Book {
    /// Record `kind` for each written page until its next publication
    /// (OG-RULES Rule 8): a submit's kinds, or a host operation's own.
    fn took(&mut self, pages: impl IntoIterator<Item = PageKey>, kind: EditKind) {
        for page in pages {
            let kinds = self.kinds.entry(page).or_default();
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }

    /// The keys whose index the publication consumer owns.
    pub fn owned(&self) -> &BTreeSet<PageKey> {
        &self.owned
    }

    /// Take `keys`' index ownership ahead of the host holding them (an
    /// Open's admission, launch's recovered pages); one never held is an
    /// orphan from the start (B2: a failed Open still ends in eviction).
    pub fn own<F: HostIo>(&mut self, keys: impl IntoIterator<Item = PageKey>, host: &Host<F>) {
        for key in keys {
            if !host.pages.contains_key(&key) {
                self.orphans.insert(key.clone());
            }
            self.owned.insert(key);
        }
    }

    /// Take this step's results: its events (drained once progress has read
    /// them) routed to answers and publications, and every mail.
    pub fn collect<F: HostIo, C: Clock>(&mut self, progress: &mut Progress<F, C>) -> Delivery {
        progress.host.held = Some(BTreeSet::new());
        let now = progress.clock.now_ms();
        let events = progress.take_events();
        let mut delivery = Delivery::default();
        // Ownership follows the keys whose holding changed (M4, B2): a newly
        // held page, or the save job's page, is claimed; an owned key the
        // host no longer holds becomes an orphan.
        let moved = progress.take_moved();
        let host = &progress.host;
        for key in &moved {
            work(Work::Book, 1);
            if host.pages.contains_key(key) {
                self.orphans.remove(key);
            } else if self.owned.contains(key) {
                self.orphans.insert(key.clone());
            }
        }
        let held = moved.iter().filter(|key| host.pages.contains_key(*key));
        let taken = held
            .chain(host.job.as_ref().map(|job| &job.page))
            .inspect(|_| work(Work::Book, 1))
            .filter(|key| !self.owned.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in taken {
            if !host.pages.contains_key(&key) {
                self.orphans.insert(key.clone());
            }
            self.owned.insert(key.clone());
            let observed = (host.pages.get(&key))
                .and_then(|page| page.obs.clone())
                .flatten();
            delivery
                .claimed
                .push((key.clone(), host.fs.spelling(&key), observed));
        }
        let mut answered = BTreeMap::new();
        let mut refused = BTreeMap::new();
        for event in &events {
            match event {
                Event::Answer { page, answer } => {
                    let Some(handoff) = self.pending.remove(&(page.clone(), answer.id)) else {
                        continue;
                    };
                    if answer.took {
                        answered.insert((page.clone(), answer.id), handoff.bytes.clone());
                        for kind in &handoff.kinds {
                            self.took([page.clone()], *kind);
                        }
                        self.versions
                            .entry(page.clone())
                            .or_default()
                            .insert(answer.version, handoff);
                    }
                }
                Event::Published {
                    page,
                    bytes,
                    version,
                    ..
                } => {
                    if let Some(kinds) = self.kinds.remove(page) {
                        delivery.kinds.push((page.clone(), kinds));
                    }
                    // A twin that failed a save clears once one publishes; a
                    // twin found after a rename stays past that save's own
                    // publication (Q9: it neither invents nor cancels it).
                    if self.twins.get(page).is_some_and(|(_, at, saved)| {
                        *version > *at || (!*saved && *version == *at)
                    }) {
                        self.twins.remove(page);
                    }
                    let handoff = self.versions.get_mut(page).and_then(|v| v.remove(version));
                    let document = handoff.filter(|h| h.bytes == *bytes).map(|h| h.document);
                    self.retry.remove(page);
                    delivery.publications.push(Publication {
                        key: page.clone(),
                        spelling: progress.host.fs.spelling(page),
                        bytes: bytes.clone(),
                        own: Some((*version, document)),
                        stamp: progress.host.fs.take_stamp(page),
                    });
                }
                Event::Observed { page, bytes } => {
                    self.retry.remove(page);
                    delivery.publications.push(Publication {
                        key: page.clone(),
                        spelling: progress.host.fs.spelling(page),
                        bytes: bytes.clone(),
                        own: None,
                        stamp: progress.host.fs.take_stamp(page),
                    });
                }
                Event::Refused { page, id, reason } => {
                    refused.insert((page.clone(), *id), *reason);
                }
                Event::Twin {
                    page,
                    existing,
                    version,
                    saved,
                } => {
                    self.twins
                        .insert(page.clone(), (existing.clone(), *version, *saved));
                }
                Event::ObserveError(page) => {
                    self.observe_errors.insert(page.clone());
                }
                _ => {}
            }
        }
        // Due retries go first: each is older than any event of its key,
        // and an event of its key has superseded it above.
        let host = &progress.host;
        let due = self
            .retry
            .iter()
            .inspect(|_| work(Work::Book, 1))
            .filter(|(key, retry)| retry.due <= now && !self.reserved(key, host))
            .map(|(_, retry)| retry.publication.clone());
        delivery.publications.splice(0..0, due.collect::<Vec<_>>());
        for key in self.evictable(&progress.host, &delivery.publications) {
            self.owned.remove(&key);
            self.orphans.remove(&key);
            self.index.remove(&key);
            self.twins.remove(&key);
            self.observe_errors.remove(&key);
            delivery
                .evicted
                .push((key.clone(), progress.host.fs.spelling(&key)));
        }
        let host = &mut progress.host;
        // A request answered for an earlier window generation has no answer
        // event; once applied, nothing names its handoff.
        let applied = host.last_applied;
        self.pending.retain(|(_, id), _| {
            work(Work::Book, 1);
            *id > applied
        });
        let pages = &host.pages;
        let job = host.job.as_ref().map(|job| (&job.page, job.version));
        self.versions.retain(|page, versions| {
            versions.retain(|version, _| {
                work(Work::Book, 1);
                pages.get(page).is_some_and(|p| p.version == *version)
                    || job == Some((page, *version))
            });
            !versions.is_empty()
        });
        let keys: Vec<_> = host.outbox.keys().cloned().collect();
        work(Work::Book, keys.len());
        for page in keys {
            let Some(mail) = host.receive(&page) else {
                continue;
            };
            let exact = match (&mail.page, &mail.answer) {
                (Some(state), Some(answer)) => {
                    answer.took
                        && state.version == answer.version
                        && answered.get(&(page.clone(), answer.id)) == Some(&state.buf)
                }
                _ => false,
            };
            let refused = mail
                .answer
                .as_ref()
                .and_then(|answer| refused.get(&(page.clone(), answer.id)).copied());
            delivery.mail.push((
                page,
                mail,
                MailFacts {
                    notice: Notice::default(),
                    content: !exact,
                    refused,
                    spelling: String::new(),
                    binding: BindingNotice::default(),
                    session: 0,
                },
            ));
        }
        // A notice that changed without a page change still reaches the
        // window (§3.3); only subscribed pages are walked.
        let mailed: BTreeSet<_> = delivery
            .mail
            .iter()
            .map(|(page, ..)| page.clone())
            .collect();
        let generation = host.generation;
        let quiet: Vec<_> = host
            .subscriptions
            .iter()
            .inspect(|_| work(Work::Book, 1))
            .filter(|page| !mailed.contains(*page) && host.pages.contains_key(*page))
            .cloned()
            .collect();
        for page in quiet {
            let notice = (progress.notice(&page), self.binding_notice(&page));
            if self.notices.get(&page) != Some(&notice) {
                delivery.mail.push((
                    page.clone(),
                    Mail {
                        page: progress.host.pages.get(&page).cloned(),
                        answer: None,
                        generation,
                    },
                    MailFacts {
                        notice: Notice::default(),
                        content: false,
                        refused: None,
                        spelling: String::new(),
                        binding: BindingNotice::default(),
                        session: 0,
                    },
                ));
            }
        }
        // `window_crash` clears the outbox under this lock, so all mail here
        // is the current window's; mail a reload overtakes in flight keeps
        // its old session, and the window drops it.
        for (page, _, facts) in &mut delivery.mail {
            facts.session = self.session;
            facts.notice = progress.notice(page);
            facts.spelling = progress.host.fs.spelling(page);
            facts.binding = self.binding_notice(page);
            self.notices
                .insert(page.clone(), (facts.notice.clone(), facts.binding.clone()));
        }
        let subscribed = &progress.host.subscriptions;
        self.notices.retain(|page, _| {
            work(Work::Book, 1);
            subscribed.contains(page)
        });
        self.delivering |= !delivery.publications.is_empty();
        delivery
    }

    /// Owned keys the host no longer holds, with nothing left to publish:
    /// no page, job, reservation or unobserved release, queued or applying
    /// request (an Open waiting for its lock: its admission owned the key),
    /// vehicle debt (R2), retry or `pending` publication (Q6: a dirty page closed by the window stays held until
    /// its save and cleanup finish).
    fn evictable<F: HostIo>(&self, host: &Host<F>, pending: &[Publication]) -> Vec<PageKey> {
        self.orphans
            .iter()
            .filter(|key| {
                work(Work::Book, 2 + host.queue.len() + pending.len());
                !host.pages.contains_key(*key)
                    && !host.busy(key)
                    && !host.vehicle_debt.contains(*key)
                    && !(host.applying.iter().chain(&host.queue))
                        .any(|request| &request.page == *key)
                    && !self.retry.contains_key(*key)
                    && !self.handover.contains(*key)
                    && !pending.iter().any(|p| &p.key == *key)
            })
            .cloned()
            .collect()
    }

    /// Record a delivery's index results in order (§5): success advances
    /// the key's watermark; a failure retries with the save backoff; a
    /// reserved key waits for its release.
    pub fn record(&mut self, results: Vec<(Publication, Indexing)>, now: u64) {
        self.delivering = false;
        for (publication, indexing) in results {
            let key = publication.key.clone();
            let failures = self.retry.get(&key).map_or(0, |retry| retry.failures);
            match indexing {
                Indexing::Indexed => {
                    self.retry.remove(&key);
                    let watermark = self.index.get(&key).map_or(0, |(_, v)| *v);
                    let own = publication.own.as_ref().map_or(0, |(v, _)| *v);
                    self.index
                        .insert(key, (publication.bytes, watermark.max(own)));
                }
                Indexing::Failed => {
                    let failures = failures.saturating_add(1);
                    let due = now.saturating_add(backoff(failures));
                    self.retry.insert(
                        key,
                        Retry {
                            publication,
                            failures,
                            due,
                        },
                    );
                }
                Indexing::Reserved => {
                    let retry = Retry {
                        publication,
                        failures,
                        due: now,
                    };
                    self.retry.insert(key, retry);
                }
            }
        }
    }

    /// The earliest retry due, for the driver's sleep (§5).
    pub fn next_retry<F: HostIo>(&self, host: &Host<F>) -> Option<u64> {
        self.retry
            .iter()
            .filter(|(key, _)| !self.reserved(key, host))
            .map(|(_, retry)| retry.due)
            .min()
    }

    /// A retained writer publishes `key`'s index: its reservation holds,
    /// or its release has not yet been observed (Q6, V2).
    fn reserved<F: HostIo>(&self, key: &str, host: &Host<F>) -> bool {
        host.retained.contains(key) || self.handover.contains(key)
    }

    /// Who publishes `key`'s index now (§5, Q6).
    pub fn owner<F: HostIo>(&self, key: &str, host: &Host<F>) -> Owner {
        if self.reserved(key, host) {
            Owner::Reservation
        } else if self.owned.contains(key) {
            Owner::Consumer
        } else {
            Owner::Watcher
        }
    }

    fn binding_notice(&self, key: &str) -> BindingNotice {
        BindingNotice {
            index_error: self.index_error(key),
            observe_error: self.observe_errors.contains(key),
            twin: self.twins.get(key).map(|(existing, ..)| existing.clone()),
        }
    }

    fn index_error(&self, key: &str) -> bool {
        self.retry.get(key).is_some_and(|retry| retry.failures >= 3)
    }
}

/// The window's page-mail sink, kept across a relaunch.
type MailSink = Arc<Mutex<Box<dyn FnMut(PageMail) + Send>>>;
/// Where each published page's edit kinds go (OG-RULES Rule 8), kept across
/// a relaunch.
type KindsSink = Arc<Mutex<Box<dyn FnMut(&str, &[EditKind]) + Send>>>;

/// A binding's launch parameters (`PageHost::start`).
#[derive(Clone)]
struct Launch {
    store: Arc<Store>,
    app_data: std::path::PathBuf,
    graph_id: String,
    mail: MailSink,
    kinds: KindsSink,
}

/// A host stopped for a restore (§7 step 4): the binding holds no host,
/// and only `relaunch` binds one again, on the tree as it then is.
#[must_use = "a stopped binding is relaunched after the restore"]
pub struct Stopped {
    launch: Launch,
}

impl Stopped {
    /// Launch one fresh host for the same binding and page-mail sink (§7
    /// step 6), on success and on a partial restore failure alike. The
    /// error says why no host could start, as `PageHost::start`'s does.
    pub fn relaunch(self) -> Result<PageHost, String> {
        PageHost::launch(self.launch)
    }
}

/// The version a request carries when it was typed on no version the host
/// holds: host versions start at 1, so it is always stale.
const STALE: u64 = 0;

/// A request to a driver that ended: nothing answers (a host that panicked).
const ENDED: &str = "page host driver ended";

/// Admit `request` under `session`'s window generation; on admission
/// its handoffs wait for its answer, and an Open's key is the
/// consumer's (§5) in the same step, so no collection can evict it in
/// between. A session that is not current is not admitted.
fn admit(
    state: &mut State<ProductionIo, SystemClock>,
    session: u64,
    mut request: Request,
    handoffs: Vec<(PageKey, Handoff)>,
) -> Result<(), PageRefusal> {
    if session != state.book.session {
        return Err(PageRefusal::NotAdmitted);
    }
    request.generation = state.progress.host.generation;
    let id = request.id;
    let open = (request.kind == RequestKind::Open).then(|| request.page.clone());
    match state.progress.with_host(|host| host.admit(request)) {
        Disposition::Applied => {
            for (page, handoff) in handoffs {
                state.book.pending.insert((page, id), handoff);
            }
            state.book.own(open, &state.progress.host);
            Ok(())
        }
        _ => Err(PageRefusal::NotAdmitted),
    }
}

impl PageHost {
    /// Plan, lock, revalidate on the driver (§1), waiting for its answer
    /// as long as it runs. None once it is stopping or ended.
    fn locked<R: Send + 'static>(
        &self,
        mut step: impl FnMut(&mut Host<ProductionIo>) -> R + Send + 'static,
    ) -> Option<R> {
        let shared = &self.driver.shared;
        let step =
            move |state: &mut State<ProductionIo, SystemClock>| state.progress.with_host(&mut step);
        shared.locked_step(Deadline::Never, step).ok()
    }

    /// The session commands must carry now (tests; the window learns it
    /// from `window_reloaded`).
    #[cfg(test)]
    pub(crate) fn session(&self) -> u64 {
        let shared = &self.driver.shared;
        shared.read(|state| state.book.session).expect(ENDED)
    }

    /// A window (re)load (`window_crash`): unsent input is lost, admitted
    /// requests keep their custody, and the window gets a fresh session and
    /// the first request id the host can admit (R8).
    pub fn window_reloaded(&self) -> Reloaded {
        let reloaded = self.driver.shared.call(|state| {
            let next_id = state.progress.with_host(|host| {
                host.window_crash();
                host.last_admitted.saturating_add(1)
            });
            state.book.session = next_session();
            state.book.orphaned = false;
            Reloaded {
                session: state.book.session,
                next_id,
            }
        });
        reloaded.expect(ENDED)
    }

    /// [`admit`] on the driver; a host that ended admits nothing.
    fn admit(
        &self,
        session: u64,
        request: Request,
        handoffs: Vec<(PageKey, Handoff)>,
    ) -> Result<(), PageRefusal> {
        let admit = move |state: &mut State<ProductionIo, SystemClock>| {
            admit(state, session, request, handoffs)
        };
        let admitted = self.driver.shared.admission(admit);
        admitted.unwrap_or(Err(PageRefusal::NotAdmitted))
    }

    /// `page_open` (§2, F4): every editable page, including one with no file
    /// yet, is opened first. With no file the create-only checks run here,
    /// never on its keystrokes. Returns the page's key, and whether `page`
    /// names that key's entry (the window's baseline, S5).
    /// The page's index moves to the publication consumer here (§5), under
    /// the writer: a watcher reconcile already running for it finishes
    /// first, and the consumer publishes the Open read. An unchanged file
    /// keeps its row through the hold, so an open publishes nothing
    /// (PERF-FIXES D4).
    pub fn open(
        &self,
        session: u64,
        id: u64,
        page: &PageId,
        name: &str,
    ) -> Result<Opened, PageRefusal> {
        // A host that ended admits nothing.
        let Some((key, spelling, held, baseline_entry)) = self.try_identify(page) else {
            return Err(PageRefusal::NotAdmitted);
        };
        if !held {
            self.store
                .transaction(None)
                .page_open_checks(&spelling.file(), name)?;
        }
        self.try_register(&key, &spelling)
            .ok_or(PageRefusal::NotAdmitted)?;
        let request = Request {
            id,
            generation: 0,
            page: key.clone(),
            kind: RequestKind::Open,
        };
        // Under the writer, which the driver takes first (lock order).
        let (held, store) = (key.clone(), self.store.clone());
        let admitted = self
            .driver
            .shared
            .admission_writer(&self.store.writer, move |state| {
                store.watch.hold_current(held.clone());
                let admitted = admit(state, session, request, vec![]);
                if admitted.is_err() && !state.book.owned().contains(&held) {
                    store.watch.release_hold(&held);
                }
                store.publish_retired();
                admitted
            });
        let admitted = admitted.unwrap_or(Err(PageRefusal::NotAdmitted));
        admitted.map(|()| Opened {
            key,
            baseline_entry,
        })
    }

    /// The comparison source for a page DTO typed on `version` (§3.2, Q1),
    /// the resolve base, and the version the request carries. An ordinary
    /// submit on the current version compares with the host buffer at that
    /// version. A resolve compares with exactly the bytes its token names,
    /// which become its base: one snapshot, so the bytes checked are the
    /// bytes the resolve installs and replaces, whatever is observed later.
    /// A stale submit has no trustworthy source. So has a resolve whose token
    /// no longer names the observed disk state: the host no longer holds the
    /// bytes the user chose to overwrite, so it is admitted as stale input
    /// (`STALE`; the model's `submitTo` makes it a conflict with an unknown
    /// base) and the window is shown the newer disk to resolve again.
    fn source(
        &self,
        key: &str,
        version: u64,
        resolve: Option<&DiskToken>,
    ) -> (String, Option<Text>, Option<Text>, u64) {
        let (key, resolve) = (key.to_owned(), resolve.cloned());
        let unknown = (key.clone(), None, None, STALE);
        let read = move |state: &State<ProductionIo, SystemClock>| {
            let host = &state.progress.host;
            let spelling = if host.keys.contains(&key) {
                host.fs.spelling(&key)
            } else {
                key.clone()
            };
            match (resolve.as_ref(), host.pages.get(&key)) {
                (Some(token), Some(page)) => match &page.obs {
                    Some(obs) if DiskToken::of(obs) == *token => {
                        (spelling, Some(obs.clone()), Some(obs.clone()), version)
                    }
                    _ => (spelling, None, None, STALE),
                },
                (Some(_), None) => (spelling, None, None, STALE),
                (None, Some(page)) if page.version == version => {
                    (spelling, Some(page.buf.clone()), None, version)
                }
                (None, _) => (spelling, None, None, version),
            }
        };
        // A host that ended admits nothing: the request is stale.
        self.driver.shared.read(read).unwrap_or(unknown)
    }

    /// The per-submit checks and serialization (§3.2), before admission.
    fn serialize(
        &self,
        spelling: &str,
        dto: &PageDto,
        source: Option<&Text>,
        kinds: &[EditKind],
    ) -> Result<(Text, Handoff), PageRefusal> {
        let file = PageId::from(spelling).file();
        let tx = self.store.transaction(None);
        tx.page_save_target(&file, dto)?;
        let old = source.and_then(|text| text.as_deref());
        let (bytes, document) = tx.serialize_page(&file, dto, old, false)?;
        let bytes: Text = Some(Arc::from(bytes));
        let handoff = Handoff {
            bytes: bytes.clone(),
            document,
            kinds: kinds.to_vec(),
        };
        Ok((bytes, handoff))
    }

    /// `page_submit` (§3.1): serialize against the request's comparison
    /// source, then admit. A refusal here was never sent.
    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &self,
        session: u64,
        id: u64,
        key: &str,
        dto: &PageDto,
        version: u64,
        resolve: Option<&DiskToken>,
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (spelling, source, base, version) = self.source(key, version, resolve);
        let (bytes, handoff) = self.serialize(&spelling, dto, source.as_ref(), kinds)?;
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Submit {
                bytes,
                version,
                resolve: base,
            },
        };
        self.admit(session, request, vec![(key.into(), handoff)])
    }

    /// `page_move` (§3.1, §8): both endpoints serialized as ordinary submits
    /// on their versions, admitted as the model's two-page move.
    #[allow(clippy::too_many_arguments)]
    pub fn move_blocks(
        &self,
        session: u64,
        id: u64,
        source: (&str, &PageDto, u64),
        receiver: (&str, &PageDto, u64),
        kinds: &[EditKind],
    ) -> Result<(), PageRefusal> {
        let (source_key, source_dto, source_version) = source;
        let (receiver_key, receiver_dto, receiver_version) = receiver;
        let (spelling, old, ..) = self.source(source_key, source_version, None);
        let (source_text, source_handoff) =
            self.serialize(&spelling, source_dto, old.as_ref(), kinds)?;
        let (spelling, old, ..) = self.source(receiver_key, receiver_version, None);
        let (receiver_text, receiver_handoff) =
            self.serialize(&spelling, receiver_dto, old.as_ref(), kinds)?;
        let request = Request {
            id,
            generation: 0,
            page: source_key.into(),
            kind: RequestKind::Move {
                receiver: receiver_key.into(),
                source_text,
                receiver_text,
                source_version,
                receiver_version,
            },
        };
        self.admit(
            session,
            request,
            vec![
                (source_key.into(), source_handoff),
                (receiver_key.into(), receiver_handoff),
            ],
        )
    }

    /// `page_discard` (§3.1): the window consumed its unsent input first.
    pub fn discard(&self, session: u64, id: u64, key: &str) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Discard,
        };
        self.admit(session, request, vec![])
    }

    /// `page_close` (§3.1): the last surface showing the page released it.
    pub fn close(&self, session: u64, id: u64, key: &str) -> Result<(), PageRefusal> {
        let request = Request {
            id,
            generation: 0,
            page: key.into(),
            kind: RequestKind::Close,
        };
        self.admit(session, request, vec![])
    }

    /// `page_delete` (§7): the host's delete operation, for the current
    /// window `session` only (S2). D4 refuses a page with unsaved input;
    /// Waiting means the page is busy (retry when clean). `checked` is the
    /// file's bytes the caller's identity check read (GH #620,
    /// `pages::delete_page_expected`); the operation refuses when its own
    /// read differs (Q-P2b-5). An admitted deletion is waited for, at most
    /// `OPERATION_WAIT` (Finding B): `Applied` once its own save
    /// published it; `Refused` when its draft failed (nothing changed);
    /// `Pending` when the page cannot publish without the user (a conflict
    /// or a persistent save error; it completes once it can); `Superseded`
    /// or `Uncertain` otherwise.
    pub fn delete(&self, session: u64, page: &PageId, checked: &[u8]) -> PageOperation {
        // A host that ended deletes nothing.
        let Some((key, spelling, ..)) = self.try_identify(page) else {
            return PageOperation::Refused;
        };
        // `open`'s invalid-target refusal (docs/storage-contract.md
        // `page_host::delete`): a path that is not a page file of this graph
        // (outside the root, not a graph text file) deletes nothing. In-scope
        // scenario: web content, a plugin or a frontend defect passing a path.
        if self.store.as_page(&spelling.file()).is_none() {
            return PageOperation::Refused;
        }
        if self.try_register(&key, &spelling).is_none() {
            return PageOperation::Refused;
        }
        let deadline = std::time::Instant::now() + retained::OPERATION_WAIT;
        let (shared, checked) = (Arc::downgrade(&self.driver.shared), checked.to_vec());
        let delete = move |state: &mut State<ProductionIo, SystemClock>| {
            if state.book.session != session {
                return (Disposition::Refused, None);
            }
            let (disposition, reply) = state.progress.with_host(|host| {
                let disposition = host.delete_checked(&key, &checked);
                let pending = disposition == Disposition::Pending;
                (
                    disposition,
                    pending.then(|| host.register_reply()).flatten(),
                )
            });
            // OG-RULES Rule 8 (A-K1): the operation's write is a deletion.
            if disposition == Disposition::Pending {
                state.book.took([key.clone()], EditKind::DeletePage);
            }
            let slot = reply.map(|id| retained::ReplySlot::new(shared.clone(), id));
            (disposition, slot)
        };
        let deleted = (self.driver.shared).locked_step(Deadline::At(deadline), delete);
        match deleted {
            Ok((Disposition::Pending, Some(slot))) => {
                match self.settle(&slot, Some(session), deadline) {
                    retained::Settled::Applied => PageOperation::Applied,
                    retained::Settled::DraftFailed => PageOperation::Refused,
                    retained::Settled::Unwritten(_) => PageOperation::Pending,
                    retained::Settled::Superseded(_) => PageOperation::Superseded,
                    retained::Settled::Uncertain => PageOperation::Uncertain,
                }
            }
            // Not admitted in time: the page stayed busy (a path lock held).
            Ok((Disposition::Waiting, _)) | Err(Unanswered::Withdrawn) => PageOperation::Waiting,
            Err(Unanswered::Late) => PageOperation::Uncertain,
            _ => PageOperation::Refused,
        }
    }
}

pub(super) fn stop_state<F: HostIo, C: Clock>(progress: &Progress<F, C>, book: &Book) -> StopState {
    let host = &progress.host;
    let Some(stopping) = &progress.stopping else {
        return StopState::Waiting;
    };
    let mut affected = progress.draft_error_pages();
    // R2: known vehicles draft I/O cannot clear while it is down.
    affected.extend(host.vehicle_debt.iter().cloned());
    if stopping.restore {
        // A restore aborts where its flush would fail today (§7 step 3).
        affected.extend(stopping.failed.iter().cloned());
        let keys = host.pages.keys().chain(host.custody.keys());
        affected.extend(
            keys.filter(|key| {
                host.pages.get(*key).is_some_and(|page| page.conflict)
                    || !host.custody_errors(key).is_empty()
                    || book.index_error(key)
            })
            .cloned(),
        );
        if affected.is_empty() && host.custody_unknown.is_some() {
            return StopState::Aborted(affected);
        }
    }
    if !affected.is_empty() {
        return StopState::Aborted(affected);
    }
    // Readiness waits for every save the stop owes first, whatever the
    // caller polls (V1): a page's exact draft from an earlier failure is a
    // switch's fallback only once this stop's save of it failed.
    if progress.owes_save_first() {
        return StopState::Waiting;
    }
    // A restore closes only over completed work (§7 step 3, V1, V5): every
    // page saved, no recoverable draft, no custody debt, and every
    // publication delivered and indexed. A clean page needs no save (Q5).
    let complete = !stopping.restore
        || (host.pages.values().all(|page| page.clean())
            && host.logical_drafts().is_empty()
            && host.custody.is_empty()
            && host.retire.is_empty()
            && book.retry.is_empty()
            && !book.delivering);
    if host.can_switch() && complete {
        StopState::Ready
    } else {
        StopState::Waiting
    }
}

impl Drop for PageHost {
    /// Stop and join the driver (its last step's results have been
    /// delivered), then hand every held page's index back to the watcher.
    fn drop(&mut self) {
        self.driver.join();
        let _writer = self.store.writer.lock().unwrap();
        self.store.watch.release_holds();
    }
}

#[path = "binding_bridge.rs"]
mod bridge;
#[cfg(test)]
use bridge::index;
use bridge::Bridge;

#[path = "binding_retained.rs"]
mod retained;

#[path = "binding_publication.rs"]
mod publication;

#[path = "binding_launch.rs"]
mod launch;

#[path = "binding_read.rs"]
mod read;
pub use read::{HostRead, HostReadError};
#[path = "binding_apply.rs"]
mod apply;
#[cfg(test)]
pub(super) use apply::creation_refusal;
pub use apply::{ApplyRefusal, Effect, Expected, HostOp};
#[cfg(test)]
pub(super) use retained::unsaved;
#[cfg(feature = "test-faults")]
pub use retained::OPERATION_WAIT;
pub use retained::{RenameRefusal, Reservation, ReserveRefusal};
#[cfg(test)]
#[path = "binding_tests.rs"]
mod tests;
